using System.Net.NetworkInformation;
using Microsoft.Win32;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>
/// Reports the system's events as they happen, facts only: what they mean together, such as the user
/// being away, is the runtime's call. Events that can come just before the system sleeps take their
/// time in the handler, since their message may reach the runtime only after the wake. The window
/// events (displays off and on, the app activated) come with the UI.
/// </summary>
internal sealed class SystemEvents(Action<SystemEvent> post, AppLifetime lifetime)
{
    /// <summary>
    /// How long signing out or shutting down waits for the runtime to end the tunnel and the log.
    /// Windows asks the user about an app that holds it up longer.
    /// </summary>
    private static readonly TimeSpan QuitWait = TimeSpan.FromSeconds(5);

    private readonly object gate = new();
    private string timeZone = "";
    private bool locked;

    /// <summary>Whether the session is locked, from the lock and unlock events since the start.</summary>
    public bool Locked
    {
        get
        {
            lock (gate)
            {
                return locked;
            }
        }
    }

    public void Start()
    {
        timeZone = IanaTimeZone();
        // These come through the subscribing thread's SynchronizationContext, and WinUI's UI thread has
        // one that cannot deliver them. Subscribed from a pool thread, which has none, they come on the
        // thread .NET keeps for its hidden window, so the wait on signing out holds up no app window.
        // The first use of them decides that thread, once, so the wait for the Helper at launch, the
        // app's first use, subscribes the same way.
        Task.Run(Subscribe).Wait();
        NetworkChange.NetworkAddressChanged += (_, _) => PathChanged();
        PathChanged();
    }

    /// <summary>The system time zone's IANA name, which the runtime computes with.</summary>
    public static string IanaTimeZone() =>
        TimeZoneInfo.TryConvertWindowsIdToIanaId(TimeZoneInfo.Local.Id, out var iana) ? iana : TimeZoneInfo.Local.Id;

    private void Subscribe()
    {
        Microsoft.Win32.SystemEvents.PowerModeChanged += (_, change) =>
        {
            switch (change.Mode)
            {
                case PowerModes.Suspend:
                    post(new SystemEvent.WillSleep(DateTimeOffset.Now));
                    break;
                case PowerModes.Resume:
                    post(new SystemEvent.DidWake());
                    break;
            }
        };
        Microsoft.Win32.SystemEvents.SessionSwitch += (_, change) =>
        {
            lock (gate)
            {
                switch (change.Reason)
                {
                    case SessionSwitchReason.SessionLock:
                        locked = true;
                        post(new SystemEvent.ScreenLocked(DateTimeOffset.Now));
                        break;
                    case SessionSwitchReason.SessionUnlock:
                        locked = false;
                        post(new SystemEvent.ScreenUnlocked());
                        break;
                }
            }
        };
        // Windows reports a time zone change as a time change too.
        Microsoft.Win32.SystemEvents.TimeChanged += (_, _) =>
        {
            post(new SystemEvent.ClockChanged());
            TimeZoneInfo.ClearCachedData();
            var current = IanaTimeZone();
            lock (gate)
            {
                if (current == timeZone)
                {
                    return;
                }
                timeZone = current;
            }
            post(new SystemEvent.TimeZoneChanged(current));
        };
        // Signing out or shutting down quits the app as its menu's Quit does, and waits for the
        // runtime's approval: the session ends right after this returns.
        Microsoft.Win32.SystemEvents.SessionEnding += (_, _) =>
        {
            lifetime.RequestQuit();
            lifetime.WaitForApproval(QuitWait);
        };
    }

    private void PathChanged() =>
        post(new SystemEvent.NetworkPathChanged(SystemFacts.PathSatisfied(), SystemFacts.PathInterfaces()));
}
