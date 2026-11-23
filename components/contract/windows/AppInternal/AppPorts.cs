using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract.AppInternal;

/// <summary>What the runtime asks of the OS (components/contract/src/app_internal/port.rs).</summary>
[JsonConverter(typeof(CasesConverter<PortRequest>))]
[JsonCase(typeof(PortRequestHelperRegistration), "helperRegistration")]
[JsonCase(typeof(PortRequestRegisterHelper), "registerHelper")]
[JsonCase(typeof(PortRequestHelperBuild), "helperBuild")]
[JsonCase(typeof(PortRequestCallHelper), "callHelper")]
[JsonCase(typeof(PortRequestRouteInterface), "routeInterface")]
[JsonCase(typeof(PortRequestPrimaryRouter), "primaryRouter")]
[JsonCase(typeof(PortRequestScreenState), "screenState")]
[JsonCase(typeof(PortRequestOpenBrowser), "openBrowser")]
[JsonCase(typeof(PortRequestLoginItemRegistration), "loginItemRegistration")]
[JsonCase(typeof(PortRequestSetLoginItem), "setLoginItem")]
[JsonCase(typeof(PortRequestFortiClientConnections), "fortiClientConnections")]
[JsonCase(typeof(PortRequestStartUpdateWaiter), "startUpdateWaiter")]
public abstract record PortRequest
{
    private protected PortRequest() { }
}

public sealed record PortRequestHelperRegistration : PortRequest;

public sealed record PortRequestRegisterHelper : PortRequest;

/// <summary>
/// A connection whose Helper answers with the app's build counts as verified. Retried once on
/// a fresh connection when the connection fails.
/// </summary>
public sealed record PortRequestHelperBuild : PortRequest;

public sealed record PortRequestCallHelper(HelperCall Call) : PortRequest;

public sealed record PortRequestRouteInterface(string Destination) : PortRequest;

public sealed record PortRequestPrimaryRouter : PortRequest;

public sealed record PortRequestScreenState : PortRequest;

public sealed record PortRequestOpenBrowser(string Url) : PortRequest;

public sealed record PortRequestLoginItemRegistration : PortRequest;

public sealed record PortRequestSetLoginItem(bool Enabled) : PortRequest;

/// <summary>The official FortiClient's SSL VPN connections, read only; Windows imports its settings from them.</summary>
public sealed record PortRequestFortiClientConnections : PortRequest;

public sealed record PortRequestStartUpdateWaiter(ulong InstallId) : PortRequest;

[JsonConverter(typeof(CasesConverter<PortResponse>))]
[JsonCase(typeof(PortResponseRegistration), "registration")]
[JsonCase(typeof(PortResponseRegistered), "registered")]
[JsonCase(typeof(PortResponseRegistrationFailed), "registrationFailed")]
[JsonCase(typeof(PortResponseHelperBuild), "helperBuild")]
[JsonCase(typeof(PortResponseHelperReply), "helperReply")]
[JsonCase(typeof(PortResponseHelperReplyFlag), "helperReplyFlag")]
[JsonCase(typeof(PortResponseHelperUnreachable), "helperUnreachable")]
[JsonCase(typeof(PortResponseHelperBuildMismatch), "helperBuildMismatch")]
[JsonCase(typeof(PortResponseRouteInterface), "routeInterface")]
[JsonCase(typeof(PortResponsePrimaryRouter), "primaryRouter")]
[JsonCase(typeof(PortResponseScreenState), "screenState")]
[JsonCase(typeof(PortResponseBrowserOpened), "browserOpened")]
[JsonCase(typeof(PortResponseFortiClientConnections), "fortiClientConnections")]
[JsonCase(typeof(PortResponseUpdateWaiterStarted), "updateWaiterStarted")]
[JsonCase(typeof(PortResponseUpdateWaiterFailed), "updateWaiterFailed")]
public abstract record PortResponse
{
    private protected PortResponse() { }
}

public sealed record PortResponseRegistration(RegistrationStatus Status) : PortResponse;

public sealed record PortResponseRegistered : PortResponse;

public sealed record PortResponseRegistrationFailed(RegistrationError Error) : PortResponse;

public sealed record PortResponseHelperBuild(string Build) : PortResponse;

