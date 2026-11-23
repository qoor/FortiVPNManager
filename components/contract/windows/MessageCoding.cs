using System.Diagnostics.CodeAnalysis;
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
[JsonSerializable(typeof(AppInternal.ToRuntime.Launched), TypeInfoPropertyName = "ToRuntimeLaunched")]
[JsonSerializable(typeof(AppInternal.ToRuntime.Command), TypeInfoPropertyName = "ToRuntimeCommand")]
[JsonSerializable(typeof(AppInternal.ToRuntime.PortResponse), TypeInfoPropertyName = "ToRuntimePortResponse")]
[JsonSerializable(typeof(AppInternal.ToRuntime.Event), TypeInfoPropertyName = "ToRuntimeEvent")]

[JsonSerializable(typeof(AppInternal.FromRuntime.Section), TypeInfoPropertyName = "FromRuntimeSection")]
[JsonSerializable(typeof(AppInternal.FromRuntime.LogLines), TypeInfoPropertyName = "FromRuntimeLogLines")]
[JsonSerializable(typeof(AppInternal.FromRuntime.PortRequest), TypeInfoPropertyName = "FromRuntimePortRequest")]
[JsonSerializable(typeof(AppInternal.FromRuntime.PortAction), TypeInfoPropertyName = "FromRuntimePortAction")]
[JsonSerializable(typeof(AppInternal.FromRuntime.OpenSettings), TypeInfoPropertyName = "FromRuntimeOpenSettings")]

[JsonSerializable(typeof(AppInternal.Command.Connect), TypeInfoPropertyName = "CommandConnect")]
[JsonSerializable(typeof(AppInternal.Command.Disconnect), TypeInfoPropertyName = "CommandDisconnect")]
[JsonSerializable(typeof(AppInternal.Command.ReconnectNow), TypeInfoPropertyName = "CommandReconnectNow")]
[JsonSerializable(typeof(AppInternal.Command.SettingsOpened), TypeInfoPropertyName = "CommandSettingsOpened")]
[JsonSerializable(typeof(AppInternal.Command.SettingsClosed), TypeInfoPropertyName = "CommandSettingsClosed")]
[JsonSerializable(typeof(AppInternal.Command.EditAccount), TypeInfoPropertyName = "CommandEditAccount")]
[JsonSerializable(typeof(AppInternal.Command.SaveAccount), TypeInfoPropertyName = "CommandSaveAccount")]
[JsonSerializable(typeof(AppInternal.Command.ImportClientSettings), TypeInfoPropertyName = "CommandImportClientSettings")]
[JsonSerializable(typeof(AppInternal.Command.DismissImportReport), TypeInfoPropertyName = "CommandDismissImportReport")]
[JsonSerializable(typeof(AppInternal.Command.SetOtpMethod), TypeInfoPropertyName = "CommandSetOtpMethod")]
[JsonSerializable(typeof(AppInternal.Command.TestGmail), TypeInfoPropertyName = "CommandTestGmail")]
[JsonSerializable(typeof(AppInternal.Command.ConnectGoogleAccount), TypeInfoPropertyName = "CommandConnectGoogleAccount")]
[JsonSerializable(typeof(AppInternal.Command.CancelGoogleSignIn), TypeInfoPropertyName = "CommandCancelGoogleSignIn")]
[JsonSerializable(typeof(AppInternal.Command.DisconnectGoogleAccount), TypeInfoPropertyName = "CommandDisconnectGoogleAccount")]
[JsonSerializable(typeof(AppInternal.Command.ImportOAuthClient), TypeInfoPropertyName = "CommandImportOAuthClient")]
[JsonSerializable(typeof(AppInternal.Command.UseBuiltInOAuthClient), TypeInfoPropertyName = "CommandUseBuiltInOAuthClient")]
[JsonSerializable(typeof(AppInternal.Command.SetConnectionOption), TypeInfoPropertyName = "CommandSetConnectionOption")]
[JsonSerializable(typeof(AppInternal.Command.SetScheduleHours), TypeInfoPropertyName = "CommandSetScheduleHours")]
[JsonSerializable(typeof(AppInternal.Command.SetOpenAtLogin), TypeInfoPropertyName = "CommandSetOpenAtLogin")]
[JsonSerializable(typeof(AppInternal.Command.RegisterHelper), TypeInfoPropertyName = "CommandRegisterHelper")]
[JsonSerializable(typeof(AppInternal.Command.OpenHelperApprovalSettings), TypeInfoPropertyName = "CommandOpenHelperApprovalSettings")]
[JsonSerializable(typeof(AppInternal.Command.SetBypassRows), TypeInfoPropertyName = "CommandSetBypassRows")]
[JsonSerializable(typeof(AppInternal.Command.AddCurrentNetwork), TypeInfoPropertyName = "CommandAddCurrentNetwork")]
[JsonSerializable(typeof(AppInternal.Command.CheckForUpdates), TypeInfoPropertyName = "CommandCheckForUpdates")]
[JsonSerializable(typeof(AppInternal.Command.UpdatePageOpened), TypeInfoPropertyName = "CommandUpdatePageOpened")]
[JsonSerializable(typeof(AppInternal.Command.UpdateNow), TypeInfoPropertyName = "CommandUpdateNow")]
[JsonSerializable(typeof(AppInternal.Command.UpdateNowFromNotification), TypeInfoPropertyName = "CommandUpdateNowFromNotification")]
[JsonSerializable(typeof(AppInternal.Command.CancelUpdateDownload), TypeInfoPropertyName = "CommandCancelUpdateDownload")]
[JsonSerializable(typeof(AppInternal.Command.RestartToUpdate), TypeInfoPropertyName = "CommandRestartToUpdate")]

[JsonSerializable(typeof(AppInternal.ConnectionOption.ScheduleEnabled), TypeInfoPropertyName = "ConnectionOptionScheduleEnabled")]
[JsonSerializable(typeof(AppInternal.ConnectionOption.ScheduleDay), TypeInfoPropertyName = "ConnectionOptionScheduleDay")]
[JsonSerializable(typeof(AppInternal.ConnectionOption.AwayMinutes), TypeInfoPropertyName = "ConnectionOptionAwayMinutes")]
[JsonSerializable(typeof(AppInternal.ConnectionOption.AutoConnectOnLaunch), TypeInfoPropertyName = "ConnectionOptionAutoConnectOnLaunch")]
[JsonSerializable(typeof(AppInternal.ConnectionOption.SessionMaxMinutes), TypeInfoPropertyName = "ConnectionOptionSessionMaxMinutes")]
[JsonSerializable(typeof(AppInternal.ConnectionOption.ProactiveReconnect), TypeInfoPropertyName = "ConnectionOptionProactiveReconnect")]
[JsonSerializable(typeof(AppInternal.ConnectionOption.ProactiveMarginMinutes), TypeInfoPropertyName = "ConnectionOptionProactiveMarginMinutes")]
[JsonSerializable(typeof(AppInternal.ConnectionOption.ReconnectAfterSleep), TypeInfoPropertyName = "ConnectionOptionReconnectAfterSleep")]
[JsonSerializable(typeof(AppInternal.ConnectionOption.MinSleepMinutes), TypeInfoPropertyName = "ConnectionOptionMinSleepMinutes")]

