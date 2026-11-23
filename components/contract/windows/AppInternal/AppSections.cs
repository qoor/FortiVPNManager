using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract.AppInternal;

/// <summary>
/// Domain sections, drawn as they are (components/contract/src/app_internal/section.rs). Each
/// value appears once; the UI may show it in several places.
/// </summary>
[JsonConverter(typeof(CasesConverter<Section>))]
[JsonCase(typeof(SectionConnection), "connection")]
[JsonCase(typeof(SectionSchedule), "schedule")]
[JsonCase(typeof(SectionReadiness), "readiness")]
[JsonCase(typeof(SectionGmail), "gmail")]
[JsonCase(typeof(SectionBypassRules), "bypassRules")]
[JsonCase(typeof(SectionConfig), "config")]
[JsonCase(typeof(SectionAutoUpdate), "autoUpdate")]
public abstract record Section
{
    private protected Section() { }
}

public sealed record SectionConnection([property: JsonPropertyName("_0")] ConnectionSection Value) : Section;

public sealed record SectionSchedule([property: JsonPropertyName("_0")] ScheduleSection Value) : Section;

public sealed record SectionReadiness([property: JsonPropertyName("_0")] ReadinessSection Value) : Section;

public sealed record SectionGmail([property: JsonPropertyName("_0")] GmailSection Value) : Section;

public sealed record SectionBypassRules([property: JsonPropertyName("_0")] BypassRulesSection Value) : Section;

public sealed record SectionConfig([property: JsonPropertyName("_0")] ConfigSection Value) : Section;

public sealed record SectionAutoUpdate([property: JsonPropertyName("_0")] AutoUpdateSection Value) : Section;

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

[JsonConverter(typeof(EnumNameConverter<ConnectionIcon>))]
public enum ConnectionIcon
{
    Connected,
    Disconnected,
    InProgress,
    Error,
}

[JsonConverter(typeof(CasesConverter<ConnectionStatus>))]
[JsonCase(typeof(ConnectionStatusNotConnected), "notConnected")]
[JsonCase(typeof(ConnectionStatusSetupRequired), "setupRequired")]
[JsonCase(typeof(ConnectionStatusConnecting), "connecting")]
[JsonCase(typeof(ConnectionStatusConnected), "connected")]
[JsonCase(typeof(ConnectionStatusReconnectOnHold), "reconnectOnHold")]
[JsonCase(typeof(ConnectionStatusWaitingToReconnect), "waitingToReconnect")]
[JsonCase(typeof(ConnectionStatusWaitingForNetwork), "waitingForNetwork")]
[JsonCase(typeof(ConnectionStatusDisconnecting), "disconnecting")]
[JsonCase(typeof(ConnectionStatusFailed), "failed")]
public abstract record ConnectionStatus
{
    private protected ConnectionStatus() { }
}

public sealed record ConnectionStatusNotConnected : ConnectionStatus;

/// <summary>Also the line shown with the button that opens Settings.</summary>
public sealed record ConnectionStatusSetupRequired(IReadOnlyList<SetupItem> Items) : ConnectionStatus;

public sealed record ConnectionStatusConnecting(bool Reconnecting, ConnectProgress Progress) : ConnectionStatus;

/// <summary>
/// The status line ends with the planned proactive reconnect, or else with the schedule
/// outlook when it is suspended or holding while away.
/// </summary>
public sealed record ConnectionStatusConnected(
    DateTimeOffset Since,
    AppInternal.ProactiveReconnect ProactiveReconnect,
    string? TunnelIp = null) : ConnectionStatus;

public sealed record ConnectionStatusReconnectOnHold(DateTimeOffset Until) : ConnectionStatus;

/// <summary><c>Until</c> is also the next attempt's time.</summary>
public sealed record ConnectionStatusWaitingToReconnect(DateTimeOffset Until) : ConnectionStatus;

public sealed record ConnectionStatusWaitingForNetwork : ConnectionStatus;

public sealed record ConnectionStatusDisconnecting(AfterDisconnect Then) : ConnectionStatus;

/// <summary>Also the line shown with the button that opens Settings.</summary>
public sealed record ConnectionStatusFailed(FailureReason Reason) : ConnectionStatus;

[JsonConverter(typeof(EnumNameConverter<ConnectProgress>))]
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

[JsonConverter(typeof(CasesConverter<ProactiveReconnect>))]
[JsonCase(typeof(ProactiveReconnectAt), "at")]
[JsonCase(typeof(ProactiveReconnectPausedBySchedule), "pausedBySchedule")]
[JsonCase(typeof(ProactiveReconnectOff), "off")]
public abstract record ProactiveReconnect
{
    private protected ProactiveReconnect() { }
}

public sealed record ProactiveReconnectAt([property: JsonPropertyName("at")] DateTimeOffset Time) : ProactiveReconnect;

public sealed record ProactiveReconnectPausedBySchedule : ProactiveReconnect;

public sealed record ProactiveReconnectOff : ProactiveReconnect;

