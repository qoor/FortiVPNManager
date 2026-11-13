namespace FortiVPNManager.Contract;

public static partial class AppInternal
{
    /// <summary>
    /// Facts reported without being asked, in the order they happened
    /// (components/contract/src/app_internal/event.rs). Events that can come just before the system
    /// sleeps carry the time taken in the OS callback: their message may reach the runtime only
    /// after the wake.
    /// </summary>
    [Cases]
    public abstract record SystemEvent
    {
        private SystemEvent() { }

        public sealed record WillSleep(DateTimeOffset At) : SystemEvent;
        public sealed record DidWake : SystemEvent;
        public sealed record ScreenLocked(DateTimeOffset At) : SystemEvent;
        public sealed record ScreenUnlocked : SystemEvent;
        public sealed record DisplaysSlept(DateTimeOffset At) : SystemEvent;
        public sealed record DisplaysWoke : SystemEvent;

        /// <summary>On every path update, including the first.</summary>
        public sealed record NetworkPathChanged(bool Satisfied, IReadOnlyList<PathInterface> Interfaces) : SystemEvent;

        public sealed record NetworkSettingsChanged : SystemEvent;
        public sealed record ClockChanged : SystemEvent;

        /// <param name="TimeZone">The IANA name, such as <c>Asia/Seoul</c>.</param>
        public sealed record TimeZoneChanged(string TimeZone) : SystemEvent;

        public sealed record AppActivated : SystemEvent;

        /// <summary>The app then waits for <see cref="PortAction.ApproveQuit"/>.</summary>
        public sealed record QuitRequested : SystemEvent;

        /// <summary>Helper payloads exactly as received on the connection that carries the runs.</summary>
        public sealed record HelperEvent(string Json) : SystemEvent;

        public sealed record HelperOutput(string Json) : SystemEvent;

        /// <summary>Also after <see cref="HelperCall.ExitWhenIdle"/>, whose reply drops the connection.</summary>
        public sealed record HelperConnectionLost : SystemEvent;
    }

    public sealed record PathInterface(string Name, InterfaceKind Kind);

    public enum InterfaceKind
    {
        Wifi,
        WiredEthernet,
        Cellular,
        Loopback,

        /// <summary>Anything else, such as a tunnel.</summary>
        Other,
    }
}