[JsonSerializable(typeof(AppInternal.Section.Connection), TypeInfoPropertyName = "SectionConnection")]
[JsonSerializable(typeof(AppInternal.Section.Schedule), TypeInfoPropertyName = "SectionSchedule")]
[JsonSerializable(typeof(AppInternal.Section.Readiness), TypeInfoPropertyName = "SectionReadiness")]
[JsonSerializable(typeof(AppInternal.Section.Gmail), TypeInfoPropertyName = "SectionGmail")]
[JsonSerializable(typeof(AppInternal.Section.BypassRules), TypeInfoPropertyName = "SectionBypassRules")]
[JsonSerializable(typeof(AppInternal.Section.Config), TypeInfoPropertyName = "SectionConfig")]
[JsonSerializable(typeof(AppInternal.Section.AutoUpdate), TypeInfoPropertyName = "SectionAutoUpdate")]

[JsonSerializable(typeof(AppInternal.ConnectionStatus.NotConnected), TypeInfoPropertyName = "ConnectionStatusNotConnected")]
[JsonSerializable(typeof(AppInternal.ConnectionStatus.SetupRequired), TypeInfoPropertyName = "ConnectionStatusSetupRequired")]
[JsonSerializable(typeof(AppInternal.ConnectionStatus.Connecting), TypeInfoPropertyName = "ConnectionStatusConnecting")]
[JsonSerializable(typeof(AppInternal.ConnectionStatus.Connected), TypeInfoPropertyName = "ConnectionStatusConnected")]
[JsonSerializable(typeof(AppInternal.ConnectionStatus.ReconnectOnHold), TypeInfoPropertyName = "ConnectionStatusReconnectOnHold")]
[JsonSerializable(typeof(AppInternal.ConnectionStatus.WaitingToReconnect), TypeInfoPropertyName = "ConnectionStatusWaitingToReconnect")]
[JsonSerializable(typeof(AppInternal.ConnectionStatus.WaitingForNetwork), TypeInfoPropertyName = "ConnectionStatusWaitingForNetwork")]
[JsonSerializable(typeof(AppInternal.ConnectionStatus.Disconnecting), TypeInfoPropertyName = "ConnectionStatusDisconnecting")]
[JsonSerializable(typeof(AppInternal.ConnectionStatus.Failed), TypeInfoPropertyName = "ConnectionStatusFailed")]

[JsonSerializable(typeof(AppInternal.ProactiveReconnect.At), TypeInfoPropertyName = "ProactiveReconnectAt")]
[JsonSerializable(typeof(AppInternal.ProactiveReconnect.PausedBySchedule), TypeInfoPropertyName = "ProactiveReconnectPausedBySchedule")]
[JsonSerializable(typeof(AppInternal.ProactiveReconnect.Off), TypeInfoPropertyName = "ProactiveReconnectOff")]

[JsonSerializable(typeof(AppInternal.FailureReason.VpnLoginRejected), TypeInfoPropertyName = "FailureReasonVpnLoginRejected")]
[JsonSerializable(typeof(AppInternal.FailureReason.OtpNotReceived), TypeInfoPropertyName = "FailureReasonOtpNotReceived")]
[JsonSerializable(typeof(AppInternal.FailureReason.OtpRejected), TypeInfoPropertyName = "FailureReasonOtpRejected")]
[JsonSerializable(typeof(AppInternal.FailureReason.TunnelSetupFailed), TypeInfoPropertyName = "FailureReasonTunnelSetupFailed")]
[JsonSerializable(typeof(AppInternal.FailureReason.CertificateChangedAfterLogin), TypeInfoPropertyName = "FailureReasonCertificateChangedAfterLogin")]
[JsonSerializable(typeof(AppInternal.FailureReason.GmailRejected), TypeInfoPropertyName = "FailureReasonGmailRejected")]
[JsonSerializable(typeof(AppInternal.FailureReason.TooManyReconnects), TypeInfoPropertyName = "FailureReasonTooManyReconnects")]
[JsonSerializable(typeof(AppInternal.FailureReason.Network), TypeInfoPropertyName = "FailureReasonNetwork")]
[JsonSerializable(typeof(AppInternal.FailureReason.Helper), TypeInfoPropertyName = "FailureReasonHelper")]

[JsonSerializable(typeof(AppInternal.WaitReason.Network), TypeInfoPropertyName = "WaitReasonNetwork")]
[JsonSerializable(typeof(AppInternal.WaitReason.NetworkSetupFailed), TypeInfoPropertyName = "WaitReasonNetworkSetupFailed")]
[JsonSerializable(typeof(AppInternal.WaitReason.ServerEndedSession), TypeInfoPropertyName = "WaitReasonServerEndedSession")]
[JsonSerializable(typeof(AppInternal.WaitReason.ForcedTermination), TypeInfoPropertyName = "WaitReasonForcedTermination")]
[JsonSerializable(typeof(AppInternal.WaitReason.AuthFailure), TypeInfoPropertyName = "WaitReasonAuthFailure")]
[JsonSerializable(typeof(AppInternal.WaitReason.AuthCooldown), TypeInfoPropertyName = "WaitReasonAuthCooldown")]
[JsonSerializable(typeof(AppInternal.WaitReason.Sleep), TypeInfoPropertyName = "WaitReasonSleep")]
[JsonSerializable(typeof(AppInternal.WaitReason.UserAway), TypeInfoPropertyName = "WaitReasonUserAway")]

[JsonSerializable(typeof(AppInternal.SetupItem.ConfigFile), TypeInfoPropertyName = "SetupItemConfigFile")]
[JsonSerializable(typeof(AppInternal.SetupItem.AccountDetails), TypeInfoPropertyName = "SetupItemAccountDetails")]
[JsonSerializable(typeof(AppInternal.SetupItem.VpnPassword), TypeInfoPropertyName = "SetupItemVpnPassword")]
[JsonSerializable(typeof(AppInternal.SetupItem.GmailCredential), TypeInfoPropertyName = "SetupItemGmailCredential")]
[JsonSerializable(typeof(AppInternal.SetupItem.HelperApproval), TypeInfoPropertyName = "SetupItemHelperApproval")]
[JsonSerializable(typeof(AppInternal.SetupItem.HelperVersion), TypeInfoPropertyName = "SetupItemHelperVersion")]

