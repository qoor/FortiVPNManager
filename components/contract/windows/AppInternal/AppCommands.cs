using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract.AppInternal;

/// <summary>User actions (components/contract/src/app_internal/command.rs).</summary>
[JsonConverter(typeof(CasesConverter<Command>))]
[JsonCase(typeof(CommandConnect), "connect")]
[JsonCase(typeof(CommandDisconnect), "disconnect")]
[JsonCase(typeof(CommandReconnectNow), "reconnectNow")]
[JsonCase(typeof(CommandSettingsOpened), "settingsOpened")]
[JsonCase(typeof(CommandSettingsClosed), "settingsClosed")]
[JsonCase(typeof(CommandEditAccount), "editAccount")]
[JsonCase(typeof(CommandSaveAccount), "saveAccount")]
[JsonCase(typeof(CommandImportClientSettings), "importClientSettings")]
[JsonCase(typeof(CommandDismissImportReport), "dismissImportReport")]
[JsonCase(typeof(CommandSetOtpMethod), "setOtpMethod")]
[JsonCase(typeof(CommandTestGmail), "testGmail")]
[JsonCase(typeof(CommandConnectGoogleAccount), "connectGoogleAccount")]
[JsonCase(typeof(CommandCancelGoogleSignIn), "cancelGoogleSignIn")]
[JsonCase(typeof(CommandDisconnectGoogleAccount), "disconnectGoogleAccount")]
[JsonCase(typeof(CommandImportOAuthClient), "importOAuthClient")]
[JsonCase(typeof(CommandUseBuiltInOAuthClient), "useBuiltInOAuthClient")]
[JsonCase(typeof(CommandSetConnectionOption), "setConnectionOption")]
[JsonCase(typeof(CommandSetScheduleHours), "setScheduleHours")]
[JsonCase(typeof(CommandSetOpenAtLogin), "setOpenAtLogin")]
[JsonCase(typeof(CommandRegisterHelper), "registerHelper")]
[JsonCase(typeof(CommandOpenHelperApprovalSettings), "openHelperApprovalSettings")]
[JsonCase(typeof(CommandSetBypassRows), "setBypassRows")]
[JsonCase(typeof(CommandAddCurrentNetwork), "addCurrentNetwork")]
[JsonCase(typeof(CommandCheckForUpdates), "checkForUpdates")]
[JsonCase(typeof(CommandUpdatePageOpened), "updatePageOpened")]
[JsonCase(typeof(CommandUpdateNow), "updateNow")]
[JsonCase(typeof(CommandUpdateNowFromNotification), "updateNowFromNotification")]
[JsonCase(typeof(CommandCancelUpdateDownload), "cancelUpdateDownload")]
[JsonCase(typeof(CommandRestartToUpdate), "restartToUpdate")]
public abstract record Command
{
    private protected Command() { }
}

public sealed record CommandConnect : Command;

public sealed record CommandDisconnect : Command;

public sealed record CommandReconnectNow : Command;

public sealed record CommandSettingsOpened : Command;

/// <summary>Passwords typed in the Account pane are forgotten.</summary>
public sealed record CommandSettingsClosed : Command;

/// <summary>
/// The Account pane's fields as typed, after any edit, with the form's revision they were
/// typed against; the runtime drops an edit against an older revision.
/// </summary>
public sealed record CommandEditAccount(ulong Revision, AccountForm Form) : Command;

public sealed record CommandSaveAccount : Command;

/// <summary>The VPN client's own settings file; null is its default location.</summary>
public sealed record CommandImportClientSettings(string? File = null) : Command;

public sealed record CommandDismissImportReport : Command;

public sealed record CommandSetOtpMethod(OtpMethod Method) : Command;

public sealed record CommandTestGmail : Command;

public sealed record CommandConnectGoogleAccount : Command;

public sealed record CommandCancelGoogleSignIn : Command;

public sealed record CommandDisconnectGoogleAccount : Command;

