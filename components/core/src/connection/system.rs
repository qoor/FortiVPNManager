//! Turning the OS's facts into what the connection rules judge: whether the user is away, the
//! route to the gateway, and what survived a sleep.

use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

use fvm_contract::app_internal::event::{InterfaceKind, PathInterface};
use fvm_contract::app_internal::port::InterfaceAddress;

use super::machine::{ConnectionManager, Phase};
use super::policy::{self, elapsed};
use super::{Effect, GatewayLookup, NetworkProbe, ProbeResult};

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

/// What survived a sleep, once the network came back or the wait for it ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct WakeObservation {
    pub(super) path_satisfied: bool,
    pub(super) underlay: Option<Underlay>,
    pub(super) client_running: bool,
    /// The tunnel interface is still there and up.
    pub(super) tunnel_up: bool,
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
        if !self.screen_locked {
            self.lock_poll_at = None;
        } else if self.lock_poll_at.is_none() {
            self.lock_poll_at = Some(self.now + policy::LOCK_POLL);
        }
        let away = self.screen_locked || self.displays_asleep || self.system_asleep;
        if away != self.reported_away {
            self.reported_away = away;
            self.presence_changed(away, at);
        }
    }

    pub(super) fn lock_poll_due(&mut self) {
        self.lock_poll_at = Some(self.now + policy::LOCK_POLL);
        self.effects.push(Effect::ReadScreenState);
    }

    // MARK: Sleep and wake

    pub(super) fn will_sleep_reported(&mut self, at: SystemTime) {
        self.sleep_reported_at = Some(at);
        self.wake_settle = None;
        self.system_asleep = true;
        self.update_presence(at);
        self.will_sleep();
    }

    pub(super) fn did_wake_reported(&mut self) {
        let slept_for = self
            .sleep_reported_at
            .take()
            .map_or(Duration::ZERO, |at| elapsed(at, self.now));
        self.did_wake(slept_for);
        // Whether the screen is locked after the wake decides presence; the answer judges it.
        self.system_asleep = false;
        self.effects.push(Effect::ReadScreenState);
        self.wake_settle = Some(WakeSettle {
            deadline: self.now + policy::WAKE_SETTLE_TIMEOUT,
            late: false,
            step: SettleStep::Waiting(self.now),
        });
        self.wake_look_due();
    }

    /// The network is back when the path is satisfied and the primary network has a router.
    pub(super) fn wake_look_due(&mut self) {
        let Some(settle) = &mut self.wake_settle else {
            return;
        };
        if self.raw_path.as_ref().is_some_and(|path| path.satisfied) {
            settle.step = SettleStep::ReadingRouter;
            self.effects.push(Effect::ReadRouter);
        } else {
            self.wake_network_missing();
        }
    }

    pub(super) fn router_read(&mut self, present: bool) {
        let Some(settle) = &self.wake_settle else {
            return;
        };
        if settle.step != SettleStep::ReadingRouter {
            return;
        }
        if present && self.raw_path.as_ref().is_some_and(|path| path.satisfied) {
            self.wake_network_back();
        } else {
            self.wake_network_missing();
        }
    }

    fn wake_network_missing(&mut self) {
        let now = self.now;
        let Some(settle) = &mut self.wake_settle else {
            return;
        };
        if settle.late {
            settle.step = SettleStep::Waiting(now + policy::LATE_SETTLE_POLL);
        } else if now < settle.deadline {
            settle.step = SettleStep::Waiting(now + policy::WAKE_SETTLE_POLL);
        } else {
            // Judged without a network; what survived does not matter then.
            settle.late = true;
            self.wake_settled(WakeObservation {
                path_satisfied: false,
                underlay: None,
                client_running: false,
                tunnel_up: false,
            });
            self.wake_look_due();
        }
    }

    fn wake_network_back(&mut self) {
        let Some(settle) = &self.wake_settle else {
            return;
        };
        if settle.late {
            self.wake_settle = None;
            self.report_path();
            return;
        }
        let client_running = self.run_active();
        let (id, host) = self.probe(client_running);
        if let Some(settle) = &mut self.wake_settle {
            settle.step = SettleStep::Observing(id, host);
        }
    }

    // MARK: Network path

    pub(super) fn network_path_changed(&mut self, satisfied: bool, interfaces: Vec<PathInterface>) {
        self.raw_path = Some(RawPath {
            satisfied,
            interfaces,
        });
        self.path_settle_at = Some(self.now + policy::PATH_SETTLE);
        self.path_probe = None;
    }

    pub(super) fn path_settled(&mut self) {
        self.path_settle_at = None;
        self.report_path();
    }

    fn report_path(&mut self) {
        if self.raw_path.as_ref().is_some_and(|path| path.satisfied) {
            self.path_probe = Some(self.probe(false));
        } else {
            self.path_changed(PathSnapshot {
                satisfied: false,
                underlay: None,
            });
        }
    }

    /// Asks for the route to the gateway; returns the probe's ID and the host it looks up.
    fn probe(&mut self, client_running: bool) -> (u64, String) {
        self.next_probe += 1;
        let host = self.settings.account.host.clone();
        let gateway = (!host.is_empty()).then(|| GatewayLookup {
            host: host.clone(),
            known_ip: self
                .gateway_ip
                .as_ref()
                .filter(|(resolved, _)| *resolved == host)
                .map(|(_, ip)| *ip),
        });
        self.effects.push(Effect::ProbeNetwork(NetworkProbe {
            id: self.next_probe,
            gateway,
            client_running,
        }));
        (self.next_probe, host)
    }

    pub(super) fn network_probed(&mut self, result: ProbeResult) {
        let for_path = self
            .path_probe
            .as_ref()
            .filter(|(id, _)| *id == result.id)
            .map(|(_, host)| host.clone());
        let for_wake = match &self.wake_settle {
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
            self.gateway_ip = Some((host, ip));
        }
        let underlay = self.underlay(&result);
        if for_path.is_some() {
            self.path_probe = None;
            self.path_changed(PathSnapshot {
                satisfied: true,
                underlay,
            });
        } else {
            self.wake_settle = None;
            let tunnel_up = self.tunnel_interface_up(&result.addresses);
            self.wake_settled(WakeObservation {
                path_satisfied: true,
                underlay,
                client_running: result.client_running.unwrap_or(false),
                tunnel_up,
            });
        }
    }

    /// The route interface, unless it is the tunnel's own or one the path reports as a tunnel or
    /// other virtual interface. The tunnel interface is the one holding the tunnel IP the Helper
    /// reported.
    fn underlay(&self, result: &ProbeResult) -> Option<Underlay> {
        let name = result.route_interface.as_ref()?;
        let tunnel_ip = self.tunnel_ip();
        let carries_tunnel = result.addresses.iter().any(|address| {
            address.name == *name && address.ipv4.is_some() && address.ipv4 == tunnel_ip
        });
        let virtual_interface = self.raw_path.as_ref().is_some_and(|path| {
            path.interfaces
                .iter()
                .any(|interface| interface.name == *name && interface.kind == InterfaceKind::Other)
        });
        if carries_tunnel || virtual_interface {
            return None;
        }
        Some(Underlay {
            interface: name.clone(),
            address: result
                .addresses
                .iter()
                .find(|address| address.name == *name && address.ipv4.is_some())
                .and_then(|address| address.ipv4),
        })
    }

    /// Without a reported tunnel IP the interface cannot be told, so it counts as there: a wrong
    /// "gone" would replace a working tunnel after every wake.
    fn tunnel_interface_up(&self, addresses: &[InterfaceAddress]) -> bool {
        self.tunnel_ip().is_none_or(|ip| {
            addresses
                .iter()
                .any(|address| address.ipv4 == Some(ip) && address.up)
        })
    }

    fn tunnel_ip(&self) -> Option<Ipv4Addr> {
        match &self.phase {
            Phase::Connected(session) => session.tunnel_ip,
            _ => None,
        }
    }
}
