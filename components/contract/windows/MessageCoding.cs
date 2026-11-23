using System.Reflection;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract;

/// <summary>
/// The JSON coding of every message to and from the runtime and over the Helper's pipe. It is
/// Swift's Codable coding, which the Rust side follows, so C# reads and writes the same shapes as
/// Swift: a type coded by <see cref="CasesConverter{T}"/> is an object keyed by its case name, a
/// value without a label uses the key <c>_0</c>, a case without data is <c>{}</c>, an enum value is
/// its name in camel case, times are whole milliseconds since 1970, and absent optional values are
/// left out. Decoding is strict: an unknown key, a missing value or an unknown case is an error,
/// since the app and the runtime are one build.
/// </summary>
/// <remarks>
/// The coding is generated when the app is built, without reflection, as the app is compiled ahead
/// of time (Native AOT). The generator follows every type a message holds but not from a type to
/// its cases, so each case is listed below; MessageCodingTests fails on a case missing there.
/// </remarks>
[JsonSourceGenerationOptions(
    PropertyNamingPolicy = JsonKnownNamingPolicy.CamelCase,
    DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
    RespectNullableAnnotations = true,
    RespectRequiredConstructorParameters = true,
    Converters = [typeof(MillisecondsConverter)])]
[JsonSerializable(typeof(AppInternal.ToRuntime))]
[JsonSerializable(typeof(AppInternal.FromRuntime))]
[JsonSerializable(typeof(HelperPipe.Request))]
[JsonSerializable(typeof(HelperPipe.Message))]
// Every case, under its type's name and its own.
[JsonSerializable(typeof(AppInternal.ToRuntimeLaunched), TypeInfoPropertyName = "ToRuntimeLaunched")]
[JsonSerializable(typeof(AppInternal.ToRuntimeCommand), TypeInfoPropertyName = "ToRuntimeCommand")]
[JsonSerializable(typeof(AppInternal.ToRuntimePortResponse), TypeInfoPropertyName = "ToRuntimePortResponse")]
[JsonSerializable(typeof(AppInternal.ToRuntimeEvent), TypeInfoPropertyName = "ToRuntimeEvent")]

[JsonSerializable(typeof(AppInternal.FromRuntimeSection), TypeInfoPropertyName = "FromRuntimeSection")]
[JsonSerializable(typeof(AppInternal.FromRuntimeLogLines), TypeInfoPropertyName = "FromRuntimeLogLines")]
[JsonSerializable(typeof(AppInternal.FromRuntimePortRequest), TypeInfoPropertyName = "FromRuntimePortRequest")]
[JsonSerializable(typeof(AppInternal.FromRuntimePortAction), TypeInfoPropertyName = "FromRuntimePortAction")]
[JsonSerializable(typeof(AppInternal.FromRuntimeOpenSettings), TypeInfoPropertyName = "FromRuntimeOpenSettings")]

[JsonSerializable(typeof(AppInternal.CommandConnect), TypeInfoPropertyName = "CommandConnect")]
[JsonSerializable(typeof(AppInternal.CommandDisconnect), TypeInfoPropertyName = "CommandDisconnect")]
[JsonSerializable(typeof(AppInternal.CommandReconnectNow), TypeInfoPropertyName = "CommandReconnectNow")]
[JsonSerializable(typeof(AppInternal.CommandSettingsOpened), TypeInfoPropertyName = "CommandSettingsOpened")]
[JsonSerializable(typeof(AppInternal.CommandSettingsClosed), TypeInfoPropertyName = "CommandSettingsClosed")]
[JsonSerializable(typeof(AppInternal.CommandEditAccount), TypeInfoPropertyName = "CommandEditAccount")]
[JsonSerializable(typeof(AppInternal.CommandSaveAccount), TypeInfoPropertyName = "CommandSaveAccount")]
[JsonSerializable(typeof(AppInternal.CommandImportClientSettings), TypeInfoPropertyName = "CommandImportClientSettings")]
[JsonSerializable(typeof(AppInternal.CommandDismissImportReport), TypeInfoPropertyName = "CommandDismissImportReport")]
[JsonSerializable(typeof(AppInternal.CommandSetOtpMethod), TypeInfoPropertyName = "CommandSetOtpMethod")]
[JsonSerializable(typeof(AppInternal.CommandTestGmail), TypeInfoPropertyName = "CommandTestGmail")]
[JsonSerializable(typeof(AppInternal.CommandConnectGoogleAccount), TypeInfoPropertyName = "CommandConnectGoogleAccount")]
[JsonSerializable(typeof(AppInternal.CommandCancelGoogleSignIn), TypeInfoPropertyName = "CommandCancelGoogleSignIn")]
[JsonSerializable(typeof(AppInternal.CommandDisconnectGoogleAccount), TypeInfoPropertyName = "CommandDisconnectGoogleAccount")]
[JsonSerializable(typeof(AppInternal.CommandImportOAuthClient), TypeInfoPropertyName = "CommandImportOAuthClient")]
[JsonSerializable(typeof(AppInternal.CommandUseBuiltInOAuthClient), TypeInfoPropertyName = "CommandUseBuiltInOAuthClient")]
[JsonSerializable(typeof(AppInternal.CommandSetConnectionOption), TypeInfoPropertyName = "CommandSetConnectionOption")]
[JsonSerializable(typeof(AppInternal.CommandSetScheduleHours), TypeInfoPropertyName = "CommandSetScheduleHours")]
[JsonSerializable(typeof(AppInternal.CommandSetOpenAtLogin), TypeInfoPropertyName = "CommandSetOpenAtLogin")]
[JsonSerializable(typeof(AppInternal.CommandRegisterHelper), TypeInfoPropertyName = "CommandRegisterHelper")]
[JsonSerializable(typeof(AppInternal.CommandOpenHelperApprovalSettings), TypeInfoPropertyName = "CommandOpenHelperApprovalSettings")]
[JsonSerializable(typeof(AppInternal.CommandSetBypassRows), TypeInfoPropertyName = "CommandSetBypassRows")]
[JsonSerializable(typeof(AppInternal.CommandAddCurrentNetwork), TypeInfoPropertyName = "CommandAddCurrentNetwork")]
[JsonSerializable(typeof(AppInternal.CommandCheckForUpdates), TypeInfoPropertyName = "CommandCheckForUpdates")]
[JsonSerializable(typeof(AppInternal.CommandUpdatePageOpened), TypeInfoPropertyName = "CommandUpdatePageOpened")]
[JsonSerializable(typeof(AppInternal.CommandUpdateNow), TypeInfoPropertyName = "CommandUpdateNow")]
[JsonSerializable(typeof(AppInternal.CommandUpdateNowFromNotification), TypeInfoPropertyName = "CommandUpdateNowFromNotification")]
[JsonSerializable(typeof(AppInternal.CommandCancelUpdateDownload), TypeInfoPropertyName = "CommandCancelUpdateDownload")]
[JsonSerializable(typeof(AppInternal.CommandRestartToUpdate), TypeInfoPropertyName = "CommandRestartToUpdate")]