[JsonConverter(typeof(EnumNameConverter<AfterDisconnect>))]
public enum AfterDisconnect
{
    Idle,
    Reconnect,
    Sleep,
    Quit,
}

[JsonConverter(typeof(CasesConverter<FailureReason>))]
[JsonCase(typeof(FailureReasonVpnLoginRejected), "vpnLoginRejected")]
[JsonCase(typeof(FailureReasonOtpNotReceived), "otpNotReceived")]
[JsonCase(typeof(FailureReasonOtpRejected), "otpRejected")]
[JsonCase(typeof(FailureReasonTunnelSetupFailed), "tunnelSetupFailed")]
[JsonCase(typeof(FailureReasonCertificateChangedAfterLogin), "certificateChangedAfterLogin")]
[JsonCase(typeof(FailureReasonGmailRejected), "gmailRejected")]
[JsonCase(typeof(FailureReasonTooManyReconnects), "tooManyReconnects")]
[JsonCase(typeof(FailureReasonNetwork), "network")]
[JsonCase(typeof(FailureReasonHelper), "helper")]
public abstract record FailureReason
{
    private protected FailureReason() { }
}

public sealed record FailureReasonVpnLoginRejected : FailureReason;

public sealed record FailureReasonOtpNotReceived(bool InSpam, ulong WaitedSeconds) : FailureReason;

public sealed record FailureReasonOtpRejected : FailureReason;

public sealed record FailureReasonTunnelSetupFailed : FailureReason;

public sealed record FailureReasonCertificateChangedAfterLogin : FailureReason;

public sealed record FailureReasonGmailRejected(OtpMethod Method) : FailureReason;

public sealed record FailureReasonTooManyReconnects(ulong WindowSeconds) : FailureReason;

public sealed record FailureReasonNetwork : FailureReason;

public sealed record FailureReasonHelper(HelperErrorCode Code) : FailureReason;

/// <summary>The Helper's error codes (components/contract/src/app_helper.rs).</summary>
[JsonConverter(typeof(EnumNameConverter<HelperErrorCode>))]
public enum HelperErrorCode
{
    Busy,
    InvalidRequest,
    UnknownAttempt,
    WrongPhase,
    NotAuthorized,
    InternalError,
}

[JsonConverter(typeof(CasesConverter<WaitReason>))]
[JsonCase(typeof(WaitReasonNetwork), "network")]
[JsonCase(typeof(WaitReasonNetworkSetupFailed), "networkSetupFailed")]
[JsonCase(typeof(WaitReasonServerEndedSession), "serverEndedSession")]
[JsonCase(typeof(WaitReasonForcedTermination), "forcedTermination")]
[JsonCase(typeof(WaitReasonAuthFailure), "authFailure")]
[JsonCase(typeof(WaitReasonAuthCooldown), "authCooldown")]
[JsonCase(typeof(WaitReasonSleep), "sleep")]
[JsonCase(typeof(WaitReasonUserAway), "userAway")]
public abstract record WaitReason
{
    private protected WaitReason() { }
}

public sealed record WaitReasonNetwork : WaitReason;

public sealed record WaitReasonNetworkSetupFailed : WaitReason;

public sealed record WaitReasonServerEndedSession : WaitReason;

public sealed record WaitReasonForcedTermination : WaitReason;

public sealed record WaitReasonAuthFailure(FailureReason Reason) : WaitReason;

public sealed record WaitReasonAuthCooldown(ulong DelaySeconds) : WaitReason;

public sealed record WaitReasonSleep : WaitReason;

public sealed record WaitReasonUserAway : WaitReason;

[JsonConverter(typeof(CasesConverter<SetupItem>))]
[JsonCase(typeof(SetupItemConfigFile), "configFile")]
[JsonCase(typeof(SetupItemAccountDetails), "accountDetails")]
[JsonCase(typeof(SetupItemVpnPassword), "vpnPassword")]
[JsonCase(typeof(SetupItemGmailCredential), "gmailCredential")]
[JsonCase(typeof(SetupItemHelperApproval), "helperApproval")]
[JsonCase(typeof(SetupItemHelperVersion), "helperVersion")]
public abstract record SetupItem
{
    private protected SetupItem() { }
}

public sealed record SetupItemConfigFile : SetupItem;

public sealed record SetupItemAccountDetails : SetupItem;

public sealed record SetupItemVpnPassword : SetupItem;

public sealed record SetupItemGmailCredential(OtpMethod Method) : SetupItem;

public sealed record SetupItemHelperApproval : SetupItem;

public sealed record SetupItemHelperVersion : SetupItem;

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

