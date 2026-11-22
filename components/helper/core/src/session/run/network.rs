use std::collections::VecDeque;
use std::mem;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use fvm_contract::app_helper::{EndFacts, Milestone, StopReason};
use fvm_contract::helper_internal::{
    IpNetwork, LogCategory, LogLevel, NetworkChange, NetworkOutcome, NetworkSettings,
    OutsideChanges, OutsideRoute,
};

use super::{Input, Phase, Watch, tunnel_ipv4};
use crate::session::{NetworkJob, Report};

const RETRIES: u32 = 3;
const APPLIED: &str = "‹network settings applied›";

#[derive(Default)]
pub(super) struct Network {
    watching: bool,
    applied: Option<NetworkSettings>,
    wanted: Option<NetworkSettings>,
    next: Option<NetworkSettings>,
    asked: VecDeque<Asked>,
    failures: u32,
    retry_at: Option<Instant>,
    owned: Vec<OutsideRoute>,
    prepared: Option<NetworkSettings>,
    dns_service_id: String,
    switch: Switch,
    pub(super) hand_over_to: Option<Sender<Input>>,
    handing_over: bool,
    handed_over: bool,
}

#[derive(Debug)]
enum Asked {
    LookUp,
    Apply { remove: Vec<OutsideRoute> },
    Prepare,
    Switch,
    Revert,
}

pub struct Replaced {
    inputs: Sender<Input>,
    dns_service_id: String,
    address: IpAddr,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotReplaceable {
    NotUp,
}

pub(crate) struct HandOver {
    settings: Box<NetworkSettings>,
    routes: Vec<OutsideRoute>,
    dns_service_id: String,
}

pub(crate) enum SwitchEnd {
    Released {
        routes: Vec<OutsideRoute>,
        dns: bool,
    },
    Kept,
    Failed,
}

#[derive(Default)]
enum Switch {
    #[default]
    Idle,
    Asked,
    Declined,
    Switching {
        replaced: Box<NetworkSettings>,
        routes: Vec<OutsideRoute>,
        dns: bool,
    },
}

impl Network {
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.retry_at
    }

    pub(super) fn watching(&self) -> bool {
        self.watching
    }

    pub(super) fn stop_watching(&mut self) {
        self.watching = false;
        self.retry_at = None;
        self.next = None;
    }

    fn busy(&self) -> bool {
        self.handing_over
            || self.retry_at.is_some()
            || self
                .asked
                .iter()
                .any(|asked| !matches!(asked, Asked::Revert))
    }
}

impl<R: FnMut(Report)> Watch<R> {
    fn changing_network(&self) -> bool {
        self.running() && self.stop.is_none() && !self.network.handed_over
    }

    pub(super) fn tunnel_ready(&mut self, settings: Result<Box<NetworkSettings>, String>) {
        if !self.changing_network() {
            return;
        }
        match settings {
            Ok(settings) => {
                self.tunnel_ip = Some(tunnel_ipv4(settings.ipv4.address.address));
                self.network.watching = true;
                self.settings_received(*settings);
            }
            Err(reason) => {
                self.network_setup_failed(format!("‹invalid network settings: {reason}›"));
            }
        }
    }

    pub(super) fn reconnected(&mut self) {
        if !self.changing_network() {
            return;
        }
        if let Some(applied) = self.network.applied.clone() {
            self.settings_received(applied);
        }
    }

    pub(super) fn tunnel_failed(&mut self, address: Option<Ipv4Addr>, reason: Option<String>) {
        let in_use = self.replaced.as_ref().is_some_and(|replaced| {
            address.is_some_and(|address| replaced.address == IpAddr::V4(address))
        });
        if in_use {
            self.tunnel_address_in_use = true;
            self.meta("‹the connection to replace has the same tunnel address›");
            return;
        }
        self.tunnel_setup_failed = true;
        if let Some(reason) = reason {
            self.meta(format!("‹the tunnel device could not be set up: {reason}›"));
        } else {
            self.meta("‹the tunnel device could not be set up›");
        }
    }

    pub(super) fn network_deadlines_passed(&mut self, now: Instant) {
        if self.network.retry_at.is_some_and(|at| at <= now) {
            self.network.retry_at = None;
            self.retry_network();
        }
    }