[JsonSerializable(typeof(AppInternal.ConnectionOptionScheduleEnabled), TypeInfoPropertyName = "ConnectionOptionScheduleEnabled")]
[JsonSerializable(typeof(AppInternal.ConnectionOptionScheduleDay), TypeInfoPropertyName = "ConnectionOptionScheduleDay")]
[JsonSerializable(typeof(AppInternal.ConnectionOptionAwayMinutes), TypeInfoPropertyName = "ConnectionOptionAwayMinutes")]
[JsonSerializable(typeof(AppInternal.ConnectionOptionAutoConnectOnLaunch), TypeInfoPropertyName = "ConnectionOptionAutoConnectOnLaunch")]
[JsonSerializable(typeof(AppInternal.ConnectionOptionSessionMaxMinutes), TypeInfoPropertyName = "ConnectionOptionSessionMaxMinutes")]
[JsonSerializable(typeof(AppInternal.ConnectionOptionProactiveReconnect), TypeInfoPropertyName = "ConnectionOptionProactiveReconnect")]
[JsonSerializable(typeof(AppInternal.ConnectionOptionProactiveMarginMinutes), TypeInfoPropertyName = "ConnectionOptionProactiveMarginMinutes")]
[JsonSerializable(typeof(AppInternal.ConnectionOptionReconnectAfterSleep), TypeInfoPropertyName = "ConnectionOptionReconnectAfterSleep")]
[JsonSerializable(typeof(AppInternal.ConnectionOptionMinSleepMinutes), TypeInfoPropertyName = "ConnectionOptionMinSleepMinutes")]

[JsonSerializable(typeof(AppInternal.SectionConnection), TypeInfoPropertyName = "SectionConnection")]
[JsonSerializable(typeof(AppInternal.SectionSchedule), TypeInfoPropertyName = "SectionSchedule")]
[JsonSerializable(typeof(AppInternal.SectionReadiness), TypeInfoPropertyName = "SectionReadiness")]
[JsonSerializable(typeof(AppInternal.SectionGmail), TypeInfoPropertyName = "SectionGmail")]
[JsonSerializable(typeof(AppInternal.SectionBypassRules), TypeInfoPropertyName = "SectionBypassRules")]
[JsonSerializable(typeof(AppInternal.SectionConfig), TypeInfoPropertyName = "SectionConfig")]
[JsonSerializable(typeof(AppInternal.SectionAutoUpdate), TypeInfoPropertyName = "SectionAutoUpdate")]

[JsonSerializable(typeof(AppInternal.ConnectionStatusNotConnected), TypeInfoPropertyName = "ConnectionStatusNotConnected")]
[JsonSerializable(typeof(AppInternal.ConnectionStatusSetupRequired), TypeInfoPropertyName = "ConnectionStatusSetupRequired")]
[JsonSerializable(typeof(AppInternal.ConnectionStatusConnecting), TypeInfoPropertyName = "ConnectionStatusConnecting")]
[JsonSerializable(typeof(AppInternal.ConnectionStatusConnected), TypeInfoPropertyName = "ConnectionStatusConnected")]
[JsonSerializable(typeof(AppInternal.ConnectionStatusReconnectOnHold), TypeInfoPropertyName = "ConnectionStatusReconnectOnHold")]
[JsonSerializable(typeof(AppInternal.ConnectionStatusWaitingToReconnect), TypeInfoPropertyName = "ConnectionStatusWaitingToReconnect")]
[JsonSerializable(typeof(AppInternal.ConnectionStatusWaitingForNetwork), TypeInfoPropertyName = "ConnectionStatusWaitingForNetwork")]
[JsonSerializable(typeof(AppInternal.ConnectionStatusDisconnecting), TypeInfoPropertyName = "ConnectionStatusDisconnecting")]
[JsonSerializable(typeof(AppInternal.ConnectionStatusFailed), TypeInfoPropertyName = "ConnectionStatusFailed")]

[JsonSerializable(typeof(AppInternal.ProactiveReconnectAt), TypeInfoPropertyName = "ProactiveReconnectAt")]
[JsonSerializable(typeof(AppInternal.ProactiveReconnectPausedBySchedule), TypeInfoPropertyName = "ProactiveReconnectPausedBySchedule")]
[JsonSerializable(typeof(AppInternal.ProactiveReconnectOff), TypeInfoPropertyName = "ProactiveReconnectOff")]

[JsonSerializable(typeof(AppInternal.FailureReasonVpnLoginRejected), TypeInfoPropertyName = "FailureReasonVpnLoginRejected")]
[JsonSerializable(typeof(AppInternal.FailureReasonOtpNotReceived), TypeInfoPropertyName = "FailureReasonOtpNotReceived")]
[JsonSerializable(typeof(AppInternal.FailureReasonOtpRejected), TypeInfoPropertyName = "FailureReasonOtpRejected")]
[JsonSerializable(typeof(AppInternal.FailureReasonTunnelSetupFailed), TypeInfoPropertyName = "FailureReasonTunnelSetupFailed")]
[JsonSerializable(typeof(AppInternal.FailureReasonCertificateChangedAfterLogin), TypeInfoPropertyName = "FailureReasonCertificateChangedAfterLogin")]
[JsonSerializable(typeof(AppInternal.FailureReasonGmailRejected), TypeInfoPropertyName = "FailureReasonGmailRejected")]
[JsonSerializable(typeof(AppInternal.FailureReasonTooManyReconnects), TypeInfoPropertyName = "FailureReasonTooManyReconnects")]
[JsonSerializable(typeof(AppInternal.FailureReasonNetwork), TypeInfoPropertyName = "FailureReasonNetwork")]
[JsonSerializable(typeof(AppInternal.FailureReasonHelper), TypeInfoPropertyName = "FailureReasonHelper")]

[JsonSerializable(typeof(AppInternal.WaitReasonNetwork), TypeInfoPropertyName = "WaitReasonNetwork")]
[JsonSerializable(typeof(AppInternal.WaitReasonNetworkSetupFailed), TypeInfoPropertyName = "WaitReasonNetworkSetupFailed")]
[JsonSerializable(typeof(AppInternal.WaitReasonServerEndedSession), TypeInfoPropertyName = "WaitReasonServerEndedSession")]
[JsonSerializable(typeof(AppInternal.WaitReasonForcedTermination), TypeInfoPropertyName = "WaitReasonForcedTermination")]
[JsonSerializable(typeof(AppInternal.WaitReasonAuthFailure), TypeInfoPropertyName = "WaitReasonAuthFailure")]
[JsonSerializable(typeof(AppInternal.WaitReasonAuthCooldown), TypeInfoPropertyName = "WaitReasonAuthCooldown")]
[JsonSerializable(typeof(AppInternal.WaitReasonSleep), TypeInfoPropertyName = "WaitReasonSleep")]
[JsonSerializable(typeof(AppInternal.WaitReasonUserAway), TypeInfoPropertyName = "WaitReasonUserAway")]

