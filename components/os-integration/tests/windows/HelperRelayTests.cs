using FortiVPNManager.Contract.AppInternal;
using FortiVPNManager.Contract.HelperPipe;
using Xunit;

namespace FortiVPNManager.OsIntegration.Tests;

/// <summary>The Helper relay's rules for the build check, retries and Exit When Idle, as on macOS.</summary>
public class HelperRelayTests
{
    private const string AppBuild = "20261113.1";

    [Fact]
    public async Task ASecretCallChecksTheBuildOnceThenRelays()
    {
        var helper = new FakeHelper(AppBuild);
        var harness = new Harness(helper);
        Assert.Equal(new PortResponseHelperReply(), await harness.Call(new HelperCallConnect("{}")));
        Assert.Equal(new PortResponseHelperReply(), await harness.Call(new HelperCallConnect("{}")));
        Assert.Equal(
            [typeof(RequestBuild), typeof(RequestConnect), typeof(RequestConnect)],
            helper.Received.Select(request => request.GetType()));
    }

    [Fact]
    public async Task AHelperOfAnotherBuildGetsNoSecret()
    {
        var helper = new FakeHelper("another build");
        var harness = new Harness(helper);
        Assert.Equal(
            new PortResponseHelperBuildMismatch("another build"),
            await harness.Call(new HelperCallSubmitOtp("attempt", "123456")));
        Assert.Equal([typeof(RequestBuild)], helper.Received.Select(request => request.GetType()));
    }

    [Fact]
    public async Task TheStatusIsTriedOnceMoreOnAFreshConnection()
    {
        var gone = new FakeHelper(AppBuild, failing: true);
        var harness = new Harness(gone, new FakeHelper(AppBuild));
        Assert.Equal(new PortResponseHelperReply("{}"), await harness.Call(new HelperCallStatus()));
        Assert.Equal(2, harness.Opened);
        Assert.True(gone.Disposed);
        // The first connection carried no run, but its loss is reported before the answer all the same.
        Assert.Equal(["SystemEventHelperConnectionLost", "reply"], harness.Seen);
    }

    [Fact]
    public async Task TheStatusIsTriedOnceMoreWhenTheFirstConnectionCannotBeOpened()
    {
        var harness = new Harness(null, new FakeHelper(AppBuild));
        Assert.Equal(new PortResponseHelperReply("{}"), await harness.Call(new HelperCallStatus()));
        Assert.Equal(2, harness.Opened);
        // No connection was active, so none is reported lost.
        Assert.Equal(["reply"], harness.Seen);
    }

    [Fact]
    public async Task TheBuildCheckWaitsForAHelperThatWindowsRestarts()
    {
        var harness = new Harness(new FakeHelper(AppBuild)) { PipeAppearsAfter = TimeSpan.FromSeconds(8) };
        Assert.Equal(new PortResponseHelperBuild(AppBuild), await harness.Build());
        Assert.Equal(2, harness.Opened);
    }

    [Fact]
    public async Task TheBuildCheckGivesUpOnAHelperThatStaysAway()
    {
        var harness = new Harness(new FakeHelper(AppBuild)) { PipeAppearsAfter = TimeSpan.FromSeconds(21) };
        Assert.Equal(
            new PortResponseHelperUnreachable(new HelperChannelFailureConnection("no pipe")),
            await harness.Build());
        Assert.Equal(2, harness.Opened);
    }

    [Fact]
    public async Task TheStatusDoesNotWaitForAHelperThatWindowsRestarts()
    {
        var harness = new Harness(new FakeHelper(AppBuild)) { PipeAppearsAfter = TimeSpan.FromSeconds(8) };
        Assert.Equal(
            new PortResponseHelperUnreachable(new HelperChannelFailureConnection("no pipe")),
            await harness.Call(new HelperCallStatus()));
        Assert.Equal(2, harness.Opened);
    }

    [Fact]
    public async Task AConnectIsNeverRepeated()
    {
        var helper = new FakeHelper(AppBuild, failing: true);
        var harness = new Harness(helper, new FakeHelper(AppBuild));
        Assert.Equal(
            new PortResponseHelperUnreachable(new HelperChannelFailureConnectionLost()),
            await harness.Call(new HelperCallConnect("{}")));
        Assert.Equal(1, harness.Opened);
    }

    private static readonly (HelperCall Call, Type Request)[] s_updateCalls =
    [
        (new HelperCallPrepareUpdate("{}"), typeof(RequestPrepareUpdate)),
        (new HelperCallInstallUpdate("{}"), typeof(RequestInstallUpdate)),
        (new HelperCallReadPreparedUpdate(), typeof(RequestReadPreparedUpdate)),
        (new HelperCallDiscardPreparedUpdate(), typeof(RequestDiscardPreparedUpdate)),
    ];