    fn settings_received(&mut self, settings: NetworkSettings) {
        if !self.changing_network() {
            return;
        }
        if self.network.busy() {
            self.network.next = Some(settings);
        } else {
            self.look_up(settings);
        }
    }

    fn outside(settings: &NetworkSettings) -> Vec<IpNetwork> {
        let gateway = IpNetwork {
            address: settings.gateway,
            prefix_length: match settings.gateway {
                IpAddr::V4(_) => 32,
                IpAddr::V6(_) => 128,
            },
        };
        (!settings.gateway.is_loopback())
            .then_some(gateway)
            .into_iter()
            .chain(
                settings
                    .split_exclude
                    .iter()
                    .copied()
                    .filter(|network| cfg!(not(windows)) || network.address.is_ipv4()),
            )
            .collect()
    }

    fn look_up(&mut self, settings: NetworkSettings) {
        let change = NetworkChange::LookUp {
            tunnel: settings.tunnel.clone(),
            outside: Self::outside(&settings),
        };
        self.network.wanted = Some(settings);
        self.ask(change, Asked::LookUp);
    }

    fn ask(&mut self, change: NetworkChange, asked: Asked) {
        self.network.asked.push_back(asked);
        let inputs = self.inputs.clone();
        let job = NetworkJob::new(change, Arc::clone(&self.returned), move |outcome| {
            let _ = inputs.send(Input::NetworkChanged(outcome));
        });
        (self.on_report)(Report::ChangeNetwork(job));
    }

    pub(super) fn network_changed(&mut self, outcome: NetworkOutcome) {
        let asked = self
            .network
            .asked
            .pop_front()
            .expect("the OS side answers only the network changes asked for");
        match (asked, outcome) {
            (
                Asked::LookUp,
                NetworkOutcome::LookedUp {
                    dns_service_id,
                    missing,
                },
            ) => self.looked_up(dns_service_id, missing),
            (Asked::LookUp | Asked::Switch, NetworkOutcome::Failed { step, error, .. }) => {
                self.network_failed(&format!("{step:?}: {error}"));
            }
            (Asked::Apply { remove }, NetworkOutcome::Applied { added, left }) => {
                self.own(&remove, &left, &added);
                self.applied();
            }
            (
                Asked::Apply { remove },
                NetworkOutcome::Failed {
                    step,
                    error,
                    added,
                    left,
                },
            ) => {
                self.own(&remove, &left, &added);
                self.network_failed(&format!("{step:?}: {error}"));
            }
            (Asked::Prepare, NetworkOutcome::Applied { added, left }) => {
                self.own(&[], &left, &added);
                self.prepared();
            }
            (
                Asked::Prepare,
                NetworkOutcome::Failed {
                    step,
                    error,
                    added,
                    left,
                },
            ) => {
                self.own(&[], &left, &added);
                self.network_failed(&format!("{step:?}: {error}"));
            }
            (Asked::Switch, NetworkOutcome::Applied { .. }) => self.switched(),
            (Asked::Revert, NetworkOutcome::Reverted { left }) => self.reverted(&left),
            (
                Asked::Revert,
                NetworkOutcome::Failed {
                    step, error, left, ..
                },
            ) => {
                self.meta(format!(
                    "‹network settings not fully reverted ({step:?}): {error}›"
                ));
                self.reverted(&left);
            }
            (asked, outcome) => {
                panic!("the OS side answered {asked:?} with {outcome:?}")
            }
        }
    }

