using Xunit;

namespace FortiVPNManager.OsIntegration.Tests;

/// <summary>The wait for the Helper's service at launch, on a clock that moves only while the wait pauses or a start takes time.</summary>
public class HelperStartTests
{
    [Fact]
    public void ARunningServiceNeedsNothing()
    {
        var harness = new Harness { Before = HelperServiceState.Running };
        Assert.IsType<HelperStartResult.Running>(harness.Wait());
        Assert.Empty(harness.Starts);
        Assert.Equal(TimeSpan.Zero, harness.Now);
    }

    [Fact]
    public void AStoppedServiceIsStartedAndItsStartWaitedForHoweverLong()
    {
        var harness = new Harness { After = (_, since) => since < TimeSpan.FromHours(2) ? HelperServiceState.Changing : HelperServiceState.Running };
        Assert.IsType<HelperStartResult.Running>(harness.Wait());
        Assert.Equal([TimeSpan.Zero], harness.Starts);
        Assert.Equal(TimeSpan.FromHours(2), harness.Now);
        Assert.Empty(harness.Notes);
    }

    [Fact]
    public void AServiceOnItsWayIsNotStarted()
    {
        var harness = new Harness
        {
            Before = HelperServiceState.Changing,
            BeforeUntil = TimeSpan.FromSeconds(20),
            ThenBefore = HelperServiceState.Running,
        };
        Assert.IsType<HelperStartResult.Running>(harness.Wait());
        Assert.Empty(harness.Starts);
        Assert.Equal(TimeSpan.FromSeconds(20), harness.Now);
    }

    [Fact]
    public void AStartOfWindowsOwnThatFailedIsNotCounted()
    {
        var harness = new Harness
        {
            Before = HelperServiceState.Changing,
            BeforeUntil = TimeSpan.FromSeconds(10),
            ThenBefore = HelperServiceState.Stopped,
        };
        Assert.IsType<HelperStartResult.Running>(harness.Wait());
        Assert.Equal([TimeSpan.FromSeconds(10)], harness.Starts);
        Assert.Empty(harness.Notes);
    }

    [Fact]
    public void AMissingServiceFailsAtOnce()
    {
        var harness = new Harness { Before = HelperServiceState.NotInstalled };
        Assert.Equal(HelperStartFailure.NotInstalled, Assert.IsType<HelperStartResult.Failed>(harness.Wait()).Reason);
        Assert.Empty(harness.Starts);
        Assert.Equal(TimeSpan.Zero, harness.Now);
    }

    [Fact]
    public void AServiceThisAccountMayNotReadFailsAtOnce()
    {
        var harness = new Harness { Before = HelperServiceState.NoPermission };
        Assert.Equal(HelperStartFailure.NoPermission, Assert.IsType<HelperStartResult.Failed>(harness.Wait()).Reason);
        Assert.Empty(harness.Starts);
        Assert.Equal(TimeSpan.Zero, harness.Now);
    }

    [Theory]
    [InlineData(HelperStartFailure.Disabled)]
    [InlineData(HelperStartFailure.NoPermission)]
    [InlineData(HelperStartFailure.NotInstalled)]
    public void ARefusedStartFailsAtOnce(HelperStartFailure reason)
    {
        var harness = new Harness { Answer = _ => new StartAnswer.Refused(reason) };
        Assert.Equal(reason, Assert.IsType<HelperStartResult.Failed>(harness.Wait()).Reason);
        Assert.Equal([TimeSpan.Zero], harness.Starts);
        Assert.Equal(TimeSpan.Zero, harness.Now);
    }

    [Fact]
    public void ATimedOutStartIsMadeOnceMoreAfterItsProcessHasExited()
    {
        // Windows gave up on the first start after 30 s; its process stays until 90 s.
        var harness = new Harness
        {
            Answer = start => start == 1 ? new StartAnswer.TimedOut() : new StartAnswer.Requested(),
            StartTakes = start => start == 1 ? TimeSpan.FromSeconds(30) : TimeSpan.Zero,
            ProcessLeftAt = now => now >= TimeSpan.FromSeconds(30) && now < TimeSpan.FromSeconds(90),
            After = (_, since) => since < TimeSpan.FromSeconds(10) ? HelperServiceState.Changing : HelperServiceState.Running,
        };
        Assert.IsType<HelperStartResult.Running>(harness.Wait());
        Assert.Equal([TimeSpan.Zero, TimeSpan.FromSeconds(90)], harness.Starts);
        (string? line, TimeSpan at) = Assert.Single(harness.Notes);
        Assert.Contains("did not start in time", line);
        Assert.EndsWith("(attempt 1/2)", line);
        // Written when the first start failed, not when the second was made.
        Assert.Equal(TimeSpan.FromSeconds(30), at);
    }

    [Fact]
    public void AStartOfWindowsOwnAfterAFailedFirstStartCountsAsRunning()
    {
        // Windows restarts the service itself at 40 s, while the first start's process still exits.
        var harness = new Harness
        {
            BeforeUntil = TimeSpan.FromSeconds(40),
            ThenBefore = HelperServiceState.Running,
            Answer = _ => new StartAnswer.TimedOut(),
            StartTakes = _ => TimeSpan.FromSeconds(30),
            ProcessLeftAt = now => now >= TimeSpan.FromSeconds(30) && now < TimeSpan.FromSeconds(90),
        };
        Assert.IsType<HelperStartResult.Running>(harness.Wait());
        Assert.Equal([TimeSpan.Zero], harness.Starts);
        Assert.EndsWith("(attempt 1/2)", Assert.Single(harness.Notes).Line);
    }

