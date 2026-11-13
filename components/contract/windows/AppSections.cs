using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract;

public static partial class AppInternal
{
    /// <summary>
    /// Domain sections, drawn as they are (components/contract/src/app_internal/section.rs). Each
    /// value appears once; the UI may show it in several places.
    /// </summary>
    [Cases]
    public abstract record Section
    {
        private Section() { }

        public sealed record Connection([property: JsonPropertyName("_0")] ConnectionSection Value) : Section;
        public sealed record Schedule([property: JsonPropertyName("_0")] ScheduleSection Value) : Section;
        public sealed record Readiness([property: JsonPropertyName("_0")] ReadinessSection Value) : Section;
        public sealed record Gmail([property: JsonPropertyName("_0")] GmailSection Value) : Section;
        public sealed record BypassRules([property: JsonPropertyName("_0")] BypassRulesSection Value) : Section;
        public sealed record Config([property: JsonPropertyName("_0")] ConfigSection Value) : Section;
    }

    // Connection

    /// <param name="AccountAppliesNextConnection">The account change applies from the next connection.</param>
    public sealed record ConnectionSection(
        ConnectionIcon Icon,
        ConnectionStatus Status,
        ButtonState Connect,
        ButtonState Disconnect,
        ButtonState ReconnectNow,
        bool AccountAppliesNextConnection,
        WaitReason? WaitReason = null);

    public enum ConnectionIcon
    {
        Connected,
        Disconnected,
        InProgress,
        Error,
    }

    [Cases]
    public abstract record ConnectionStatus
    {
        private ConnectionStatus() { }

        public sealed record NotConnected : ConnectionStatus;

        /// <summary>Also the line shown with the button that opens Settings.</summary>
        public sealed record SetupRequired(IReadOnlyList<SetupItem> Items) : ConnectionStatus;

        public sealed record Connecting(bool Reconnecting, ConnectProgress Progress) : ConnectionStatus;

        /// <summary>
        /// The status line ends with the planned proactive reconnect, or else with the schedule
        /// outlook when it is suspended or holding while away.
        /// </summary>
        public sealed record Connected(
            DateTimeOffset Since,
            AppInternal.ProactiveReconnect ProactiveReconnect,
            string? TunnelIp = null) : ConnectionStatus;

        public sealed record ReconnectOnHold(DateTimeOffset Until) : ConnectionStatus;

        /// <summary><c>Until</c> is also the next attempt's time.</summary>
        public sealed record WaitingToReconnect(DateTimeOffset Until) : ConnectionStatus;

        public sealed record WaitingForNetwork : ConnectionStatus;
        public sealed record Disconnecting(AfterDisconnect Then) : ConnectionStatus;

        /// <summary>Also the line shown with the button that opens Settings.</summary>
        public sealed record Failed(FailureReason Reason) : ConnectionStatus;
    }

    public enum ConnectProgress
    {
        CheckingReadiness,
        ConnectingToGmail,
        ContactingServer,
        LoggingIn,
        WaitingForOtp,
        VerifyingOtp,
        OpeningTunnel,
    }

    [Cases]
    public abstract record ProactiveReconnect
    {
        private ProactiveReconnect() { }

        public sealed record At([property: JsonPropertyName("at")] DateTimeOffset Time) : ProactiveReconnect;
        public sealed record PausedBySchedule : ProactiveReconnect;
        public sealed record Off : ProactiveReconnect;
    }

    public enum AfterDisconnect
    {
        Idle,
        Reconnect,
        Sleep,
        Quit,
    }

    [Cases]
    public abstract record FailureReason
    {
        private FailureReason() { }

        public sealed record VpnLoginRejected : FailureReason;
        public sealed record OtpNotReceived(bool InSpam, ulong WaitedSeconds) : FailureReason;
        public sealed record OtpRejected : FailureReason;
        public sealed record TunnelSetupFailed : FailureReason;
        public sealed record TunnelModeRejected : FailureReason;
        public sealed record DroppedEarly(ulong WithinSeconds) : FailureReason;
        public sealed record CertificateChangedAfterLogin : FailureReason;
        public sealed record GmailRejected(OtpMethod Method) : FailureReason;
        public sealed record TooManyReconnects(ulong WindowSeconds) : FailureReason;
        public sealed record Network : FailureReason;
        public sealed record Helper(HelperErrorCode Code) : FailureReason;
    }