    fn looked_up(&mut self, dns_service_id: String, missing: Vec<OutsideRoute>) {
        if !self.changing_network() {
            return;
        }
        let settings = self
            .network
            .wanted
            .clone()
            .expect("a look-up is for the settings wanted");
        if self
            .replaced
            .as_ref()
            .is_some_and(|replaced| replaced.address == settings.ipv4.address.address)
        {
            self.network.wanted = None;
            self.address_in_use();
            return;
        }
        let outside = Self::outside(&settings);
        let remove: Vec<OutsideRoute> = self
            .network
            .owned
            .iter()
            .filter(|owned| {
                !outside.contains(&owned.destination)
                    || missing
                        .iter()
                        .any(|route| route.destination == owned.destination)
            })
            .cloned()
            .collect();
        let mut routes = self.network.owned.clone();
        routes.extend(
            missing
                .iter()
                .filter(|route| !self.network.owned.contains(route))
                .cloned(),
        );
        let recorded_dns = match &self.replaced {
            Some(replaced) if replaced.dns_service_id == dns_service_id => String::new(),
            _ => dns_service_id.clone(),
        };
        self.record.outside_changes = Some(OutsideChanges {
            dns_service_id: recorded_dns,
            routes,
        });
        if let Err(failure) = self.files.write_record(&self.record) {
            let message = format!("cannot write the run record: {failure}");
            self.log(LogLevel::Error, LogCategory::Helper, message.clone());
            self.network_failed(&message);
            return;
        }
        if self.replaced.is_some() {
            self.network.dns_service_id = dns_service_id;
            let change = NetworkChange::Prepare {
                settings: Box::new(settings),
                add: missing,
            };
            self.ask(change, Asked::Prepare);
            return;
        }
        let change = NetworkChange::Apply {
            settings: Box::new(settings),
            previous: self.network.applied.clone().map(Box::new),
            remove: remove.clone(),
            add: missing,
            outside,
        };
        self.ask(change, Asked::Apply { remove });
    }

    fn own(&mut self, removed: &[OutsideRoute], left: &[OutsideRoute], added: &[OutsideRoute]) {
        let owned = &mut self.network.owned;
        owned.retain(|route| !removed.contains(route) || left.contains(route));
        for route in added {
            if !owned.contains(route) {
                owned.push(route.clone());
            }
        }
        if let Some(changes) = &mut self.record.outside_changes {
            changes.routes.clone_from(owned);
            self.save_record();
        }
    }

    fn applied(&mut self) {
        let settings = self
            .network
            .wanted
            .take()
            .expect("an apply is for the settings wanted");
        self.network.applied = Some(settings);
        self.network.prepared = None;
        self.network.failures = 0;
        let resuming = self.furthest < Milestone::TunnelUp;
        if !resuming {
            self.meta(APPLIED);
        }
        if let Some(next) = self.network.next.take().filter(|_| self.changing_network()) {
            self.look_up(next);
        }
        if resuming && self.changing_network() {
            self.network_ready();
        }
    }

    fn network_failed(&mut self, reason: &str) {
        self.meta(format!("‹network settings not applied: {reason}›"));
        if !self.changing_network() {
            return;
        }
        self.network.failures += 1;
        if self.network.failures > RETRIES {
            self.network_setup_failed(format!(
                "‹network settings still not applied after {} tries›",
                RETRIES + 1
            ));
        } else {
            self.network.retry_at = Some(Instant::now() + self.timing.network_retry);
        }
    }

    fn retry_network(&mut self) {
        if !self.changing_network() {
            return;
        }
        if matches!(self.network.switch, Switch::Switching { .. }) {
            self.switch_traffic();
            return;
        }
        let settings = match self.network.next.take() {
            Some(next) => next,
            None => self
                .network
                .wanted
                .take()
                .expect("a retry is for the settings wanted"),
        };
        self.look_up(settings);
    }

    pub(super) fn network_setup_failed(&mut self, text: String) {
        self.meta(text);
        self.network.stop_watching();
        self.stop(StopReason::NetworkSetupFailed);
    }

    pub(super) fn revert_network_or_continue(&mut self, facts: EndFacts) {
        if self.hand_over_open() {
            self.flush_batch();
            self.phase = Phase::AwaitingSwitch(facts);
            return;
        }
        let Some(changes) = self.record.outside_changes.clone() else {
            self.report_exit(facts);
            return;
        };
        self.flush_batch();
        self.phase = Phase::RevertingNetwork(facts);
        self.ask(NetworkChange::Revert { changes }, Asked::Revert);
    }

    fn prepared(&mut self) {
        self.network.prepared = self.network.wanted.take();
        self.network.failures = 0;
        if let Some(next) = self.network.next.take().filter(|_| self.changing_network()) {
            self.look_up(next);
        } else if self.changing_network() {
            self.network_ready();
        }
    }