[JsonSerializable(typeof(AppInternal.ScheduleOutlook.Connects), TypeInfoPropertyName = "ScheduleOutlookConnects")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlook.NoDays), TypeInfoPropertyName = "ScheduleOutlookNoDays")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlook.WithinHours), TypeInfoPropertyName = "ScheduleOutlookWithinHours")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlook.OutsideHoursPresent), TypeInfoPropertyName = "ScheduleOutlookOutsideHoursPresent")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlook.HoldingWhileAway), TypeInfoPropertyName = "ScheduleOutlookHoldingWhileAway")]
[JsonSerializable(typeof(AppInternal.ScheduleOutlook.Suspended), TypeInfoPropertyName = "ScheduleOutlookSuspended")]

[JsonSerializable(typeof(AppInternal.ScheduleHoursError.EndNotAfterStart), TypeInfoPropertyName = "ScheduleHoursErrorEndNotAfterStart")]
[JsonSerializable(typeof(AppInternal.ScheduleHoursError.NotSaved), TypeInfoPropertyName = "ScheduleHoursErrorNotSaved")]

[JsonSerializable(typeof(AppInternal.HelperState.Checking), TypeInfoPropertyName = "HelperStateChecking")]
[JsonSerializable(typeof(AppInternal.HelperState.NotRegistered), TypeInfoPropertyName = "HelperStateNotRegistered")]
[JsonSerializable(typeof(AppInternal.HelperState.RequiresApproval), TypeInfoPropertyName = "HelperStateRequiresApproval")]
[JsonSerializable(typeof(AppInternal.HelperState.Running), TypeInfoPropertyName = "HelperStateRunning")]
[JsonSerializable(typeof(AppInternal.HelperState.VersionMismatch), TypeInfoPropertyName = "HelperStateVersionMismatch")]
[JsonSerializable(typeof(AppInternal.HelperState.Unreachable), TypeInfoPropertyName = "HelperStateUnreachable")]

[JsonSerializable(typeof(AppInternal.GoogleAccountStatus.NotConnected), TypeInfoPropertyName = "GoogleAccountStatusNotConnected")]
[JsonSerializable(typeof(AppInternal.GoogleAccountStatus.SigningIn), TypeInfoPropertyName = "GoogleAccountStatusSigningIn")]
[JsonSerializable(typeof(AppInternal.GoogleAccountStatus.Disconnecting), TypeInfoPropertyName = "GoogleAccountStatusDisconnecting")]
[JsonSerializable(typeof(AppInternal.GoogleAccountStatus.Connected), TypeInfoPropertyName = "GoogleAccountStatusConnected")]

[JsonSerializable(typeof(AppInternal.OAuthClientSource.BuiltIn), TypeInfoPropertyName = "OAuthClientSourceBuiltIn")]
[JsonSerializable(typeof(AppInternal.OAuthClientSource.Imported), TypeInfoPropertyName = "OAuthClientSourceImported")]
[JsonSerializable(typeof(AppInternal.OAuthClientSource.NotConfigured), TypeInfoPropertyName = "OAuthClientSourceNotConfigured")]

[JsonSerializable(typeof(AppInternal.GoogleError.ClientFileUnreadable), TypeInfoPropertyName = "GoogleErrorClientFileUnreadable")]
[JsonSerializable(typeof(AppInternal.GoogleError.ClientFileInvalid), TypeInfoPropertyName = "GoogleErrorClientFileInvalid")]
[JsonSerializable(typeof(AppInternal.GoogleError.ListenerFailed), TypeInfoPropertyName = "GoogleErrorListenerFailed")]
[JsonSerializable(typeof(AppInternal.GoogleError.BrowserUnavailable), TypeInfoPropertyName = "GoogleErrorBrowserUnavailable")]
[JsonSerializable(typeof(AppInternal.GoogleError.TimedOut), TypeInfoPropertyName = "GoogleErrorTimedOut")]
[JsonSerializable(typeof(AppInternal.GoogleError.CancelledInBrowser), TypeInfoPropertyName = "GoogleErrorCancelledInBrowser")]
[JsonSerializable(typeof(AppInternal.GoogleError.Denied), TypeInfoPropertyName = "GoogleErrorDenied")]
[JsonSerializable(typeof(AppInternal.GoogleError.TokenRejected), TypeInfoPropertyName = "GoogleErrorTokenRejected")]
[JsonSerializable(typeof(AppInternal.GoogleError.ClientRejected), TypeInfoPropertyName = "GoogleErrorClientRejected")]
[JsonSerializable(typeof(AppInternal.GoogleError.NoRefreshToken), TypeInfoPropertyName = "GoogleErrorNoRefreshToken")]
[JsonSerializable(typeof(AppInternal.GoogleError.ScopeNotGranted), TypeInfoPropertyName = "GoogleErrorScopeNotGranted")]
[JsonSerializable(typeof(AppInternal.GoogleError.WrongAccount), TypeInfoPropertyName = "GoogleErrorWrongAccount")]
[JsonSerializable(typeof(AppInternal.GoogleError.GmailApiRefused), TypeInfoPropertyName = "GoogleErrorGmailApiRefused")]
[JsonSerializable(typeof(AppInternal.GoogleError.Network), TypeInfoPropertyName = "GoogleErrorNetwork")]
[JsonSerializable(typeof(AppInternal.GoogleError.SecretStore), TypeInfoPropertyName = "GoogleErrorSecretStore")]
[JsonSerializable(typeof(AppInternal.GoogleError.Config), TypeInfoPropertyName = "GoogleErrorConfig")]

[JsonSerializable(typeof(AppInternal.OAuthClientFileProblem.NotJson), TypeInfoPropertyName = "OAuthClientFileProblemNotJson")]
[JsonSerializable(typeof(AppInternal.OAuthClientFileProblem.WebClient), TypeInfoPropertyName = "OAuthClientFileProblemWebClient")]
[JsonSerializable(typeof(AppInternal.OAuthClientFileProblem.NoDesktopClient), TypeInfoPropertyName = "OAuthClientFileProblemNoDesktopClient")]
[JsonSerializable(typeof(AppInternal.OAuthClientFileProblem.MissingField), TypeInfoPropertyName = "OAuthClientFileProblemMissingField")]
[JsonSerializable(typeof(AppInternal.OAuthClientFileProblem.InvalidField), TypeInfoPropertyName = "OAuthClientFileProblemInvalidField")]

[JsonSerializable(typeof(AppInternal.GmailTest.Idle), TypeInfoPropertyName = "GmailTestIdle")]
[JsonSerializable(typeof(AppInternal.GmailTest.Running), TypeInfoPropertyName = "GmailTestRunning")]
[JsonSerializable(typeof(AppInternal.GmailTest.ImapSucceeded), TypeInfoPropertyName = "GmailTestImapSucceeded")]
[JsonSerializable(typeof(AppInternal.GmailTest.ApiSucceeded), TypeInfoPropertyName = "GmailTestApiSucceeded")]
[JsonSerializable(typeof(AppInternal.GmailTest.Failed), TypeInfoPropertyName = "GmailTestFailed")]

