using System.ComponentModel;
using System.Diagnostics;
using System.Reflection;
using static FortiVPNManager.Contract.AppInternal;

namespace FortiVPNManager.OsIntegration;

/// <summary>What the UI does for the runtime; the WinUI app implements it. Calls come on the runtime's reader thread.</summary>
public interface IAppUi
{
    void Apply(Section section);

    void AppendLog(IReadOnlyList<LogLine> lines);

    void OpenSettings();

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
    private static readonly TimeSpan HelperRetryDelay = TimeSpan.FromMilliseconds(1500);

    private readonly IAppUi ui;
    private readonly Action<ToRuntime> send;
    private readonly HelperRelay relay;
    private readonly SystemEvents events;

    /// <param name="quit">Ends the app once the runtime approves a quit.</param>
    public AppRouter(string appBuild, IAppUi ui, Action quit, Action<ToRuntime> send)
    {
        this.ui = ui;
        this.send = send;
        Action<SystemEvent> post = systemEvent => send(new ToRuntime.Event(systemEvent));
        Lifetime = new AppLifetime(post, quit);
        relay = new HelperRelay(appBuild, new HelperInbound(post), PipeHelperChannel.Open, HelperRetryDelay);
        events = new SystemEvents(post, Lifetime);
    }

    public AppLifetime Lifetime { get; }

    /// <summary>Reports the app's facts, which the runtime takes before anything else, then the system's events.</summary>
    public void Start(AppFacts facts)
    {
        send(new ToRuntime.Launched(facts));
        events.Start();
    }

    /// <summary>A command from the UI.</summary>
    public void Send(Command command) => send(new ToRuntime.Command(command));

    public void Receive(FromRuntime message)
    {
        switch (message)
        {
            case FromRuntime.Section section:
                ui.Apply(section.Value);
                break;
            case FromRuntime.LogLines lines:
                ui.AppendLog(lines.Lines);
                break;
            case FromRuntime.PortRequest request:
                Answer(request.Request, response => send(new ToRuntime.PortResponse(request.Id, response)));
                break;
            case FromRuntime.PortAction action:
                Perform(action.Value);
                break;
            case FromRuntime.OpenSettings:
                ui.OpenSettings();
                break;
        }
    }

    private void Answer(PortRequest request, Action<PortResponse> reply)
    {
        switch (request)
        {
            case PortRequest.ReadSecret read:
                reply(SecretStore.Read(read.Item));
                break;
            case PortRequest.HasSecret has:
                reply(SecretStore.Exists(has.Item));
                break;
            case PortRequest.WriteSecret write:
                reply(SecretStore.Write(write.Value, write.Item));
                break;
            case PortRequest.DeleteSecret delete:
                reply(SecretStore.Delete(delete.Item));
                break;
            case PortRequest.HelperRegistration:
                reply(ServiceRegistration.HelperStatus());
                break;
            case PortRequest.RegisterHelper:
                reply(ServiceRegistration.RegisterHelper());
                break;
            // Opening the pipe can take seconds, so a Helper call never runs on the runtime's reader thread.
            case PortRequest.HelperBuild:
                Watch(Task.Run(() => relay.Build(reply)));
                break;
            case PortRequest.CallHelper call:
                Watch(Task.Run(() => relay.Handle(call.Call, reply)));
                break;
            case PortRequest.RouteInterface route:
                reply(new PortResponse.RouteInterface(SystemFacts.RouteInterface(route.Destination)));
                break;
            case PortRequest.InterfaceAddresses:
                reply(new PortResponse.InterfaceAddresses(SystemFacts.InterfaceAddresses()));
                break;
            case PortRequest.PrimaryRouter:
                reply(new PortResponse.PrimaryRouter(SystemFacts.PrimaryRouter()));
                break;
            case PortRequest.ScreenState:
                // Displays off comes with the UI, which receives that event.
                reply(new PortResponse.ScreenState(events.Locked, DisplaysAsleep: false));
                break;
            case PortRequest.OpenBrowser browser:
                reply(new PortResponse.BrowserOpened(OpenBrowser(browser.Url)));
                break;
            case PortRequest.LoginItemRegistration:
                reply(ServiceRegistration.LoginItemStatus());
                break;
            case PortRequest.SetLoginItem login:
                reply(ServiceRegistration.SetLoginItem(login.Enabled));
                break;
        }
    }

    private void Perform(PortAction action)
    {
        switch (action)
        {
            case PortAction.Log log:
                SystemLog.Write(log.Level, log.Category, log.Message);
                break;
            case PortAction.PostNotification notification:
                ui.PostNotification(notification.Notification);
                break;
            // Windows neither suspends a desktop app nor asks the user to approve the Helper.
            case PortAction.HoldActivity or PortAction.OpenHelperApprovalSettings:
                break;
            case PortAction.ApproveQuit:
                Lifetime.ApproveQuit();
                break;
        }
    }

    /// <summary>The default browser, for web addresses only.</summary>
    private static bool OpenBrowser(string url)
    {
        if (!Uri.TryCreate(url, UriKind.Absolute, out var uri)
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
            TaskContinuationOptions.OnlyOnFaulted);
}

/// <summary>The running app's facts for the runtime.</summary>
public static class CurrentAppFacts
{
    /// <summary>
    /// The build comes from the app's informational version, which the build script sets for the app
    /// and the Helper alike. The built-in OAuth client is a file the build script puts next to the
    /// app (packaging/windows, gitignored), as on macOS.
    /// </summary>
    /// <param name="signInPages">The pages the Google sign-in listener shows, in the UI's language.</param>
    public static AppFacts Read(SignInPages signInPages)
    {
        var executable = Environment.ProcessPath ?? "";
        var build = Assembly.GetEntryAssembly()
            ?.GetCustomAttribute<AssemblyInformationalVersionAttribute>()
            ?.InformationalVersion ?? "unknown";
        var client = Path.Combine(Path.GetDirectoryName(executable) ?? "", "GoogleOAuthClient.json");
        return new AppFacts(
            build,
            ServiceRegistration.IsInstalledCopy(executable),
            SystemEvents.IanaTimeZone(),
            signInPages,
            File.Exists(client) ? File.ReadAllText(client) : null);
    }
}