[JsonConverter(typeof(CasesConverter<ScheduleOutlook>))]
[JsonCase(typeof(ScheduleOutlookConnects), "connects")]
[JsonCase(typeof(ScheduleOutlookNoDays), "noDays")]
[JsonCase(typeof(ScheduleOutlookWithinHours), "withinHours")]
[JsonCase(typeof(ScheduleOutlookOutsideHoursPresent), "outsideHoursPresent")]
[JsonCase(typeof(ScheduleOutlookHoldingWhileAway), "holdingWhileAway")]
[JsonCase(typeof(ScheduleOutlookSuspended), "suspended")]
public abstract record ScheduleOutlook
{
    private protected ScheduleOutlook() { }
}

public sealed record ScheduleOutlookConnects(DateTimeOffset At) : ScheduleOutlook;

public sealed record ScheduleOutlookNoDays : ScheduleOutlook;

public sealed record ScheduleOutlookWithinHours(DateTimeOffset End) : ScheduleOutlook;

public sealed record ScheduleOutlookOutsideHoursPresent : ScheduleOutlook;

public sealed record ScheduleOutlookHoldingWhileAway(DateTimeOffset Until) : ScheduleOutlook;

public sealed record ScheduleOutlookSuspended : ScheduleOutlook;

[JsonConverter(typeof(CasesConverter<ScheduleHoursError>))]
[JsonCase(typeof(ScheduleHoursErrorEndNotAfterStart), "endNotAfterStart")]
[JsonCase(typeof(ScheduleHoursErrorNotSaved), "notSaved")]
public abstract record ScheduleHoursError
{
    private protected ScheduleHoursError() { }
}

public sealed record ScheduleHoursErrorEndNotAfterStart : ScheduleHoursError;

public sealed record ScheduleHoursErrorNotSaved(ConfigFileError Error) : ScheduleHoursError;

// Readiness and Helper

public sealed record ReadinessSection(
    HelperState Helper,
    ButtonState RegisterHelper,
    RegistrationError? RegisterError = null,
    VpnBackendInfo? Backend = null);

[JsonConverter(typeof(CasesConverter<HelperState>))]
[JsonCase(typeof(HelperStateChecking), "checking")]
[JsonCase(typeof(HelperStateNotRegistered), "notRegistered")]
[JsonCase(typeof(HelperStateRequiresApproval), "requiresApproval")]
[JsonCase(typeof(HelperStateRunning), "running")]
[JsonCase(typeof(HelperStateVersionMismatch), "versionMismatch")]
[JsonCase(typeof(HelperStateUnreachable), "unreachable")]
public abstract record HelperState
{
    private protected HelperState() { }
}

public sealed record HelperStateChecking : HelperState;

public sealed record HelperStateNotRegistered : HelperState;

public sealed record HelperStateRequiresApproval : HelperState;

public sealed record HelperStateRunning(string Build) : HelperState;

public sealed record HelperStateVersionMismatch(string HelperBuild, string AppBuild) : HelperState;

public sealed record HelperStateUnreachable(HelperChannelFailure Failure) : HelperState;

public sealed record VpnBackendInfo(string? Version = null);

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

[JsonConverter(typeof(CasesConverter<GoogleAccountStatus>))]
[JsonCase(typeof(GoogleAccountStatusNotConnected), "notConnected")]
[JsonCase(typeof(GoogleAccountStatusSigningIn), "signingIn")]
[JsonCase(typeof(GoogleAccountStatusDisconnecting), "disconnecting")]
[JsonCase(typeof(GoogleAccountStatusConnected), "connected")]
public abstract record GoogleAccountStatus
{
    private protected GoogleAccountStatus() { }
}

public sealed record GoogleAccountStatusNotConnected : GoogleAccountStatus;

public sealed record GoogleAccountStatusSigningIn : GoogleAccountStatus;

public sealed record GoogleAccountStatusDisconnecting : GoogleAccountStatus;

public sealed record GoogleAccountStatusConnected(string Address) : GoogleAccountStatus;

[JsonConverter(typeof(EnumNameConverter<GoogleConnectBlocker>))]
public enum GoogleConnectBlocker
{
    NoOAuthClient,
    EnterAddress,
    SaveAddressFirst,
}

[JsonConverter(typeof(CasesConverter<OAuthClientSource>))]
[JsonCase(typeof(OAuthClientSourceBuiltIn), "builtIn")]
[JsonCase(typeof(OAuthClientSourceImported), "imported")]
[JsonCase(typeof(OAuthClientSourceNotConfigured), "notConfigured")]
public abstract record OAuthClientSource
{
    private protected OAuthClientSource() { }
}

public sealed record OAuthClientSourceBuiltIn(string ClientId) : OAuthClientSource;

public sealed record OAuthClientSourceImported(string ClientId) : OAuthClientSource;

public sealed record OAuthClientSourceNotConfigured : OAuthClientSource;