    /// <summary>The Helper's error codes (components/contract/src/app_helper.rs).</summary>
    public enum HelperErrorCode
    {
        Busy,
        InvalidRequest,
        UnknownAttempt,
        WrongPhase,

        [JsonStringEnumMemberName("binaryUnavailable")]
        ClientUnavailable,

        NotAuthorized,
        InternalError,
    }

    [Cases]
    public abstract record WaitReason
    {
        private WaitReason() { }

        public sealed record Network : WaitReason;
        public sealed record ServerEndedSession : WaitReason;
        public sealed record ForcedTermination : WaitReason;
        public sealed record AuthFailure(FailureReason Reason) : WaitReason;
        public sealed record AuthCooldown(ulong DelaySeconds) : WaitReason;
        public sealed record Sleep : WaitReason;
        public sealed record UserAway : WaitReason;
    }

    [Cases]
    public abstract record SetupItem
    {
        private SetupItem() { }

        public sealed record ConfigFile : SetupItem;
        public sealed record AccountDetails : SetupItem;
        public sealed record VpnPassword : SetupItem;
        public sealed record GmailCredential(OtpMethod Method) : SetupItem;
        public sealed record HelperApproval : SetupItem;
        public sealed record HelperVersion : SetupItem;
    }

    // Schedule

    /// <param name="Days">Monday first.</param>
    /// <param name="Outlook">Null while the schedule is off.</param>
    public sealed record ScheduleSection(
        bool Enabled,
        IReadOnlyList<Weekday> Days,
        TimeOfDay Start,
        TimeOfDay End,
        MinutesSetting AwayMinutes,
        ScheduleOutlook? Outlook = null,
        ScheduleHoursError? HoursError = null);

    [Cases]
    public abstract record ScheduleOutlook
    {
        private ScheduleOutlook() { }

        public sealed record Connects(DateTimeOffset At) : ScheduleOutlook;
        public sealed record NoDays : ScheduleOutlook;
        public sealed record WithinHours(DateTimeOffset End) : ScheduleOutlook;
        public sealed record OutsideHoursPresent : ScheduleOutlook;
        public sealed record HoldingWhileAway(DateTimeOffset Until) : ScheduleOutlook;
        public sealed record Suspended : ScheduleOutlook;
    }

    [Cases]
    public abstract record ScheduleHoursError
    {
        private ScheduleHoursError() { }

        public sealed record EndNotAfterStart : ScheduleHoursError;
        public sealed record NotSaved(ConfigFileError Error) : ScheduleHoursError;
    }

    // Readiness and Helper

    public sealed record ReadinessSection(
        HelperState Helper,
        ButtonState RegisterHelper,
        RegistrationError? RegisterError = null,
        VpnClientInfo? Client = null);

    [Cases]
    public abstract record HelperState
    {
        private HelperState() { }

        public sealed record Checking : HelperState;
        public sealed record NotRegistered : HelperState;
        public sealed record RequiresApproval : HelperState;
        public sealed record Running(string Build) : HelperState;
        public sealed record VersionMismatch(string HelperBuild, string AppBuild) : HelperState;
        public sealed record Unreachable(HelperChannelFailure Failure) : HelperState;
    }

    public sealed record VpnClientInfo(
        string? Path = null,
        string? Version = null,
        UntestedVersion? UntestedVersion = null,
        ClientProblem? Problem = null);

    public sealed record UntestedVersion(string TestedSeries);

    /// <summary>components/contract/src/backend.rs</summary>
    public sealed record ClientProblem(ClientProblemKind Kind, string Path);

    [Cases]
    public abstract record ClientProblemKind
    {
        private ClientProblemKind() { }

        public sealed record NotFound : ClientProblemKind;
        public sealed record OutsideAllowedLocation : ClientProblemKind;
        public sealed record NotRegularFile : ClientProblemKind;
        public sealed record WritableByOthers(uint Mode) : ClientProblemKind;
        public sealed record NotExecutable : ClientProblemKind;
    }

