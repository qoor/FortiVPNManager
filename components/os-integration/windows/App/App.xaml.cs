using FortiVPNManager.Contract;
using FortiVPNManager.OsIntegration;
using FortiVPNManager.Ui;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using Microsoft.Windows.AppNotifications;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.App;

/// <summary>
/// Starts the runtime and joins it to the OS integration and the UI (components/os-integration/macos/AppEntry.swift).
/// The app lives in the notification area; quitting goes through the runtime, which ends the tunnel
/// and the log before it approves.
/// </summary>
public partial class App : Application
{
    private readonly DispatcherQueue dispatcher = DispatcherQueue.GetForCurrentThread();
    private AppUi? ui;
    private TrayIcon? tray;
    private AppRouter? router;
    private Notifier? notifier;

    public App()
    {
        InitializeComponent();
        // The settings window is the app's only window; closing it releases it, and the app lives on in
        // the notification area until it quits.
        DispatcherShutdownMode = DispatcherShutdownMode.OnExplicitShutdown;
        // A fault on the UI thread stops the app; the event log keeps what it was, as macOS keeps a
        // crash report. Program.Main does the same for the other threads.
        UnhandledException += (_, args) => SystemLog.WriteFault(args.Exception);
    }

    protected override void OnLaunched(Microsoft.UI.Xaml.LaunchActivatedEventArgs args)
    {
        notifier = new Notifier((identifier, button) => dispatcher.TryEnqueue(() => ui?.OpenNotification(identifier, button)));
        var facts = CurrentAppFacts.Read(GmailText.SignInPages());
        RuntimeLink? link = null;
        ui = new AppUi(dispatcher, command => router!.Send(command), notifier.Post);
        // The runtime sends nothing before the launch message, which Start sends; the link exists by then.
        router = new AppRouter(facts.AppBuild, new UiBridge(ui), Quit, message => link!.Send(message));
        link = RuntimeLink.Start(router.Receive);
        tray = new TrayIcon(ui, () =>
        {
            // What is still typed in Settings reaches the runtime before it is asked to quit.
            ui.FinishSettingsTyping();
            // The app is gone for the user at once while the runtime ends the tunnel, and a start
            // meanwhile waits for this process instead of handing itself to it (Program.Leave).
            Program.Leave();
            ui.CloseSettings();
            tray?.Dispose();
            tray = null;
            router.Lifetime.RequestQuit();
        });
        router.Start(facts);
        var activation = AppInstance.GetCurrent().GetActivatedEventArgs();
        if (activation.Kind == ExtendedActivationKind.AppNotification)
        {
            var notification = (AppNotificationActivatedEventArgs)activation.Data;
            ui.OpenNotification(Notifier.Identifier(notification), Notifier.FromButton(notification));
        }
        Program.Launched(this);
    }

    /// <summary>The app was started again: a notification opens what it is about, anything else the settings window.</summary>
    public void Reopened(AppActivationArguments activation) => dispatcher.TryEnqueue(() =>
    {
        if (activation.Kind == ExtendedActivationKind.AppNotification)
        {
            var notification = (AppNotificationActivatedEventArgs)activation.Data;
            ui?.OpenNotification(Notifier.Identifier(notification), Notifier.FromButton(notification));
        }
        else
        {
            ui?.Settings.Show(null);
        }
    });

    /// <summary>Ends the app once the runtime approved the quit; called from the runtime's reader thread.</summary>
    private void Quit() => dispatcher.TryEnqueue(() =>
    {
        ui?.CloseSettings();
        tray?.Dispose();
        notifier?.Dispose();
        Exit();
    });

    /// <summary>The UI as the router sees it.</summary>
    private sealed class UiBridge(AppUi ui) : IAppUi
    {
        public void Apply(Section section) => ui.Apply(section);

        public void AppendLog(IReadOnlyList<LogLine> lines) => ui.AppendLog(lines);

        public void PostNotification(UserNotification notification) => ui.PostNotification(notification);
    }
}
