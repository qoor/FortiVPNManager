//! Turning the OS's facts into what the connection rules judge: whether the user is away, the
//! route to the gateway, and what survived a sleep.

use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

use fvm_contract::app_internal::event::{InterfaceKind, PathInterface};

use super::machine::{ConnectionManager, Phase};
use super::policy::{self, elapsed};
use super::{Effect, GatewayLookup, InterfaceAddress, NetworkProbe, ProbeResult};

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
    satisfied: bool,
    interfaces: Vec<PathInterface>,
}

#[derive(Default)]
pub(super) struct SystemObservation {
    pub(super) sleeping: bool,
    raw_path: Option<RawPath>,
    pub(super) path_settle_at: Option<SystemTime>,
    /// The probe a network path report waits for, with the host it looks up.
    path_probe: Option<(u64, String)>,
    pub(super) wake_settle: Option<WakeSettle>,
    next_probe: u64,
    /// The gateway address resolved for a host.
    pub(super) gateway_ip: Option<(String, Ipv4Addr)>,
    pub(super) screen_locked: bool,
    pub(super) displays_asleep: bool,
    pub(super) lock_poll_at: Option<SystemTime>,
    sleep_reported_at: Option<SystemTime>,
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
    deadline: SystemTime,
    /// The wake was judged without a network.
    late: bool,
    step: SettleStep,
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum SettleStep {
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

impl ConnectionManager {
    // MARK: Presence

    /// The user is away while the screen is locked, the displays sleep or the system sleeps.
    pub(super) fn update_presence(&mut self, at: SystemTime) {
        if !self.system.screen_locked {
            self.system.lock_poll_at = None;
        } else if self.system.lock_poll_at.is_none() {
            self.system.lock_poll_at = Some(self.now + policy::LOCK_POLL);
        }
        let away = self.system.screen_locked || self.system.displays_asleep || self.system.sleeping;
        if away != self.away_since.is_some() {
            self.presence_changed(away, at);
        }
    }

    pub(super) fn lock_poll_due(&mut self) {
        self.system.lock_poll_at = Some(self.now + policy::LOCK_POLL);
        self.effects.push(Effect::ReadScreenState);
    }

    // MARK: Sleep and wake

    pub(super) fn will_sleep_reported(&mut self, at: SystemTime) {
        self.system.sleep_reported_at = Some(at);
        self.system.wake_settle = None;
        self.system.sleeping = true;
        self.update_presence(at);
        self.will_sleep();
    }

    pub(super) fn did_wake_reported(&mut self) {
        let slept_for = self
            .system
            .sleep_reported_at
            .take()
            .map_or(Duration::ZERO, |at| elapsed(at, self.now));
        self.did_wake(slept_for);
        // Whether the screen is locked after the wake decides presence; the answer judges it.
        self.effects.push(Effect::ReadScreenState);
        self.system.wake_settle = Some(WakeSettle {
            deadline: self.now + policy::WAKE_SETTLE_TIMEOUT,
            late: false,
            step: SettleStep::Waiting(self.now),
        });
        self.wake_look_due();
    }

    /// The network is back when the path is satisfied and the primary network has a router.
    pub(super) fn wake_look_due(&mut self) {
        let Some(settle) = &mut self.system.wake_settle else {
            return;
        };
        if self
            .system
            .raw_path
            .as_ref()
            .is_some_and(|path| path.satisfied)
        {
            settle.step = SettleStep::ReadingRouter;
            self.effects.push(Effect::ReadRouter);
        } else {
            self.wake_network_missing();
        }
    }

    pub(super) fn router_read(&mut self, present: bool) {
        let Some(settle) = &self.system.wake_settle else {
            return;
        };
        if settle.step != SettleStep::ReadingRouter {
            return;
        }
        if present
            && self
                .system
                .raw_path
                .as_ref()
                .is_some_and(|path| path.satisfied)
        {
            self.wake_network_back();
        } else {
            self.wake_network_missing();
        }
    }

    fn wake_network_missing(&mut self) {
        let now = self.now;
        let Some(settle) = &mut self.system.wake_settle else {
            return;
        };
        if settle.late {
            settle.step = SettleStep::Waiting(now + policy::LATE_SETTLE_POLL);
        } else if now < settle.deadline {
            settle.step = SettleStep::Waiting(now + policy::WAKE_SETTLE_POLL);
        } else {
            // Judged without a network; what survived does not matter then.
            settle.late = true;
            self.wake_settled(&WakeObservation::NoNetwork);
            self.wake_look_due();
        }
    }

    fn wake_network_back(&mut self) {
        let Some(settle) = &self.system.wake_settle else {
            return;
        };
        if settle.late {
            self.system.wake_settle = None;
            self.report_path();
            return;
        }
        let session_running = self.run_active();
        let (id, host) = self.probe(session_running);
        if let Some(settle) = &mut self.system.wake_settle {
            settle.step = SettleStep::Observing(id, host);
        }
    }

    // MARK: Network path

    pub(super) fn network_path_changed(&mut self, satisfied: bool, interfaces: Vec<PathInterface>) {
        self.system.raw_path = Some(RawPath {
            satisfied,
            interfaces,
        });
        self.system.path_settle_at = Some(self.now + policy::PATH_SETTLE);
        self.system.path_probe = None;
        // Back after a wake: look now rather than at the next poll.
        if satisfied
            && self
                .system
                .wake_settle
                .as_ref()
                .is_some_and(|settle| !settle.late && matches!(settle.step, SettleStep::Waiting(_)))
        {
            self.wake_look_due();
        }
    }

    pub(super) fn path_settled(&mut self) {
        self.system.path_settle_at = None;
        self.report_path();
    }

    fn report_path(&mut self) {
        if self
            .system
            .raw_path
            .as_ref()
            .is_some_and(|path| path.satisfied)
        {
            let connected = matches!(self.phase, Phase::Connected(_));
            self.system.path_probe = Some(self.probe(connected));
        } else {
            self.path_changed(
                PathSnapshot {
                    satisfied: false,
                    underlay: None,
                },
                false,
            );
        }
    }

    /// Asks for the route to the gateway; returns the probe's ID and the host it looks up.
    fn probe(&mut self, session_running: bool) -> (u64, String) {
        self.system.next_probe += 1;
        let host = self.settings.account.host.clone();
        let gateway = (!host.is_empty()).then(|| GatewayLookup {
            host: host.clone(),
            known_ip: self
                .system
                .gateway_ip
                .as_ref()
                .filter(|(resolved, _)| *resolved == host)
                .map(|(_, ip)| *ip),
        });
        self.effects.push(Effect::ProbeNetwork(NetworkProbe {
            id: self.system.next_probe,
            gateway,
            session_running,
        }));
        (self.system.next_probe, host)
    }

    pub(super) fn network_probed(&mut self, result: &ProbeResult) {
        let for_path = self
            .system
            .path_probe
            .as_ref()
            .filter(|(id, _)| *id == result.id)
            .map(|(_, host)| host.clone());
        let for_wake = match &self.system.wake_settle {
            Some(WakeSettle {
                step: SettleStep::Observing(id, host),
                ..
            }) if *id == result.id => Some(host.clone()),
            _ => None,
        };
        let Some(host) = for_path.clone().or(for_wake) else {
            return;
        };
        if let Some(ip) = result.gateway_ip
            && host == self.settings.account.host
        {
            self.system.gateway_ip = Some((host, ip));
        }
        let underlay = self.underlay(result);
        let routing_incomplete = result.routing_complete == Some(false);
        if for_path.is_some() {
            self.system.path_probe = None;
            self.path_changed(
                PathSnapshot {
                    satisfied: true,
                    underlay,
                },
                routing_incomplete,
            );
        } else {
            self.system.wake_settle = None;
            let tunnel_up = self.tunnel_interface_up(result.addresses.as_deref());
            self.wake_settled(&WakeObservation::NetworkBack {
                underlay,
                session_running: result.session_running.unwrap_or(false),
                tunnel_up,
                routing_incomplete,
            });
        }
    }

    /// The route interface, unless it is the tunnel's own or one the path reports as a tunnel or
    /// other virtual interface. The tunnel interface is the one holding the tunnel IP the Helper
    /// reported.
    fn underlay(&self, result: &ProbeResult) -> Option<Underlay> {
        let name = result.route_interface.as_ref()?;
        let addresses = result.addresses.as_ref()?;
        let tunnel_ip = self.tunnel_ip();
        let carries_tunnel = addresses
            .iter()
            .any(|address| address.name == *name && Some(address.ipv4) == tunnel_ip);
        let virtual_interface = self.system.raw_path.as_ref().is_some_and(|path| {
            path.interfaces
                .iter()
                .any(|interface| interface.name == *name && interface.kind == InterfaceKind::Other)
        });
        if carries_tunnel || virtual_interface {
            return None;
        }
        Some(Underlay {
            interface: name.clone(),
            address: addresses
                .iter()
                .find(|address| address.name == *name)
                .map(|address| address.ipv4),
        })
    }

    /// Without a reported tunnel IP the interface cannot be told, so it counts as there: a wrong
    /// "gone" would replace a working tunnel after every wake.
    fn tunnel_interface_up(&self, addresses: Option<&[InterfaceAddress]>) -> bool {
        self.tunnel_ip().is_none_or(|ip| {
            addresses.is_none_or(|addresses| {
                addresses
                    .iter()
                    .any(|address| address.ipv4 == ip && address.up)
            })
        })
    }

    fn tunnel_ip(&self) -> Option<Ipv4Addr> {
        match &self.phase {
            Phase::Connected(session) => session.tunnel_ip,
            Phase::Connecting(attempt) => attempt
                .replacing
                .as_ref()
                .and_then(|session| session.tunnel_ip),
            _ => None,
        }
    }
}
