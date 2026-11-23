using System.Collections.Concurrent;
using System.IO.Pipes;
using System.Security.Principal;
using System.Text.Json;
using FortiVPNManager.Contract;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>
/// A connection to the Helper service's named pipe. Opening checks that the system account owns the
/// pipe, so another program of the user that created a pipe of that name first never gets the
/// password. One thread reads: answers complete the requests they are numbered for, and a run's
/// events and output go to the inbound in arrival order. Requests are written one at a time. The
/// pipe is opened for overlapped I/O: on a pipe opened otherwise, Windows holds a write back while
/// a read waits, so the first request would wait for an answer that needs it.
/// </summary>
internal sealed class PipeHelperChannel : IHelperChannel
{
    /// <summary>Answers normally arrive within milliseconds; this only guards against a hung Helper.</summary>
    private static readonly TimeSpan s_replyLimit = TimeSpan.FromSeconds(20);

    private readonly NamedPipeClientStream _pipe;
    private readonly ulong _id;
    private readonly HelperInbound _inbound;
    private readonly Lock _writeGate = new();
    private readonly ConcurrentDictionary<ulong, TaskCompletionSource<HelperPipe.Message>> _pending = new();
    private ulong _lastRequest;
    private int _closed;

    private PipeHelperChannel(NamedPipeClientStream pipe, ulong id, HelperInbound inbound)
    {
        this._pipe = pipe;
        this._id = id;
        this._inbound = inbound;
    }

    public static IHelperChannel Open(ulong id, HelperInbound inbound, TimeSpan limit)
    {
        var pipe = new NamedPipeClientStream(".", HelperPipe.Name, PipeDirection.InOut, PipeOptions.Asynchronous);
        try
        {
            pipe.Connect(limit);
            if (pipe.GetAccessControl().GetOwner(typeof(SecurityIdentifier)) is not SecurityIdentifier owner || !owner.IsWellKnown(WellKnownSidType.LocalSystemSid))
            {
                throw new HelperChannelException(new HelperChannelFailure.Connection(
                    "the Helper's pipe is not owned by the system account"));
            }
        }
        catch (Exception error) when (error is TimeoutException or IOException or UnauthorizedAccessException)
        {
            pipe.Dispose();
            throw new HelperChannelException(new HelperChannelFailure.Connection(error.Message));
        }
        catch
        {
            pipe.Dispose();
            throw;
        }
        return new PipeHelperChannel(pipe, id, inbound);
    }

    public void Start() =>
        new Thread(Read) { Name = $"fvm-helper-read-{_id}", IsBackground = true }.Start();

    /// <summary>The time limit covers sending too: a Helper that stops reading holds the write.</summary>
    public async Task<HelperPipe.Message> Ask(Func<ulong, HelperPipe.Request> request)
    {
        ulong id = Interlocked.Increment(ref _lastRequest);
        HelperPipe.Request message = request(id);
        var limit = Task.Delay(s_replyLimit);
        var answer = new TaskCompletionSource<HelperPipe.Message>(TaskCreationOptions.RunContinuationsAsynchronously);
        _pending[id] = answer;
        // Closing fails what is pending; a request added after it would wait for nothing.
        if (Volatile.Read(ref _closed) == 1)
        {
            _ = _pending.TryRemove(id, out _);
            throw new HelperChannelException(new HelperChannelFailure.ConnectionLost());
        }
        var sending = Task.Run(() =>
        {
            lock (_writeGate)
            {
                MessageFraming.Write(_pipe, message);
            }
        });
        if (await Task.WhenAny(sending, limit).ConfigureAwait(false) == limit)
        {
            // A request left sending could reach the Helper after the runtime gave up on it, such as a
            // connect that starts a run nobody waits for. Closing cancels it. One that got through just
            // then is undone by the close, as the Helper ends the run of a connection that goes away,
            // or, for a route write, found stale when it is tried again on a fresh connection.
            Close();
            throw new HelperChannelException(new HelperChannelFailure.Connection(
                "the Helper did not take the request in time"));
        }
        try
        {
            await sending.ConfigureAwait(false);
        }
        // Closing the connection cancels a write still under way.
        catch (Exception error) when (error is IOException or ObjectDisposedException or OperationCanceledException)
        {
            _ = _pending.TryRemove(id, out _);
            throw new HelperChannelException(new HelperChannelFailure.Connection(error.Message));
        }
        if (await Task.WhenAny(answer.Task, limit).ConfigureAwait(false) == limit)
        {
            _ = _pending.TryRemove(id, out _);
            throw new HelperChannelException(new HelperChannelFailure.NoReply());
        }
        return await answer.Task.ConfigureAwait(false);
    }

    public void Dispose() => Close();

    private void Read()
    {
        try
        {
            while (true)
            {
                switch (MessageFraming.ReadFromHelper(_pipe))
                {
                    case HelperPipe.Message.Event message:
                        _inbound.Pass(new SystemEvent.HelperEvent(message.Json), _id);
                        break;
                    case HelperPipe.Message.Output message:
                        _inbound.Pass(new SystemEvent.HelperOutput(message.Json), _id);
                        break;
                    case var message:
                        Answer(message);
                        break;
                }
            }
        }
        catch (Exception error)
            when (error is IOException or ObjectDisposedException or OperationCanceledException or JsonException)
        {
            // The end of the pipe, the connection closed here, which cancels the read under way, or a
            // message this build cannot read: the connection is done.
        }
        Close();
        _inbound.End(_id);
    }

    private void Answer(HelperPipe.Message message)
    {
        ulong id = message switch
        {
            HelperPipe.Message.Build build => build.Id,
            HelperPipe.Message.Reply reply => reply.Id,
            HelperPipe.Message.ReplyFlag flag => flag.Id,
            _ => throw new InvalidOperationException($"no request is answered by {message.GetType().Name}"),
        };
        // An answer after the time limit finds no request.
        if (_pending.TryRemove(id, out TaskCompletionSource<HelperPipe.Message>? answer))
        {
            _ = answer.TrySetResult(message);
        }
    }

    private void Close()
    {
        if (Interlocked.Exchange(ref _closed, 1) == 1)
        {
            return;
        }
        _pipe.Dispose();
        foreach (ulong id in _pending.Keys)
        {
            if (_pending.TryRemove(id, out TaskCompletionSource<HelperPipe.Message>? answer))
            {
                _ = answer.TrySetException(new HelperChannelException(new HelperChannelFailure.ConnectionLost()));
            }
        }
    }
}