[JsonConverter(typeof(CasesConverter<GoogleError>))]
[JsonCase(typeof(GoogleErrorClientFileUnreadable), "clientFileUnreadable")]
[JsonCase(typeof(GoogleErrorClientFileInvalid), "clientFileInvalid")]
[JsonCase(typeof(GoogleErrorListenerFailed), "listenerFailed")]
[JsonCase(typeof(GoogleErrorBrowserUnavailable), "browserUnavailable")]
[JsonCase(typeof(GoogleErrorTimedOut), "timedOut")]
[JsonCase(typeof(GoogleErrorCancelledInBrowser), "cancelledInBrowser")]
[JsonCase(typeof(GoogleErrorDenied), "denied")]
[JsonCase(typeof(GoogleErrorTokenRejected), "tokenRejected")]
[JsonCase(typeof(GoogleErrorClientRejected), "clientRejected")]
[JsonCase(typeof(GoogleErrorNoRefreshToken), "noRefreshToken")]
[JsonCase(typeof(GoogleErrorScopeNotGranted), "scopeNotGranted")]
[JsonCase(typeof(GoogleErrorWrongAccount), "wrongAccount")]
[JsonCase(typeof(GoogleErrorGmailApiRefused), "gmailApiRefused")]
[JsonCase(typeof(GoogleErrorNetwork), "network")]
[JsonCase(typeof(GoogleErrorSecretStore), "secretStore")]
[JsonCase(typeof(GoogleErrorConfig), "config")]
public abstract record GoogleError
{
    private protected GoogleError() { }
}

public sealed record GoogleErrorClientFileUnreadable(FileProblem Problem) : GoogleError;

public sealed record GoogleErrorClientFileInvalid(OAuthClientFileProblem Problem) : GoogleError;

/// <summary>The details of this and the other network failures are the runtime's English text, shown as they are.</summary>
public sealed record GoogleErrorListenerFailed(string Detail) : GoogleError;

public sealed record GoogleErrorBrowserUnavailable : GoogleError;

public sealed record GoogleErrorTimedOut(ulong AfterSeconds) : GoogleError;

public sealed record GoogleErrorCancelledInBrowser : GoogleError;

public sealed record GoogleErrorDenied(string Code) : GoogleError;

public sealed record GoogleErrorTokenRejected(string Detail) : GoogleError;

public sealed record GoogleErrorClientRejected : GoogleError;

public sealed record GoogleErrorNoRefreshToken : GoogleError;

public sealed record GoogleErrorScopeNotGranted : GoogleError;

public sealed record GoogleErrorWrongAccount(string Expected, string Actual) : GoogleError;

public sealed record GoogleErrorGmailApiRefused(string Reason) : GoogleError;

public sealed record GoogleErrorNetwork(string Detail) : GoogleError;

public sealed record GoogleErrorSecretStore(SecretStoreFailure Failure) : GoogleError;

public sealed record GoogleErrorConfig(ConfigFileError Error) : GoogleError;

[JsonConverter(typeof(CasesConverter<OAuthClientFileProblem>))]
[JsonCase(typeof(OAuthClientFileProblemNotJson), "notJson")]
[JsonCase(typeof(OAuthClientFileProblemWebClient), "webClient")]
[JsonCase(typeof(OAuthClientFileProblemNoDesktopClient), "noDesktopClient")]
[JsonCase(typeof(OAuthClientFileProblemMissingField), "missingField")]
[JsonCase(typeof(OAuthClientFileProblemInvalidField), "invalidField")]
public abstract record OAuthClientFileProblem
{
    private protected OAuthClientFileProblem() { }
}

public sealed record OAuthClientFileProblemNotJson : OAuthClientFileProblem;

public sealed record OAuthClientFileProblemWebClient : OAuthClientFileProblem;

public sealed record OAuthClientFileProblemNoDesktopClient : OAuthClientFileProblem;

public sealed record OAuthClientFileProblemMissingField(string Name) : OAuthClientFileProblem;

public sealed record OAuthClientFileProblemInvalidField(string Name) : OAuthClientFileProblem;

[JsonConverter(typeof(CasesConverter<GmailTest>))]
[JsonCase(typeof(GmailTestIdle), "idle")]
[JsonCase(typeof(GmailTestRunning), "running")]
[JsonCase(typeof(GmailTestImapSucceeded), "imapSucceeded")]
[JsonCase(typeof(GmailTestApiSucceeded), "apiSucceeded")]
[JsonCase(typeof(GmailTestFailed), "failed")]
public abstract record GmailTest
{
    private protected GmailTest() { }
}

public sealed record GmailTestIdle : GmailTest;

public sealed record GmailTestRunning : GmailTest;

public sealed record GmailTestImapSucceeded(string Archive, string Trash, string? Spam = null) : GmailTest;

public sealed record GmailTestApiSucceeded(string Address) : GmailTest;

public sealed record GmailTestFailed(GmailTestFailure Failure) : GmailTest;