    // Gmail and Google account

    public sealed record GmailSection(
        OtpMethod Method,
        GoogleAccountStatus GoogleAccount,
        ButtonState ConnectGoogle,
        ButtonState DisconnectGoogle,
        ButtonState CancelSignIn,
        OAuthClientSource OauthClient,
        bool ImportedClientSecretMissing,
        ButtonState UseBuiltInClient,
        ButtonState ImportClient,
        GmailTest Test,
        ButtonState TestButton,
        ConfigFileError? MethodError = null,
        GoogleConnectBlocker? ConnectBlocker = null,
        GoogleError? GoogleError = null);

    [Cases]
    public abstract record GoogleAccountStatus
    {
        private GoogleAccountStatus() { }

        public sealed record NotConnected : GoogleAccountStatus;
        public sealed record SigningIn : GoogleAccountStatus;
        public sealed record Disconnecting : GoogleAccountStatus;
        public sealed record Connected(string Address) : GoogleAccountStatus;
    }

    public enum GoogleConnectBlocker
    {
        NoOAuthClient,
        EnterAddress,
        SaveAddressFirst,
    }

    [Cases]
    public abstract record OAuthClientSource
    {
        private OAuthClientSource() { }

        public sealed record BuiltIn(string ClientId) : OAuthClientSource;
        public sealed record Imported(string ClientId) : OAuthClientSource;
        public sealed record NotConfigured : OAuthClientSource;
    }

    [Cases]
    public abstract record GoogleError
    {
        private GoogleError() { }

        public sealed record ClientFileUnreadable(FileProblem Problem) : GoogleError;
        public sealed record ClientFileInvalid(OAuthClientFileProblem Problem) : GoogleError;

        /// <summary>The details of this and the other network failures are the runtime's English text, shown as they are.</summary>
        public sealed record ListenerFailed(string Detail) : GoogleError;

        public sealed record BrowserUnavailable : GoogleError;
        public sealed record TimedOut(ulong AfterSeconds) : GoogleError;
        public sealed record CancelledInBrowser : GoogleError;
        public sealed record Denied(string Code) : GoogleError;
        public sealed record TokenRejected(string Detail) : GoogleError;
        public sealed record ClientRejected : GoogleError;
        public sealed record NoRefreshToken : GoogleError;
        public sealed record ScopeNotGranted : GoogleError;
        public sealed record WrongAccount(string Expected, string Actual) : GoogleError;
        public sealed record GmailApiRefused(string Reason) : GoogleError;
        public sealed record Network(string Detail) : GoogleError;
        public sealed record SecretStore(SecretStoreFailure Failure) : GoogleError;
        public sealed record Config(ConfigFileError Error) : GoogleError;
    }

    [Cases]
    public abstract record OAuthClientFileProblem
    {
        private OAuthClientFileProblem() { }

        public sealed record NotJson : OAuthClientFileProblem;
        public sealed record WebClient : OAuthClientFileProblem;
        public sealed record NoDesktopClient : OAuthClientFileProblem;
        public sealed record MissingField(string Name) : OAuthClientFileProblem;
        public sealed record InvalidField(string Name) : OAuthClientFileProblem;
    }

    [Cases]
    public abstract record GmailTest
    {
        private GmailTest() { }

        public sealed record Idle : GmailTest;
        public sealed record Running : GmailTest;
        public sealed record ImapSucceeded(string Archive, string Trash, string? Spam = null) : GmailTest;
        public sealed record ApiSucceeded(string Address) : GmailTest;
        public sealed record Failed(GmailTestFailure Failure) : GmailTest;
    }

    [Cases]
    public abstract record GmailTestFailure
    {
        private GmailTestFailure() { }

