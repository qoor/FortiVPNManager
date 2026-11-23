using FortiVPNManager.Contract;
using Xunit;
using static FortiVPNManager.Contract.AppInternal;

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
        Assert.Equal(new PortResponse.HelperReply(), await harness.Call(new HelperCall.Connect("{}")));
        Assert.Equal(new PortResponse.HelperReply(), await harness.Call(new HelperCall.Connect("{}")));
        Assert.Equal(
            [typeof(HelperPipe.Request.Build), typeof(HelperPipe.Request.Connect), typeof(HelperPipe.Request.Connect)],
            helper.Received.Select(request => request.GetType()));
    }

    [Fact]
    public async Task AHelperOfAnotherBuildGetsNoSecret()
    {
        var helper = new FakeHelper("another build");
        var harness = new Harness(helper);
        Assert.Equal(
            new PortResponse.HelperBuildMismatch("another build"),
            await harness.Call(new HelperCall.SubmitOtp("attempt", "123456")));
        Assert.Equal([typeof(HelperPipe.Request.Build)], helper.Received.Select(request => request.GetType()));
    }

    [Fact]
    public async Task TheStatusIsTriedOnceMoreOnAFreshConnection()
    {
        var gone = new FakeHelper(AppBuild, failing: true);
        var harness = new Harness(gone, new FakeHelper(AppBuild));
        Assert.Equal(new PortResponse.HelperReply("{}"), await harness.Call(new HelperCall.Status()));
        Assert.Equal(2, harness.Opened);
        Assert.True(gone.Disposed);
        // The first connection carried no run, but its loss is reported before the answer all the same.
        Assert.Equal(["HelperConnectionLost", "reply"], harness.Seen);
    }

    [Fact]
    public async Task TheStatusIsTriedOnceMoreWhenTheFirstConnectionCannotBeOpened()
    {
        var harness = new Harness(null, new FakeHelper(AppBuild));
        Assert.Equal(new PortResponse.HelperReply("{}"), await harness.Call(new HelperCall.Status()));
        Assert.Equal(2, harness.Opened);
        // No connection was active, so none is reported lost.
        Assert.Equal(["reply"], harness.Seen);
    }

    [Fact]
    public async Task TheBuildCheckWaitsForAHelperThatWindowsRestarts()
    {
        var harness = new Harness(new FakeHelper(AppBuild)) { PipeAppearsAfter = TimeSpan.FromSeconds(8) };
        Assert.Equal(new PortResponse.HelperBuild(AppBuild), await harness.Build());
        Assert.Equal(2, harness.Opened);
    }

    [Fact]
    public async Task TheBuildCheckGivesUpOnAHelperThatStaysAway()
    {
        var harness = new Harness(new FakeHelper(AppBuild)) { PipeAppearsAfter = TimeSpan.FromSeconds(21) };
        Assert.Equal(
            new PortResponse.HelperUnreachable(new HelperChannelFailure.Connection("no pipe")),
            await harness.Build());
        Assert.Equal(2, harness.Opened);
    }

    [Fact]
    public async Task TheStatusDoesNotWaitForAHelperThatWindowsRestarts()
    {
        var harness = new Harness(new FakeHelper(AppBuild)) { PipeAppearsAfter = TimeSpan.FromSeconds(8) };
        Assert.Equal(
            new PortResponse.HelperUnreachable(new HelperChannelFailure.Connection("no pipe")),
            await harness.Call(new HelperCall.Status()));
        Assert.Equal(2, harness.Opened);
    }

    [Fact]
    public async Task AConnectIsNeverRepeated()
    {
        var helper = new FakeHelper(AppBuild, failing: true);
        var harness = new Harness(helper, new FakeHelper(AppBuild));
        Assert.Equal(
            new PortResponse.HelperUnreachable(new HelperChannelFailure.ConnectionLost()),
            await harness.Call(new HelperCall.Connect("{}")));
        Assert.Equal(1, harness.Opened);
    }

    private static readonly (HelperCall Call, Type Request)[] s_updateCalls =
    [
        (new HelperCall.PrepareUpdate("{}"), typeof(HelperPipe.Request.PrepareUpdate)),
        (new HelperCall.InstallUpdate("{}"), typeof(HelperPipe.Request.InstallUpdate)),
        (new HelperCall.ReadPreparedUpdate(), typeof(HelperPipe.Request.ReadPreparedUpdate)),
        (new HelperCall.DiscardPreparedUpdate(), typeof(HelperPipe.Request.DiscardPreparedUpdate)),
    ];

    [Fact]
    public async Task AnUpdateCallChecksTheBuildAndPassesTheReplyOnUnread()
    {
        foreach ((HelperCall? call, Type? request) in s_updateCalls)
        {
            var helper = new FakeHelper(AppBuild);
            var harness = new Harness(helper);
            Assert.Equal(new PortResponse.HelperReply(FakeHelper.UpdateReply), await harness.Call(call));
            Assert.Equal([typeof(HelperPipe.Request.Build), request], helper.Received.Select(sent => sent.GetType()));
        }
    }

    [Fact]
    public async Task AHelperOfAnotherBuildGetsNoUpdateCall()
    {
        foreach ((HelperCall? call, Type _) in s_updateCalls)
        {
            var helper = new FakeHelper("another build");
            var harness = new Harness(helper);
            Assert.Equal(new PortResponse.HelperBuildMismatch("another build"), await harness.Call(call));
            Assert.Equal([typeof(HelperPipe.Request.Build)], helper.Received.Select(sent => sent.GetType()));
        }
    }

    [Fact]
    public async Task AnUpdateCallIsNeverRepeated()
    {
        foreach ((HelperCall? call, Type _) in s_updateCalls)
        {
            var harness = new Harness(new FakeHelper(AppBuild, failing: true), new FakeHelper(AppBuild));
            Assert.Equal(new PortResponse.HelperUnreachable(new HelperChannelFailure.ConnectionLost()), await harness.Call(call));
            Assert.Equal(1, harness.Opened);
        }
    }

    [Fact]
    public async Task ExitWhenIdleDropsTheConnectionAfterTheReply()
    {
        var helper = new FakeHelper(AppBuild);
        var harness = new Harness(helper, new FakeHelper(AppBuild));
        Assert.Equal(new PortResponse.HelperReplyFlag(false), await harness.Call(new HelperCall.ExitWhenIdle()));
        Assert.Equal(["reply", "HelperConnectionLost"], harness.Seen);
        Assert.True(helper.Disposed);
        _ = await harness.Call(new HelperCall.Status());
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
            });
            return answer!;
        }

        public async Task<PortResponse> Build()
        {
            PortResponse? answer = null;
            await _relay.Build(response =>
            {
                answer = response;
                Record("reply");
            });
            return answer!;
        }

        private FakeHelper Open(TimeSpan limit)
        {
            Opened++;
            return PipeAppearsAfter <= limit && _helpers.Dequeue() is { } helper
                ? helper
                : throw new HelperChannelException(new HelperChannelFailure.Connection("no pipe"));
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

        public List<HelperPipe.Request> Received { get; } = [];

        public bool Disposed { get; private set; }

        public void Start() { }

        public Task<HelperPipe.Message> Ask(Func<ulong, HelperPipe.Request> request)
        {
            HelperPipe.Request sent = request(++_lastRequest);
            Received.Add(sent);
            if (failing)
            {
                throw new HelperChannelException(new HelperChannelFailure.ConnectionLost());
            }
            HelperPipe.Message answer = sent switch
            {
                HelperPipe.Request.Build check => new HelperPipe.Message.Build(check.Id, build),
                HelperPipe.Request.ExitWhenIdle exit => new HelperPipe.Message.ReplyFlag(exit.Id, false),
                HelperPipe.Request.Status status => new HelperPipe.Message.Reply(status.Id, "{}"),
                HelperPipe.Request.Connect connect => new HelperPipe.Message.Reply(connect.Id),
                HelperPipe.Request.PrepareUpdate prepare => new HelperPipe.Message.Reply(prepare.Id, UpdateReply),
                HelperPipe.Request.InstallUpdate install => new HelperPipe.Message.Reply(install.Id, UpdateReply),
                HelperPipe.Request.ReadPreparedUpdate read => new HelperPipe.Message.Reply(read.Id, UpdateReply),
                HelperPipe.Request.DiscardPreparedUpdate discard => new HelperPipe.Message.Reply(discard.Id, UpdateReply),
                _ => throw new InvalidOperationException($"unexpected {sent}"),
            };
            return Task.FromResult(answer);
        }

        public void Dispose() => Disposed = true;
    }
}