[JsonConverter(typeof(CasesConverter<GmailTestFailure>))]
[JsonCase(typeof(GmailTestFailureEnterAddress), "enterAddress")]
[JsonCase(typeof(GmailTestFailureEnterAppPassword), "enterAppPassword")]
[JsonCase(typeof(GmailTestFailureAppPasswordUnreadable), "appPasswordUnreadable")]
[JsonCase(typeof(GmailTestFailureImapLoginRejected), "imapLoginRejected")]
[JsonCase(typeof(GmailTestFailureMailboxesMissing), "mailboxesMissing")]
[JsonCase(typeof(GmailTestFailureImapUnreachable), "imapUnreachable")]
[JsonCase(typeof(GmailTestFailureNoOAuthClient), "noOAuthClient")]
[JsonCase(typeof(GmailTestFailureNotSignedIn), "notSignedIn")]
[JsonCase(typeof(GmailTestFailureAccessRevoked), "accessRevoked")]
[JsonCase(typeof(GmailTestFailureClientRejected), "clientRejected")]
[JsonCase(typeof(GmailTestFailureSecretStore), "secretStore")]
[JsonCase(typeof(GmailTestFailureOtherAccount), "otherAccount")]
[JsonCase(typeof(GmailTestFailureAccessTokenRejected), "accessTokenRejected")]
[JsonCase(typeof(GmailTestFailureGmailApiRefused), "gmailApiRefused")]
[JsonCase(typeof(GmailTestFailureRateLimited), "rateLimited")]
[JsonCase(typeof(GmailTestFailureGoogleUnreachable), "googleUnreachable")]
[JsonCase(typeof(GmailTestFailureGoogleReturnedError), "googleReturnedError")]
public abstract record GmailTestFailure
{
    private protected GmailTestFailure() { }
}

public sealed record GmailTestFailureEnterAddress : GmailTestFailure;

public sealed record GmailTestFailureEnterAppPassword : GmailTestFailure;

public sealed record GmailTestFailureAppPasswordUnreadable(SecretStoreFailure Failure) : GmailTestFailure;

public sealed record GmailTestFailureImapLoginRejected(string? ServerText = null) : GmailTestFailure;

public sealed record GmailTestFailureMailboxesMissing(bool AllMail, bool Trash) : GmailTestFailure;

public sealed record GmailTestFailureImapUnreachable(ImapFailure Failure) : GmailTestFailure;

public sealed record GmailTestFailureNoOAuthClient : GmailTestFailure;

public sealed record GmailTestFailureNotSignedIn : GmailTestFailure;

public sealed record GmailTestFailureAccessRevoked : GmailTestFailure;

public sealed record GmailTestFailureClientRejected : GmailTestFailure;

public sealed record GmailTestFailureSecretStore(SecretStoreFailure Failure) : GmailTestFailure;

public sealed record GmailTestFailureOtherAccount(string Authorized, string Expected) : GmailTestFailure;

public sealed record GmailTestFailureAccessTokenRejected : GmailTestFailure;

public sealed record GmailTestFailureGmailApiRefused(string Reason) : GmailTestFailure;

public sealed record GmailTestFailureRateLimited : GmailTestFailure;

public sealed record GmailTestFailureGoogleUnreachable(string Detail) : GmailTestFailure;

public sealed record GmailTestFailureGoogleReturnedError(string Detail) : GmailTestFailure;

[JsonConverter(typeof(CasesConverter<ImapFailure>))]
[JsonCase(typeof(ImapFailureNetwork), "network")]
[JsonCase(typeof(ImapFailureTimedOut), "timedOut")]
[JsonCase(typeof(ImapFailureServerClosed), "serverClosed")]
[JsonCase(typeof(ImapFailureProtocol), "protocol")]
[JsonCase(typeof(ImapFailureRejected), "rejected")]
public abstract record ImapFailure
{
    private protected ImapFailure() { }
}

public sealed record ImapFailureNetwork(string Detail) : ImapFailure;

public sealed record ImapFailureTimedOut(ImapStep Step) : ImapFailure;

public sealed record ImapFailureServerClosed(string? Text = null) : ImapFailure;

public sealed record ImapFailureProtocol(string Detail) : ImapFailure;

public sealed record ImapFailureRejected(string Command, string Status, string Text) : ImapFailure;

[JsonConverter(typeof(CasesConverter<ImapStep>))]
[JsonCase(typeof(ImapStepConnecting), "connecting")]
[JsonCase(typeof(ImapStepCommand), "command")]
public abstract record ImapStep
{
    private protected ImapStep() { }
}

public sealed record ImapStepConnecting : ImapStep;

public sealed record ImapStepCommand(string Name) : ImapStep;

// Exception rules

public sealed record BypassRulesSection(
    RuleList Networks,
    RuleList Targets,
    BypassStatus Status,
    ButtonState AddCurrentNetwork,
    CurrentNetwork? CurrentNetwork = null,
    BypassNotice? Notice = null);

/// <param name="Within">The saved network the address is in; null outside all of them.</param>
public sealed record CurrentNetwork(
    string ServiceName,
    string Address,
    byte? PrefixLength = null,
    string? Within = null);

public sealed record RuleList(IReadOnlyList<string> Rows, IReadOnlyList<uint> ProblemRows, ButtonState AddRow);