[JsonSerializable(typeof(AppInternal.GmailTestFailure.EnterAddress), TypeInfoPropertyName = "GmailTestFailureEnterAddress")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.EnterAppPassword), TypeInfoPropertyName = "GmailTestFailureEnterAppPassword")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.AppPasswordUnreadable), TypeInfoPropertyName = "GmailTestFailureAppPasswordUnreadable")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.ImapLoginRejected), TypeInfoPropertyName = "GmailTestFailureImapLoginRejected")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.MailboxesMissing), TypeInfoPropertyName = "GmailTestFailureMailboxesMissing")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.ImapUnreachable), TypeInfoPropertyName = "GmailTestFailureImapUnreachable")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.NoOAuthClient), TypeInfoPropertyName = "GmailTestFailureNoOAuthClient")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.NotSignedIn), TypeInfoPropertyName = "GmailTestFailureNotSignedIn")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.AccessRevoked), TypeInfoPropertyName = "GmailTestFailureAccessRevoked")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.ClientRejected), TypeInfoPropertyName = "GmailTestFailureClientRejected")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.SecretStore), TypeInfoPropertyName = "GmailTestFailureSecretStore")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.OtherAccount), TypeInfoPropertyName = "GmailTestFailureOtherAccount")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.AccessTokenRejected), TypeInfoPropertyName = "GmailTestFailureAccessTokenRejected")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.GmailApiRefused), TypeInfoPropertyName = "GmailTestFailureGmailApiRefused")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.RateLimited), TypeInfoPropertyName = "GmailTestFailureRateLimited")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.GoogleUnreachable), TypeInfoPropertyName = "GmailTestFailureGoogleUnreachable")]
[JsonSerializable(typeof(AppInternal.GmailTestFailure.GoogleReturnedError), TypeInfoPropertyName = "GmailTestFailureGoogleReturnedError")]

[JsonSerializable(typeof(AppInternal.ImapFailure.Network), TypeInfoPropertyName = "ImapFailureNetwork")]
[JsonSerializable(typeof(AppInternal.ImapFailure.TimedOut), TypeInfoPropertyName = "ImapFailureTimedOut")]
[JsonSerializable(typeof(AppInternal.ImapFailure.ServerClosed), TypeInfoPropertyName = "ImapFailureServerClosed")]
[JsonSerializable(typeof(AppInternal.ImapFailure.Protocol), TypeInfoPropertyName = "ImapFailureProtocol")]
[JsonSerializable(typeof(AppInternal.ImapFailure.Rejected), TypeInfoPropertyName = "ImapFailureRejected")]

[JsonSerializable(typeof(AppInternal.ImapStep.Connecting), TypeInfoPropertyName = "ImapStepConnecting")]
[JsonSerializable(typeof(AppInternal.ImapStep.Command), TypeInfoPropertyName = "ImapStepCommand")]

[JsonSerializable(typeof(AppInternal.BypassNotice.RowProblem), TypeInfoPropertyName = "BypassNoticeRowProblem")]
[JsonSerializable(typeof(AppInternal.BypassNotice.ApplyFailed), TypeInfoPropertyName = "BypassNoticeApplyFailed")]

[JsonSerializable(typeof(AppInternal.RowProblem.Invalid), TypeInfoPropertyName = "RowProblemInvalid")]
[JsonSerializable(typeof(AppInternal.RowProblem.Duplicate), TypeInfoPropertyName = "RowProblemDuplicate")]
[JsonSerializable(typeof(AppInternal.RowProblem.Overlap), TypeInfoPropertyName = "RowProblemOverlap")]
[JsonSerializable(typeof(AppInternal.RowProblem.VpnServer), TypeInfoPropertyName = "RowProblemVpnServer")]

[JsonSerializable(typeof(AppInternal.BypassStatus.Checking), TypeInfoPropertyName = "BypassStatusChecking")]
[JsonSerializable(typeof(AppInternal.BypassStatus.NoConfigFile), TypeInfoPropertyName = "BypassStatusNoConfigFile")]
[JsonSerializable(typeof(AppInternal.BypassStatus.Applied), TypeInfoPropertyName = "BypassStatusApplied")]
[JsonSerializable(typeof(AppInternal.BypassStatus.VpnOff), TypeInfoPropertyName = "BypassStatusVpnOff")]
[JsonSerializable(typeof(AppInternal.BypassStatus.OutsideNetworks), TypeInfoPropertyName = "BypassStatusOutsideNetworks")]
[JsonSerializable(typeof(AppInternal.BypassStatus.NoNetworksListed), TypeInfoPropertyName = "BypassStatusNoNetworksListed")]
[JsonSerializable(typeof(AppInternal.BypassStatus.ConfigInvalid), TypeInfoPropertyName = "BypassStatusConfigInvalid")]
[JsonSerializable(typeof(AppInternal.BypassStatus.NoNetwork), TypeInfoPropertyName = "BypassStatusNoNetwork")]
[JsonSerializable(typeof(AppInternal.BypassStatus.Failed), TypeInfoPropertyName = "BypassStatusFailed")]

[JsonSerializable(typeof(AppInternal.BypassFailure.RoutesNotKept), TypeInfoPropertyName = "BypassFailureRoutesNotKept")]
[JsonSerializable(typeof(AppInternal.BypassFailure.NetworkChanged), TypeInfoPropertyName = "BypassFailureNetworkChanged")]
[JsonSerializable(typeof(AppInternal.BypassFailure.HelperRefused), TypeInfoPropertyName = "BypassFailureHelperRefused")]
[JsonSerializable(typeof(AppInternal.BypassFailure.SettingsNotChanged), TypeInfoPropertyName = "BypassFailureSettingsNotChanged")]
[JsonSerializable(typeof(AppInternal.BypassFailure.HelperUnreachable), TypeInfoPropertyName = "BypassFailureHelperUnreachable")]
[JsonSerializable(typeof(AppInternal.BypassFailure.HelperBuildMismatch), TypeInfoPropertyName = "BypassFailureHelperBuildMismatch")]

