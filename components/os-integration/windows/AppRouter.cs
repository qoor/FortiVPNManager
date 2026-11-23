using System.ComponentModel;
using System.Diagnostics;
using System.Reflection;
using FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>What the UI does for the runtime; the WinUI app implements it. Calls come on the runtime's reader thread.</summary>
public interface IAppUi
{
    void Apply(Section section);

    void AppendLog(IReadOnlyList<LogLine> lines);

    /// <summary>Builds the text and posts the notification, replacing an earlier one of the same kind.</summary>
    void PostNotification(UserNotification notification);
}

/// <summary>
/// Passes the runtime's messages on: sections, log lines, the settings request and notifications to
/// the UI, port requests and actions to the OS integration. Sends the answers to port requests and
/// the system's events to the runtime. The UI's commands go to the runtime through <see cref="Send"/>.
/// </summary>
public sealed class AppRouter
{
    /// <summary>Gives Windows time to start a Helper that stopped before a call is tried again.</summary>
    private static readonly TimeSpan s_helperRetryDelay = TimeSpan.FromMilliseconds(1500);

    private readonly IAppUi _ui;
    private readonly Action<ToRuntime> _send;
    private readonly HelperRelay _relay;
    private readonly SystemEvents _events;
    private readonly UpdateWaiter _waiter = new();

    /// <param name="quit">Ends the app once the runtime approves a quit.</param>
    public AppRouter(string appBuild, IAppUi ui, Action quit, Action<ToRuntime> send)
    {
        this._ui = ui;
        this._send = send;
        void post(SystemEvent systemEvent) => send(new ToRuntimeEvent(systemEvent));
        Lifetime = new AppLifetime(post, quit);
        _relay = new HelperRelay(appBuild, new HelperInbound(post), PipeHelperChannel.Open, s_helperRetryDelay);
        _events = new SystemEvents(post, Lifetime);
    }

    public AppLifetime Lifetime { get; }

    /// <summary>Reports the app's facts, which the runtime takes before anything else, then the system's events.</summary>
    public void Start(AppFacts facts)
    {
        _send(new ToRuntimeLaunched(facts));
        _events.Start();
    }

    /// <summary>A command from the UI.</summary>
    public void Send(Command command) => _send(new ToRuntimeCommand(command));

    public void Receive(FromRuntime message)
    {
        switch (message)
        {
            case FromRuntimeSection section:
                _ui.Apply(section.Value);
                break;
            case FromRuntimeLogLines lines:
                _ui.AppendLog(lines.Lines);
                break;
            case FromRuntimePortRequest request:
                Answer(request.Request, response => _send(new ToRuntimePortResponse(request.Id, response)));
                break;
            case FromRuntimePortAction action:
                Perform(action.Value);
                break;
            // The app starts in the notification area only, even when setup is needed; Settings opens
            // from the icon's double-click or its menu (user decision, 2026-11-13).
            case FromRuntimeOpenSettings:
                break;
            default:
                throw new InvalidOperationException($"unknown message from the runtime {message}");
        }
    }

    private void Answer(PortRequest request, Action<PortResponse> reply)
    {
        switch (request)
        {
            case PortRequestHelperRegistration:
                reply(ServiceRegistration.HelperStatus());
                break;
            case PortRequestRegisterHelper:
                reply(ServiceRegistration.RegisterHelper());
                break;
            // Opening the pipe can take seconds, so a Helper call never runs on the runtime's reader thread.
            case PortRequestHelperBuild:
                Watch(Task.Run(() => _relay.Build(reply)));
                break;
            case PortRequestCallHelper call:
                Watch(Task.Run(() => _relay.Handle(call.Call, reply)));
                break;
            case PortRequestRouteInterface route:
                reply(new PortResponseRouteInterface(SystemFacts.RouteInterface(route.Destination)));
                break;
            case PortRequestPrimaryRouter:
                reply(new PortResponsePrimaryRouter(SystemFacts.PrimaryRouter()));
                break;
            case PortRequestScreenState:
                // Displays off comes with the UI, which receives that event.
                reply(new PortResponseScreenState(_events.Locked, DisplaysAsleep: false));
                break;
            case PortRequestOpenBrowser browser:
                reply(new PortResponseBrowserOpened(OpenBrowser(browser.Url)));
                break;
            case PortRequestLoginItemRegistration:
                reply(ServiceRegistration.LoginItemStatus());
                break;
            case PortRequestSetLoginItem login:
                reply(ServiceRegistration.SetLoginItem(login.Enabled));
                break;
            case PortRequestFortiClientConnections:
                reply(new PortResponseFortiClientConnections(FortiClientSettings.Connections()));
                break;
            case PortRequestStartUpdateWaiter:
                reply(_waiter.Start());
                break;
            default:
                throw new InvalidOperationException($"unknown port request {request}");
        }
    }