[JsonConverter(typeof(CasesConverter<BypassNotice>))]
[JsonCase(typeof(BypassNoticeRowProblem), "rowProblem")]
[JsonCase(typeof(BypassNoticeApplyFailed), "applyFailed")]
public abstract record BypassNotice
{
    private protected BypassNotice() { }
}

public sealed record BypassNoticeRowProblem(string Row, AppInternal.RowProblem Problem) : BypassNotice;

public sealed record BypassNoticeApplyFailed(ConfigFileError Error) : BypassNotice;

[JsonConverter(typeof(CasesConverter<RowProblem>))]
[JsonCase(typeof(RowProblemInvalid), "invalid")]
[JsonCase(typeof(RowProblemDuplicate), "duplicate")]
[JsonCase(typeof(RowProblemOverlap), "overlap")]
[JsonCase(typeof(RowProblemVpnServer), "vpnServer")]
public abstract record RowProblem
{
    private protected RowProblem() { }
}

public sealed record RowProblemInvalid(AddressProblem Problem) : RowProblem;

public sealed record RowProblemDuplicate(string Entry) : RowProblem;

public sealed record RowProblemOverlap(string Other) : RowProblem;

/// <summary>A target that is the VPN server's address, which always goes out directly.</summary>
public sealed record RowProblemVpnServer : RowProblem;

[JsonConverter(typeof(CasesConverter<BypassStatus>))]
[JsonCase(typeof(BypassStatusChecking), "checking")]
[JsonCase(typeof(BypassStatusNoConfigFile), "noConfigFile")]
[JsonCase(typeof(BypassStatusApplied), "applied")]
[JsonCase(typeof(BypassStatusVpnOff), "vpnOff")]
[JsonCase(typeof(BypassStatusOutsideNetworks), "outsideNetworks")]
[JsonCase(typeof(BypassStatusNoNetworksListed), "noNetworksListed")]
[JsonCase(typeof(BypassStatusConfigInvalid), "configInvalid")]
[JsonCase(typeof(BypassStatusNoNetwork), "noNetwork")]
[JsonCase(typeof(BypassStatusFailed), "failed")]
public abstract record BypassStatus
{
    private protected BypassStatus() { }
}

public sealed record BypassStatusChecking : BypassStatus;

public sealed record BypassStatusNoConfigFile : BypassStatus;

public sealed record BypassStatusApplied(string ServiceName, string Router, uint OnLinkTargets) : BypassStatus;

/// <summary>On a listed network, but the app does not want the VPN up.</summary>
public sealed record BypassStatusVpnOff : BypassStatus;

public sealed record BypassStatusOutsideNetworks : BypassStatus;

public sealed record BypassStatusNoNetworksListed : BypassStatus;

public sealed record BypassStatusConfigInvalid : BypassStatus;

public sealed record BypassStatusNoNetwork : BypassStatus;

public sealed record BypassStatusFailed(
    BypassFailure Failure,
    ulong? RetryInSeconds = null,
    uint? StoppedAfterRetries = null) : BypassStatus;

[JsonConverter(typeof(CasesConverter<BypassFailure>))]
[JsonCase(typeof(BypassFailureRoutesNotKept), "routesNotKept")]
[JsonCase(typeof(BypassFailureNetworkChanged), "networkChanged")]
[JsonCase(typeof(BypassFailureHelperRefused), "helperRefused")]
[JsonCase(typeof(BypassFailureSettingsNotChanged), "settingsNotChanged")]
[JsonCase(typeof(BypassFailureHelperUnreachable), "helperUnreachable")]
[JsonCase(typeof(BypassFailureHelperBuildMismatch), "helperBuildMismatch")]
public abstract record BypassFailure
{
    private protected BypassFailure() { }
}

public sealed record BypassFailureRoutesNotKept : BypassFailure;

public sealed record BypassFailureNetworkChanged : BypassFailure;

public sealed record BypassFailureHelperRefused(string Message) : BypassFailure;

public sealed record BypassFailureSettingsNotChanged(string Message) : BypassFailure;

public sealed record BypassFailureHelperUnreachable(HelperChannelFailure Failure) : BypassFailure;

public sealed record BypassFailureHelperBuildMismatch(string HelperBuild, string AppBuild) : BypassFailure;

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
    string Server,
    string Username,
    string Realm,
    string GmailAddress,
    bool VpnPasswordEntered,
    bool AppPasswordEntered,
    bool VpnPasswordStored,
    bool AppPasswordStored,
    AccountValueProblem? SavedAccountProblem = null,
    AccountSaveError? SaveError = null,
    ClientSettingsImport? ImportReport = null,
    string? DefaultImportFile = null);