[JsonSerializable(typeof(AppInternal.AccountSaveError.InvalidPort), TypeInfoPropertyName = "AccountSaveErrorInvalidPort")]
[JsonSerializable(typeof(AppInternal.AccountSaveError.PasswordNeedsAccount), TypeInfoPropertyName = "AccountSaveErrorPasswordNeedsAccount")]
[JsonSerializable(typeof(AppInternal.AccountSaveError.AppPasswordNeedsAddress), TypeInfoPropertyName = "AccountSaveErrorAppPasswordNeedsAddress")]
[JsonSerializable(typeof(AppInternal.AccountSaveError.AccountValue), TypeInfoPropertyName = "AccountSaveErrorAccountValue")]
[JsonSerializable(typeof(AppInternal.AccountSaveError.InvalidVpnPassword), TypeInfoPropertyName = "AccountSaveErrorInvalidVpnPassword")]
[JsonSerializable(typeof(AppInternal.AccountSaveError.Config), TypeInfoPropertyName = "AccountSaveErrorConfig")]
[JsonSerializable(typeof(AppInternal.AccountSaveError.SecretStore), TypeInfoPropertyName = "AccountSaveErrorSecretStore")]

[JsonSerializable(typeof(AppInternal.ClientSettingsImport.Imported), TypeInfoPropertyName = "ClientSettingsImportImported")]
[JsonSerializable(typeof(AppInternal.ClientSettingsImport.Failed), TypeInfoPropertyName = "ClientSettingsImportFailed")]

[JsonSerializable(typeof(AppInternal.ImportedPassword.Saved), TypeInfoPropertyName = "ImportedPasswordSaved")]
[JsonSerializable(typeof(AppInternal.ImportedPassword.NotSavedWithoutUsername), TypeInfoPropertyName = "ImportedPasswordNotSavedWithoutUsername")]
[JsonSerializable(typeof(AppInternal.ImportedPassword.NotSaved), TypeInfoPropertyName = "ImportedPasswordNotSaved")]

[JsonSerializable(typeof(AppInternal.ImportError.SourceNotFound), TypeInfoPropertyName = "ImportErrorSourceNotFound")]
[JsonSerializable(typeof(AppInternal.ImportError.Unreadable), TypeInfoPropertyName = "ImportErrorUnreadable")]
[JsonSerializable(typeof(AppInternal.ImportError.MissingHost), TypeInfoPropertyName = "ImportErrorMissingHost")]
[JsonSerializable(typeof(AppInternal.ImportError.Config), TypeInfoPropertyName = "ImportErrorConfig")]
[JsonSerializable(typeof(AppInternal.ImportError.NoFortiClientConnection), TypeInfoPropertyName = "ImportErrorNoFortiClientConnection")]
[JsonSerializable(typeof(AppInternal.ImportError.InvalidFortiClientServer), TypeInfoPropertyName = "ImportErrorInvalidFortiClientServer")]

[JsonSerializable(typeof(AppInternal.UpdateStatus.Checking), TypeInfoPropertyName = "UpdateStatusChecking")]
[JsonSerializable(typeof(AppInternal.UpdateStatus.UpToDate), TypeInfoPropertyName = "UpdateStatusUpToDate")]
[JsonSerializable(typeof(AppInternal.UpdateStatus.Available), TypeInfoPropertyName = "UpdateStatusAvailable")]
[JsonSerializable(typeof(AppInternal.UpdateStatus.CheckFailed), TypeInfoPropertyName = "UpdateStatusCheckFailed")]

[JsonSerializable(typeof(AppInternal.UpdateCheckFailure.HttpStatus), TypeInfoPropertyName = "UpdateCheckFailureHttpStatus")]
[JsonSerializable(typeof(AppInternal.UpdateCheckFailure.Network), TypeInfoPropertyName = "UpdateCheckFailureNetwork")]
[JsonSerializable(typeof(AppInternal.UpdateCheckFailure.InvalidReply), TypeInfoPropertyName = "UpdateCheckFailureInvalidReply")]
[JsonSerializable(typeof(AppInternal.UpdateCheckFailure.InstallerMissing), TypeInfoPropertyName = "UpdateCheckFailureInstallerMissing")]

[JsonSerializable(typeof(AppInternal.UpdateProgress.Idle), TypeInfoPropertyName = "UpdateProgressIdle")]
[JsonSerializable(typeof(AppInternal.UpdateProgress.Downloading), TypeInfoPropertyName = "UpdateProgressDownloading")]
[JsonSerializable(typeof(AppInternal.UpdateProgress.Preparing), TypeInfoPropertyName = "UpdateProgressPreparing")]
[JsonSerializable(typeof(AppInternal.UpdateProgress.ReadyToRestart), TypeInfoPropertyName = "UpdateProgressReadyToRestart")]
[JsonSerializable(typeof(AppInternal.UpdateProgress.Installing), TypeInfoPropertyName = "UpdateProgressInstalling")]
[JsonSerializable(typeof(AppInternal.UpdateProgress.Failed), TypeInfoPropertyName = "UpdateProgressFailed")]

[JsonSerializable(typeof(AppInternal.RemainingTime.Calculating), TypeInfoPropertyName = "RemainingTimeCalculating")]
[JsonSerializable(typeof(AppInternal.RemainingTime.Minutes), TypeInfoPropertyName = "RemainingTimeMinutes")]
[JsonSerializable(typeof(AppInternal.RemainingTime.Seconds), TypeInfoPropertyName = "RemainingTimeSeconds")]

[JsonSerializable(typeof(AppInternal.InstallFailure.DownloadFailed), TypeInfoPropertyName = "InstallFailureDownloadFailed")]
[JsonSerializable(typeof(AppInternal.InstallFailure.WaiterFailed), TypeInfoPropertyName = "InstallFailureWaiterFailed")]
[JsonSerializable(typeof(AppInternal.InstallFailure.HelperUnreachable), TypeInfoPropertyName = "InstallFailureHelperUnreachable")]
[JsonSerializable(typeof(AppInternal.InstallFailure.HelperBuildMismatch), TypeInfoPropertyName = "InstallFailureHelperBuildMismatch")]
[JsonSerializable(typeof(AppInternal.InstallFailure.HelperFailed), TypeInfoPropertyName = "InstallFailureHelperFailed")]
[JsonSerializable(typeof(AppInternal.InstallFailure.HelperRefused), TypeInfoPropertyName = "InstallFailureHelperRefused")]