    fn network_ready(&mut self) {
        let Some(prepared) = self.network.prepared.clone() else {
            self.meta(APPLIED);
            self.advance(Milestone::TunnelUp);
            return;
        };
        let Some(replaced) = &self.replaced else {
            self.look_up(prepared);
            return;
        };
        let asked = replaced
            .inputs
            .send(Input::HandOverAsked(self.inputs.clone()))
            .is_ok();
        self.network.switch = if asked {
            Switch::Asked
        } else {
            Switch::Declined
        };
    }

    pub(super) fn replaceable(&self) -> Result<Replaced, NotReplaceable> {
        if !self.changing_network() || self.furthest != Milestone::TunnelUp {
            return Err(NotReplaceable::NotUp);
        }
        let changes = self
            .record
            .outside_changes
            .as_ref()
            .expect("a tunnel up with the Helper's network settings recorded them");
        let applied = self
            .network
            .applied
            .as_ref()
            .expect("a tunnel is up once its network settings are applied");
        Ok(Replaced {
            inputs: self.inputs.clone(),
            dns_service_id: changes.dns_service_id.clone(),
            address: applied.ipv4.address.address,
        })
    }

    pub(super) fn routing_check(&self) -> Option<NetworkChange> {
        if !self.changing_network() || self.furthest != Milestone::TunnelUp || self.network.busy() {
            return None;
        }
        let settings = self
            .network
            .applied
            .clone()
            .expect("a tunnel is up once its network settings are applied");
        Some(NetworkChange::Check {
            outside: Self::outside(&settings),
            settings: Box::new(settings),
        })
    }

    pub(super) fn replaced_exited(&mut self) {
        self.replaced = None;
        if !matches!(self.network.switch, Switch::Asked | Switch::Declined) {
            return;
        }
        self.network.switch = Switch::Idle;
        if self.changing_network() {
            let prepared = self
                .network
                .prepared
                .clone()
                .expect("a hand-over is asked for once the network is prepared");
            self.look_up(prepared);
        }
    }

    pub(super) fn handed_over(&mut self, hand_over: Option<HandOver>) {
        let Some(hand_over) = hand_over else {
            self.network.switch = Switch::Declined;
            return;
        };
        self.network.switch = Switch::Idle;
        let replaced = self
            .replaced
            .as_ref()
            .expect("only a run that replaces another asks it to hand over");
        let settings = self
            .network
            .prepared
            .clone()
            .expect("a hand-over is asked for once the network is prepared");
        if !self.changing_network() {
            let _ = replaced.inputs.send(Input::SwitchEnded(SwitchEnd::Kept));
            return;
        }
        if hand_over.settings.ipv4.address.address == settings.ipv4.address.address {
            let _ = replaced.inputs.send(Input::SwitchEnded(SwitchEnd::Kept));
            self.address_in_use();
            return;
        }
        let outside = Self::outside(&settings);
        let routes = hand_over
            .routes
            .into_iter()
            .filter(|route| outside.contains(&route.destination))
            .collect();
        self.network.switch = Switch::Switching {
            replaced: hand_over.settings,
            routes,
            dns: hand_over.dns_service_id == self.network.dns_service_id,
        };
        self.switch_traffic();
    }

    fn switch_traffic(&mut self) {
        let Switch::Switching {
            replaced, routes, ..
        } = &self.network.switch
        else {
            unreachable!("the traffic moves only during a hand-over")
        };
        let settings = self
            .network
            .prepared
            .clone()
            .expect("a hand-over is asked for once the network is prepared");
        let change = NetworkChange::Switch {
            outside: Self::outside(&settings),
            settings: Box::new(settings),
            replaced: replaced.clone(),
        };
        for route in routes {
            if !self.network.owned.contains(route) {
                self.network.owned.push(route.clone());
            }
        }
        self.record.outside_changes = Some(OutsideChanges {
            dns_service_id: self.network.dns_service_id.clone(),
            routes: self.network.owned.clone(),
        });
        if let Err(failure) = self.files.write_record(&self.record) {
            let message = format!("cannot write the run record: {failure}");
            self.log(LogLevel::Error, LogCategory::Helper, message.clone());
            self.network_failed(&message);
            return;
        }
        self.ask(change, Asked::Switch);
    }

