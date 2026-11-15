using System.Diagnostics;
using System.Runtime.InteropServices;
using FortiVPNManager.OsIntegration;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;

namespace FortiVPNManager.App;

/// <summary>
/// The entry: one copy per user. Starting the app again, from the Start menu or a notification,
/// hands the start to the running copy, which shows its settings window (as reopening the app does
/// on macOS), and ends. The running copy shows nothing until the Helper's service runs; when it
/// cannot be made to, a message box says why and the app quits.
/// </summary>
public static class Program
{
    private const string InstanceKey = "FortiVPNManager";

    /// <summary>
    /// MB_OK with MB_ICONERROR, MB_SETFOREGROUND and MB_TOPMOST: the box can come long after the
    /// launch, when another window is in front.
    /// </summary>
    private const uint ErrorBox = 0x10 | 0x10000 | 0x40000;

    /// <summary>
    /// Held by the running copy until its process ends. A copy that quits gives up its key at once and
    /// ends the tunnel unseen, up to the runtime's quit limit; a copy started meanwhile becomes the
    /// running one and waits here for that process to end, as if the user had waited. Before, such a
    /// start handed itself to the quitting copy and ended with it (user decision, 2026-11-15).
    /// </summary>
    private const string RunningName = @"Local\FortiVPNManager.Running";

    private static readonly object LaunchGate = new();
    private static AppInstance? instance;
    private static Mutex? running;
    private static App? launched;
    private static AppActivationArguments? reopenedEarly;
    private static bool leaving;

    [STAThread]
    public static void Main()
    {
        // A fault stops the app; the event log keeps what it was, as macOS keeps a crash report. Set
        // first, so it covers the wait for the Helper and every thread after it.
        AppDomain.CurrentDomain.UnhandledException += (_, args) => SystemLog.WriteFault((Exception)args.ExceptionObject);
        WinRT.ComWrappersSupport.InitializeComWrappers();
        var main = AppInstance.FindOrRegisterForKey(InstanceKey);
        if (!main.IsCurrent)
        {
            var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
            // The handover is a call to the other process, made off this single-threaded thread.
            Task.Run(() => main.RedirectActivationToAsync(activation).AsTask()).Wait();
            return;
        }
        instance = main;
        running = new Mutex(false, RunningName);
        try
        {
            running.WaitOne();
        }
        catch (AbandonedMutexException)
        {
            // The copy before ended, as a quitting one does, without releasing it.
        }
        // Windows calls this on a pool thread, and the other copy waits until it returns, so it never
        // waits for this copy's launch.
        main.Activated += (_, args) => Reopened(args);
        switch (HelperStart.Run())
        {
            case HelperStartResult.SessionEnding:
                return;
            case HelperStartResult.Failed failed:
                _ = MessageBoxW(0, Message(failed.Reason), "FortiVPNManager", ErrorBox);
                return;
        }
        Application.Start(start =>
        {
            SynchronizationContext.SetSynchronizationContext(new DispatcherQueueSynchronizationContext(DispatcherQueue.GetForCurrentThread()));
            _ = new App();
        });
    }

    /// <summary>The app has launched: a start that came before it is shown now.</summary>
    /// <remarks>
    /// Both hand a start on under the lock, so a later start is never shown before an earlier one.
    /// </remarks>
    internal static void Launched(App app)
    {
        lock (LaunchGate)
        {
            launched = app;
            if (reopenedEarly is { } early)
            {
                app.Reopened(early);
            }
        }
    }

    /// <summary>
    /// The app quits: starts from now on go to a new copy, which waits for this process to end. A
    /// start already on its way here is dropped, as this copy shows nothing more.
    /// </summary>
    internal static void Leave()
    {
        lock (LaunchGate)
        {
            leaving = true;
        }
        instance?.UnregisterKey();
    }

    /// <summary>A start of another copy, kept until the app has launched; only the latest counts.</summary>
    private static void Reopened(AppActivationArguments activation)
    {
        lock (LaunchGate)
        {
            if (leaving)
            {
                return;
            }
            if (launched is null)
            {
                reopenedEarly = activation;
                return;
            }
            launched.Reopened(activation);
        }
    }

    /// <summary>The start that timed out or stopped with an error was the second; the others fail at once.</summary>
    private static string Message(HelperStartFailure failure) => failure switch
    {
        HelperStartFailure.NotInstalled => HelperStartText.Message(HelperStartText.NotInstalled()),
        HelperStartFailure.Disabled => HelperStartText.Message(HelperStartText.Disabled()),
        HelperStartFailure.NoPermission => HelperStartText.Message(HelperStartText.NoPermission()),
        HelperStartFailure.TimedOut => HelperStartText.MessageAfterRetry(HelperStartText.TimedOut()),
        HelperStartFailure.ErrorWhileStarting => HelperStartText.MessageAfterRetry(HelperStartText.StoppedWithError()),
        _ => throw new UnreachableException(),
    };

    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int MessageBoxW(nint owner, string text, string caption, uint type);
}
