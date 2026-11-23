using System.Diagnostics.CodeAnalysis;
using FortiVPNManager.Contract.AppInternal;
using FortiVPNManager.OsIntegration;
using FortiVPNManager.Ui;
using FortiVPNManager.Ui.Text;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.Windows.AppLifecycle;
using Microsoft.Windows.AppNotifications;

namespace FortiVPNManager.App;

/// <summary>
/// Starts the runtime and joins it to the OS integration and the UI (components/os-integration/macos/AppEntry.swift).
/// The app lives in the notification area; quitting goes through the runtime, which ends the tunnel
/// and the log before it approves.
/// </summary>
[SuppressMessage("Maintainability", "CA1515:Consider making public types internal", Justification = "The XAML compiler generates the other part of App as public.")]
public sealed partial class App : Application, IDisposable
{
    private readonly DispatcherQueue _dispatcher = DispatcherQueue.GetForCurrentThread();
    private AppUi? _ui;
    private TrayIcon? _tray;
    private AppRouter? _router;
    private Notifier? _notifier;

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
        _notifier = new Notifier((identifier, button) => _dispatcher.TryEnqueue(() => _ui?.OpenNotification(identifier, button)));
        AppFacts facts = CurrentAppFacts.Read(GmailText.SignInPages());
        RuntimeLink? link = null;
        _ui = new AppUi(_dispatcher, command => _router!.Send(command), Notifier.Post);
        // The runtime sends nothing before the launch message, which Start sends; the link exists by then.
        _router = new AppRouter(facts.AppBuild, new UiBridge(_ui), Quit, message => link!.Send(message));
        link = RuntimeLink.Start(_router.Receive);
        _tray = new TrayIcon(_ui, () =>
        {
            // What is still typed in Settings reaches the runtime before it is asked to quit.
            _ui.FinishSettingsTyping();
            // The app is gone for the user at once while the runtime ends the tunnel, and a start
            // meanwhile waits for this process instead of handing itself to it (Program.Leave).
            Program.Leave();
            _ui.CloseSettings();
            _tray?.Dispose();
            _tray = null;
            _router.Lifetime.RequestQuit();
        });
        _router.Start(facts);
        AppActivationArguments activation = AppInstance.GetCurrent().GetActivatedEventArgs();
        if (activation.Kind == ExtendedActivationKind.AppNotification)
        {
            var notification = (AppNotificationActivatedEventArgs)activation.Data;
            _ui.OpenNotification(Notifier.Identifier(notification), Notifier.FromButton(notification));
        }
        Program.Launched(this);
    }

    /// <summary>The app was started again: a notification opens what it is about, anything else the settings window.</summary>
    public void Reopened(AppActivationArguments activation) => _dispatcher.TryEnqueue(() =>
    {
        if (activation.Kind == ExtendedActivationKind.AppNotification)
        {
            var notification = (AppNotificationActivatedEventArgs)activation.Data;
            _ui?.OpenNotification(Notifier.Identifier(notification), Notifier.FromButton(notification));
        }
        else
        {
            _ui?.Settings.Show(null);
        }
    });

    /// <summary>Ends the app once the runtime approved the quit; called from the runtime's reader thread.</summary>
    private void Quit() => _dispatcher.TryEnqueue(() =>
    {
        _ui?.CloseSettings();
        Dispose();
        Exit();
    });

    public void Dispose()
    {
        _tray?.Dispose();
        _notifier?.Dispose();
        GC.SuppressFinalize(this);
    }

    /// <summary>The UI as the router sees it.</summary>
    private sealed class UiBridge(AppUi ui) : IAppUi
    {
        public void Apply(Section section) => ui.Apply(section);

        public void AppendLog(IReadOnlyList<LogLine> lines) => ui.AppendLog(lines);

        public void PostNotification(UserNotification notification) => ui.PostNotification(notification);
    }
}