        public sealed record EnterAddress : GmailTestFailure;
        public sealed record EnterAppPassword : GmailTestFailure;
        public sealed record AppPasswordUnreadable(SecretStoreFailure Failure) : GmailTestFailure;
        public sealed record ImapLoginRejected(string? ServerText = null) : GmailTestFailure;
        public sealed record MailboxesMissing(bool AllMail, bool Trash) : GmailTestFailure;
        public sealed record ImapUnreachable(ImapFailure Failure) : GmailTestFailure;
        public sealed record NoOAuthClient : GmailTestFailure;
        public sealed record NotSignedIn : GmailTestFailure;
        public sealed record AccessRevoked : GmailTestFailure;
        public sealed record ClientRejected : GmailTestFailure;
        public sealed record SecretStore(SecretStoreFailure Failure) : GmailTestFailure;
        public sealed record OtherAccount(string Authorized, string Expected) : GmailTestFailure;
        public sealed record AccessTokenRejected : GmailTestFailure;
        public sealed record GmailApiRefused(string Reason) : GmailTestFailure;
        public sealed record RateLimited : GmailTestFailure;
        public sealed record GoogleUnreachable(string Detail) : GmailTestFailure;
        public sealed record GoogleReturnedError(string Detail) : GmailTestFailure;
    }

    [Cases]
    public abstract record ImapFailure
    {
        private ImapFailure() { }

        public sealed record Network(string Detail) : ImapFailure;
        public sealed record TimedOut(ImapStep Step) : ImapFailure;
        public sealed record ServerClosed(string? Text = null) : ImapFailure;
        public sealed record Protocol(string Detail) : ImapFailure;
        public sealed record Rejected(string Command, string Status, string Text) : ImapFailure;
    }

    [Cases]
    public abstract record ImapStep
    {
        private ImapStep() { }

        public sealed record Connecting : ImapStep;
        public sealed record Command(string Name) : ImapStep;
    }

    // Exception rules

    public sealed record BypassRulesSection(
        RuleList Networks,
        RuleList Targets,
        BypassStatus Status,
        bool HasChanges,
        ButtonState AddCurrentNetwork,
        ButtonState Discard,
        ButtonState Apply,
        CurrentNetwork? CurrentNetwork = null,
        BypassNotice? Notice = null);

    /// <param name="Within">The saved network the address is in; null outside all of them.</param>
    public sealed record CurrentNetwork(
        string ServiceName,
        string Address,
        byte? PrefixLength = null,
        string? Within = null);

    public sealed record RuleList(IReadOnlyList<string> Rows, IReadOnlyList<uint> ProblemRows, ButtonState AddRow);

    [Cases]
    public abstract record BypassNotice
    {
        private BypassNotice() { }

        public sealed record RowProblem(string Row, AppInternal.RowProblem Problem) : BypassNotice;
        public sealed record ApplyFailed(ConfigFileError Error) : BypassNotice;
    }

    [Cases]
    public abstract record RowProblem
    {
        private RowProblem() { }

        public sealed record Invalid(AddressProblem Problem) : RowProblem;
        public sealed record Duplicate(string Entry) : RowProblem;
        public sealed record Overlap(string Other) : RowProblem;
    }

    [Cases]
    public abstract record BypassStatus
    {
        private BypassStatus() { }

        public sealed record Checking : BypassStatus;
        public sealed record NoConfigFile : BypassStatus;

        public sealed record Applied(
            string ServiceName,
            string Router,
            uint OnLinkTargets,
            uint NotImported,
            uint KeptThroughOtherGateways) : BypassStatus;

        public sealed record OutsideNetworks(uint KeptThroughOtherGateways) : BypassStatus;
        public sealed record NoNetworksListed(uint KeptThroughOtherGateways) : BypassStatus;
        public sealed record ConfigInvalid : BypassStatus;
        public sealed record NoNetwork : BypassStatus;

        public sealed record Failed(
            BypassFailure Failure,
            ulong? RetryInSeconds = null,
            uint? StoppedAfterRetries = null) : BypassStatus;
    }

    [Cases]
    public abstract record BypassFailure
    {
        private BypassFailure() { }