[JsonSerializable(typeof(AppInternal.SetupItemConfigFile), TypeInfoPropertyName = "SetupItemConfigFile")]
[JsonSerializable(typeof(AppInternal.SetupItemAccountDetails), TypeInfoPropertyName = "SetupItemAccountDetails")]
[JsonSerializable(typeof(AppInternal.SetupItemVpnPassword), TypeInfoPropertyName = "SetupItemVpnPassword")]
[JsonSerializable(typeof(AppInternal.SetupItemGmailCredential), TypeInfoPropertyName = "SetupItemGmailCredential")]
[JsonSerializable(typeof(AppInternal.SetupItemHelperApproval), TypeInfoPropertyName = "SetupItemHelperApproval")]
[JsonSerializable(typeof(AppInternal.SetupItemHelperVersion), TypeInfoPropertyName = "SetupItemHelperVersion")]

[JsonSerializable(typeof(AppInternal.ScheduleOutlookConnects), TypeInfoPropertyName = "ScheduleOutlookConnects")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlookNoDays), TypeInfoPropertyName = "ScheduleOutlookNoDays")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlookWithinHours), TypeInfoPropertyName = "ScheduleOutlookWithinHours")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlookOutsideHoursPresent), TypeInfoPropertyName = "ScheduleOutlookOutsideHoursPresent")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlookHoldingWhileAway), TypeInfoPropertyName = "ScheduleOutlookHoldingWhileAway")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlookSuspended), TypeInfoPropertyName = "ScheduleOutlookSuspended")]

[JsonSerializable(typeof(AppInternal.ScheduleHoursErrorEndNotAfterStart), TypeInfoPropertyName = "ScheduleHoursErrorEndNotAfterStart")]
[JsonSerializable(typeof(AppInternal.ScheduleHoursErrorNotSaved), TypeInfoPropertyName = "ScheduleHoursErrorNotSaved")]

[JsonSerializable(typeof(AppInternal.HelperStateChecking), TypeInfoPropertyName = "HelperStateChecking")]
[JsonSerializable(typeof(AppInternal.HelperStateNotRegistered), TypeInfoPropertyName = "HelperStateNotRegistered")]
[JsonSerializable(typeof(AppInternal.HelperStateRequiresApproval), TypeInfoPropertyName = "HelperStateRequiresApproval")]
[JsonSerializable(typeof(AppInternal.HelperStateRunning), TypeInfoPropertyName = "HelperStateRunning")]
[JsonSerializable(typeof(AppInternal.HelperStateVersionMismatch), TypeInfoPropertyName = "HelperStateVersionMismatch")]
[JsonSerializable(typeof(AppInternal.HelperStateUnreachable), TypeInfoPropertyName = "HelperStateUnreachable")]

[JsonSerializable(typeof(AppInternal.GoogleAccountStatusNotConnected), TypeInfoPropertyName = "GoogleAccountStatusNotConnected")]
[JsonSerializable(typeof(AppInternal.GoogleAccountStatusSigningIn), TypeInfoPropertyName = "GoogleAccountStatusSigningIn")]
[JsonSerializable(typeof(AppInternal.GoogleAccountStatusDisconnecting), TypeInfoPropertyName = "GoogleAccountStatusDisconnecting")]
[JsonSerializable(typeof(AppInternal.GoogleAccountStatusConnected), TypeInfoPropertyName = "GoogleAccountStatusConnected")]

[JsonSerializable(typeof(AppInternal.OAuthClientSourceBuiltIn), TypeInfoPropertyName = "OAuthClientSourceBuiltIn")]
[JsonSerializable(typeof(AppInternal.OAuthClientSourceImported), TypeInfoPropertyName = "OAuthClientSourceImported")]
[JsonSerializable(typeof(AppInternal.OAuthClientSourceNotConfigured), TypeInfoPropertyName = "OAuthClientSourceNotConfigured")]

[JsonSerializable(typeof(AppInternal.GoogleErrorClientFileUnreadable), TypeInfoPropertyName = "GoogleErrorClientFileUnreadable")]
[JsonSerializable(typeof(AppInternal.GoogleErrorClientFileInvalid), TypeInfoPropertyName = "GoogleErrorClientFileInvalid")]
[JsonSerializable(typeof(AppInternal.GoogleErrorListenerFailed), TypeInfoPropertyName = "GoogleErrorListenerFailed")]
[JsonSerializable(typeof(AppInternal.GoogleErrorBrowserUnavailable), TypeInfoPropertyName = "GoogleErrorBrowserUnavailable")]
[JsonSerializable(typeof(AppInternal.GoogleErrorTimedOut), TypeInfoPropertyName = "GoogleErrorTimedOut")]
[JsonSerializable(typeof(AppInternal.GoogleErrorCancelledInBrowser), TypeInfoPropertyName = "GoogleErrorCancelledInBrowser")]
[JsonSerializable(typeof(AppInternal.GoogleErrorDenied), TypeInfoPropertyName = "GoogleErrorDenied")]
[JsonSerializable(typeof(AppInternal.GoogleErrorTokenRejected), TypeInfoPropertyName = "GoogleErrorTokenRejected")]
[JsonSerializable(typeof(AppInternal.GoogleErrorClientRejected), TypeInfoPropertyName = "GoogleErrorClientRejected")]
[JsonSerializable(typeof(AppInternal.GoogleErrorNoRefreshToken), TypeInfoPropertyName = "GoogleErrorNoRefreshToken")]
[JsonSerializable(typeof(AppInternal.GoogleErrorScopeNotGranted), TypeInfoPropertyName = "GoogleErrorScopeNotGranted")]
[JsonSerializable(typeof(AppInternal.GoogleErrorWrongAccount), TypeInfoPropertyName = "GoogleErrorWrongAccount")]
[JsonSerializable(typeof(AppInternal.GoogleErrorGmailApiRefused), TypeInfoPropertyName = "GoogleErrorGmailApiRefused")]
[JsonSerializable(typeof(AppInternal.GoogleErrorNetwork), TypeInfoPropertyName = "GoogleErrorNetwork")]
[JsonSerializable(typeof(AppInternal.GoogleErrorSecretStore), TypeInfoPropertyName = "GoogleErrorSecretStore")]
[JsonSerializable(typeof(AppInternal.GoogleErrorConfig), TypeInfoPropertyName = "GoogleErrorConfig")]

[JsonSerializable(typeof(AppInternal.OAuthClientFileProblemNotJson), TypeInfoPropertyName = "OAuthClientFileProblemNotJson")]
[JsonSerializable(typeof(AppInternal.OAuthClientFileProblemWebClient), TypeInfoPropertyName = "OAuthClientFileProblemWebClient")]
[JsonSerializable(typeof(AppInternal.OAuthClientFileProblemNoDesktopClient), TypeInfoPropertyName = "OAuthClientFileProblemNoDesktopClient")]
[JsonSerializable(typeof(AppInternal.OAuthClientFileProblemMissingField), TypeInfoPropertyName = "OAuthClientFileProblemMissingField")]
[JsonSerializable(typeof(AppInternal.OAuthClientFileProblemInvalidField), TypeInfoPropertyName = "OAuthClientFileProblemInvalidField")]

