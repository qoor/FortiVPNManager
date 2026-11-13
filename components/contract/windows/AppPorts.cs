using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract;

public static partial class AppInternal
{
    /// <summary>What the runtime asks of the OS (components/contract/src/app_internal/port.rs).</summary>
    [Cases]
    public abstract record PortRequest
    {
        private PortRequest() { }

        public sealed record ReadSecret(SecretItem Item) : PortRequest;

        /// <summary>Never reads the secret, so the OS asks the user for nothing.</summary>
        public sealed record HasSecret(SecretItem Item) : PortRequest;

        public sealed record WriteSecret(SecretItem Item, string Value) : PortRequest
        {
            public override string ToString() => $"WriteSecret {{ Item = {Item}, Value = <redacted> }}";
        }

        /// <summary>Deleting a missing item succeeds.</summary>
        public sealed record DeleteSecret(SecretItem Item) : PortRequest;

        public sealed record HelperRegistration : PortRequest;
        public sealed record RegisterHelper : PortRequest;

        /// <summary>
        /// A connection whose Helper answers with the app's build counts as verified. Retried once on
        /// a fresh connection when the connection fails.
        /// </summary>
        public sealed record HelperBuild : PortRequest;

        public sealed record CallHelper(HelperCall Call) : PortRequest;

        public sealed record RouteInterface(string Destination) : PortRequest;
        public sealed record InterfaceAddresses : PortRequest;
        public sealed record PrimaryRouter : PortRequest;
        public sealed record ScreenState : PortRequest;

        public sealed record OpenBrowser(string Url) : PortRequest;
        public sealed record LoginItemRegistration : PortRequest;
        public sealed record SetLoginItem(bool Enabled) : PortRequest;

        /// <summary>The official FortiClient's SSL VPN connections, read only; Windows imports its settings from them.</summary>
        public sealed record FortiClientConnections : PortRequest;
    }

    [Cases]
    public abstract record PortResponse
    {
        private PortResponse() { }

        public sealed record Secret(string? Value = null) : PortResponse
        {
            public override string ToString() =>
                $"Secret {{ Value = {(Value is null ? "none" : "<redacted>")} }}";
        }

        public sealed record HasSecret(bool Exists) : PortResponse;
        public sealed record SecretChanged : PortResponse;
        public sealed record SecretStoreFailed(SecretStoreFailure Failure) : PortResponse;

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
        public sealed record InterfaceAddresses(IReadOnlyList<InterfaceAddress> Addresses) : PortResponse;
        public sealed record PrimaryRouter(string? Router = null) : PortResponse;
        public sealed record ScreenState(bool Locked, bool DisplaysAsleep) : PortResponse;

        public sealed record BrowserOpened(bool Opened) : PortResponse;

        public sealed record FortiClientConnections(IReadOnlyList<FortiClientConnection> Connections) : PortResponse;
    }

    /// <summary>Carried out without an answer.</summary>
    [Cases]
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

        /// <summary>The runtime's last message.</summary>
        public sealed record ApproveQuit : PortAction;
    }

    [Cases]
    public abstract record SecretItem
    {
        private SecretItem() { }

        public sealed record VpnPassword(string Username, string Host) : SecretItem;
        public sealed record GmailAppPassword(string Address) : SecretItem;
        public sealed record GoogleRefreshToken(string Address) : SecretItem;
        public sealed record GoogleClientSecret(string ClientId) : SecretItem;
    }

    public sealed record SecretStoreFailure(string Description);

    /// <summary>
    /// One call on the Helper over the one connection the OS integration keeps (see
    /// components/contract/src/app_internal/port.rs). JSON payloads and the attempt and reason
    /// values are relayed as they are, never read.
    /// </summary>
    [Cases]
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
    }

    [Cases]
    public abstract record HelperChannelFailure
    {
        private HelperChannelFailure() { }

        public sealed record AppNotSigned : HelperChannelFailure;
        public sealed record Connection(string Description) : HelperChannelFailure;
        public sealed record ConnectionLost : HelperChannelFailure;
        public sealed record NoReply : HelperChannelFailure;
        public sealed record UnreadableReply(string Description) : HelperChannelFailure;
    }

    public enum RegistrationStatus
    {
        NotRegistered,
        Enabled,
        RequiresApproval,
        NotFound,
    }

    [Cases]
    public abstract record RegistrationError
    {
        private RegistrationError() { }

        public sealed record NotInstalledCopy : RegistrationError;
        public sealed record Failed(string Description) : RegistrationError;
    }

    /// <summary>An interface appears once per address.</summary>
    public sealed record InterfaceAddress(string Name, bool Up, string? Ipv4 = null);

    /// <summary>A FortiClient SSL VPN connection: its name and its Server value, usually host:port.</summary>
    public sealed record FortiClientConnection(string Name, string Server);

    public enum LogLevel
    {
        Info,
        Notice,
        Error,
    }

    /// <summary>The log category.</summary>
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
    }

    [Cases]
    public abstract record UserNotification
    {
        private UserNotification() { }

        public sealed record ExceptionRulesNotApplied(uint Retries, BypassFailure Failure) : UserNotification;
    }
}
