extension AppInternal {
    /// Facts reported without being asked, in the order they happened
    /// (components/contract/src/app_internal/event.rs).
    enum SystemEvent: Encodable, Sendable {
        case willSleep
        case didWake
        case screenLocked
        case screenUnlocked
        case displaysSlept
        case displaysWoke
        /// On every path update, including the first.
        case networkPathChanged(satisfied: Bool, interfaces: [PathInterface])
        case networkSettingsChanged
        case clockChanged
        case timeZoneChanged(timeZone: String)
        case appActivated
        /// The app then waits for `PortAction.approveQuit`.
        case quitRequested
        /// Helper payloads exactly as received from the connection that carries the runs.
        case helperEvent(json: String)
        case helperOutput(json: String)
        case helperConnectionLost
    }

    struct PathInterface: Encodable, Sendable {
        var name: String
        var kind: InterfaceKind
    }

    enum InterfaceKind: String, Encodable, Sendable {
        case wifi
        case wiredEthernet
        case cellular
        case loopback
        case other
    }
}