/// <summary>The Helper's reply as received; null for an empty (success) reply.</summary>
public sealed record PortResponseHelperReply(string? Json = null) : PortResponse;

public sealed record PortResponseHelperReplyFlag(bool Value) : PortResponse;

public sealed record PortResponseHelperUnreachable(HelperChannelFailure Failure) : PortResponse;

/// <summary>
/// A call that needs a verified connection was not sent. The value's C# name differs from
/// its key, since a case inherits its sibling <see cref="PortResponseHelperBuild"/>'s name.
/// </summary>
public sealed record PortResponseHelperBuildMismatch([property: JsonPropertyName("helperBuild")] string Build) : PortResponse;

public sealed record PortResponseRouteInterface([property: JsonPropertyName("interface")] string? Name = null) : PortResponse;

public sealed record PortResponsePrimaryRouter(string? Router = null) : PortResponse;

public sealed record PortResponseScreenState(bool Locked, bool DisplaysAsleep) : PortResponse;

public sealed record PortResponseBrowserOpened(bool Opened) : PortResponse;

public sealed record PortResponseFortiClientConnections(IReadOnlyList<FortiClientConnection> Connections) : PortResponse;

public sealed record PortResponseUpdateWaiterStarted : PortResponse;

public sealed record PortResponseUpdateWaiterFailed(string Description) : PortResponse;

/// <summary>Carried out without an answer.</summary>
[JsonConverter(typeof(CasesConverter<PortAction>))]
[JsonCase(typeof(PortActionLog), "log")]
[JsonCase(typeof(PortActionPostNotification), "postNotification")]
[JsonCase(typeof(PortActionHoldActivity), "holdActivity")]
[JsonCase(typeof(PortActionOpenHelperApprovalSettings), "openHelperApprovalSettings")]
[JsonCase(typeof(PortActionUseHelperTeam), "useHelperTeam")]
[JsonCase(typeof(PortActionReleaseUpdateWaiter), "releaseUpdateWaiter")]
[JsonCase(typeof(PortActionStopUpdateWaiter), "stopUpdateWaiter")]
[JsonCase(typeof(PortActionApproveQuit), "approveQuit")]
public abstract record PortAction
{
    private protected PortAction() { }
}

public sealed record PortActionLog(LogLevel Level, LogCategory Category, string Message) : PortAction;

/// <summary>
/// Replaces an earlier notification of the same kind. The UI builds the text; the OS
/// integration only posts it.
/// </summary>
public sealed record PortActionPostNotification(UserNotification Notification) : PortAction;

public sealed record PortActionHoldActivity(bool Hold) : PortAction;

public sealed record PortActionOpenHelperApprovalSettings : PortAction;

public sealed record PortActionUseHelperTeam(string? TeamId = null) : PortAction;

public sealed record PortActionReleaseUpdateWaiter(uint InstallerProcessId) : PortAction;

public sealed record PortActionStopUpdateWaiter : PortAction;

/// <summary>
/// The runtime's last message. Also comes unasked once an update's installer started: then
/// the app quits.
/// </summary>
public sealed record PortActionApproveQuit : PortAction;

public sealed record SecretStoreFailure(int Code, string? Description = null);

/// <summary>
/// One call on the Helper over the one connection the OS integration keeps (see
/// components/contract/src/app_internal/port.rs). JSON payloads and the attempt and reason
/// values are relayed as they are, never read.
/// </summary>
[JsonConverter(typeof(CasesConverter<HelperCall>))]
[JsonCase(typeof(HelperCallConnect), "connect")]
[JsonCase(typeof(HelperCallSubmitOtp), "submitOtp")]
[JsonCase(typeof(HelperCallDisconnect), "disconnect")]
[JsonCase(typeof(HelperCallStatus), "status")]
[JsonCase(typeof(HelperCallExitWhenIdle), "exitWhenIdle")]
[JsonCase(typeof(HelperCallReadBypassRoutes), "readBypassRoutes")]
[JsonCase(typeof(HelperCallWriteBypassRoutes), "writeBypassRoutes")]
[JsonCase(typeof(HelperCallPrepareUpdate), "prepareUpdate")]
[JsonCase(typeof(HelperCallInstallUpdate), "installUpdate")]
[JsonCase(typeof(HelperCallReadPreparedUpdate), "readPreparedUpdate")]
[JsonCase(typeof(HelperCallDiscardPreparedUpdate), "discardPreparedUpdate")]
public abstract record HelperCall
{
    private protected HelperCall() { }
}

