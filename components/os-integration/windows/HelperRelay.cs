using System.Diagnostics;
using FortiVPNManager.Contract.AppInternal;
using FortiVPNManager.Contract.HelperPipe;

namespace FortiVPNManager.OsIntegration;

/// <summary>A failure of the channel to the Helper, carried as an exception inside the relay.</summary>
public sealed class HelperChannelException : Exception
{
    public HelperChannelException()
    {
    }

    public HelperChannelException(string message)
        : base(message)
    {
    }

    public HelperChannelException(string message, Exception innerException)
        : base(message, innerException)
    {
    }

    public HelperChannelException(HelperChannelFailure failure)
        : base((failure ?? throw new ArgumentNullException(nameof(failure))).ToString()) => Failure = failure;

    public HelperChannelFailure Failure
    {
        get => field ?? throw new InvalidOperationException("the exception carries no channel failure");
        private init;
    }
}

/// <summary>One connection to the Helper; the pipe in the app.</summary>
internal interface IHelperChannel : IDisposable
{
    /// <summary>Starts passing the Helper's messages on, once the relay counts this connection as active.</summary>
    void Start();

    /// <summary>
    /// Sends the request <paramref name="request"/> makes for a fresh number and completes with the
    /// Helper's answer to it. Fails with <see cref="HelperChannelException"/> when the connection
    /// breaks or no answer comes in time.
    /// </summary>
    Task<Message> Ask(Func<ulong, Request> request);
}

/// <summary>Opens connection <paramref name="id"/>, whose events go to <paramref name="inbound"/>.</summary>
internal delegate IHelperChannel OpenHelperChannel(ulong id, HelperInbound inbound, TimeSpan limit);

/// <summary>
/// Carries the runtime's Helper calls over one connection and passes the Helper's messages back,
/// as the Swift relay does on macOS. Payloads cross unread; the one answer read here is the build,
/// which marks its connection verified when it is the app's. A new connection is opened when there
/// is none or the last one was lost.
/// <list type="bullet">
/// <item>Calls that send secrets, change routes or handle an update go only over a verified connection.</item>
/// <item>The build, the status and the route calls are tried once more on a fresh connection when
/// the connection fails or cannot be opened. Other calls are never repeated or moved.</item>
/// </list>
/// </summary>
/// <param name="retryDelay">How long a retry waits for the Helper; Windows restarts a Helper that stopped.</param>
internal sealed class HelperRelay(string appBuild, HelperInbound inbound, OpenHelperChannel open, TimeSpan retryDelay)
{
    private sealed record Link(IHelperChannel Channel, ulong Id);

    /// <summary>The service keeps the pipe ready; a missing one means it is not running.</summary>
    private static readonly TimeSpan s_connectLimit = TimeSpan.FromSeconds(2);

    private static readonly TimeSpan s_restartLimit = TimeSpan.FromSeconds(20);

    private readonly string _appBuild = appBuild;
    private readonly HelperInbound _inbound = inbound;
    private readonly OpenHelperChannel _open = open;
    private readonly TimeSpan _retryDelay = retryDelay;
    private readonly Lock _gate = new();
    private Link? _link;
    private ulong _lastLinkId;

    public async Task Build(Action<PortResponse> reply)
    {
        try
        {
            reply(new PortResponseHelperBuild(await OnConnection(retrying: true, s_restartLimit, Version).ConfigureAwait(false)));
        }
        catch (HelperChannelException error)
        {
            reply(new PortResponseHelperUnreachable(error.Failure));
        }
    }

    public Task Handle(HelperCall call, Action<PortResponse> reply) => call switch
    {
        HelperCallConnect connect =>
            Relay(verified: true, retrying: false, id => new RequestConnect(id, connect.Json), reply),
        HelperCallSubmitOtp otp =>
            Relay(verified: true, retrying: false, id => new RequestSubmitOtp(id, otp.Attempt, otp.Code), reply),
        HelperCallDisconnect disconnect =>
            Relay(verified: false, retrying: false, id => new RequestDisconnect(id, disconnect.Attempt, disconnect.Reason), reply),
        HelperCallStatus =>
            Relay(verified: false, retrying: true, id => new RequestStatus(id), reply),
        HelperCallExitWhenIdle => ExitWhenIdle(reply),
        HelperCallReadBypassRoutes =>
            Relay(verified: true, retrying: true, id => new RequestReadBypassRoutes(id), reply),
        HelperCallWriteBypassRoutes write =>
            Relay(verified: true, retrying: true, id => new RequestWriteBypassRoutes(id, write.Json), reply),
        HelperCallPrepareUpdate prepare =>
            Relay(verified: true, retrying: false, id => new RequestPrepareUpdate(id, prepare.Json), reply),
        HelperCallInstallUpdate install =>
            Relay(verified: true, retrying: false, id => new RequestInstallUpdate(id, install.Json), reply),
        HelperCallReadPreparedUpdate =>
            Relay(verified: true, retrying: false, id => new RequestReadPreparedUpdate(id), reply),
        HelperCallDiscardPreparedUpdate =>
            Relay(verified: true, retrying: false, id => new RequestDiscardPreparedUpdate(id), reply),
        _ => throw new UnreachableException(),
    };

