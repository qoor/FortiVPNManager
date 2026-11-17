use std::collections::VecDeque;
use std::fs;
use std::io;
use std::mem;
use std::net::IpAddr;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Instant;

use fvm_contract::app_helper::{ExitFacts, Milestone, StopReason};
use fvm_contract::backend::{IpNetwork, NetworkSettings};
use fvm_contract::helper_internal::{
    LogCategory, LogLevel, NetworkChange, NetworkOutcome, OutsideChanges, OutsideRoute,
};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use super::{Input, Phase, Watch, seconds};
use crate::process::{NetworkJob, Report, os};
use crate::recovery::run_directory::{self, RunFiles};

const RETRIES: u32 = 3;
const APPLIED: &str = "‹network settings applied›";

#[derive(Default)]
pub(super) struct Network {
    values_watch: Option<RecommendedWatcher>,
    watching: bool,
    values_deadline: Option<Instant>,
    applied: Option<NetworkSettings>,
    wanted: Option<NetworkSettings>,
    next: Option<NetworkSettings>,
    asked: VecDeque<Asked>,
    failures: u32,
    retry_at: Option<Instant>,
    owned: Vec<OutsideRoute>,
    script_wait: bool,
    script_left: bool,
}

#[derive(Debug)]
enum Asked {
    LookUp,
    Apply { remove: Vec<OutsideRoute> },
    Revert,
}

impl Network {
    pub(super) fn new(values_watch: Option<RecommendedWatcher>) -> Self {
        Self {
            values_watch,
            ..Self::default()
        }
    }

    pub(super) fn deadline(&self) -> Option<Instant> {
        [self.values_deadline, self.retry_at]
            .into_iter()
            .flatten()
            .min()
    }

    pub(super) fn watching(&self) -> bool {
        self.watching
    }

    pub(super) fn stop_watching(&mut self) {
        self.values_watch = None;
        self.watching = false;
        self.values_deadline = None;
        self.retry_at = None;
        self.script_wait = false;
        self.next = None;
    }

    fn busy(&self) -> bool {
        self.retry_at.is_some()
            || self
                .asked
                .iter()
                .any(|asked| !matches!(asked, Asked::Revert))
    }
}

pub(super) fn values_watcher(files: &RunFiles, inputs: &Sender<Input>) -> RecommendedWatcher {
    let name = files
        .values
        .file_name()
        .expect("the value file has a name")
        .to_owned();
    let inputs = inputs.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        if event.is_ok_and(|event| {
            event.need_rescan()
                || event
                    .paths
                    .iter()
                    .any(|path| path.file_name() == Some(&name))
        }) {
            let _ = inputs.send(Input::ValuesWritten);
        }
    })
    .expect("creating the value folder's watcher");
    watcher
        .watch(&files.values_directory, RecursiveMode::NonRecursive)
        .expect("watching the value folder");
    watcher
}

impl<R: FnMut(Report)> Watch<R> {
    fn changing_network(&self) -> bool {
        self.script_runner.is_some() && self.running() && self.stop.is_none()
    }

    pub(super) fn watch_values(&mut self) {
        self.network.values_watch = Some(values_watcher(&self.files, &self.inputs));
    }

    pub(super) fn tunnel_configured(&mut self) {
        if !self.changing_network() || self.network.watching {
            return;
        }
        self.network.watching = true;
        self.network.values_deadline = Some(Instant::now() + self.timing.network_values);
        self.read_values();
    }

    pub(super) fn values_written(&mut self) {
        if self.network.watching && self.changing_network() {
            self.read_values();
        }
    }

    pub(super) fn gateway_route_requested(&mut self) {
        if !self.changing_network() {
            return;
        }
        if let Some(applied) = self.network.applied.clone() {
            self.settings_received(applied);
        }
    }

    pub(super) fn network_deadlines_passed(&mut self, now: Instant) {
        if self.network.values_deadline.is_some_and(|at| at <= now) {
            self.values_expired();
        }
        if self.network.retry_at.is_some_and(|at| at <= now) {
            self.network.retry_at = None;
            self.retry_network();
        }
    }

    fn values_expired(&mut self) {
        if self.changing_network() {
            self.read_values();
        }
        if self.network.values_deadline.take().is_some() && self.changing_network() {
            let text = format!(
                "‹no network settings from {} within {}›",
                self.backend.client_name(),
                seconds(self.timing.network_values)
            );
            self.network_setup_failed(text);
        }
    }