[JsonSerializable(typeof(AppInternal.PortRequest.HelperRegistration), TypeInfoPropertyName = "PortRequestHelperRegistration")]
[JsonSerializable(typeof(AppInternal.PortRequest.RegisterHelper), TypeInfoPropertyName = "PortRequestRegisterHelper")]
[JsonSerializable(typeof(AppInternal.PortRequest.HelperBuild), TypeInfoPropertyName = "PortRequestHelperBuild")]
[JsonSerializable(typeof(AppInternal.PortRequest.CallHelper), TypeInfoPropertyName = "PortRequestCallHelper")]
[JsonSerializable(typeof(AppInternal.PortRequest.RouteInterface), TypeInfoPropertyName = "PortRequestRouteInterface")]
[JsonSerializable(typeof(AppInternal.PortRequest.PrimaryRouter), TypeInfoPropertyName = "PortRequestPrimaryRouter")]
[JsonSerializable(typeof(AppInternal.PortRequest.ScreenState), TypeInfoPropertyName = "PortRequestScreenState")]
[JsonSerializable(typeof(AppInternal.PortRequest.OpenBrowser), TypeInfoPropertyName = "PortRequestOpenBrowser")]
[JsonSerializable(typeof(AppInternal.PortRequest.LoginItemRegistration), TypeInfoPropertyName = "PortRequestLoginItemRegistration")]
[JsonSerializable(typeof(AppInternal.PortRequest.SetLoginItem), TypeInfoPropertyName = "PortRequestSetLoginItem")]
[JsonSerializable(typeof(AppInternal.PortRequest.FortiClientConnections), TypeInfoPropertyName = "PortRequestFortiClientConnections")]
[JsonSerializable(typeof(AppInternal.PortRequest.StartUpdateWaiter), TypeInfoPropertyName = "PortRequestStartUpdateWaiter")]

[JsonSerializable(typeof(AppInternal.PortResponse.Registration), TypeInfoPropertyName = "PortResponseRegistration")]
[JsonSerializable(typeof(AppInternal.PortResponse.Registered), TypeInfoPropertyName = "PortResponseRegistered")]
[JsonSerializable(typeof(AppInternal.PortResponse.RegistrationFailed), TypeInfoPropertyName = "PortResponseRegistrationFailed")]
[JsonSerializable(typeof(AppInternal.PortResponse.HelperBuild), TypeInfoPropertyName = "PortResponseHelperBuild")]
[JsonSerializable(typeof(AppInternal.PortResponse.HelperReply), TypeInfoPropertyName = "PortResponseHelperReply")]
[JsonSerializable(typeof(AppInternal.PortResponse.HelperReplyFlag), TypeInfoPropertyName = "PortResponseHelperReplyFlag")]
[JsonSerializable(typeof(AppInternal.PortResponse.HelperUnreachable), TypeInfoPropertyName = "PortResponseHelperUnreachable")]
[JsonSerializable(typeof(AppInternal.PortResponse.HelperBuildMismatch), TypeInfoPropertyName = "PortResponseHelperBuildMismatch")]
[JsonSerializable(typeof(AppInternal.PortResponse.RouteInterface), TypeInfoPropertyName = "PortResponseRouteInterface")]
[JsonSerializable(typeof(AppInternal.PortResponse.PrimaryRouter), TypeInfoPropertyName = "PortResponsePrimaryRouter")]
[JsonSerializable(typeof(AppInternal.PortResponse.ScreenState), TypeInfoPropertyName = "PortResponseScreenState")]
[JsonSerializable(typeof(AppInternal.PortResponse.BrowserOpened), TypeInfoPropertyName = "PortResponseBrowserOpened")]
[JsonSerializable(typeof(AppInternal.PortResponse.FortiClientConnections), TypeInfoPropertyName = "PortResponseFortiClientConnections")]
[JsonSerializable(typeof(AppInternal.PortResponse.UpdateWaiterStarted), TypeInfoPropertyName = "PortResponseUpdateWaiterStarted")]
[JsonSerializable(typeof(AppInternal.PortResponse.UpdateWaiterFailed), TypeInfoPropertyName = "PortResponseUpdateWaiterFailed")]

[JsonSerializable(typeof(AppInternal.PortAction.Log), TypeInfoPropertyName = "PortActionLog")]
[JsonSerializable(typeof(AppInternal.PortAction.PostNotification), TypeInfoPropertyName = "PortActionPostNotification")]
[JsonSerializable(typeof(AppInternal.PortAction.HoldActivity), TypeInfoPropertyName = "PortActionHoldActivity")]
[JsonSerializable(typeof(AppInternal.PortAction.OpenHelperApprovalSettings), TypeInfoPropertyName = "PortActionOpenHelperApprovalSettings")]
[JsonSerializable(typeof(AppInternal.PortAction.UseHelperTeam), TypeInfoPropertyName = "PortActionUseHelperTeam")]
[JsonSerializable(typeof(AppInternal.PortAction.ReleaseUpdateWaiter), TypeInfoPropertyName = "PortActionReleaseUpdateWaiter")]
[JsonSerializable(typeof(AppInternal.PortAction.StopUpdateWaiter), TypeInfoPropertyName = "PortActionStopUpdateWaiter")]
[JsonSerializable(typeof(AppInternal.PortAction.ApproveQuit), TypeInfoPropertyName = "PortActionApproveQuit")]


[JsonSerializable(typeof(AppInternal.HelperCall.Connect), TypeInfoPropertyName = "HelperCallConnect")]
[JsonSerializable(typeof(AppInternal.HelperCall.SubmitOtp), TypeInfoPropertyName = "HelperCallSubmitOtp")]
[JsonSerializable(typeof(AppInternal.HelperCall.Disconnect), TypeInfoPropertyName = "HelperCallDisconnect")]
[JsonSerializable(typeof(AppInternal.HelperCall.Status), TypeInfoPropertyName = "HelperCallStatus")]
[JsonSerializable(typeof(AppInternal.HelperCall.ExitWhenIdle), TypeInfoPropertyName = "HelperCallExitWhenIdle")]
[JsonSerializable(typeof(AppInternal.HelperCall.ReadBypassRoutes), TypeInfoPropertyName = "HelperCallReadBypassRoutes")]
[JsonSerializable(typeof(AppInternal.HelperCall.WriteBypassRoutes), TypeInfoPropertyName = "HelperCallWriteBypassRoutes")]
[JsonSerializable(typeof(AppInternal.HelperCall.PrepareUpdate), TypeInfoPropertyName = "HelperCallPrepareUpdate")]
[JsonSerializable(typeof(AppInternal.HelperCall.InstallUpdate), TypeInfoPropertyName = "HelperCallInstallUpdate")]
[JsonSerializable(typeof(AppInternal.HelperCall.ReadPreparedUpdate), TypeInfoPropertyName = "HelperCallReadPreparedUpdate")]
[JsonSerializable(typeof(AppInternal.HelperCall.DiscardPreparedUpdate), TypeInfoPropertyName = "HelperCallDiscardPreparedUpdate")]

[JsonSerializable(typeof(AppInternal.HelperChannelFailure.AppNotSigned), TypeInfoPropertyName = "HelperChannelFailureAppNotSigned")]
[JsonSerializable(typeof(AppInternal.HelperChannelFailure.Connection), TypeInfoPropertyName = "HelperChannelFailureConnection")]
[JsonSerializable(typeof(AppInternal.HelperChannelFailure.ConnectionLost), TypeInfoPropertyName = "HelperChannelFailureConnectionLost")]
[JsonSerializable(typeof(AppInternal.HelperChannelFailure.NoReply), TypeInfoPropertyName = "HelperChannelFailureNoReply")]
[JsonSerializable(typeof(AppInternal.HelperChannelFailure.UnreadableReply), TypeInfoPropertyName = "HelperChannelFailureUnreadableReply")]