    /// <summary>A Helper of another build gets no call that needs a verified connection.</summary>
    private async Task Relay(
        bool verified, bool retrying, Func<ulong, Request> request, Action<PortResponse> reply)
    {
        try
        {
            reply(await OnConnection<PortResponse>(retrying, s_connectLimit, async current =>
            {
                if (verified && VerifiedLinkId != current.Id)
                {
                    string build = await Version(current).ConfigureAwait(false);
                    if (build != _appBuild)
                    {
                        return new PortResponseHelperBuildMismatch(build);
                    }
                }
                if (!_inbound.IsActive(current.Id))
                {
                    throw new HelperChannelException(new HelperChannelFailureConnectionLost());
                }
                Message answer = await current.Channel.Ask(request).ConfigureAwait(false);
                MessageReply replied = answer as MessageReply ?? throw Unreadable(answer);
                return new PortResponseHelperReply(replied.Json);
            }).ConfigureAwait(false));
        }
        catch (HelperChannelException error)
        {
            reply(new PortResponseHelperUnreachable(error.Failure));
        }
    }

    /// <summary>
    /// Asks a Helper with no run to exit, then drops the connection after the reply: that clears its
    /// verified mark and reports it lost, which the runtime expects.
    /// </summary>
    private async Task ExitWhenIdle(Action<PortResponse> reply)
    {
        Link current;
        try
        {
            current = CurrentLink(s_connectLimit);
            Message answer = await current.Channel.Ask(id => new RequestExitWhenIdle(id)).ConfigureAwait(false);
            MessageReplyFlag flag = answer as MessageReplyFlag ?? throw Unreadable(answer);
            reply(new PortResponseHelperReplyFlag(flag.Value));
        }
        catch (HelperChannelException error)
        {
            reply(new PortResponseHelperUnreachable(error.Failure));
            return;
        }
        Invalidate(current);
    }

    /// <summary>The Helper's build on <paramref name="current"/>, which is verified when the build is the app's.</summary>
    private async Task<string> Version(Link current)
    {
        Message answer = await current.Channel.Ask(id => new RequestBuild(id, _appBuild)).ConfigureAwait(false);
        string build = (answer as MessageBuild ?? throw Unreadable(answer)).Build;
        lock (_gate)
        {
            if (build == _appBuild && _inbound.IsActive(current.Id))
            {
                VerifiedLinkId = current.Id;
            }
        }
        return build;
    }

    /// <summary>
    /// Runs <paramref name="body"/> on the current connection, and once more on a fresh one when
    /// <paramref name="retrying"/> and the connection failed. Opening it counts: the pipe is missing
    /// while Windows restarts the Helper.
    /// </summary>
    private async Task<T> OnConnection<T>(bool retrying, TimeSpan retryLimit, Func<Link, Task<T>> body)
    {
        Link? current = null;
        try
        {
            current = CurrentLink(s_connectLimit);
            return await body(current).ConfigureAwait(false);
        }
        catch (HelperChannelException error) when (retrying && IsConnectionFailure(error.Failure))
        {
            if (current is not null)
            {
                Invalidate(current);
            }
            await Task.Delay(_retryDelay).ConfigureAwait(false);
            return await body(CurrentLink(retryLimit)).ConfigureAwait(false);
        }
    }

    private ulong? VerifiedLinkId
    {
        get
        {
            lock (_gate)
            {
                return field;
            }
        }

        set;
    }

    /// <summary>The live connection, or a new one when there is none or the last one was lost.</summary>
    private Link CurrentLink(TimeSpan limit)
    {
        lock (_gate)
        {
            if (_link is { } current)
            {
                if (_inbound.IsActive(current.Id))
                {
                    return current;
                }
                InvalidateLocked(current);
            }
            ulong id = ++_lastLinkId;
            IHelperChannel channel = _open(id, _inbound, limit);
            // Active before anything is read, so a connection that ends at once is reported and dropped.
            _inbound.Activate(id);
            channel.Start();
            _link = new Link(channel, id);
            return _link;
        }
    }

    private void Invalidate(Link target)
    {
        lock (_gate)
        {
            InvalidateLocked(target);
        }
    }

    /// <summary>
    /// Drops one connection, never a newer one that replaced it. The Helper stops the run of a
    /// connection that goes away, so it is reported lost if it carried the runs.
    /// </summary>
    private void InvalidateLocked(Link target)
    {
        _inbound.End(target.Id);
        target.Channel.Dispose();
        if (_link?.Id == target.Id)
        {
            _link = null;
            VerifiedLinkId = null;
        }
    }

    /// <summary>The connection went away, so a fresh one may answer.</summary>
    private static bool IsConnectionFailure(HelperChannelFailure failure) =>
        failure is HelperChannelFailureConnection or HelperChannelFailureConnectionLost;

    private static HelperChannelException Unreadable(Message answer) =>
        new(new HelperChannelFailureUnreadableReply($"an answer of the wrong kind: {answer.GetType().Name}"));
}

/// <summary>
/// The messages of the connection that carries the runs, passed on as system events in the order
/// the Helper sent them. Only the active connection passes anything, and its loss is reported once,
/// after all it passed: both happen under the lock.
/// </summary>
internal sealed class HelperInbound(Action<SystemEvent> post)
{
    private readonly Lock _gate = new();
    private ulong? _activeId;

    public void Activate(ulong id)
    {
        lock (_gate)
        {
            _activeId = id;
        }
    }

    public bool IsActive(ulong id)
    {
        lock (_gate)
        {
            return _activeId == id;
        }
    }

    public void Pass(SystemEvent systemEvent, ulong id)
    {
        lock (_gate)
        {
            if (_activeId == id)
            {
                post(systemEvent);
            }
        }
    }

    /// <summary>Reports the loss once, if <paramref name="id"/> is still the active connection.</summary>
    public void End(ulong id)
    {
        lock (_gate)
        {
            if (_activeId != id)
            {
                return;
            }
            _activeId = null;
            post(new SystemEventHelperConnectionLost());
        }
    }
}