        public sealed record RoutesNotKept : BypassFailure;
        public sealed record NetworkChanged : BypassFailure;
        public sealed record HelperRefused(string Message) : BypassFailure;
        public sealed record SettingsNotChanged(string Message) : BypassFailure;
        public sealed record ImportNotSaved(ConfigFileError Error) : BypassFailure;
        public sealed record HelperUnreachable(HelperChannelFailure Failure) : BypassFailure;
        public sealed record HelperBuildMismatch(string HelperBuild, string AppBuild) : BypassFailure;
    }

    // Settings

    public sealed record ConfigSection(
        ConfigFileState File,
        AccountFormState Account,
        ConnectionOptions Options,
        OpenAtLogin OpenAtLogin,
        string LogFile,
        ConfigFileError? OptionsError = null);

    public sealed record ConfigFileState(
        string Path,
        bool Exists,
        IReadOnlyList<ConfigWarning> Warnings,
        ConfigFileError? LoadError = null);

    public sealed record ConnectionOptions(
        bool AutoConnectOnLaunch,
        MinutesSetting SessionMaxMinutes,
        bool ProactiveReconnect,
        MinutesSetting MarginMinutes,
        bool ReconnectAfterSleep,
        MinutesSetting MinSleepMinutes);

    public sealed record OpenAtLogin(
        bool Available,
        bool Enabled,
        bool RequiresApproval,
        RegistrationError? Error = null);

    /// <summary>The Account pane's draft. Replace the typed fields only when <c>Revision</c> changes.</summary>
    public sealed record AccountFormState(
        ulong Revision,
        string Host,
        string Port,
        string Username,
        string Realm,
        string GmailAddress,
        bool VpnPasswordEntered,
        bool AppPasswordEntered,
        bool VpnPasswordStored,
        bool AppPasswordStored,
        ButtonState Save,
        ButtonState Revert,
        AccountValueProblem? SavedAccountProblem = null,
        AccountFeedback? Feedback = null,
        ClientSettingsImport? ImportReport = null,
        string? DefaultImportFile = null);

    [Cases]
    public abstract record AccountFeedback
    {
        private AccountFeedback() { }

        public sealed record Saved : AccountFeedback;
        public sealed record Failed(AccountSaveError Error) : AccountFeedback;
    }

    [Cases]
    public abstract record AccountSaveError
    {
        private AccountSaveError() { }

        public sealed record InvalidPort : AccountSaveError;
        public sealed record PasswordNeedsAccount : AccountSaveError;
        public sealed record AppPasswordNeedsAddress : AccountSaveError;
        public sealed record AccountValue(AccountValueProblem Problem) : AccountSaveError;
        public sealed record InvalidVpnPassword(uint MaxBytes) : AccountSaveError;
        public sealed record Config(ConfigFileError Error) : AccountSaveError;
        public sealed record SecretStore(SecretStoreFailure Failure) : AccountSaveError;
    }

    [Cases]
    public abstract record ClientSettingsImport
    {
        private ClientSettingsImport() { }

        public sealed record Imported(ImportReport Report) : ClientSettingsImport;
        public sealed record Failed(ImportError Error) : ClientSettingsImport;
    }

    /// <param name="Password">Null when the file has no password.</param>
    public sealed record ImportReport(
        string Source,
        string Host,
        ushort Port,
        string Username,
        string Realm,
        uint TrustedCerts,
        IReadOnlyList<string> UnknownKeys,
        IReadOnlyList<string> IgnoredKeys,
        IReadOnlyList<uint> UnreadableLines,
        ImportedPassword? Password = null);

    [Cases]
    public abstract record ImportedPassword
    {
        private ImportedPassword() { }

        public sealed record Saved : ImportedPassword;
        public sealed record NotSavedWithoutUsername : ImportedPassword;
        public sealed record NotSaved(SecretStoreFailure Failure) : ImportedPassword;
    }

    [Cases]
    public abstract record ImportError
    {
        private ImportError() { }

        public sealed record SourceNotFound(string Path) : ImportError;
        public sealed record Unreadable(string Path, FileProblem Problem) : ImportError;
        public sealed record MissingHost(string Path) : ImportError;
        public sealed record Config(ConfigFileError Error) : ImportError;
        public sealed record NoFortiClientConnection : ImportError;
        public sealed record InvalidFortiClientServer(string Connection) : ImportError;
    }
}