[JsonSerializable(typeof(AppInternal.GmailTestIdle), TypeInfoPropertyName = "GmailTestIdle")]
[JsonSerializable(typeof(AppInternal.GmailTestRunning), TypeInfoPropertyName = "GmailTestRunning")]
[JsonSerializable(typeof(AppInternal.GmailTestImapSucceeded), TypeInfoPropertyName = "GmailTestImapSucceeded")]
[JsonSerializable(typeof(AppInternal.GmailTestApiSucceeded), TypeInfoPropertyName = "GmailTestApiSucceeded")]
[JsonSerializable(typeof(AppInternal.GmailTestFailed), TypeInfoPropertyName = "GmailTestFailed")]

[JsonSerializable(typeof(AppInternal.GmailTestFailureEnterAddress), TypeInfoPropertyName = "GmailTestFailureEnterAddress")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureEnterAppPassword), TypeInfoPropertyName = "GmailTestFailureEnterAppPassword")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureAppPasswordUnreadable), TypeInfoPropertyName = "GmailTestFailureAppPasswordUnreadable")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureImapLoginRejected), TypeInfoPropertyName = "GmailTestFailureImapLoginRejected")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureMailboxesMissing), TypeInfoPropertyName = "GmailTestFailureMailboxesMissing")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureImapUnreachable), TypeInfoPropertyName = "GmailTestFailureImapUnreachable")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureNoOAuthClient), TypeInfoPropertyName = "GmailTestFailureNoOAuthClient")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureNotSignedIn), TypeInfoPropertyName = "GmailTestFailureNotSignedIn")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureAccessRevoked), TypeInfoPropertyName = "GmailTestFailureAccessRevoked")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureClientRejected), TypeInfoPropertyName = "GmailTestFailureClientRejected")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureSecretStore), TypeInfoPropertyName = "GmailTestFailureSecretStore")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureOtherAccount), TypeInfoPropertyName = "GmailTestFailureOtherAccount")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureAccessTokenRejected), TypeInfoPropertyName = "GmailTestFailureAccessTokenRejected")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureGmailApiRefused), TypeInfoPropertyName = "GmailTestFailureGmailApiRefused")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureRateLimited), TypeInfoPropertyName = "GmailTestFailureRateLimited")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureGoogleUnreachable), TypeInfoPropertyName = "GmailTestFailureGoogleUnreachable")]
[JsonSerializable(typeof(AppInternal.GmailTestFailureGoogleReturnedError), TypeInfoPropertyName = "GmailTestFailureGoogleReturnedError")]

[JsonSerializable(typeof(AppInternal.ImapFailureNetwork), TypeInfoPropertyName = "ImapFailureNetwork")]
[JsonSerializable(typeof(AppInternal.ImapFailureTimedOut), TypeInfoPropertyName = "ImapFailureTimedOut")]
[JsonSerializable(typeof(AppInternal.ImapFailureServerClosed), TypeInfoPropertyName = "ImapFailureServerClosed")]
[JsonSerializable(typeof(AppInternal.ImapFailureProtocol), TypeInfoPropertyName = "ImapFailureProtocol")]
[JsonSerializable(typeof(AppInternal.ImapFailureRejected), TypeInfoPropertyName = "ImapFailureRejected")]

[JsonSerializable(typeof(AppInternal.ImapStepConnecting), TypeInfoPropertyName = "ImapStepConnecting")]
[JsonSerializable(typeof(AppInternal.ImapStepCommand), TypeInfoPropertyName = "ImapStepCommand")]

[JsonSerializable(typeof(AppInternal.BypassNoticeRowProblem), TypeInfoPropertyName = "BypassNoticeRowProblem")]
[JsonSerializable(typeof(AppInternal.BypassNoticeApplyFailed), TypeInfoPropertyName = "BypassNoticeApplyFailed")]

[JsonSerializable(typeof(AppInternal.RowProblemInvalid), TypeInfoPropertyName = "RowProblemInvalid")]
[JsonSerializable(typeof(AppInternal.RowProblemDuplicate), TypeInfoPropertyName = "RowProblemDuplicate")]
[JsonSerializable(typeof(AppInternal.RowProblemOverlap), TypeInfoPropertyName = "RowProblemOverlap")]
[JsonSerializable(typeof(AppInternal.RowProblemVpnServer), TypeInfoPropertyName = "RowProblemVpnServer")]

[JsonSerializable(typeof(AppInternal.BypassStatusChecking), TypeInfoPropertyName = "BypassStatusChecking")]
[JsonSerializable(typeof(AppInternal.BypassStatusNoConfigFile), TypeInfoPropertyName = "BypassStatusNoConfigFile")]
[JsonSerializable(typeof(AppInternal.BypassStatusApplied), TypeInfoPropertyName = "BypassStatusApplied")]
[JsonSerializable(typeof(AppInternal.BypassStatusVpnOff), TypeInfoPropertyName = "BypassStatusVpnOff")]
[JsonSerializable(typeof(AppInternal.BypassStatusOutsideNetworks), TypeInfoPropertyName = "BypassStatusOutsideNetworks")]
[JsonSerializable(typeof(AppInternal.BypassStatusNoNetworksListed), TypeInfoPropertyName = "BypassStatusNoNetworksListed")]
[JsonSerializable(typeof(AppInternal.BypassStatusConfigInvalid), TypeInfoPropertyName = "BypassStatusConfigInvalid")]
[JsonSerializable(typeof(AppInternal.BypassStatusNoNetwork), TypeInfoPropertyName = "BypassStatusNoNetwork")]
[JsonSerializable(typeof(AppInternal.BypassStatusFailed), TypeInfoPropertyName = "BypassStatusFailed")]

[JsonSerializable(typeof(AppInternal.BypassFailureRoutesNotKept), TypeInfoPropertyName = "BypassFailureRoutesNotKept")]
[JsonSerializable(typeof(AppInternal.BypassFailureNetworkChanged), TypeInfoPropertyName = "BypassFailureNetworkChanged")]
[JsonSerializable(typeof(AppInternal.BypassFailureHelperRefused), TypeInfoPropertyName = "BypassFailureHelperRefused")]
[JsonSerializable(typeof(AppInternal.BypassFailureSettingsNotChanged), TypeInfoPropertyName = "BypassFailureSettingsNotChanged")]
[JsonSerializable(typeof(AppInternal.BypassFailureHelperUnreachable), TypeInfoPropertyName = "BypassFailureHelperUnreachable")]
[JsonSerializable(typeof(AppInternal.BypassFailureHelperBuildMismatch), TypeInfoPropertyName = "BypassFailureHelperBuildMismatch")]

[JsonSerializable(typeof(AppInternal.AccountSaveErrorInvalidPort), TypeInfoPropertyName = "AccountSaveErrorInvalidPort")]
[JsonSerializable(typeof(AppInternal.AccountSaveErrorPasswordNeedsAccount), TypeInfoPropertyName = "AccountSaveErrorPasswordNeedsAccount")]
[JsonSerializable(typeof(AppInternal.AccountSaveErrorAppPasswordNeedsAddress), TypeInfoPropertyName = "AccountSaveErrorAppPasswordNeedsAddress")]
[JsonSerializable(typeof(AppInternal.AccountSaveErrorAccountValue), TypeInfoPropertyName = "AccountSaveErrorAccountValue")]
[JsonSerializable(typeof(AppInternal.AccountSaveErrorInvalidVpnPassword), TypeInfoPropertyName = "AccountSaveErrorInvalidVpnPassword")]
[JsonSerializable(typeof(AppInternal.AccountSaveErrorConfig), TypeInfoPropertyName = "AccountSaveErrorConfig")]
[JsonSerializable(typeof(AppInternal.AccountSaveErrorSecretStore), TypeInfoPropertyName = "AccountSaveErrorSecretStore")]