    [Fact]
    public async Task AnUpdateCallChecksTheBuildAndPassesTheReplyOnUnread()
    {
        foreach ((HelperCall? call, Type? request) in s_updateCalls)
        {
            var helper = new FakeHelper(AppBuild);
            var harness = new Harness(helper);
            Assert.Equal(new PortResponseHelperReply(FakeHelper.UpdateReply), await harness.Call(call));
            Assert.Equal([typeof(RequestBuild), request], helper.Received.Select(sent => sent.GetType()));
        }
    }

    [Fact]
    public async Task AHelperOfAnotherBuildGetsNoUpdateCall()
    {
        foreach ((HelperCall? call, Type _) in s_updateCalls)
        {
            var helper = new FakeHelper("another build");
            var harness = new Harness(helper);
            Assert.Equal(new PortResponseHelperBuildMismatch("another build"), await harness.Call(call));
            Assert.Equal([typeof(RequestBuild)], helper.Received.Select(sent => sent.GetType()));
        }
    }

    [Fact]
    public async Task AnUpdateCallIsNeverRepeated()
    {
        foreach ((HelperCall? call, Type _) in s_updateCalls)
        {
            var harness = new Harness(new FakeHelper(AppBuild, failing: true), new FakeHelper(AppBuild));
            Assert.Equal(new PortResponseHelperUnreachable(new HelperChannelFailureConnectionLost()), await harness.Call(call));
            Assert.Equal(1, harness.Opened);
        }
    }

    [Fact]
    public async Task ExitWhenIdleDropsTheConnectionAfterTheReply()
    {
        var helper = new FakeHelper(AppBuild);
        var harness = new Harness(helper, new FakeHelper(AppBuild));
        Assert.Equal(new PortResponseHelperReplyFlag(false), await harness.Call(new HelperCallExitWhenIdle()));
        Assert.Equal(["reply", "SystemEventHelperConnectionLost"], harness.Seen);
        Assert.True(helper.Disposed);
        _ = await harness.Call(new HelperCallStatus());
        Assert.Equal(2, harness.Opened);
    }

    /// <summary>
    /// A relay over fake connections, opened in order, that records its replies and events in order.
    /// A missing one fails to open, as when the Helper's pipe is not there.
    /// </summary>
    private sealed class Harness
    {
        private readonly Queue<FakeHelper?> _helpers;
        private readonly HelperRelay _relay;

        public Harness(params FakeHelper?[] helpers)
        {
            this._helpers = new Queue<FakeHelper?>(helpers);
            var inbound = new HelperInbound(systemEvent => Record(systemEvent.GetType().Name));
            _relay = new HelperRelay(AppBuild, inbound, (_, _, limit) => Open(limit), TimeSpan.Zero);
        }

        public TimeSpan PipeAppearsAfter { get; init; }

        public int Opened { get; private set; }

        public List<string> Seen { get; } = [];

        public async Task<PortResponse> Call(HelperCall call)
        {
            PortResponse? answer = null;
            await _relay.Handle(call, response =>
            {
                answer = response;
                Record("reply");
            }).ConfigureAwait(true);
            return answer!;
        }

        public async Task<PortResponse> Build()
        {
            PortResponse? answer = null;
            await _relay.Build(response =>
            {
                answer = response;
                Record("reply");
            }).ConfigureAwait(true);
            return answer!;
        }

        private FakeHelper Open(TimeSpan limit)
        {
            Opened++;
            return PipeAppearsAfter <= limit && _helpers.Dequeue() is { } helper
                ? helper
                : throw new HelperChannelException(new HelperChannelFailureConnection("no pipe"));
        }

        private void Record(string what)
        {
            lock (Seen)
            {
                Seen.Add(what);
            }
        }
    }

    /// <summary>Answers like a Helper of <paramref name="build"/>, or fails every call like a Helper that stopped.</summary>
    private sealed class FakeHelper(string build, bool failing = false) : IHelperChannel
    {
        public const string UpdateReply = "{\"done\":{}}";

        private ulong _lastRequest;

        public List<Request> Received { get; } = [];

        public bool Disposed { get; private set; }

        public void Start() { }

        public Task<Message> Ask(Func<ulong, Request> request)
        {
            Request sent = request(++_lastRequest);
            Received.Add(sent);
            if (failing)
            {
                throw new HelperChannelException(new HelperChannelFailureConnectionLost());
            }
            Message answer = sent switch
            {
                RequestBuild check => new MessageBuild(check.Id, build),
                RequestExitWhenIdle exit => new MessageReplyFlag(exit.Id, false),
                RequestStatus status => new MessageReply(status.Id, "{}"),
                RequestConnect connect => new MessageReply(connect.Id),
                RequestPrepareUpdate prepare => new MessageReply(prepare.Id, UpdateReply),
                RequestInstallUpdate install => new MessageReply(install.Id, UpdateReply),
                RequestReadPreparedUpdate read => new MessageReply(read.Id, UpdateReply),
                RequestDiscardPreparedUpdate discard => new MessageReply(discard.Id, UpdateReply),
                _ => throw new InvalidOperationException($"unexpected {sent}"),
            };
            return Task.FromResult(answer);
        }

        public void Dispose() => Disposed = true;
    }
}