    private void Perform(PortAction action)
    {
        switch (action)
        {
            case PortActionLog log:
                SystemLog.Write(log.Level, log.Category, log.Message);
                break;
            case PortActionPostNotification notification:
                _ui.PostNotification(notification.Notification);
                break;
            // Windows neither suspends a desktop app nor asks the user to approve the Helper.
            case PortActionHoldActivity or PortActionOpenHelperApprovalSettings:
                break;
            case PortActionUseHelperTeam:
                break;
            case PortActionReleaseUpdateWaiter release:
                _waiter.Release(release.InstallerProcessId);
                break;
            case PortActionStopUpdateWaiter:
                _waiter.Stop();
                break;
            case PortActionApproveQuit:
                Lifetime.ApproveQuit();
                break;
            default:
                throw new InvalidOperationException($"unknown port action {action}");
        }
    }

    /// <summary>The default browser, for web addresses only.</summary>
    private static bool OpenBrowser(string url)
    {
        if (!Uri.TryCreate(url, UriKind.Absolute, out Uri? uri)
            || (uri.Scheme != Uri.UriSchemeHttps && uri.Scheme != Uri.UriSchemeHttp))
        {
            return false;
        }
        try
        {
            using var _ = Process.Start(new ProcessStartInfo(uri.AbsoluteUri) { UseShellExecute = true });
            return true;
        }
        catch (Win32Exception)
        {
            return false;
        }
    }

    /// <summary>A Helper call answers through its reply; anything else it throws is a fault, which stops the app.</summary>
    private static void Watch(Task call) =>
        call.ContinueWith(
            failed => Environment.FailFast("a Helper call failed", failed.Exception),
            CancellationToken.None,
            TaskContinuationOptions.OnlyOnFaulted,
            TaskScheduler.Default);
}

/// <summary>The running app's facts for the runtime.</summary>
public static class CurrentAppFacts
{
    /// <summary>
    /// The build comes from the app's informational version, which the build script sets for the app
    /// and the Helper alike, and the version from its assembly version, which the build script sets to
    /// the installer's. The built-in OAuth client is a file the build script puts next to the
    /// app (packaging/GoogleOAuthClient.json, gitignored), the one the macOS build uses.
    /// </summary>
    /// <param name="signInPages">The pages the Google sign-in listener shows, in the UI's language.</param>
    public static AppFacts Read(SignInPages signInPages)
    {
        string executable = Environment.ProcessPath ?? "";
        string build = Assembly.GetEntryAssembly()
            ?.GetCustomAttribute<AssemblyInformationalVersionAttribute>()
            ?.InformationalVersion ?? "unknown";
        string client = Path.Combine(Path.GetDirectoryName(executable) ?? "", "GoogleOAuthClient.json");
        return new AppFacts(
            build,
            Assembly.GetEntryAssembly()!.GetName().Version!.ToString(3),
            ServiceRegistration.IsInstalledCopy(executable),
            SystemEvents.IanaTimeZone(),
            signInPages,
            File.Exists(client) ? File.ReadAllText(client) : null);
    }
}