public sealed record CommandImportOAuthClient(string File) : Command;

public sealed record CommandUseBuiltInOAuthClient : Command;

public sealed record CommandSetConnectionOption(ConnectionOption Option) : Command;

/// <summary>On every change of either time picker.</summary>
public sealed record CommandSetScheduleHours(TimeOfDay Start, TimeOfDay End) : Command;

public sealed record CommandSetOpenAtLogin(bool Enabled) : Command;

public sealed record CommandRegisterHelper : Command;

public sealed record CommandOpenHelperApprovalSettings : Command;

public sealed record CommandSetBypassRows(BypassList List, IReadOnlyList<string> Rows) : Command;

public sealed record CommandAddCurrentNetwork : Command;

public sealed record CommandCheckForUpdates : Command;

/// <summary>The Software Update section came into view.</summary>
public sealed record CommandUpdatePageOpened : Command;

public sealed record CommandUpdateNow : Command;

/// <summary>The update notification's button.</summary>
public sealed record CommandUpdateNowFromNotification : Command;

public sealed record CommandCancelUpdateDownload : Command;

public sealed record CommandRestartToUpdate : Command;

/// <summary>An empty password field keeps the stored password.</summary>
public sealed record AccountForm(
    string Server,
    string Username,
    string Realm,
    string GmailAddress,
    string VpnPassword,
    string AppPassword)
{
    /// <summary>Debug output can reach logs and crash reports, so it leaves the passwords out.</summary>
    public override string ToString() =>
        $"AccountForm {{ Server = {Server}, Username = {Username}, Realm = {Realm}, GmailAddress = {GmailAddress}, VpnPassword = <redacted>, AppPassword = <redacted> }}";
}

/// <summary>Minutes are the number as typed.</summary>
[JsonConverter(typeof(CasesConverter<ConnectionOption>))]
[JsonCase(typeof(ConnectionOptionScheduleEnabled), "scheduleEnabled")]
[JsonCase(typeof(ConnectionOptionScheduleDay), "scheduleDay")]
[JsonCase(typeof(ConnectionOptionAwayMinutes), "awayMinutes")]
[JsonCase(typeof(ConnectionOptionAutoConnectOnLaunch), "autoConnectOnLaunch")]
[JsonCase(typeof(ConnectionOptionSessionMaxMinutes), "sessionMaxMinutes")]
[JsonCase(typeof(ConnectionOptionProactiveReconnect), "proactiveReconnect")]
[JsonCase(typeof(ConnectionOptionProactiveMarginMinutes), "proactiveMarginMinutes")]
[JsonCase(typeof(ConnectionOptionReconnectAfterSleep), "reconnectAfterSleep")]
[JsonCase(typeof(ConnectionOptionMinSleepMinutes), "minSleepMinutes")]
public abstract record ConnectionOption
{
    private protected ConnectionOption() { }
}

public sealed record ConnectionOptionScheduleEnabled(bool Enabled) : ConnectionOption;

public sealed record ConnectionOptionScheduleDay(Weekday Day, bool Selected) : ConnectionOption;

public sealed record ConnectionOptionAwayMinutes(long Minutes) : ConnectionOption;

public sealed record ConnectionOptionAutoConnectOnLaunch(bool Enabled) : ConnectionOption;

public sealed record ConnectionOptionSessionMaxMinutes(long Minutes) : ConnectionOption;

public sealed record ConnectionOptionProactiveReconnect(bool Enabled) : ConnectionOption;

public sealed record ConnectionOptionProactiveMarginMinutes(long Minutes) : ConnectionOption;

public sealed record ConnectionOptionReconnectAfterSleep(bool Enabled) : ConnectionOption;

public sealed record ConnectionOptionMinSleepMinutes(long Minutes) : ConnectionOption;

[JsonConverter(typeof(EnumNameConverter<BypassList>))]
public enum BypassList
{
    Networks,
    Targets,
}
