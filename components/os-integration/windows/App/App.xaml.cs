using FortiVPNManager.Contract;
using FortiVPNManager.OsIntegration;
using FortiVPNManager.Ui;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
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
        Running = this;
        // A fault stops the app; the event log keeps what it was, as macOS keeps a crash report.
        UnhandledException += (_, args) => SystemLog.WriteFault(args.Exception);
        // The same for a fault on another thread, such as the runtime link's reader and writer.
        AppDomain.CurrentDomain.UnhandledException += (_, args) => SystemLog.WriteFault((Exception)args.ExceptionObject);
    }

    /// <summary>The running app, which a second start hands its activation to.</summary>
    public static App? Running { get; private set; }

    protected override void OnLaunched(Microsoft.UI.Xaml.LaunchActivatedEventArgs args)
    {
        notifier = new Notifier(() => dispatcher.TryEnqueue(() => ui?.Settings.Show(SettingsPane.ExceptionRules)));
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
            router.Lifetime.RequestQuit();
        });
        router.Start(facts);
        if (AppInstance.GetCurrent().GetActivatedEventArgs().Kind == ExtendedActivationKind.AppNotification)
        {
            ui.Settings.Show(SettingsPane.ExceptionRules);
        }
    }

    /// <summary>The app was started again: a notification opens its pane, anything else the settings window.</summary>
    public void Reopened(AppActivationArguments activation) => dispatcher.TryEnqueue(() =>
        ui?.Settings.Show(activation.Kind == ExtendedActivationKind.AppNotification ? SettingsPane.ExceptionRules : null));

    /// <summary>Ends the app once the runtime approved the quit; called from the runtime's reader thread.</summary>
    private void Quit() => dispatcher.TryEnqueue(() =>
    {
        ui?.HideSettings();
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
