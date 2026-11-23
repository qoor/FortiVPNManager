//! Turning the OS's facts into what the connection rules judge: whether the user is away, the
//! route to the gateway, and what survived a sleep.

use std::net::Ipv4Addr;
use std::time::SystemTime;

use fvm_contract::app_internal::event::PathInterface;

/// The physical route to the gateway: the interface the routing table uses for its IPv4 address,
/// and this Mac's address on it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Underlay {
    pub(super) interface: String,
    pub(super) address: Option<Ipv4Addr>,
}

/// The network path as judged.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct PathSnapshot {
    pub(super) satisfied: bool,
    pub(super) underlay: Option<Underlay>,
}

/// The path as the OS last reported it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct RawPath {
    pub(super) satisfied: bool,
    pub(super) interfaces: Vec<PathInterface>,
}

#[derive(Default)]
pub(super) struct SystemObservation {
    pub(super) sleeping: bool,
    pub(super) raw_path: Option<RawPath>,
    pub(super) path_settle_at: Option<SystemTime>,
    /// The probe a network path report waits for, with the host it looks up.
    pub(super) path_probe: Option<(u64, String)>,
    pub(super) wake_settle: Option<WakeSettle>,
    pub(super) next_probe: u64,
    /// The gateway address resolved for a host.
    pub(super) gateway_ip: Option<(String, Ipv4Addr)>,
    pub(super) screen_locked: bool,
    pub(super) displays_asleep: bool,
    pub(super) lock_poll_at: Option<SystemTime>,
    pub(super) sleep_reported_at: Option<SystemTime>,
}

/// What survived a sleep, once the network came back or the wait for it ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum WakeObservation {
    NoNetwork,
    NetworkBack {
        underlay: Option<Underlay>,
        session_running: bool,
        /// The tunnel interface is still there and up.
        tunnel_up: bool,
        routing_incomplete: bool,
    },
}

/// Waiting for the network after a wake: until it is back or the time is up, looking every
/// second; after that, every two seconds until it is back, to report the path.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct WakeSettle {
    pub(super) deadline: SystemTime,
    /// The wake was judged without a network.
    pub(super) late: bool,
    pub(super) step: SettleStep,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum SettleStep {
    /// The next look is due at this time.
    Waiting(SystemTime),
    /// The router was asked for.
    ReadingRouter,
    /// The network is back; the probe with this ID observes what survived.
    Observing(u64, String),
}

impl WakeSettle {
    pub(super) fn next_look(&self) -> Option<SystemTime> {
        match self.step {
            SettleStep::Waiting(at) => Some(at),
            SettleStep::ReadingRouter | SettleStep::Observing(..) => None,
        }
    }
}
