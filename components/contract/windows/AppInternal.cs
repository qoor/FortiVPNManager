using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract;

/// <summary>
/// Messages between the app's C# parts (the UI and the OS integration) and the runtime (Rust),
/// defined in components/contract/src/app_internal as Swift's components/contract/macos/app also
/// defines them; type, case and value names give the JSON keys, and <see cref="MessageCoding"/>
/// codes them.
/// </summary>
public static partial class AppInternal
{
    /// <summary>Sent to the runtime.</summary>
    [Cases]
    public abstract record ToRuntime
    {
        private ToRuntime() { }

        /// <summary>Always the first message.</summary>
        public sealed record Launched(AppFacts Facts) : ToRuntime;

        public sealed record Command([property: JsonPropertyName("_0")] AppInternal.Command Value) : ToRuntime;

        /// <summary>Answers the <see cref="FromRuntime.PortRequest"/> with the same id.</summary>
        public sealed record PortResponse(ulong Id, AppInternal.PortResponse Response) : ToRuntime;

        public sealed record Event([property: JsonPropertyName("_0")] SystemEvent Value) : ToRuntime;
    }

    /// <summary>Sent by the runtime.</summary>
    [Cases]
    public abstract record FromRuntime
    {
        private FromRuntime() { }

        /// <summary>One domain section, whole. Every section comes once first, then only changed ones.</summary>
        public sealed record Section([property: JsonPropertyName("_0")] AppInternal.Section Value) : FromRuntime;

        /// <summary>Log lines not sent before, oldest first.</summary>
        public sealed record LogLines(IReadOnlyList<LogLine> Lines) : FromRuntime;

        /// <summary>Answer with <see cref="ToRuntime.PortResponse"/> and the same id.</summary>
        public sealed record PortRequest(ulong Id, AppInternal.PortRequest Request) : FromRuntime;

        /// <summary>Carry out without an answer.</summary>
        public sealed record PortAction([property: JsonPropertyName("_0")] AppInternal.PortAction Value) : FromRuntime;

        /// <summary>
        /// Show the settings window, at most once per launch (the rule is in
        /// components/contract/src/app_internal/mod.rs).
        /// </summary>
        public sealed record OpenSettings : FromRuntime;
    }

    /// <param name="AppBuild">The build the Helper must also report.</param>
    /// <param name="InstalledCopy">The app runs from where it was installed, the only copy allowed to register the Helper and the login item (on Windows the install script registers the Helper).</param>
    /// <param name="BuiltInOauthClient">The OAuth client JSON built into the app, if this build has one.</param>
    /// <param name="TimeZone">The system time zone's IANA name, such as <c>Asia/Seoul</c>.</param>
    public sealed record AppFacts(
        string AppBuild,
        bool InstalledCopy,
        string TimeZone,
        SignInPages SignInPages,
        string? BuiltInOauthClient = null)
    {
        /// <summary>The OAuth client's secret is not confidential, but it stays out of debug output all the same.</summary>
        public override string ToString() =>
            $"AppFacts {{ AppBuild = {AppBuild}, InstalledCopy = {InstalledCopy}, TimeZone = {TimeZone}, BuiltInOauthClient = {(BuiltInOauthClient is null ? "none" : "<redacted>")} }}";
    }

    /// <summary>The pages the Google sign-in listener shows in the browser, in the UI's language.</summary>
    public sealed record SignInPages(string Completed, string NotCompleted, string Invalid);

    /// <summary>One line for the Log pane.</summary>
    public sealed record LogLine(DateTimeOffset Time, LogSource Source, string Text);

    public enum LogSource
    {
        Stdout,
        Stderr,

        /// <summary>An explanation line from the app or the Helper.</summary>
        Note,
    }

    public enum OtpMethod
    {
        Imap,
        GmailApi,
    }

    public enum Weekday
    {
        Mon,
        Tue,
        Wed,
        Thu,
        Fri,
        Sat,
        Sun,
    }

    /// <summary>A local wall-clock time.</summary>
    public sealed record TimeOfDay(byte Hour, byte Minute);

    public enum ButtonState
    {
        Hidden,
        Enabled,
        Disabled,
    }

    /// <summary>A setting in minutes with the range the settings file accepts.</summary>
    public sealed record MinutesSetting(uint Value, uint Minimum, uint Maximum);
}