    [Fact]
    public void ASecondTimedOutStartFails()
    {
        var harness = new Harness { Answer = _ => new StartAnswer.TimedOut(), StartTakes = _ => TimeSpan.FromSeconds(30) };
        HelperStartResult.Failed failed = Assert.IsType<HelperStartResult.Failed>(harness.Wait());
        Assert.Equal(HelperStartFailure.TimedOut, failed.Reason);
        Assert.EndsWith("(attempt 2/2)", failed.Detail);
        Assert.Equal(2, harness.Starts.Count);
        Assert.EndsWith("(attempt 1/2)", Assert.Single(harness.Notes).Line);
    }

    [Fact]
    public void AServiceThatStopsWhileStartingIsStartedOnceMoreAfterItsProcessHasExited()
    {
        var harness = new Harness
        {
            After = (start, since) => start == 1
                ? since < TimeSpan.FromSeconds(20) ? HelperServiceState.Changing : HelperServiceState.Stopped
                : HelperServiceState.Running,
            ProcessLeftAt = now => now >= TimeSpan.FromSeconds(20) && now < TimeSpan.FromSeconds(35),
        };
        Assert.IsType<HelperStartResult.Running>(harness.Wait());
        Assert.Equal([TimeSpan.Zero, TimeSpan.FromSeconds(35)], harness.Starts);
        (string? line, TimeSpan at) = Assert.Single(harness.Notes);
        Assert.Contains("stopped with an error while starting", line);
        Assert.Equal(TimeSpan.FromSeconds(20), at);
    }

    [Fact]
    public void AServiceThatStopsWhileStartingTwiceFails()
    {
        var harness = new Harness { After = (_, since) => since < TimeSpan.FromSeconds(5) ? HelperServiceState.Changing : HelperServiceState.Stopped };
        Assert.Equal(HelperStartFailure.ErrorWhileStarting, Assert.IsType<HelperStartResult.Failed>(harness.Wait()).Reason);
        Assert.Equal([TimeSpan.Zero, TimeSpan.FromSeconds(5)], harness.Starts);
        Assert.Equal(TimeSpan.FromSeconds(10), harness.Now);
    }

    [Fact]
    public void AStartWindowsCannotMakeIsMadeOnceMoreThenFails()
    {
        var harness = new Harness { Answer = _ => new StartAnswer.Failed(1069) };
        HelperStartResult.Failed failed = Assert.IsType<HelperStartResult.Failed>(harness.Wait());
        Assert.Equal(HelperStartFailure.ErrorWhileStarting, failed.Reason);
        Assert.Contains("1069", failed.Detail);
        Assert.Equal(2, harness.Starts.Count);
        Assert.Contains("error 1069", Assert.Single(harness.Notes).Line);
    }

    [Fact]
    public void AProcessLeftFromAnEarlierStartIsWaitedForBeforeTheFirstStart()
    {
        var harness = new Harness { ProcessLeftAt = now => now < TimeSpan.FromSeconds(45) };
        Assert.IsType<HelperStartResult.Running>(harness.Wait());
        Assert.Equal([TimeSpan.FromSeconds(45)], harness.Starts);
        Assert.Empty(harness.Notes);
    }

    [Fact]
    public void TheSessionEndingEndsTheWaitQuietly()
    {
        var harness = new Harness { After = (_, _) => HelperServiceState.Changing, SessionEndsAt = TimeSpan.FromSeconds(30) };
        Assert.IsType<HelperStartResult.SessionEnding>(harness.Wait());
        Assert.Equal(TimeSpan.FromSeconds(30), harness.Now);
    }

    /// <summary>
    /// A service that is <see cref="Before"/> until a start it takes, then <see cref="After"/> by that
    /// start's number and the time since. Each start takes <see cref="StartTakes"/> of the clock.
    /// </summary>
    private sealed class Harness : IHelperService
    {
        private int _accepted;
        private TimeSpan _acceptedAt;

        public HelperServiceState Before { get; init; } = HelperServiceState.Stopped;

        /// <summary>When <see cref="Before"/> turns into <see cref="ThenBefore"/> by itself.</summary>
        public TimeSpan BeforeUntil { get; init; } = TimeSpan.MaxValue;

        public HelperServiceState ThenBefore { get; init; }

        public Func<int, StartAnswer> Answer { get; init; } = _ => new StartAnswer.Requested();

        public Func<int, TimeSpan> StartTakes { get; init; } = _ => TimeSpan.Zero;

        public Func<int, TimeSpan, HelperServiceState> After { get; init; } = (_, _) => HelperServiceState.Running;

        public Func<TimeSpan, bool> ProcessLeftAt { get; init; } = _ => false;

        public TimeSpan SessionEndsAt { get; init; } = TimeSpan.MaxValue;

        public TimeSpan Now { get; private set; }

        public List<TimeSpan> Starts { get; } = [];

        /// <summary>The event log lines of failed first starts, and when they were written.</summary>
        public List<(string Line, TimeSpan At)> Notes { get; } = [];

        public HelperStartResult Wait() => HelperStart.Wait(
            this,
            interval =>
            {
                Now += interval;
                return Now < SessionEndsAt;
            },
            line => Notes.Add((line, Now)));

        public HelperServiceState State() => _accepted > 0
            ? After(_accepted, Now - _acceptedAt)
            : Now < BeforeUntil ? Before : ThenBefore;

        public StartAnswer Start()
        {
            Starts.Add(Now);
            int number = Starts.Count;
            StartAnswer answer = Answer(number);
            Now += StartTakes(number);
            if (answer is StartAnswer.Requested)
            {
                _accepted = number;
                _acceptedAt = Now;
            }
            return answer;
        }

        public bool ProcessLeft() => ProcessLeftAt(Now);
    }
}