public sealed record HelperCallConnect(string Json) : HelperCall
{
    public override string ToString() => "Connect { Json = <redacted> }";
}

public sealed record HelperCallSubmitOtp(string Attempt, string Code) : HelperCall
{
    public override string ToString() => $"SubmitOtp {{ Attempt = {Attempt}, Code = <redacted> }}";
}

public sealed record HelperCallDisconnect(string Attempt, string Reason) : HelperCall;

public sealed record HelperCallStatus : HelperCall;

/// <summary>After the reply, drop the connection, as the runtime expects.</summary>
public sealed record HelperCallExitWhenIdle : HelperCall;

public sealed record HelperCallReadBypassRoutes : HelperCall;

public sealed record HelperCallWriteBypassRoutes(string Json) : HelperCall;

public sealed record HelperCallPrepareUpdate(string Json) : HelperCall;

public sealed record HelperCallInstallUpdate(string Json) : HelperCall;

public sealed record HelperCallReadPreparedUpdate : HelperCall;

public sealed record HelperCallDiscardPreparedUpdate : HelperCall;

[JsonConverter(typeof(CasesConverter<HelperChannelFailure>))]
[JsonCase(typeof(HelperChannelFailureAppNotSigned), "appNotSigned")]
[JsonCase(typeof(HelperChannelFailureConnection), "connection")]
[JsonCase(typeof(HelperChannelFailureConnectionLost), "connectionLost")]
[JsonCase(typeof(HelperChannelFailureNoReply), "noReply")]
[JsonCase(typeof(HelperChannelFailureUnreadableReply), "unreadableReply")]
public abstract record HelperChannelFailure
{
    private protected HelperChannelFailure() { }
}

public sealed record HelperChannelFailureAppNotSigned : HelperChannelFailure;

public sealed record HelperChannelFailureConnection(string Description) : HelperChannelFailure;

public sealed record HelperChannelFailureConnectionLost : HelperChannelFailure;

public sealed record HelperChannelFailureNoReply : HelperChannelFailure;

public sealed record HelperChannelFailureUnreadableReply(string Description) : HelperChannelFailure;

[JsonConverter(typeof(EnumNameConverter<RegistrationStatus>))]
public enum RegistrationStatus
{
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
}

[JsonConverter(typeof(CasesConverter<RegistrationError>))]
[JsonCase(typeof(RegistrationErrorNotInstalledCopy), "notInstalledCopy")]
[JsonCase(typeof(RegistrationErrorFailed), "failed")]
public abstract record RegistrationError
{
    private protected RegistrationError() { }
}

public sealed record RegistrationErrorNotInstalledCopy : RegistrationError;

public sealed record RegistrationErrorFailed(string Description) : RegistrationError;

/// <summary>A FortiClient SSL VPN connection: its name and its Server value, usually host:port.</summary>
public sealed record FortiClientConnection(string Name, string Server);

[JsonConverter(typeof(EnumNameConverter<LogLevel>))]
public enum LogLevel
{
    Info,
    Notice,
    Error,
}

/// <summary>The log category.</summary>
[JsonConverter(typeof(EnumNameConverter<LogCategory>))]
public enum LogCategory
{
    Connection,
    Readiness,
    Config,
    State,
    Logfile,
    Import,
    Oauth,
    Otp,

    [JsonStringEnumMemberName("bypass-routes")]
    BypassRoutes,

    [JsonStringEnumMemberName("login-item")]
    LoginItem,

    Update,
}

[JsonConverter(typeof(CasesConverter<UserNotification>))]
[JsonCase(typeof(UserNotificationExceptionRulesNotApplied), "exceptionRulesNotApplied")]
[JsonCase(typeof(UserNotificationUpdateAvailable), "updateAvailable")]
public abstract record UserNotification
{
    private protected UserNotification() { }
}

public sealed record UserNotificationExceptionRulesNotApplied(uint Retries, BypassFailure Failure) : UserNotification;

public sealed record UserNotificationUpdateAvailable(string Version) : UserNotification;