[JsonConverter(typeof(CasesConverter<AccountSaveError>))]
[JsonCase(typeof(AccountSaveErrorInvalidPort), "invalidPort")]
[JsonCase(typeof(AccountSaveErrorPasswordNeedsAccount), "passwordNeedsAccount")]
[JsonCase(typeof(AccountSaveErrorAppPasswordNeedsAddress), "appPasswordNeedsAddress")]
[JsonCase(typeof(AccountSaveErrorAccountValue), "accountValue")]
[JsonCase(typeof(AccountSaveErrorInvalidVpnPassword), "invalidVpnPassword")]
[JsonCase(typeof(AccountSaveErrorConfig), "config")]
[JsonCase(typeof(AccountSaveErrorSecretStore), "secretStore")]
public abstract record AccountSaveError
{
    private protected AccountSaveError() { }
}

public sealed record AccountSaveErrorInvalidPort : AccountSaveError;

public sealed record AccountSaveErrorPasswordNeedsAccount : AccountSaveError;

public sealed record AccountSaveErrorAppPasswordNeedsAddress : AccountSaveError;

public sealed record AccountSaveErrorAccountValue(AccountValueProblem Problem) : AccountSaveError;

public sealed record AccountSaveErrorInvalidVpnPassword(uint MaxBytes) : AccountSaveError;

public sealed record AccountSaveErrorConfig(ConfigFileError Error) : AccountSaveError;

public sealed record AccountSaveErrorSecretStore(SecretStoreFailure Failure) : AccountSaveError;

[JsonConverter(typeof(CasesConverter<ClientSettingsImport>))]
[JsonCase(typeof(ClientSettingsImportImported), "imported")]
[JsonCase(typeof(ClientSettingsImportFailed), "failed")]
public abstract record ClientSettingsImport
{
    private protected ClientSettingsImport() { }
}

public sealed record ClientSettingsImportImported(ImportReport Report) : ClientSettingsImport;

public sealed record ClientSettingsImportFailed(ImportError Error) : ClientSettingsImport;

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

[JsonConverter(typeof(CasesConverter<ImportedPassword>))]
[JsonCase(typeof(ImportedPasswordSaved), "saved")]
[JsonCase(typeof(ImportedPasswordNotSavedWithoutUsername), "notSavedWithoutUsername")]
[JsonCase(typeof(ImportedPasswordNotSaved), "notSaved")]
public abstract record ImportedPassword
{
    private protected ImportedPassword() { }
}

public sealed record ImportedPasswordSaved : ImportedPassword;

public sealed record ImportedPasswordNotSavedWithoutUsername : ImportedPassword;

public sealed record ImportedPasswordNotSaved(SecretStoreFailure Failure) : ImportedPassword;

[JsonConverter(typeof(CasesConverter<ImportError>))]
[JsonCase(typeof(ImportErrorSourceNotFound), "sourceNotFound")]
[JsonCase(typeof(ImportErrorUnreadable), "unreadable")]
[JsonCase(typeof(ImportErrorMissingHost), "missingHost")]
[JsonCase(typeof(ImportErrorConfig), "config")]
[JsonCase(typeof(ImportErrorNoFortiClientConnection), "noFortiClientConnection")]
[JsonCase(typeof(ImportErrorInvalidFortiClientServer), "invalidFortiClientServer")]
public abstract record ImportError
{
    private protected ImportError() { }
}

public sealed record ImportErrorSourceNotFound(string Path) : ImportError;

public sealed record ImportErrorUnreadable(string Path, FileProblem Problem) : ImportError;

public sealed record ImportErrorMissingHost(string Path) : ImportError;

public sealed record ImportErrorConfig(ConfigFileError Error) : ImportError;

public sealed record ImportErrorNoFortiClientConnection : ImportError;

public sealed record ImportErrorInvalidFortiClientServer(string Connection) : ImportError;

// Auto update

/// <param name="UpdateNow">Also "Try Again" after a failure.</param>
/// <param name="Cancel">Cancels the download.</param>
/// <param name="CardDot">The dot on the Software Update card.</param>
/// <param name="MenuBadge">The badge in the menu.</param>
public sealed record AutoUpdateSection(
    string CurrentVersion,
    UpdateStatus Status,
    UpdateProgress Progress,
    ButtonState Check,
    ButtonState UpdateNow,
    ButtonState Cancel,
    ButtonState Restart,
    bool CardDot,
    bool MenuBadge,
    string? FailedInstallVersion = null);

[JsonConverter(typeof(CasesConverter<UpdateStatus>))]
[JsonCase(typeof(UpdateStatusChecking), "checking")]
[JsonCase(typeof(UpdateStatusUpToDate), "upToDate")]
[JsonCase(typeof(UpdateStatusAvailable), "available")]
[JsonCase(typeof(UpdateStatusCheckFailed), "checkFailed")]
public abstract record UpdateStatus
{
    private protected UpdateStatus() { }
}

public sealed record UpdateStatusChecking : UpdateStatus;

public sealed record UpdateStatusUpToDate : UpdateStatus;

public sealed record UpdateStatusAvailable(
    string Version,
    string ReleasePageUrl,
    ulong InstallerBytes,
    IReadOnlyDictionary<string, IReadOnlyList<string>> ReleaseNoteTitles) : UpdateStatus;