[JsonSerializable(typeof(AppInternal.ClientSettingsImportImported), TypeInfoPropertyName = "ClientSettingsImportImported")]
[JsonSerializable(typeof(AppInternal.ClientSettingsImportFailed), TypeInfoPropertyName = "ClientSettingsImportFailed")]

[JsonSerializable(typeof(AppInternal.ImportedPasswordSaved), TypeInfoPropertyName = "ImportedPasswordSaved")]
[JsonSerializable(typeof(AppInternal.ImportedPasswordNotSavedWithoutUsername), TypeInfoPropertyName = "ImportedPasswordNotSavedWithoutUsername")]
[JsonSerializable(typeof(AppInternal.ImportedPasswordNotSaved), TypeInfoPropertyName = "ImportedPasswordNotSaved")]

[JsonSerializable(typeof(AppInternal.ImportErrorSourceNotFound), TypeInfoPropertyName = "ImportErrorSourceNotFound")]
[JsonSerializable(typeof(AppInternal.ImportErrorUnreadable), TypeInfoPropertyName = "ImportErrorUnreadable")]
[JsonSerializable(typeof(AppInternal.ImportErrorMissingHost), TypeInfoPropertyName = "ImportErrorMissingHost")]
[JsonSerializable(typeof(AppInternal.ImportErrorConfig), TypeInfoPropertyName = "ImportErrorConfig")]
[JsonSerializable(typeof(AppInternal.ImportErrorNoFortiClientConnection), TypeInfoPropertyName = "ImportErrorNoFortiClientConnection")]
[JsonSerializable(typeof(AppInternal.ImportErrorInvalidFortiClientServer), TypeInfoPropertyName = "ImportErrorInvalidFortiClientServer")]

[JsonSerializable(typeof(AppInternal.UpdateStatusChecking), TypeInfoPropertyName = "UpdateStatusChecking")]
[JsonSerializable(typeof(AppInternal.UpdateStatusUpToDate), TypeInfoPropertyName = "UpdateStatusUpToDate")]
[JsonSerializable(typeof(AppInternal.UpdateStatusAvailable), TypeInfoPropertyName = "UpdateStatusAvailable")]
[JsonSerializable(typeof(AppInternal.UpdateStatusCheckFailed), TypeInfoPropertyName = "UpdateStatusCheckFailed")]

[JsonSerializable(typeof(AppInternal.UpdateCheckFailureHttpStatus), TypeInfoPropertyName = "UpdateCheckFailureHttpStatus")]
[JsonSerializable(typeof(AppInternal.UpdateCheckFailureNetwork), TypeInfoPropertyName = "UpdateCheckFailureNetwork")]
[JsonSerializable(typeof(AppInternal.UpdateCheckFailureInvalidReply), TypeInfoPropertyName = "UpdateCheckFailureInvalidReply")]
[JsonSerializable(typeof(AppInternal.UpdateCheckFailureInstallerMissing), TypeInfoPropertyName = "UpdateCheckFailureInstallerMissing")]

[JsonSerializable(typeof(AppInternal.UpdateProgressIdle), TypeInfoPropertyName = "UpdateProgressIdle")]
[JsonSerializable(typeof(AppInternal.UpdateProgressDownloading), TypeInfoPropertyName = "UpdateProgressDownloading")]
[JsonSerializable(typeof(AppInternal.UpdateProgressPreparing), TypeInfoPropertyName = "UpdateProgressPreparing")]
[JsonSerializable(typeof(AppInternal.UpdateProgressReadyToRestart), TypeInfoPropertyName = "UpdateProgressReadyToRestart")]
[JsonSerializable(typeof(AppInternal.UpdateProgressInstalling), TypeInfoPropertyName = "UpdateProgressInstalling")]
[JsonSerializable(typeof(AppInternal.UpdateProgressFailed), TypeInfoPropertyName = "UpdateProgressFailed")]

[JsonSerializable(typeof(AppInternal.RemainingTimeCalculating), TypeInfoPropertyName = "RemainingTimeCalculating")]
[JsonSerializable(typeof(AppInternal.RemainingTimeMinutes), TypeInfoPropertyName = "RemainingTimeMinutes")]
[JsonSerializable(typeof(AppInternal.RemainingTimeSeconds), TypeInfoPropertyName = "RemainingTimeSeconds")]

[JsonSerializable(typeof(AppInternal.InstallFailureDownloadFailed), TypeInfoPropertyName = "InstallFailureDownloadFailed")]
[JsonSerializable(typeof(AppInternal.InstallFailureWaiterFailed), TypeInfoPropertyName = "InstallFailureWaiterFailed")]
[JsonSerializable(typeof(AppInternal.InstallFailureHelperUnreachable), TypeInfoPropertyName = "InstallFailureHelperUnreachable")]
[JsonSerializable(typeof(AppInternal.InstallFailureHelperBuildMismatch), TypeInfoPropertyName = "InstallFailureHelperBuildMismatch")]
[JsonSerializable(typeof(AppInternal.InstallFailureHelperFailed), TypeInfoPropertyName = "InstallFailureHelperFailed")]
[JsonSerializable(typeof(AppInternal.InstallFailureHelperRefused), TypeInfoPropertyName = "InstallFailureHelperRefused")]

[JsonSerializable(typeof(AppInternal.PortRequestHelperRegistration), TypeInfoPropertyName = "PortRequestHelperRegistration")]
[JsonSerializable(typeof(AppInternal.PortRequestRegisterHelper), TypeInfoPropertyName = "PortRequestRegisterHelper")]
[JsonSerializable(typeof(AppInternal.PortRequestHelperBuild), TypeInfoPropertyName = "PortRequestHelperBuild")]
[JsonSerializable(typeof(AppInternal.PortRequestCallHelper), TypeInfoPropertyName = "PortRequestCallHelper")]
[JsonSerializable(typeof(AppInternal.PortRequestRouteInterface), TypeInfoPropertyName = "PortRequestRouteInterface")]
[JsonSerializable(typeof(AppInternal.PortRequestPrimaryRouter), TypeInfoPropertyName = "PortRequestPrimaryRouter")]
[JsonSerializable(typeof(AppInternal.PortRequestScreenState), TypeInfoPropertyName = "PortRequestScreenState")]
[JsonSerializable(typeof(AppInternal.PortRequestOpenBrowser), TypeInfoPropertyName = "PortRequestOpenBrowser")]
[JsonSerializable(typeof(AppInternal.PortRequestLoginItemRegistration), TypeInfoPropertyName = "PortRequestLoginItemRegistration")]
[JsonSerializable(typeof(AppInternal.PortRequestSetLoginItem), TypeInfoPropertyName = "PortRequestSetLoginItem")]
[JsonSerializable(typeof(AppInternal.PortRequestFortiClientConnections), TypeInfoPropertyName = "PortRequestFortiClientConnections")]
[JsonSerializable(typeof(AppInternal.PortRequestStartUpdateWaiter), TypeInfoPropertyName = "PortRequestStartUpdateWaiter")]

