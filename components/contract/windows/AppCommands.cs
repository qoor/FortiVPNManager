namespace FortiVPNManager.Contract;

public static partial class AppInternal
{
    /// <summary>User actions (components/contract/src/app_internal/command.rs).</summary>
    [Cases]
    public abstract record Command
    {
        private Command() { }

        public sealed record Connect : Command;
        public sealed record Disconnect : Command;
        public sealed record ReconnectNow : Command;
        public sealed record SettingsOpened : Command;

        /// <summary>Passwords typed in the Account pane are forgotten.</summary>
        public sealed record SettingsClosed : Command;

        /// <summary>
        /// The Account pane's fields as typed, after any edit, with the form's revision they were
        /// typed against; the runtime drops an edit against an older revision.
        /// </summary>
        public sealed record EditAccount(ulong Revision, AccountForm Form) : Command;

        public sealed record SaveAccount : Command;
        public sealed record RevertAccount : Command;

        /// <summary>The VPN client's own settings file; null is its default location.</summary>
        public sealed record ImportClientSettings(string? File = null) : Command;

        public sealed record DismissImportReport : Command;
        public sealed record SetOtpMethod(OtpMethod Method) : Command;
        public sealed record TestGmail : Command;
        public sealed record ConnectGoogleAccount : Command;
        public sealed record CancelGoogleSignIn : Command;
        public sealed record DisconnectGoogleAccount : Command;
        public sealed record ImportOAuthClient(string File) : Command;
        public sealed record UseBuiltInOAuthClient : Command;

        public sealed record SetConnectionOption(ConnectionOption Option) : Command;

        /// <summary>On every change of either time picker.</summary>
        public sealed record SetScheduleHours(TimeOfDay Start, TimeOfDay End) : Command;

        public sealed record SetOpenAtLogin(bool Enabled) : Command;
        public sealed record RegisterHelper : Command;
        public sealed record OpenHelperApprovalSettings : Command;

        public sealed record SetBypassRows(BypassList List, IReadOnlyList<string> Rows) : Command;
        public sealed record AddCurrentNetwork : Command;
        public sealed record DiscardBypassChanges : Command;
        public sealed record ApplyBypassRules : Command;
    }

    /// <summary>An empty password field keeps the stored password.</summary>
    public sealed record AccountForm(
        string Host,
        string Port,
        string Username,
        string Realm,
        string GmailAddress,
        string VpnPassword,
        string AppPassword)
    {
        /// <summary>Debug output can reach logs and crash reports, so it leaves the passwords out.</summary>
        public override string ToString() =>
            $"AccountForm {{ Host = {Host}, Port = {Port}, Username = {Username}, Realm = {Realm}, GmailAddress = {GmailAddress}, VpnPassword = <redacted>, AppPassword = <redacted> }}";
    }

    /// <summary>Minutes are the number as typed.</summary>
    [Cases]
    public abstract record ConnectionOption
    {
        private ConnectionOption() { }

        public sealed record ScheduleEnabled(bool Enabled) : ConnectionOption;
        public sealed record ScheduleDay(Weekday Day, bool Selected) : ConnectionOption;
        public sealed record AwayMinutes(long Minutes) : ConnectionOption;
        public sealed record AutoConnectOnLaunch(bool Enabled) : ConnectionOption;
        public sealed record SessionMaxMinutes(long Minutes) : ConnectionOption;
        public sealed record ProactiveReconnect(bool Enabled) : ConnectionOption;
        public sealed record ProactiveMarginMinutes(long Minutes) : ConnectionOption;
        public sealed record ReconnectAfterSleep(bool Enabled) : ConnectionOption;
        public sealed record MinSleepMinutes(long Minutes) : ConnectionOption;
    }

    public enum BypassList
    {
        Networks,
        Targets,
    }
}