public sealed record UpdateStatusCheckFailed(UpdateCheckFailure Reason) : UpdateStatus;

[JsonConverter(typeof(CasesConverter<UpdateCheckFailure>))]
[JsonCase(typeof(UpdateCheckFailureHttpStatus), "httpStatus")]
[JsonCase(typeof(UpdateCheckFailureNetwork), "network")]
[JsonCase(typeof(UpdateCheckFailureInvalidReply), "invalidReply")]
[JsonCase(typeof(UpdateCheckFailureInstallerMissing), "installerMissing")]
public abstract record UpdateCheckFailure
{
    private protected UpdateCheckFailure() { }
}

public sealed record UpdateCheckFailureHttpStatus(ushort Code) : UpdateCheckFailure;

public sealed record UpdateCheckFailureNetwork(string Detail) : UpdateCheckFailure;

public sealed record UpdateCheckFailureInvalidReply : UpdateCheckFailure;

public sealed record UpdateCheckFailureInstallerMissing : UpdateCheckFailure;

/// <summary><c>Percent</c> is the progress bar's value; there is no bar without it.</summary>
[JsonConverter(typeof(CasesConverter<UpdateProgress>))]
[JsonCase(typeof(UpdateProgressIdle), "idle")]
[JsonCase(typeof(UpdateProgressDownloading), "downloading")]
[JsonCase(typeof(UpdateProgressPreparing), "preparing")]
[JsonCase(typeof(UpdateProgressReadyToRestart), "readyToRestart")]
[JsonCase(typeof(UpdateProgressInstalling), "installing")]
[JsonCase(typeof(UpdateProgressFailed), "failed")]
public abstract record UpdateProgress
{
    private protected UpdateProgress() { }
}

public sealed record UpdateProgressIdle : UpdateProgress;

/// <summary>Counts the installer's bytes.</summary>
public sealed record UpdateProgressDownloading(
    ulong ReceivedBytes,
    ulong TotalBytes,
    byte Percent,
    RemainingTime Remaining) : UpdateProgress;

/// <summary>The Helper checks the downloaded files.</summary>
public sealed record UpdateProgressPreparing(byte Percent) : UpdateProgress;

public sealed record UpdateProgressReadyToRestart(string Version) : UpdateProgress;

/// <summary>From Restart App until the app quits for the installer, or the install fails.</summary>
public sealed record UpdateProgressInstalling(string Version) : UpdateProgress;

public sealed record UpdateProgressFailed(InstallFailure Reason) : UpdateProgress;

[JsonConverter(typeof(CasesConverter<RemainingTime>))]
[JsonCase(typeof(RemainingTimeCalculating), "calculating")]
[JsonCase(typeof(RemainingTimeMinutes), "minutes")]
[JsonCase(typeof(RemainingTimeSeconds), "seconds")]
public abstract record RemainingTime
{
    private protected RemainingTime() { }
}

public sealed record RemainingTimeCalculating : RemainingTime;

/// <summary>Whole minutes, rounded up; over 55 seconds.</summary>
public sealed record RemainingTimeMinutes([property: JsonPropertyName("minutes")] ulong Value) : RemainingTime;

/// <summary>Rounded up to a multiple of 5, from 5 to 55.</summary>
public sealed record RemainingTimeSeconds([property: JsonPropertyName("seconds")] ulong Value) : RemainingTime;

[JsonConverter(typeof(CasesConverter<InstallFailure>))]
[JsonCase(typeof(InstallFailureDownloadFailed), "downloadFailed")]
[JsonCase(typeof(InstallFailureWaiterFailed), "waiterFailed")]
[JsonCase(typeof(InstallFailureHelperUnreachable), "helperUnreachable")]
[JsonCase(typeof(InstallFailureHelperBuildMismatch), "helperBuildMismatch")]
[JsonCase(typeof(InstallFailureHelperFailed), "helperFailed")]
[JsonCase(typeof(InstallFailureHelperRefused), "helperRefused")]
public abstract record InstallFailure
{
    private protected InstallFailure() { }
}

public sealed record InstallFailureDownloadFailed(string Detail) : InstallFailure;

public sealed record InstallFailureWaiterFailed(string Description) : InstallFailure;

public sealed record InstallFailureHelperUnreachable(HelperChannelFailure Failure) : InstallFailure;

public sealed record InstallFailureHelperBuildMismatch(string HelperBuild, string AppBuild) : InstallFailure;

public sealed record InstallFailureHelperFailed(string Message) : InstallFailure;

public sealed record InstallFailureHelperRefused(UpdateRefusal Reason, string Message) : InstallFailure;

[JsonConverter(typeof(EnumNameConverter<UpdateRefusal>))]
public enum UpdateRefusal
{
    SignatureInvalid,
    ChecksumMismatch,
    VersionNotNewer,
    AlreadyInstalling,
    FilesUnreadable,

    /// <summary>An install was asked for with no update prepared.</summary>
    NothingPrepared,
}