    fn read_values(&mut self) {
        let path = self.files.values.clone();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Err(error) => {
                self.network_setup_failed(format!("‹cannot read the network settings: {error}›"));
                return;
            }
        };
        if let Err(line) = run_directory::remove(&path) {
            self.log(LogLevel::Error, LogCategory::Helper, line);
        }
        self.network.values_deadline = None;
        match self.backend.network_settings(&bytes) {
            Ok(settings) => self.settings_received(settings),
            Err(reason) => {
                self.network_setup_failed(format!("‹invalid network settings: {reason}›"));
            }
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
        let job = NetworkJob::new(change, move |outcome| {
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
            (Asked::LookUp, NetworkOutcome::Failed { step, error, .. }) => {
                self.network_failed(format!("{step:?}: {error}"));
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
                self.network_failed(format!("{step:?}: {error}"));
            }
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
        self.record.outside_changes = Some(OutsideChanges {
            dns_service_id,
            routes,
        });
        if let Err(failure) = self.files.write_record(&self.record) {
            let message = format!("cannot write the pidfile: {failure}");
            self.log(LogLevel::Error, LogCategory::Helper, message.clone());
            self.network_failed(message);
            return;
        }
        let change = NetworkChange::Apply {
            settings: Box::new(settings),
            previous: self.network.applied.clone().map(Box::new),
            remove: remove.clone(),
            add: missing,
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
            changes.routes = owned.clone();
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
        self.network.failures = 0;
        let resuming = self.furthest < Milestone::TunnelUp;
        if !resuming {
            self.meta(APPLIED);
        }
        if let Some(next) = self.network.next.take().filter(|_| self.changing_network()) {
            self.look_up(next);
        }
        if resuming && self.changing_network() && !self.network.script_wait {
            self.wait_for_script();
        }
    }

    fn network_failed(&mut self, reason: String) {
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

    fn wait_for_script(&mut self) {
        let Phase::Running(child) = &self.phase else {
            unreachable!("the network changes only while the client runs")
        };
        let runner = self
            .script_runner
            .as_deref()
            .expect("the network changes only for a client with a script");
        let scripts = if self.network.script_left {
            Vec::new()
        } else {
            os::running_scripts(child.id(), runner)
        };
        if scripts.is_empty() {
            self.meta(APPLIED);
            self.advance(Milestone::TunnelUp);
            return;
        }
        self.network.script_wait = true;
        let inputs = self.inputs.clone();
        thread::Builder::new()
            .name("fvm-process-script".to_owned())
            .spawn(move || {
                for script in &scripts {
                    script.wait(None);
                }
                let _ = inputs.send(Input::ScriptExited);
            })
            .expect("starting the process script thread");
    }

    pub(super) fn script_exited(&mut self) {
        if mem::take(&mut self.network.script_wait) && self.changing_network() {
            self.meta(APPLIED);
            self.advance(Milestone::TunnelUp);
        }
    }

    pub(super) fn script_left_running(&mut self) {
        self.network.script_left = true;
        self.script_exited();
    }

    pub(super) fn revert_network_or_continue(&mut self, facts: ExitFacts) {
        let Some(changes) = self.record.outside_changes.clone() else {
            self.repair_routes_or_report(facts);
            return;
        };
        self.flush_batch();
        self.phase = Phase::RevertingNetwork(facts);
        self.ask(NetworkChange::Revert { changes }, Asked::Revert);
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
        self.repair_routes_or_report(facts);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Duration;
    use std::{env, process};

    use super::*;

    #[test]
    fn a_value_file_renamed_into_the_value_folder_is_reported() {
        let directory = env::temp_dir().join(format!("fvm-values-watch-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        let mut latencies = Vec::new();
        for run in 1..=20 {
            let files = RunFiles::new(&directory, run);
            fs::create_dir_all(&files.values_directory).unwrap();
            let (inputs, received) = mpsc::channel();
            let watcher = values_watcher(&files, &inputs);
            let draft = files.values_directory.join(format!("values.{run}.tmp"));
            fs::write(&draft, b"reason=connect\0").unwrap();
            if run == 1 {
                assert!(matches!(
                    received.recv_timeout(Duration::from_millis(300)),
                    Err(RecvTimeoutError::Timeout)
                ));
            }
            let renamed = Instant::now();
            fs::rename(&draft, &files.values).unwrap();
            assert!(matches!(
                received.recv_timeout(Duration::from_secs(5)),
                Ok(Input::ValuesWritten)
            ));
            latencies.push(renamed.elapsed());
            drop(watcher);
        }
        fs::remove_dir_all(&directory).unwrap();
        latencies.sort();
        println!(
            "value file seen after a rename, over {} runs: min {:?}, median {:?}, max {:?}",
            latencies.len(),
            latencies[0],
            latencies[latencies.len() / 2],
            latencies[latencies.len() - 1]
        );
    }
}