[JsonSerializable(typeof(AppInternal.PortResponseRegistration), TypeInfoPropertyName = "PortResponseRegistration")]
[JsonSerializable(typeof(AppInternal.PortResponseRegistered), TypeInfoPropertyName = "PortResponseRegistered")]
[JsonSerializable(typeof(AppInternal.PortResponseRegistrationFailed), TypeInfoPropertyName = "PortResponseRegistrationFailed")]
[JsonSerializable(typeof(AppInternal.PortResponseHelperBuild), TypeInfoPropertyName = "PortResponseHelperBuild")]
[JsonSerializable(typeof(AppInternal.PortResponseHelperReply), TypeInfoPropertyName = "PortResponseHelperReply")]
[JsonSerializable(typeof(AppInternal.PortResponseHelperReplyFlag), TypeInfoPropertyName = "PortResponseHelperReplyFlag")]
[JsonSerializable(typeof(AppInternal.PortResponseHelperUnreachable), TypeInfoPropertyName = "PortResponseHelperUnreachable")]
[JsonSerializable(typeof(AppInternal.PortResponseHelperBuildMismatch), TypeInfoPropertyName = "PortResponseHelperBuildMismatch")]
[JsonSerializable(typeof(AppInternal.PortResponseRouteInterface), TypeInfoPropertyName = "PortResponseRouteInterface")]
[JsonSerializable(typeof(AppInternal.PortResponsePrimaryRouter), TypeInfoPropertyName = "PortResponsePrimaryRouter")]
[JsonSerializable(typeof(AppInternal.PortResponseScreenState), TypeInfoPropertyName = "PortResponseScreenState")]
[JsonSerializable(typeof(AppInternal.PortResponseBrowserOpened), TypeInfoPropertyName = "PortResponseBrowserOpened")]
[JsonSerializable(typeof(AppInternal.PortResponseFortiClientConnections), TypeInfoPropertyName = "PortResponseFortiClientConnections")]
[JsonSerializable(typeof(AppInternal.PortResponseUpdateWaiterStarted), TypeInfoPropertyName = "PortResponseUpdateWaiterStarted")]
[JsonSerializable(typeof(AppInternal.PortResponseUpdateWaiterFailed), TypeInfoPropertyName = "PortResponseUpdateWaiterFailed")]

[JsonSerializable(typeof(AppInternal.PortActionLog), TypeInfoPropertyName = "PortActionLog")]
[JsonSerializable(typeof(AppInternal.PortActionPostNotification), TypeInfoPropertyName = "PortActionPostNotification")]
[JsonSerializable(typeof(AppInternal.PortActionHoldActivity), TypeInfoPropertyName = "PortActionHoldActivity")]
[JsonSerializable(typeof(AppInternal.PortActionOpenHelperApprovalSettings), TypeInfoPropertyName = "PortActionOpenHelperApprovalSettings")]
[JsonSerializable(typeof(AppInternal.PortActionUseHelperTeam), TypeInfoPropertyName = "PortActionUseHelperTeam")]
[JsonSerializable(typeof(AppInternal.PortActionReleaseUpdateWaiter), TypeInfoPropertyName = "PortActionReleaseUpdateWaiter")]
[JsonSerializable(typeof(AppInternal.PortActionStopUpdateWaiter), TypeInfoPropertyName = "PortActionStopUpdateWaiter")]
[JsonSerializable(typeof(AppInternal.PortActionApproveQuit), TypeInfoPropertyName = "PortActionApproveQuit")]


[JsonSerializable(typeof(AppInternal.HelperCallConnect), TypeInfoPropertyName = "HelperCallConnect")]
[JsonSerializable(typeof(AppInternal.HelperCallSubmitOtp), TypeInfoPropertyName = "HelperCallSubmitOtp")]
[JsonSerializable(typeof(AppInternal.HelperCallDisconnect), TypeInfoPropertyName = "HelperCallDisconnect")]
[JsonSerializable(typeof(AppInternal.HelperCallStatus), TypeInfoPropertyName = "HelperCallStatus")]
[JsonSerializable(typeof(AppInternal.HelperCallExitWhenIdle), TypeInfoPropertyName = "HelperCallExitWhenIdle")]
[JsonSerializable(typeof(AppInternal.HelperCallReadBypassRoutes), TypeInfoPropertyName = "HelperCallReadBypassRoutes")]
[JsonSerializable(typeof(AppInternal.HelperCallWriteBypassRoutes), TypeInfoPropertyName = "HelperCallWriteBypassRoutes")]
[JsonSerializable(typeof(AppInternal.HelperCallPrepareUpdate), TypeInfoPropertyName = "HelperCallPrepareUpdate")]
[JsonSerializable(typeof(AppInternal.HelperCallInstallUpdate), TypeInfoPropertyName = "HelperCallInstallUpdate")]
[JsonSerializable(typeof(AppInternal.HelperCallReadPreparedUpdate), TypeInfoPropertyName = "HelperCallReadPreparedUpdate")]
[JsonSerializable(typeof(AppInternal.HelperCallDiscardPreparedUpdate), TypeInfoPropertyName = "HelperCallDiscardPreparedUpdate")]

[JsonSerializable(typeof(AppInternal.HelperChannelFailureAppNotSigned), TypeInfoPropertyName = "HelperChannelFailureAppNotSigned")]
[JsonSerializable(typeof(AppInternal.HelperChannelFailureConnection), TypeInfoPropertyName = "HelperChannelFailureConnection")]
[JsonSerializable(typeof(AppInternal.HelperChannelFailureConnectionLost), TypeInfoPropertyName = "HelperChannelFailureConnectionLost")]
[JsonSerializable(typeof(AppInternal.HelperChannelFailureNoReply), TypeInfoPropertyName = "HelperChannelFailureNoReply")]
[JsonSerializable(typeof(AppInternal.HelperChannelFailureUnreadableReply), TypeInfoPropertyName = "HelperChannelFailureUnreadableReply")]

[JsonSerializable(typeof(AppInternal.RegistrationErrorNotInstalledCopy), TypeInfoPropertyName = "RegistrationErrorNotInstalledCopy")]
[JsonSerializable(typeof(AppInternal.RegistrationErrorFailed), TypeInfoPropertyName = "RegistrationErrorFailed")]

[JsonSerializable(typeof(AppInternal.UserNotificationExceptionRulesNotApplied), TypeInfoPropertyName = "UserNotificationExceptionRulesNotApplied")]
[JsonSerializable(typeof(AppInternal.UserNotificationUpdateAvailable), TypeInfoPropertyName = "UserNotificationUpdateAvailable")]

