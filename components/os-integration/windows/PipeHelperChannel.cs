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
    /// <summary>The service keeps the pipe ready; a missing one means it is not running.</summary>
    private static readonly TimeSpan ConnectLimit = TimeSpan.FromSeconds(2);

    /// <summary>Answers normally arrive within milliseconds; this only guards against a hung Helper.</summary>
    private static readonly TimeSpan ReplyLimit = TimeSpan.FromSeconds(20);

    /// <summary>
    /// A route call takes the Helper's PowerShell up to 20 s, and a write 0.2 s more for each route it
    /// changes: 60 s for the 200 the runtime changes at most, the bypass routes it wrote and the ones
    /// it writes. It may also wait behind the route call of the connection before. Giving up on a call
    /// the Helper still runs would get the next route call refused.
    /// </summary>
    private static readonly TimeSpan RouteLimit = TimeSpan.FromSeconds(90);

    private readonly NamedPipeClientStream pipe;
    private readonly ulong id;
    private readonly HelperInbound inbound;
    private readonly object writeGate = new();
    private readonly ConcurrentDictionary<ulong, TaskCompletionSource<HelperPipe.Message>> pending = new();
    private ulong lastRequest;
    private int closed;

    private PipeHelperChannel(NamedPipeClientStream pipe, ulong id, HelperInbound inbound)
    {
        this.pipe = pipe;
        this.id = id;
        this.inbound = inbound;
    }

    public static IHelperChannel Open(ulong id, HelperInbound inbound)
    {
        var pipe = new NamedPipeClientStream(".", HelperPipe.Name, PipeDirection.InOut, PipeOptions.Asynchronous);
        try
        {
            pipe.Connect(ConnectLimit);
            var owner = pipe.GetAccessControl().GetOwner(typeof(SecurityIdentifier)) as SecurityIdentifier;
            if (owner is null || !owner.IsWellKnown(WellKnownSidType.LocalSystemSid))
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
        new Thread(Read) { Name = $"fvm-helper-read-{id}", IsBackground = true }.Start();

    /// <summary>The time limit covers sending too: a Helper that stops reading holds the write.</summary>
    public async Task<HelperPipe.Message> Ask(Func<ulong, HelperPipe.Request> request)
    {
        var id = Interlocked.Increment(ref lastRequest);
        var message = request(id);
        var routeCall = message is HelperPipe.Request.ReadBypassRoutes or HelperPipe.Request.WriteBypassRoutes;
        var limit = Task.Delay(routeCall ? RouteLimit : ReplyLimit);
        var answer = new TaskCompletionSource<HelperPipe.Message>(TaskCreationOptions.RunContinuationsAsynchronously);
        pending[id] = answer;
        // Closing fails what is pending; a request added after it would wait for nothing.
        if (Volatile.Read(ref closed) == 1)
        {
            pending.TryRemove(id, out _);
            throw new HelperChannelException(new HelperChannelFailure.ConnectionLost());
        }
        var sending = Task.Run(() =>
        {
            lock (writeGate)
            {
                MessageFraming.Write(pipe, message);
            }
        });
        if (await Task.WhenAny(sending, limit) == limit)
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
            await sending;
        }
        // Closing the connection cancels a write still under way.
        catch (Exception error) when (error is IOException or ObjectDisposedException or OperationCanceledException)
        {
            pending.TryRemove(id, out _);
            throw new HelperChannelException(new HelperChannelFailure.Connection(error.Message));
        }
        if (await Task.WhenAny(answer.Task, limit) == limit)
        {
            pending.TryRemove(id, out _);
            throw new HelperChannelException(new HelperChannelFailure.NoReply());
        }
        return await answer.Task;
    }

    public void Dispose() => Close();

    private void Read()
    {
        try
        {
            while (true)
            {
                switch (MessageFraming.ReadFromHelper(pipe))
                {
                    case HelperPipe.Message.Event message:
                        inbound.Pass(new SystemEvent.HelperEvent(message.Json), id);
                        break;
                    case HelperPipe.Message.Output message:
                        inbound.Pass(new SystemEvent.HelperOutput(message.Json), id);
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
        inbound.End(id);
    }

    private void Answer(HelperPipe.Message message)
    {
        var id = message switch
        {
            HelperPipe.Message.Build build => build.Id,
            HelperPipe.Message.Reply reply => reply.Id,
            HelperPipe.Message.ReplyFlag flag => flag.Id,
            _ => throw new InvalidOperationException($"no request is answered by {message.GetType().Name}"),
        };
        // An answer after the time limit finds no request.
        if (pending.TryRemove(id, out var answer))
        {
            answer.TrySetResult(message);
        }
    }

    private void Close()
    {
        if (Interlocked.Exchange(ref closed, 1) == 1)
        {
            return;
        }
        pipe.Dispose();
        foreach (var id in pending.Keys)
        {
            if (pending.TryRemove(id, out var answer))
            {
                answer.TrySetException(new HelperChannelException(new HelperChannelFailure.ConnectionLost()));
            }
        }
    }
}
