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
    public async Task AConnectIsNeverRepeated()
    {
        var helper = new FakeHelper(AppBuild, failing: true);
        var harness = new Harness(helper, new FakeHelper(AppBuild));
        Assert.Equal(
            new PortResponse.HelperUnreachable(new HelperChannelFailure.ConnectionLost()),
            await harness.Call(new HelperCall.Connect("{}")));
        Assert.Equal(1, harness.Opened);
    }

    [Fact]
    public async Task ExitWhenIdleDropsTheConnectionAfterTheReply()
    {
        var helper = new FakeHelper(AppBuild);
        var harness = new Harness(helper, new FakeHelper(AppBuild));
        Assert.Equal(new PortResponse.HelperReplyFlag(false), await harness.Call(new HelperCall.ExitWhenIdle()));
        Assert.Equal(["reply", "HelperConnectionLost"], harness.Seen);
        Assert.True(helper.Disposed);
        await harness.Call(new HelperCall.Status());
        Assert.Equal(2, harness.Opened);
    }

    /// <summary>
    /// A relay over fake connections, opened in order, that records its replies and events in order.
    /// A missing one fails to open, as when the Helper's pipe is not there.
    /// </summary>
    private sealed class Harness
    {
        private readonly Queue<FakeHelper?> helpers;
        private readonly HelperRelay relay;

        public Harness(params FakeHelper?[] helpers)
        {
            this.helpers = new Queue<FakeHelper?>(helpers);
            var inbound = new HelperInbound(systemEvent => Record(systemEvent.GetType().Name));
            relay = new HelperRelay(AppBuild, inbound, (_, _) => Open(), TimeSpan.Zero);
        }

        public int Opened { get; private set; }

        public List<string> Seen { get; } = [];

        public async Task<PortResponse> Call(HelperCall call)
        {
            PortResponse? answer = null;
            await relay.Handle(call, response =>
            {
                answer = response;
                Record("reply");
            });
            return answer!;
        }

        private IHelperChannel Open()
        {
            Opened++;
            return helpers.Dequeue()
                ?? throw new HelperChannelException(new HelperChannelFailure.Connection("no pipe"));
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
        private ulong lastRequest;

        public List<HelperPipe.Request> Received { get; } = [];

        public bool Disposed { get; private set; }

        public void Start() { }

        public Task<HelperPipe.Message> Ask(Func<ulong, HelperPipe.Request> request)
        {
            var sent = request(++lastRequest);
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
                _ => throw new InvalidOperationException($"unexpected {sent}"),
            };
            return Task.FromResult(answer);
        }

        public void Dispose() => Disposed = true;
    }
}