[JsonSerializable(typeof(AppInternal.ConfigFileErrorInvalidSettings), TypeInfoPropertyName = "ConfigFileErrorInvalidSettings")]
[JsonSerializable(typeof(AppInternal.ConfigFileErrorUnreadableFile), TypeInfoPropertyName = "ConfigFileErrorUnreadableFile")]
[JsonSerializable(typeof(AppInternal.ConfigFileErrorNotRegularFile), TypeInfoPropertyName = "ConfigFileErrorNotRegularFile")]
[JsonSerializable(typeof(AppInternal.ConfigFileErrorDanglingSymlink), TypeInfoPropertyName = "ConfigFileErrorDanglingSymlink")]
[JsonSerializable(typeof(AppInternal.ConfigFileErrorKeptChanging), TypeInfoPropertyName = "ConfigFileErrorKeptChanging")]
[JsonSerializable(typeof(AppInternal.ConfigFileErrorIo), TypeInfoPropertyName = "ConfigFileErrorIo")]

[JsonSerializable(typeof(AppInternal.ConfigProblemKindFileTooLarge), TypeInfoPropertyName = "ConfigProblemKindFileTooLarge")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindInvalidUtf8), TypeInfoPropertyName = "ConfigProblemKindInvalidUtf8")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindDisallowedCharacter), TypeInfoPropertyName = "ConfigProblemKindDisallowedCharacter")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindSyntax), TypeInfoPropertyName = "ConfigProblemKindSyntax")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindMultipleDocuments), TypeInfoPropertyName = "ConfigProblemKindMultipleDocuments")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindNestingTooDeep), TypeInfoPropertyName = "ConfigProblemKindNestingTooDeep")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindDuplicateKey), TypeInfoPropertyName = "ConfigProblemKindDuplicateKey")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindRootNotMapping), TypeInfoPropertyName = "ConfigProblemKindRootNotMapping")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindAnchorOrAlias), TypeInfoPropertyName = "ConfigProblemKindAnchorOrAlias")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindMergeKey), TypeInfoPropertyName = "ConfigProblemKindMergeKey")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindExplicitTag), TypeInfoPropertyName = "ConfigProblemKindExplicitTag")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindComplexKey), TypeInfoPropertyName = "ConfigProblemKindComplexKey")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindTypeMismatch), TypeInfoPropertyName = "ConfigProblemKindTypeMismatch")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindInvalidInteger), TypeInfoPropertyName = "ConfigProblemKindInvalidInteger")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindInvalidBoolean), TypeInfoPropertyName = "ConfigProblemKindInvalidBoolean")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindQuotedNumber), TypeInfoPropertyName = "ConfigProblemKindQuotedNumber")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindQuotedBoolean), TypeInfoPropertyName = "ConfigProblemKindQuotedBoolean")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindInvalidChoice), TypeInfoPropertyName = "ConfigProblemKindInvalidChoice")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindOutOfRange), TypeInfoPropertyName = "ConfigProblemKindOutOfRange")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindMarginNotLessThanSession), TypeInfoPropertyName = "ConfigProblemKindMarginNotLessThanSession")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindInvalidTime), TypeInfoPropertyName = "ConfigProblemKindInvalidTime")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindScheduleEndNotAfterStart), TypeInfoPropertyName = "ConfigProblemKindScheduleEndNotAfterStart")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindInvalidCharacter), TypeInfoPropertyName = "ConfigProblemKindInvalidCharacter")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindNewerSchemaVersion), TypeInfoPropertyName = "ConfigProblemKindNewerSchemaVersion")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindInvalidRouteEntry), TypeInfoPropertyName = "ConfigProblemKindInvalidRouteEntry")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindTooManyRouteEntries), TypeInfoPropertyName = "ConfigProblemKindTooManyRouteEntries")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindDuplicateRouteEntry), TypeInfoPropertyName = "ConfigProblemKindDuplicateRouteEntry")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindOverlappingRouteEntry), TypeInfoPropertyName = "ConfigProblemKindOverlappingRouteEntry")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKindVpnServerAsRouteTarget), TypeInfoPropertyName = "ConfigProblemKindVpnServerAsRouteTarget")]

[JsonSerializable(typeof(AppInternal.ConfigWarningKindUnknownKey), TypeInfoPropertyName = "ConfigWarningKindUnknownKey")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKindInvalidTrustedCert), TypeInfoPropertyName = "ConfigWarningKindInvalidTrustedCert")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKindDuplicateTrustedCert), TypeInfoPropertyName = "ConfigWarningKindDuplicateTrustedCert")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKindTooManyTrustedCerts), TypeInfoPropertyName = "ConfigWarningKindTooManyTrustedCerts")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKindDuplicateRouteEntry), TypeInfoPropertyName = "ConfigWarningKindDuplicateRouteEntry")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKindMultipleHardLinks), TypeInfoPropertyName = "ConfigWarningKindMultipleHardLinks")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKindInsecurePermissions), TypeInfoPropertyName = "ConfigWarningKindInsecurePermissions")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKindPermissionsCorrected), TypeInfoPropertyName = "ConfigWarningKindPermissionsCorrected")]

[JsonSerializable(typeof(AppInternal.AddressProblemMalformed), TypeInfoPropertyName = "AddressProblemMalformed")]
[JsonSerializable(typeof(AppInternal.AddressProblemPrefixOutOfRange), TypeInfoPropertyName = "AddressProblemPrefixOutOfRange")]
[JsonSerializable(typeof(AppInternal.AddressProblemNonContiguousMask), TypeInfoPropertyName = "AddressProblemNonContiguousMask")]
[JsonSerializable(typeof(AppInternal.AddressProblemUnroutable), TypeInfoPropertyName = "AddressProblemUnroutable")]
[JsonSerializable(typeof(AppInternal.AddressProblemHostBitsSet), TypeInfoPropertyName = "AddressProblemHostBitsSet")]

[JsonSerializable(typeof(AppInternal.FileProblemNotFound), TypeInfoPropertyName = "FileProblemNotFound")]
[JsonSerializable(typeof(AppInternal.FileProblemNotRegularFile), TypeInfoPropertyName = "FileProblemNotRegularFile")]
[JsonSerializable(typeof(AppInternal.FileProblemTooLarge), TypeInfoPropertyName = "FileProblemTooLarge")]
[JsonSerializable(typeof(AppInternal.FileProblemIo), TypeInfoPropertyName = "FileProblemIo")]

[JsonSerializable(typeof(AppInternal.AccountValueProblemInvalidHost), TypeInfoPropertyName = "AccountValueProblemInvalidHost")]
[JsonSerializable(typeof(AppInternal.AccountValueProblemTooLong), TypeInfoPropertyName = "AccountValueProblemTooLong")]
[JsonSerializable(typeof(AppInternal.AccountValueProblemLeadingOrTrailingSpace), TypeInfoPropertyName = "AccountValueProblemLeadingOrTrailingSpace")]
[JsonSerializable(typeof(AppInternal.AccountValueProblemDisallowedCharacters), TypeInfoPropertyName = "AccountValueProblemDisallowedCharacters")]

