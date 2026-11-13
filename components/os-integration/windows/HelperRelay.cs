using System.Diagnostics;
using FortiVPNManager.Contract;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>A failure of the channel to the Helper, carried as an exception inside the relay.</summary>
internal sealed class HelperChannelException(HelperChannelFailure failure) : Exception(failure.ToString())
{
    public HelperChannelFailure Failure { get; } = failure;
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
    Task<HelperPipe.Message> Ask(Func<ulong, HelperPipe.Request> request);
}

/// <summary>Opens connection <paramref name="id"/>, whose events go to <paramref name="inbound"/>.</summary>
internal delegate IHelperChannel OpenHelperChannel(ulong id, HelperInbound inbound);

/// <summary>
/// Carries the runtime's Helper calls over one connection and passes the Helper's messages back,
/// as the Swift relay does on macOS. Payloads cross unread; the one answer read here is the build,
/// which marks its connection verified when it is the app's. A new connection is opened when there
/// is none or the last one was lost.
/// <list type="bullet">
/// <item>Calls that send secrets or change routes go only over a verified connection.</item>
/// <item>The build, the status and the route calls are tried once more on a fresh connection when
/// the connection fails or cannot be opened. Other calls are never repeated or moved.</item>
/// </list>
/// </summary>
internal sealed class HelperRelay
{
    private sealed record Link(IHelperChannel Channel, ulong Id);

    private readonly string appBuild;
    private readonly HelperInbound inbound;
    private readonly OpenHelperChannel open;
    private readonly TimeSpan retryDelay;
    private readonly object gate = new();
    private Link? link;
    private ulong lastLinkId;
    /// <summary>The connection whose Helper answered with the app's build.</summary>
    private ulong? verifiedLinkId;

    /// <param name="retryDelay">How long a retry waits for the Helper; Windows restarts a Helper that stopped.</param>
    public HelperRelay(string appBuild, HelperInbound inbound, OpenHelperChannel open, TimeSpan retryDelay)
    {
        this.appBuild = appBuild;
        this.inbound = inbound;
        this.open = open;
        this.retryDelay = retryDelay;
    }

    public async Task Build(Action<PortResponse> reply)
    {
        try
        {
            reply(new PortResponse.HelperBuild(await OnConnection(retrying: true, Version)));
        }
        catch (HelperChannelException error)
        {
            reply(new PortResponse.HelperUnreachable(error.Failure));
        }
    }

    public Task Handle(HelperCall call, Action<PortResponse> reply) => call switch
    {
        HelperCall.Connect connect =>
            Relay(verified: true, retrying: false, id => new HelperPipe.Request.Connect(id, connect.Json), reply),
        HelperCall.SubmitOtp otp =>
            Relay(verified: true, retrying: false, id => new HelperPipe.Request.SubmitOtp(id, otp.Attempt, otp.Code), reply),
        HelperCall.Disconnect disconnect =>
            Relay(verified: false, retrying: false, id => new HelperPipe.Request.Disconnect(id, disconnect.Attempt, disconnect.Reason), reply),
        HelperCall.Status =>
            Relay(verified: false, retrying: true, id => new HelperPipe.Request.Status(id), reply),
        HelperCall.ExitWhenIdle => ExitWhenIdle(reply),
        HelperCall.ReadBypassRoutes =>
            Relay(verified: true, retrying: true, id => new HelperPipe.Request.ReadBypassRoutes(id), reply),
        HelperCall.WriteBypassRoutes write =>
            Relay(verified: true, retrying: true, id => new HelperPipe.Request.WriteBypassRoutes(id, write.Json), reply),
        _ => throw new UnreachableException(),
    };

    /// <summary>A Helper of another build gets no call that needs a verified connection.</summary>
    private async Task Relay(
        bool verified, bool retrying, Func<ulong, HelperPipe.Request> request, Action<PortResponse> reply)
    {
        try
        {
            reply(await OnConnection<PortResponse>(retrying, async current =>
            {
                if (verified && VerifiedLinkId != current.Id)
                {
                    var build = await Version(current);
                    if (build != appBuild)
                    {
                        return new PortResponse.HelperBuildMismatch(build);
                    }
                }
                if (!inbound.IsActive(current.Id))
                {
                    throw new HelperChannelException(new HelperChannelFailure.ConnectionLost());
                }
                var answer = await current.Channel.Ask(request);
                var replied = answer as HelperPipe.Message.Reply ?? throw Unreadable(answer);
                return new PortResponse.HelperReply(replied.Json);
            }));
        }
        catch (HelperChannelException error)
        {
            reply(new PortResponse.HelperUnreachable(error.Failure));
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
            current = CurrentLink();
            var answer = await current.Channel.Ask(id => new HelperPipe.Request.ExitWhenIdle(id));
            var flag = answer as HelperPipe.Message.ReplyFlag ?? throw Unreadable(answer);
            reply(new PortResponse.HelperReplyFlag(flag.Value));
        }
        catch (HelperChannelException error)
        {
            reply(new PortResponse.HelperUnreachable(error.Failure));
            return;
        }
        Invalidate(current);
    }

