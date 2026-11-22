using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract;

public static partial class AppInternal
{
    /// <summary>What the runtime asks of the OS (components/contract/src/app_internal/port.rs).</summary>
    [JsonConverter(typeof(CasesConverter<PortRequest>))]
    public abstract record PortRequest
    {
        private PortRequest() { }

        public sealed record HelperRegistration : PortRequest;
        public sealed record RegisterHelper : PortRequest;

        /// <summary>
        /// A connection whose Helper answers with the app's build counts as verified. Retried once on
        /// a fresh connection when the connection fails.
        /// </summary>
        public sealed record HelperBuild : PortRequest;

        public sealed record CallHelper(HelperCall Call) : PortRequest;

        public sealed record RouteInterface(string Destination) : PortRequest;
        public sealed record PrimaryRouter : PortRequest;
        public sealed record ScreenState : PortRequest;

        public sealed record OpenBrowser(string Url) : PortRequest;
        public sealed record LoginItemRegistration : PortRequest;
        public sealed record SetLoginItem(bool Enabled) : PortRequest;

        /// <summary>The official FortiClient's SSL VPN connections, read only; Windows imports its settings from them.</summary>
        public sealed record FortiClientConnections : PortRequest;

        public sealed record StartUpdateWaiter(ulong InstallId) : PortRequest;
    }

    [JsonConverter(typeof(CasesConverter<PortResponse>))]
    public abstract record PortResponse
    {
        private PortResponse() { }

        public sealed record Registration(RegistrationStatus Status) : PortResponse;
        public sealed record Registered : PortResponse;
        public sealed record RegistrationFailed(RegistrationError Error) : PortResponse;
        public sealed record HelperBuild(string Build) : PortResponse;

        /// <summary>The Helper's reply as received; null for an empty (success) reply.</summary>
        public sealed record HelperReply(string? Json = null) : PortResponse;

        public sealed record HelperReplyFlag(bool Value) : PortResponse;
        public sealed record HelperUnreachable(HelperChannelFailure Failure) : PortResponse;

        /// <summary>
        /// A call that needs a verified connection was not sent. The value's C# name differs from
        /// its key, since a case inherits its sibling <see cref="HelperBuild"/>'s name.
        /// </summary>
        public sealed record HelperBuildMismatch([property: JsonPropertyName("helperBuild")] string Build) : PortResponse;

        public sealed record RouteInterface([property: JsonPropertyName("interface")] string? Name = null) : PortResponse;
        public sealed record PrimaryRouter(string? Router = null) : PortResponse;
        public sealed record ScreenState(bool Locked, bool DisplaysAsleep) : PortResponse;

        public sealed record BrowserOpened(bool Opened) : PortResponse;

        public sealed record FortiClientConnections(IReadOnlyList<FortiClientConnection> Connections) : PortResponse;

        public sealed record UpdateWaiterStarted : PortResponse;
        public sealed record UpdateWaiterFailed(string Description) : PortResponse;
    }

    /// <summary>Carried out without an answer.</summary>
    [JsonConverter(typeof(CasesConverter<PortAction>))]
    public abstract record PortAction
    {
        private PortAction() { }

        public sealed record Log(LogLevel Level, LogCategory Category, string Message) : PortAction;

        /// <summary>
        /// Replaces an earlier notification of the same kind. The UI builds the text; the OS
        /// integration only posts it.
        /// </summary>
        public sealed record PostNotification(UserNotification Notification) : PortAction;

        public sealed record HoldActivity(bool Hold) : PortAction;
        public sealed record OpenHelperApprovalSettings : PortAction;
        public sealed record UseHelperTeam(string? TeamId = null) : PortAction;
        public sealed record ReleaseUpdateWaiter(uint InstallerProcessId) : PortAction;
        public sealed record StopUpdateWaiter : PortAction;

        /// <summary>
        /// The runtime's last message. Also comes unasked once an update's installer started: then
        /// the app quits.
        /// </summary>
        public sealed record ApproveQuit : PortAction;
    }

    public sealed record SecretStoreFailure(int Code, string? Description = null);

    /// <summary>
    /// One call on the Helper over the one connection the OS integration keeps (see
    /// components/contract/src/app_internal/port.rs). JSON payloads and the attempt and reason
    /// values are relayed as they are, never read.
    /// </summary>
    [JsonConverter(typeof(CasesConverter<HelperCall>))]
    public abstract record HelperCall
    {
        private HelperCall() { }

        public sealed record Connect(string Json) : HelperCall
        {
            public override string ToString() => "Connect { Json = <redacted> }";
        }

        public sealed record SubmitOtp(string Attempt, string Code) : HelperCall
        {
            public override string ToString() => $"SubmitOtp {{ Attempt = {Attempt}, Code = <redacted> }}";
        }

        public sealed record Disconnect(string Attempt, string Reason) : HelperCall;
        public sealed record Status : HelperCall;

        /// <summary>After the reply, drop the connection, as the runtime expects.</summary>
        public sealed record ExitWhenIdle : HelperCall;

        public sealed record ReadBypassRoutes : HelperCall;
        public sealed record WriteBypassRoutes(string Json) : HelperCall;
        public sealed record PrepareUpdate(string Json) : HelperCall;
        public sealed record InstallUpdate(string Json) : HelperCall;
        public sealed record ReadPreparedUpdate : HelperCall;
        public sealed record DiscardPreparedUpdate : HelperCall;
    }

    [JsonConverter(typeof(CasesConverter<HelperChannelFailure>))]
    public abstract record HelperChannelFailure
    {
        private HelperChannelFailure() { }

        public sealed record AppNotSigned : HelperChannelFailure;
        public sealed record Connection(string Description) : HelperChannelFailure;
        public sealed record ConnectionLost : HelperChannelFailure;
        public sealed record NoReply : HelperChannelFailure;
        public sealed record UnreadableReply(string Description) : HelperChannelFailure;
    }

    [JsonConverter(typeof(EnumNameConverter<RegistrationStatus>))]
    public enum RegistrationStatus
    {
        NotRegistered,
        Enabled,
        RequiresApproval,
        NotFound,
    }

    [JsonConverter(typeof(CasesConverter<RegistrationError>))]
    public abstract record RegistrationError
    {
        private RegistrationError() { }

        public sealed record NotInstalledCopy : RegistrationError;
        public sealed record Failed(string Description) : RegistrationError;
    }

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
    public abstract record UserNotification
    {
        private UserNotification() { }

        public sealed record ExceptionRulesNotApplied(uint Retries, BypassFailure Failure) : UserNotification;
        public sealed record UpdateAvailable(string Version) : UserNotification;
    }
}
