using System.Text.Json.Serialization;

namespace FortiVPNManager.Contract.AppInternal;

/// <summary>
/// Facts reported without being asked, in the order they happened
/// (components/contract/src/app_internal/event.rs). Events that can come just before the system
/// sleeps carry the time taken in the OS callback: their message may reach the runtime only
/// after the wake.
/// </summary>
[JsonConverter(typeof(CasesConverter<SystemEvent>))]
[JsonCase(typeof(SystemEventWillSleep), "willSleep")]
[JsonCase(typeof(SystemEventDidWake), "didWake")]
[JsonCase(typeof(SystemEventScreenLocked), "screenLocked")]
[JsonCase(typeof(SystemEventScreenUnlocked), "screenUnlocked")]
[JsonCase(typeof(SystemEventDisplaysSlept), "displaysSlept")]
[JsonCase(typeof(SystemEventDisplaysWoke), "displaysWoke")]
[JsonCase(typeof(SystemEventNetworkPathChanged), "networkPathChanged")]
[JsonCase(typeof(SystemEventClockChanged), "clockChanged")]
[JsonCase(typeof(SystemEventTimeZoneChanged), "timeZoneChanged")]
[JsonCase(typeof(SystemEventAppActivated), "appActivated")]
[JsonCase(typeof(SystemEventQuitRequested), "quitRequested")]
[JsonCase(typeof(SystemEventHelperEvent), "helperEvent")]
[JsonCase(typeof(SystemEventHelperOutput), "helperOutput")]
[JsonCase(typeof(SystemEventHelperConnectionLost), "helperConnectionLost")]
public abstract record SystemEvent
{
    private protected SystemEvent() { }
}

public sealed record SystemEventWillSleep(DateTimeOffset At) : SystemEvent;

public sealed record SystemEventDidWake : SystemEvent;

public sealed record SystemEventScreenLocked(DateTimeOffset At) : SystemEvent;

public sealed record SystemEventScreenUnlocked : SystemEvent;

public sealed record SystemEventDisplaysSlept(DateTimeOffset At) : SystemEvent;

public sealed record SystemEventDisplaysWoke : SystemEvent;

/// <summary>On every path update, including the first.</summary>
public sealed record SystemEventNetworkPathChanged(bool Satisfied, IReadOnlyList<PathInterface> Interfaces) : SystemEvent;

public sealed record SystemEventClockChanged : SystemEvent;

/// <param name="TimeZone">The IANA name, such as <c>Asia/Seoul</c>.</param>
public sealed record SystemEventTimeZoneChanged(string TimeZone) : SystemEvent;

public sealed record SystemEventAppActivated : SystemEvent;

/// <summary>The app then waits for <see cref="PortActionApproveQuit"/>.</summary>
public sealed record SystemEventQuitRequested : SystemEvent;

/// <summary>Helper payloads exactly as received on the connection that carries the runs.</summary>
public sealed record SystemEventHelperEvent(string Json) : SystemEvent;

public sealed record SystemEventHelperOutput(string Json) : SystemEvent;

/// <summary>Also after <see cref="HelperCallExitWhenIdle"/>, whose reply drops the connection.</summary>
public sealed record SystemEventHelperConnectionLost : SystemEvent;

public sealed record PathInterface(string Name, InterfaceKind Kind);

[JsonConverter(typeof(EnumNameConverter<InterfaceKind>))]
public enum InterfaceKind
{
    Wifi,
    WiredEthernet,
    Cellular,
    Loopback,

    /// <summary>Anything else, such as a tunnel.</summary>
    Other,
}