[JsonSerializable(typeof(AppInternal.SystemEventWillSleep), TypeInfoPropertyName = "SystemEventWillSleep")]
[JsonSerializable(typeof(AppInternal.SystemEventDidWake), TypeInfoPropertyName = "SystemEventDidWake")]
[JsonSerializable(typeof(AppInternal.SystemEventScreenLocked), TypeInfoPropertyName = "SystemEventScreenLocked")]
[JsonSerializable(typeof(AppInternal.SystemEventScreenUnlocked), TypeInfoPropertyName = "SystemEventScreenUnlocked")]
[JsonSerializable(typeof(AppInternal.SystemEventDisplaysSlept), TypeInfoPropertyName = "SystemEventDisplaysSlept")]
[JsonSerializable(typeof(AppInternal.SystemEventDisplaysWoke), TypeInfoPropertyName = "SystemEventDisplaysWoke")]
[JsonSerializable(typeof(AppInternal.SystemEventNetworkPathChanged), TypeInfoPropertyName = "SystemEventNetworkPathChanged")]
[JsonSerializable(typeof(AppInternal.SystemEventClockChanged), TypeInfoPropertyName = "SystemEventClockChanged")]
[JsonSerializable(typeof(AppInternal.SystemEventTimeZoneChanged), TypeInfoPropertyName = "SystemEventTimeZoneChanged")]
[JsonSerializable(typeof(AppInternal.SystemEventAppActivated), TypeInfoPropertyName = "SystemEventAppActivated")]
[JsonSerializable(typeof(AppInternal.SystemEventQuitRequested), TypeInfoPropertyName = "SystemEventQuitRequested")]
[JsonSerializable(typeof(AppInternal.SystemEventHelperEvent), TypeInfoPropertyName = "SystemEventHelperEvent")]
[JsonSerializable(typeof(AppInternal.SystemEventHelperOutput), TypeInfoPropertyName = "SystemEventHelperOutput")]
[JsonSerializable(typeof(AppInternal.SystemEventHelperConnectionLost), TypeInfoPropertyName = "SystemEventHelperConnectionLost")]

[JsonSerializable(typeof(HelperPipe.RequestBuild), TypeInfoPropertyName = "RequestBuild")]
[JsonSerializable(typeof(HelperPipe.RequestConnect), TypeInfoPropertyName = "RequestConnect")]
[JsonSerializable(typeof(HelperPipe.RequestSubmitOtp), TypeInfoPropertyName = "RequestSubmitOtp")]
[JsonSerializable(typeof(HelperPipe.RequestDisconnect), TypeInfoPropertyName = "RequestDisconnect")]
[JsonSerializable(typeof(HelperPipe.RequestStatus), TypeInfoPropertyName = "RequestStatus")]
[JsonSerializable(typeof(HelperPipe.RequestExitWhenIdle), TypeInfoPropertyName = "RequestExitWhenIdle")]
[JsonSerializable(typeof(HelperPipe.RequestReadBypassRoutes), TypeInfoPropertyName = "RequestReadBypassRoutes")]
[JsonSerializable(typeof(HelperPipe.RequestWriteBypassRoutes), TypeInfoPropertyName = "RequestWriteBypassRoutes")]
[JsonSerializable(typeof(HelperPipe.RequestPrepareUpdate), TypeInfoPropertyName = "RequestPrepareUpdate")]
[JsonSerializable(typeof(HelperPipe.RequestInstallUpdate), TypeInfoPropertyName = "RequestInstallUpdate")]
[JsonSerializable(typeof(HelperPipe.RequestReadPreparedUpdate), TypeInfoPropertyName = "RequestReadPreparedUpdate")]
[JsonSerializable(typeof(HelperPipe.RequestDiscardPreparedUpdate), TypeInfoPropertyName = "RequestDiscardPreparedUpdate")]

[JsonSerializable(typeof(HelperPipe.MessageBuild), TypeInfoPropertyName = "MessageBuild")]
[JsonSerializable(typeof(HelperPipe.MessageReply), TypeInfoPropertyName = "MessageReply")]
[JsonSerializable(typeof(HelperPipe.MessageReplyFlag), TypeInfoPropertyName = "MessageReplyFlag")]
[JsonSerializable(typeof(HelperPipe.MessageEvent), TypeInfoPropertyName = "MessageEvent")]
[JsonSerializable(typeof(HelperPipe.MessageOutput), TypeInfoPropertyName = "MessageOutput")]
public sealed partial class MessageCoding : JsonSerializerContext;

internal sealed class MillisecondsConverter : JsonConverter<DateTimeOffset>
{
    public override DateTimeOffset Read(ref Utf8JsonReader reader, Type type, JsonSerializerOptions options) =>
        DateTimeOffset.FromUnixTimeMilliseconds(reader.GetInt64());

    public override void Write(Utf8JsonWriter writer, DateTimeOffset value, JsonSerializerOptions options) =>
        writer.WriteNumberValue(value.ToUnixTimeMilliseconds());
}

/// <summary>An enum value as its name in camel case; a number is an error.</summary>
internal sealed class EnumNameConverter<T>() : JsonStringEnumConverter<T>(JsonNamingPolicy.CamelCase, allowIntegerValues: false)
    where T : struct, Enum;

/// <summary>
/// Codes an abstract record whose sealed records are its cases, as a Swift enum with associated
/// values: <c>{"caseName": {…its values…}}</c>, where each case and its name come from the
/// record's <see cref="JsonCaseAttribute"/>s.
/// </summary>
internal sealed class CasesConverter<T> : JsonConverter<T> where T : class
{
    private readonly Dictionary<string, Type> _byName;
    private readonly Dictionary<Type, string> _byType;

    public CasesConverter()
    {
        _byName = typeof(T).GetCustomAttributes<JsonCaseAttribute>().ToDictionary(@case => @case.Name, @case => @case.Type);
        _byType = _byName.ToDictionary(entry => entry.Value, entry => entry.Key);
    }

    public override T Read(ref Utf8JsonReader reader, Type type, JsonSerializerOptions options)
    {
        Expect(reader.TokenType == JsonTokenType.StartObject, "an object");
        _ = reader.Read();
        Expect(reader.TokenType == JsonTokenType.PropertyName, "a case name");
        string name = reader.GetString()!;
        if (!_byName.TryGetValue(name, out Type? caseType))
        {
            throw new JsonException($"{typeof(T).Name} has no case {name}");
        }
        _ = reader.Read();
        // A case's values are an object, `{}` without any; null is no value of any case.
        Expect(reader.TokenType == JsonTokenType.StartObject, $"the values of {name}");
        var value = (T)JsonSerializer.Deserialize(ref reader, options.GetTypeInfo(caseType))!;
        _ = reader.Read();
        Expect(reader.TokenType == JsonTokenType.EndObject, "one case only");
        return value;
    }

    public override void Write(Utf8JsonWriter writer, T value, JsonSerializerOptions options)
    {
        writer.WriteStartObject();
        writer.WritePropertyName(_byType[value.GetType()]);
        JsonSerializer.Serialize(writer, value, options.GetTypeInfo(value.GetType()));
        writer.WriteEndObject();
    }

    private static void Expect(bool condition, string what)
    {
        if (!condition)
        {
            throw new JsonException($"{typeof(T).Name}: expected {what}");
        }
    }
}

[AttributeUsage(AttributeTargets.Class, AllowMultiple = true)]
internal sealed class JsonCaseAttribute(Type type, string name) : Attribute
{
    public Type Type { get; } = type;

    public string Name { get; } = name;
}