    /// <summary>The Helper's build on <paramref name="current"/>, which is verified when the build is the app's.</summary>
    private async Task<string> Version(Link current)
    {
        var answer = await current.Channel.Ask(id => new HelperPipe.Request.Build(id, appBuild));
        var build = (answer as HelperPipe.Message.Build ?? throw Unreadable(answer)).HelperBuild;
        lock (gate)
        {
            if (build == appBuild && inbound.IsActive(current.Id))
            {
                verifiedLinkId = current.Id;
            }
        }
        return build;
    }

    /// <summary>
    /// Runs <paramref name="body"/> on the current connection, and once more on a fresh one when
    /// <paramref name="retrying"/> and the connection failed. Opening it counts: the pipe is missing
    /// while Windows restarts the Helper.
    /// </summary>
    private async Task<T> OnConnection<T>(bool retrying, Func<Link, Task<T>> body)
    {
        Link? current = null;
        try
        {
            current = CurrentLink();
            return await body(current);
        }
        catch (HelperChannelException error) when (retrying && IsConnectionFailure(error.Failure))
        {
            if (current is not null)
            {
                Invalidate(current);
            }
            await Task.Delay(retryDelay);
            return await body(CurrentLink());
        }
    }

    private ulong? VerifiedLinkId
    {
        get
        {
            lock (gate)
            {
                return verifiedLinkId;
            }
        }
    }

    /// <summary>The live connection, or a new one when there is none or the last one was lost.</summary>
    private Link CurrentLink()
    {
        lock (gate)
        {
            if (link is { } current)
            {
                if (inbound.IsActive(current.Id))
                {
                    return current;
                }
                InvalidateLocked(current);
            }
            var id = ++lastLinkId;
            var channel = open(id, inbound);
            // Active before anything is read, so a connection that ends at once is reported and dropped.
            inbound.Activate(id);
            channel.Start();
            link = new Link(channel, id);
            return link;
        }
    }

    private void Invalidate(Link target)
    {
        lock (gate)
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
        inbound.End(target.Id);
        target.Channel.Dispose();
        if (link?.Id == target.Id)
        {
            link = null;
            verifiedLinkId = null;
        }
    }

    /// <summary>The connection went away, so a fresh one may answer.</summary>
    private static bool IsConnectionFailure(HelperChannelFailure failure) =>
        failure is HelperChannelFailure.Connection or HelperChannelFailure.ConnectionLost;

    private static HelperChannelException Unreadable(HelperPipe.Message answer) =>
        new(new HelperChannelFailure.UnreadableReply($"an answer of the wrong kind: {answer.GetType().Name}"));
}

/// <summary>
/// The messages of the connection that carries the runs, passed on as system events in the order
/// the Helper sent them. Only the active connection passes anything, and its loss is reported once,
/// after all it passed: both happen under the lock.
/// </summary>
internal sealed class HelperInbound(Action<SystemEvent> post)
{
    private readonly object gate = new();
    private ulong? activeId;

    public void Activate(ulong id)
    {
        lock (gate)
        {
            activeId = id;
        }
    }

    public bool IsActive(ulong id)
    {
        lock (gate)
        {
            return activeId == id;
        }
    }

    public void Pass(SystemEvent systemEvent, ulong id)
    {
        lock (gate)
        {
            if (activeId == id)
            {
                post(systemEvent);
            }
        }
    }

    /// <summary>Reports the loss once, if <paramref name="id"/> is still the active connection.</summary>
    public void End(ulong id)
    {
        lock (gate)
        {
            if (activeId != id)
            {
                return;
            }
            activeId = null;
            post(new SystemEvent.HelperConnectionLost());
        }
    }
}