[JsonSerializable(typeof(AppInternal.RegistrationError.NotInstalledCopy), TypeInfoPropertyName = "RegistrationErrorNotInstalledCopy")]
[JsonSerializable(typeof(AppInternal.RegistrationError.Failed), TypeInfoPropertyName = "RegistrationErrorFailed")]

[JsonSerializable(typeof(AppInternal.UserNotification.ExceptionRulesNotApplied), TypeInfoPropertyName = "UserNotificationExceptionRulesNotApplied")]
[JsonSerializable(typeof(AppInternal.UserNotification.UpdateAvailable), TypeInfoPropertyName = "UserNotificationUpdateAvailable")]

[JsonSerializable(typeof(AppInternal.ConfigFileError.InvalidSettings), TypeInfoPropertyName = "ConfigFileErrorInvalidSettings")]
[JsonSerializable(typeof(AppInternal.ConfigFileError.UnreadableFile), TypeInfoPropertyName = "ConfigFileErrorUnreadableFile")]
[JsonSerializable(typeof(AppInternal.ConfigFileError.NotRegularFile), TypeInfoPropertyName = "ConfigFileErrorNotRegularFile")]
[JsonSerializable(typeof(AppInternal.ConfigFileError.DanglingSymlink), TypeInfoPropertyName = "ConfigFileErrorDanglingSymlink")]
[JsonSerializable(typeof(AppInternal.ConfigFileError.KeptChanging), TypeInfoPropertyName = "ConfigFileErrorKeptChanging")]
[JsonSerializable(typeof(AppInternal.ConfigFileError.Io), TypeInfoPropertyName = "ConfigFileErrorIo")]

[JsonSerializable(typeof(AppInternal.ConfigProblemKind.FileTooLarge), TypeInfoPropertyName = "ConfigProblemKindFileTooLarge")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.InvalidUtf8), TypeInfoPropertyName = "ConfigProblemKindInvalidUtf8")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.DisallowedCharacter), TypeInfoPropertyName = "ConfigProblemKindDisallowedCharacter")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.Syntax), TypeInfoPropertyName = "ConfigProblemKindSyntax")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.MultipleDocuments), TypeInfoPropertyName = "ConfigProblemKindMultipleDocuments")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.NestingTooDeep), TypeInfoPropertyName = "ConfigProblemKindNestingTooDeep")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.DuplicateKey), TypeInfoPropertyName = "ConfigProblemKindDuplicateKey")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.RootNotMapping), TypeInfoPropertyName = "ConfigProblemKindRootNotMapping")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.AnchorOrAlias), TypeInfoPropertyName = "ConfigProblemKindAnchorOrAlias")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.MergeKey), TypeInfoPropertyName = "ConfigProblemKindMergeKey")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.ExplicitTag), TypeInfoPropertyName = "ConfigProblemKindExplicitTag")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.ComplexKey), TypeInfoPropertyName = "ConfigProblemKindComplexKey")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.TypeMismatch), TypeInfoPropertyName = "ConfigProblemKindTypeMismatch")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.InvalidInteger), TypeInfoPropertyName = "ConfigProblemKindInvalidInteger")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.InvalidBoolean), TypeInfoPropertyName = "ConfigProblemKindInvalidBoolean")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.QuotedNumber), TypeInfoPropertyName = "ConfigProblemKindQuotedNumber")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.QuotedBoolean), TypeInfoPropertyName = "ConfigProblemKindQuotedBoolean")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.InvalidChoice), TypeInfoPropertyName = "ConfigProblemKindInvalidChoice")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.OutOfRange), TypeInfoPropertyName = "ConfigProblemKindOutOfRange")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.MarginNotLessThanSession), TypeInfoPropertyName = "ConfigProblemKindMarginNotLessThanSession")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.InvalidTime), TypeInfoPropertyName = "ConfigProblemKindInvalidTime")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.ScheduleEndNotAfterStart), TypeInfoPropertyName = "ConfigProblemKindScheduleEndNotAfterStart")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.InvalidCharacter), TypeInfoPropertyName = "ConfigProblemKindInvalidCharacter")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.NewerSchemaVersion), TypeInfoPropertyName = "ConfigProblemKindNewerSchemaVersion")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.InvalidRouteEntry), TypeInfoPropertyName = "ConfigProblemKindInvalidRouteEntry")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.TooManyRouteEntries), TypeInfoPropertyName = "ConfigProblemKindTooManyRouteEntries")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.DuplicateRouteEntry), TypeInfoPropertyName = "ConfigProblemKindDuplicateRouteEntry")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.OverlappingRouteEntry), TypeInfoPropertyName = "ConfigProblemKindOverlappingRouteEntry")]
[JsonSerializable(typeof(AppInternal.ConfigProblemKind.VpnServerAsRouteTarget), TypeInfoPropertyName = "ConfigProblemKindVpnServerAsRouteTarget")]

[JsonSerializable(typeof(AppInternal.ConfigWarningKind.UnknownKey), TypeInfoPropertyName = "ConfigWarningKindUnknownKey")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKind.InvalidTrustedCert), TypeInfoPropertyName = "ConfigWarningKindInvalidTrustedCert")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKind.DuplicateTrustedCert), TypeInfoPropertyName = "ConfigWarningKindDuplicateTrustedCert")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKind.TooManyTrustedCerts), TypeInfoPropertyName = "ConfigWarningKindTooManyTrustedCerts")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKind.DuplicateRouteEntry), TypeInfoPropertyName = "ConfigWarningKindDuplicateRouteEntry")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKind.MultipleHardLinks), TypeInfoPropertyName = "ConfigWarningKindMultipleHardLinks")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKind.InsecurePermissions), TypeInfoPropertyName = "ConfigWarningKindInsecurePermissions")]
[JsonSerializable(typeof(AppInternal.ConfigWarningKind.PermissionsCorrected), TypeInfoPropertyName = "ConfigWarningKindPermissionsCorrected")]

[JsonSerializable(typeof(AppInternal.AddressProblem.Malformed), TypeInfoPropertyName = "AddressProblemMalformed")]
[JsonSerializable(typeof(AppInternal.AddressProblem.PrefixOutOfRange), TypeInfoPropertyName = "AddressProblemPrefixOutOfRange")]
[JsonSerializable(typeof(AppInternal.AddressProblem.NonContiguousMask), TypeInfoPropertyName = "AddressProblemNonContiguousMask")]
[JsonSerializable(typeof(AppInternal.AddressProblem.Unroutable), TypeInfoPropertyName = "AddressProblemUnroutable")]
[JsonSerializable(typeof(AppInternal.AddressProblem.HostBitsSet), TypeInfoPropertyName = "AddressProblemHostBitsSet")]

