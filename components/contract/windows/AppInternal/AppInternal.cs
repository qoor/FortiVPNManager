using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract.AppInternal;

/// <summary>Sent to the runtime.</summary>
[JsonConverter(typeof(CasesConverter<ToRuntime>))]
[JsonCase(typeof(ToRuntimeLaunched), "launched")]
[JsonCase(typeof(ToRuntimeCommand), "command")]
[JsonCase(typeof(ToRuntimePortResponse), "portResponse")]
[JsonCase(typeof(ToRuntimeEvent), "event")]
public abstract record ToRuntime
{
    private protected ToRuntime() { }
}

/// <summary>Always the first message.</summary>
public sealed record ToRuntimeLaunched(AppFacts Facts) : ToRuntime;

public sealed record ToRuntimeCommand([property: JsonPropertyName("_0")] AppInternal.Command Value) : ToRuntime;

/// <summary>Answers the <see cref="FromRuntimePortRequest"/> with the same id.</summary>
public sealed record ToRuntimePortResponse(ulong Id, AppInternal.PortResponse Response) : ToRuntime;

public sealed record ToRuntimeEvent([property: JsonPropertyName("_0")] SystemEvent Value) : ToRuntime;

/// <summary>Sent by the runtime.</summary>
[JsonConverter(typeof(CasesConverter<FromRuntime>))]
[JsonCase(typeof(FromRuntimeSection), "section")]
[JsonCase(typeof(FromRuntimeLogLines), "logLines")]
[JsonCase(typeof(FromRuntimePortRequest), "portRequest")]
[JsonCase(typeof(FromRuntimePortAction), "portAction")]
[JsonCase(typeof(FromRuntimeOpenSettings), "openSettings")]
public abstract record FromRuntime
{
    private protected FromRuntime() { }
}

/// <summary>One domain section, whole. Every section comes once first, then only changed ones.</summary>
public sealed record FromRuntimeSection([property: JsonPropertyName("_0")] AppInternal.Section Value) : FromRuntime;

/// <summary>Log lines not sent before, oldest first.</summary>
public sealed record FromRuntimeLogLines(IReadOnlyList<LogLine> Lines) : FromRuntime;

/// <summary>Answer with <see cref="ToRuntimePortResponse"/> and the same id.</summary>
public sealed record FromRuntimePortRequest(ulong Id, AppInternal.PortRequest Request) : FromRuntime;

/// <summary>Carry out without an answer.</summary>
public sealed record FromRuntimePortAction([property: JsonPropertyName("_0")] AppInternal.PortAction Value) : FromRuntime;

/// <summary>
/// Show the settings window, at most once per launch (the rule is in
/// components/contract/src/app_internal/mod.rs).
/// </summary>
public sealed record FromRuntimeOpenSettings : FromRuntime;

/// <param name="AppBuild">The build the Helper must also report.</param>
/// <param name="InstalledCopy">The app runs from where it was installed, the only copy allowed to register the Helper and the login item (on Windows the installer registers the Helper).</param>
/// <param name="BuiltInOauthClient">The OAuth client JSON built into the app, if this build has one.</param>
/// <param name="TimeZone">The system time zone's IANA name, such as <c>Asia/Seoul</c>.</param>
public sealed record AppFacts(
    string AppBuild,
    string AppVersion,
    bool InstalledCopy,
    string TimeZone,
    SignInPages SignInPages,
    string? BuiltInOauthClient = null)
{
    /// <summary>The OAuth client's secret is not confidential, but it stays out of debug output all the same.</summary>
    public override string ToString() =>
        $"AppFacts {{ AppBuild = {AppBuild}, AppVersion = {AppVersion}, InstalledCopy = {InstalledCopy}, TimeZone = {TimeZone}, BuiltInOauthClient = {(BuiltInOauthClient is null ? "none" : "<redacted>")} }}";
}

/// <summary>The pages the Google sign-in listener shows in the browser, in the UI's language.</summary>
public sealed record SignInPages(string Completed, string NotCompleted, string Invalid);

/// <summary>
/// One line for the Log pane. The first line of a connection attempt comes after two empty lines
/// unless no line comes before it.
/// </summary>
public sealed record LogLine(DateTimeOffset Time, LogSource Source, string Text, bool StartsAttempt);

[JsonConverter(typeof(EnumNameConverter<LogSource>))]
public enum LogSource
{
    Info,
    Error,

    /// <summary>An explanation line from the app or the Helper.</summary>
    Note,
}

[JsonConverter(typeof(EnumNameConverter<OtpMethod>))]
public enum OtpMethod
{
    Imap,
    GmailApi,
}

[JsonConverter(typeof(EnumNameConverter<Weekday>))]
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

[JsonConverter(typeof(EnumNameConverter<ButtonState>))]
public enum ButtonState
{
    Hidden,
    Enabled,
    Disabled,
}

/// <summary>A setting in minutes with the range the settings file accepts.</summary>
public sealed record MinutesSetting(uint Value, uint Minimum, uint Maximum);
