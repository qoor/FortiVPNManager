//! Facts the OS integration reports without being asked, in the order they happened. Events that
//! can come just before the system sleeps carry the time the OS reported them: their message may
//! be read only after the wake, which would make the sleep or the time away look shorter. For the
//! other events the runtime takes the time a message arrives.

use std::time::SystemTime;

use serde::Deserialize;

use super::millis;

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SystemEvent {
    /// The system is about to sleep.
    WillSleep {
        #[serde(deserialize_with = "millis::read_time")]
        at: SystemTime,
    },
    DidWake {},
    ScreenLocked {
        #[serde(deserialize_with = "millis::read_time")]
        at: SystemTime,
    },
    ScreenUnlocked {},
    /// Every display went to sleep.
    DisplaysSlept {
        #[serde(deserialize_with = "millis::read_time")]
        at: SystemTime,
    },
    DisplaysWoke {},
    /// The system's network path, as the OS reports it on every update, including the first.
    NetworkPathChanged {
        satisfied: bool,
        interfaces: Vec<PathInterface>,
    },
    /// The saved IPv4 settings of a network service or the primary IPv4 state changed; one change
    /// may be reported several times.
    NetworkSettingsChanged {},
    /// The wall clock was set.
    ClockChanged {},
    TimeZoneChanged {
        time_zone: String,
    },
    /// The app became the active app.
    AppActivated {},
    /// The user or the system asked the app to quit. The app waits for `PortAction::ApproveQuit`.
    QuitRequested {},
    /// A `HelperEvent` from the Helper connection that carries the runs.
    HelperEvent {
        json: String,
    },
    /// A batch of the Helper's `OutputLine`s from that connection.
    HelperOutput {
        json: String,
    },
    /// That connection was interrupted or invalidated, or dropped after
    /// `HelperCall::ExitWhenIdle`; no exit report follows for its runs.
    HelperConnectionLost {},
}

/// An interface the network path may use.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathInterface {
    pub name: String,
    pub kind: InterfaceKind,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InterfaceKind {
    Wifi,
    WiredEthernet,
    Cellular,
    Loopback,
    /// Anything else, such as a tunnel.
    Other,
}
