using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>
/// The app's lifetime as the runtime sees it: a quit, the user's or the system's, is reported once
/// and waits for the runtime's approval, which ends the tunnel and the log first.
/// </summary>
public sealed class AppLifetime
{
    private readonly Action<SystemEvent> post;
    private readonly Action quit;
    private readonly ManualResetEventSlim approved = new();
    private int requested;

    /// <param name="quit">Ends the app once the runtime approves.</param>
    internal AppLifetime(Action<SystemEvent> post, Action quit)
    {
        this.post = post;
        this.quit = quit;
    }

    /// <summary>The menu's Quit, or Windows signing out or shutting down.</summary>
    public void RequestQuit()
    {
        if (Interlocked.Exchange(ref requested, 1) == 0)
        {
            post(new SystemEvent.QuitRequested());
        }
    }

    internal void ApproveQuit()
    {
        approved.Set();
        quit();
    }

    /// <summary>Whether the runtime approved within <paramref name="limit"/>.</summary>
    internal bool WaitForApproval(TimeSpan limit) => approved.Wait(limit);
}