    fn switched(&mut self) {
        if !self.changing_network() {
            return;
        }
        let Switch::Switching { routes, dns, .. } =
            mem::replace(&mut self.network.switch, Switch::Idle)
        else {
            unreachable!("a switch is answered only during a hand-over")
        };
        let replaced = self
            .replaced
            .take()
            .expect("only a run that replaces another moves its traffic");
        let _ = replaced
            .inputs
            .send(Input::SwitchEnded(SwitchEnd::Released { routes, dns }));
        self.network.applied = self.network.prepared.take();
        self.network.failures = 0;
        self.meta(APPLIED);
        self.advance(Milestone::TunnelUp);
    }

    fn address_in_use(&mut self) {
        self.tunnel_address_in_use = true;
        self.network_setup_failed(
            "‹the connection to replace has the same tunnel address›".to_owned(),
        );
    }

    pub(super) fn switch_ended(&mut self, end: SwitchEnd) {
        self.network.handing_over = false;
        match end {
            SwitchEnd::Released { routes, dns } => {
                self.network.handed_over = true;
                self.network.next = None;
                self.network.owned.retain(|route| !routes.contains(route));
                if let Some(changes) = &mut self.record.outside_changes {
                    changes.routes.clone_from(&self.network.owned);
                    if dns {
                        changes.dns_service_id = String::new();
                    }
                }
                self.save_record();
                self.meta("‹network settings handed over to the new connection›");
            }
            SwitchEnd::Kept => {
                if let Some(next) = self.network.next.take().filter(|_| self.changing_network()) {
                    self.look_up(next);
                }
            }
            SwitchEnd::Failed if self.changing_network() => self.network_setup_failed(
                "‹the new connection could not take over the network settings›".to_owned(),
            ),
            SwitchEnd::Failed => {}
        }
    }

    fn hand_over_open(&self) -> bool {
        self.network.handing_over || matches!(self.network.switch, Switch::Asked)
    }

    pub(super) fn settle_hand_over(&mut self) {
        if self.network.hand_over_to.is_some() && (!self.changing_network() || !self.network.busy())
        {
            let to = self.network.hand_over_to.take().expect("checked above");
            let hand_over = self.changing_network().then(|| {
                self.network.handing_over = true;
                HandOver {
                    settings: Box::new(
                        self.network
                            .applied
                            .clone()
                            .expect("a tunnel is up once its network settings are applied"),
                    ),
                    routes: self.network.owned.clone(),
                    dns_service_id: self
                        .record
                        .outside_changes
                        .as_ref()
                        .expect("a tunnel up with the Helper's network settings recorded them")
                        .dns_service_id
                        .clone(),
                }
            });
            to.send(Input::HandedOver(hand_over))
                .expect("the replacing run waits for the hand-over it asked for");
        }
        let switch_in_flight = self
            .network
            .asked
            .iter()
            .any(|asked| matches!(asked, Asked::Switch));
        if matches!(self.network.switch, Switch::Switching { .. })
            && !self.changing_network()
            && !switch_in_flight
        {
            self.network.switch = Switch::Idle;
            if let Some(replaced) = self.replaced.take() {
                let _ = replaced.inputs.send(Input::SwitchEnded(SwitchEnd::Failed));
            }
        }
        if matches!(self.phase, Phase::AwaitingSwitch(_)) && !self.hand_over_open() {
            let Phase::AwaitingSwitch(facts) = mem::replace(&mut self.phase, Phase::Reported)
            else {
                unreachable!("checked above")
            };
            self.revert_network_or_continue(facts);
        }
    }

    fn reverted(&mut self, left: &[OutsideRoute]) {
        let Phase::RevertingNetwork(facts) = mem::replace(&mut self.phase, Phase::Reported) else {
            unreachable!("the network is reverted only at the run's end")
        };
        if !left.is_empty() {
            self.log(
                LogLevel::Error,
                LogCategory::Routes,
                format!(
                    "{} route(s) outside the tunnel are still in place after reverting",
                    left.len()
                ),
            );
        }
        self.meta("‹network settings reverted›");
        self.report_exit(facts);
    }
}