[JsonSerializable(typeof(AppInternal.FileProblem.NotFound), TypeInfoPropertyName = "FileProblemNotFound")]
[JsonSerializable(typeof(AppInternal.FileProblem.NotRegularFile), TypeInfoPropertyName = "FileProblemNotRegularFile")]
[JsonSerializable(typeof(AppInternal.FileProblem.TooLarge), TypeInfoPropertyName = "FileProblemTooLarge")]
[JsonSerializable(typeof(AppInternal.FileProblem.Io), TypeInfoPropertyName = "FileProblemIo")]

[JsonSerializable(typeof(AppInternal.AccountValueProblem.InvalidHost), TypeInfoPropertyName = "AccountValueProblemInvalidHost")]
[JsonSerializable(typeof(AppInternal.AccountValueProblem.TooLong), TypeInfoPropertyName = "AccountValueProblemTooLong")]
[JsonSerializable(typeof(AppInternal.AccountValueProblem.LeadingOrTrailingSpace), TypeInfoPropertyName = "AccountValueProblemLeadingOrTrailingSpace")]
[JsonSerializable(typeof(AppInternal.AccountValueProblem.DisallowedCharacters), TypeInfoPropertyName = "AccountValueProblemDisallowedCharacters")]

[JsonSerializable(typeof(AppInternal.SystemEvent.WillSleep), TypeInfoPropertyName = "SystemEventWillSleep")]
[JsonSerializable(typeof(AppInternal.SystemEvent.DidWake), TypeInfoPropertyName = "SystemEventDidWake")]
[JsonSerializable(typeof(AppInternal.SystemEvent.ScreenLocked), TypeInfoPropertyName = "SystemEventScreenLocked")]
[JsonSerializable(typeof(AppInternal.SystemEvent.ScreenUnlocked), TypeInfoPropertyName = "SystemEventScreenUnlocked")]
[JsonSerializable(typeof(AppInternal.SystemEvent.DisplaysSlept), TypeInfoPropertyName = "SystemEventDisplaysSlept")]
[JsonSerializable(typeof(AppInternal.SystemEvent.DisplaysWoke), TypeInfoPropertyName = "SystemEventDisplaysWoke")]
[JsonSerializable(typeof(AppInternal.SystemEvent.NetworkPathChanged), TypeInfoPropertyName = "SystemEventNetworkPathChanged")]
[JsonSerializable(typeof(AppInternal.SystemEvent.ClockChanged), TypeInfoPropertyName = "SystemEventClockChanged")]
[JsonSerializable(typeof(AppInternal.SystemEvent.TimeZoneChanged), TypeInfoPropertyName = "SystemEventTimeZoneChanged")]
[JsonSerializable(typeof(AppInternal.SystemEvent.AppActivated), TypeInfoPropertyName = "SystemEventAppActivated")]
[JsonSerializable(typeof(AppInternal.SystemEvent.QuitRequested), TypeInfoPropertyName = "SystemEventQuitRequested")]
[JsonSerializable(typeof(AppInternal.SystemEvent.HelperEvent), TypeInfoPropertyName = "SystemEventHelperEvent")]
[JsonSerializable(typeof(AppInternal.SystemEvent.HelperOutput), TypeInfoPropertyName = "SystemEventHelperOutput")]
[JsonSerializable(typeof(AppInternal.SystemEvent.HelperConnectionLost), TypeInfoPropertyName = "SystemEventHelperConnectionLost")]

[JsonSerializable(typeof(HelperPipe.Request.Build), TypeInfoPropertyName = "RequestBuild")]
[JsonSerializable(typeof(HelperPipe.Request.Connect), TypeInfoPropertyName = "RequestConnect")]
[JsonSerializable(typeof(HelperPipe.Request.SubmitOtp), TypeInfoPropertyName = "RequestSubmitOtp")]
[JsonSerializable(typeof(HelperPipe.Request.Disconnect), TypeInfoPropertyName = "RequestDisconnect")]
[JsonSerializable(typeof(HelperPipe.Request.Status), TypeInfoPropertyName = "RequestStatus")]
[JsonSerializable(typeof(HelperPipe.Request.ExitWhenIdle), TypeInfoPropertyName = "RequestExitWhenIdle")]
[JsonSerializable(typeof(HelperPipe.Request.ReadBypassRoutes), TypeInfoPropertyName = "RequestReadBypassRoutes")]
[JsonSerializable(typeof(HelperPipe.Request.WriteBypassRoutes), TypeInfoPropertyName = "RequestWriteBypassRoutes")]
[JsonSerializable(typeof(HelperPipe.Request.PrepareUpdate), TypeInfoPropertyName = "RequestPrepareUpdate")]
[JsonSerializable(typeof(HelperPipe.Request.InstallUpdate), TypeInfoPropertyName = "RequestInstallUpdate")]
[JsonSerializable(typeof(HelperPipe.Request.ReadPreparedUpdate), TypeInfoPropertyName = "RequestReadPreparedUpdate")]
[JsonSerializable(typeof(HelperPipe.Request.DiscardPreparedUpdate), TypeInfoPropertyName = "RequestDiscardPreparedUpdate")]

[JsonSerializable(typeof(HelperPipe.Message.Build), TypeInfoPropertyName = "MessageBuild")]
[JsonSerializable(typeof(HelperPipe.Message.Reply), TypeInfoPropertyName = "MessageReply")]
[JsonSerializable(typeof(HelperPipe.Message.ReplyFlag), TypeInfoPropertyName = "MessageReplyFlag")]
[JsonSerializable(typeof(HelperPipe.Message.Event), TypeInfoPropertyName = "MessageEvent")]
[JsonSerializable(typeof(HelperPipe.Message.Output), TypeInfoPropertyName = "MessageOutput")]
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
/// Codes an abstract record whose nested sealed records are its cases, as a Swift enum with
/// associated values: <c>{"caseName": {…its values…}}</c>, where each case's name is its type's
/// name in camel case.
/// </summary>
internal sealed class CasesConverter<[DynamicallyAccessedMembers(DynamicallyAccessedMemberTypes.PublicNestedTypes)] T>
    : JsonConverter<T> where T : class
{
    private readonly Dictionary<string, Type> _byName;
    private readonly Dictionary<Type, string> _byType;

    public CasesConverter()
    {
        IEnumerable<Type> cases = typeof(T)
            .GetNestedTypes(BindingFlags.Public)
            .Where(nested => nested.IsSubclassOf(typeof(T)));
        _byName = cases.ToDictionary(nested => JsonNamingPolicy.CamelCase.ConvertName(nested.Name));
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
