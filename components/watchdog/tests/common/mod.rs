//! A test-only backend that reads output the way openfortivpn 1.24.1 prints it (the fixture
//! scripts copy its strings), and a session that runs a fixture and records what the run reports.

#![allow(dead_code)] // Each test file uses a different part.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use fvm_contract::app_helper::{
    ExitFacts, HelperEventKind, Milestone, OutputLine, OutputSource, RunRequest,
};
use fvm_contract::backend::{
    Backend, ClientExecutable, ClientProblem, Finding, LaunchSpec, OutputInterpreter, Prompt,
    StopPolicy, StopScope,
};
use fvm_contract::helper_internal::RouteRepairOutcome;
use fvm_watchdog::{Report, Run, StageLimits, Timing};

pub const PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
pub const REJECTED_DIGEST: &str =
    "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11";
/// Leading and trailing spaces and non-ASCII must reach the client byte for byte.
pub const PASSWORD: &str = " pässwörd 🔑 ";
pub const OTP: &str = "042137";

pub const FULL_FLOW: [Milestone; 6] = [
    Milestone::Spawned,
    Milestone::ConnectedToGateway,
    Milestone::OtpPrompted,
    Milestone::OtpSubmitted,
    Milestone::Authenticated,
    Milestone::TunnelUp,
];

// The backend

pub struct FakeOpenFortiVpn {
    pub stop_policy: StopPolicy,
}

impl Backend for FakeOpenFortiVpn {
    type Interpreter = FakeInterpreter;

    fn client_name(&self) -> &'static str {
        "openfortivpn"
    }

    fn locate_client(&self) -> Result<ClientExecutable, ClientProblem> {
        unreachable!("the watchdog does not locate the client")
    }

    fn describe_problem(&self, _: &ClientProblem) -> String {
        unreachable!("the watchdog does not describe client problems")
    }

    fn launch_spec(
        &self,
        _: &ClientExecutable,
        _: &RunRequest,
        _: &Path,
    ) -> Result<LaunchSpec, String> {
        unreachable!("the tests build the launch themselves")
    }

    fn check_otp(&self, _: &str) -> Result<(), String> {
        unreachable!("the caller checks the OTP before the watchdog")
    }

    fn interpreter(&self) -> FakeInterpreter {
        FakeInterpreter::default()
    }

    fn stop_policy(&self) -> StopPolicy {
        self.stop_policy
    }

    fn is_own_executable(&self, _: &Path) -> bool {
        unreachable!("the watchdog does not sweep leftovers")
    }
}

#[derive(Default)]
pub struct FakeInterpreter {
    certificates: Vec<String>,
    routes: Routes,
}

#[derive(Default, PartialEq)]
enum Routes {
    #[default]
    Unchanged,
    Changed,
    Restoring,
    Restored,
}

impl OutputInterpreter for FakeInterpreter {
    fn prompt(&mut self, source: OutputSource, partial_line: &[u8]) -> Option<Prompt> {
        if source != OutputSource::Stdout {
            None
        } else if partial_line.ends_with(b"VPN account password: ") {
            Some(Prompt::Password)
        } else if partial_line.ends_with(b"Two-factor authentication token: ") {
            Some(Prompt::Otp)
        } else {
            None
        }
    }

    fn password_expected(&self, milestone: Milestone) -> bool {
        milestone < Milestone::ConnectedToGateway
    }

    fn line(&mut self, source: OutputSource, text: &str) -> Option<Finding> {
        if source != OutputSource::Stdout {
            return None;
        }
        let (level, message) = ["INFO:   ", "WARN:   ", "ERROR:  ", "DEBUG:  "]
            .into_iter()
            .find_map(|prefix| Some((prefix, text.strip_prefix(prefix)?)))?;
        if self.routes == Routes::Restoring && matches!(level, "INFO:   " | "ERROR:  ") {
            self.routes = Routes::Restored;
        }
        match message {
            "Connected to gateway." => Some(Finding::Milestone(Milestone::ConnectedToGateway)),
            "Authenticated." => Some(Finding::Milestone(Milestone::Authenticated)),
            "Tunnel is up and running." => Some(Finding::Milestone(Milestone::TunnelUp)),
            "Setting new routes..." => {
                self.routes = Routes::Changed;
                None
            }
            "Restoring routes..." => {
                if self.routes == Routes::Changed {
                    self.routes = Routes::Restoring;
                }
                None
            }
            _ => {
                if let Some(addresses) = message.strip_prefix("Got addresses: [") {
                    let (ip, _) = addresses.split_once(']')?;
                    return ip.parse::<Ipv4Addr>().ok().map(Finding::TunnelIp);
                }
                if let Some(value) = message.strip_prefix("    trusted-cert = ") {
                    if self.certificates.iter().any(|known| known == value) {
                        return None;
                    }
                    self.certificates.push(value.to_owned());
                    return Some(Finding::CertificateRejected(value.to_owned()));
                }
                (message.starts_with("Could not authenticate to ")
                    && message.contains("tunnel mode"))
                .then_some(Finding::TunnelModeRejected)
            }
        }
    }

    fn routes_left_unrestored(&self) -> bool {
        matches!(self.routes, Routes::Changed | Routes::Restoring)
    }
}

// Setup

pub struct Setup {
    pub timing: Timing,
    pub grace_period: Duration,
    pub password: String,
    /// Submitted as soon as the OTP prompt is reported.
    pub otp: Option<String>,
    pub route_repair: RouteRepairOutcome,
}

impl Default for Setup {
    /// Generous limits, so a broken test fails instead of hanging; tests of a limit shrink it.
    fn default() -> Self {
        let stage = Duration::from_secs(5);
        Self {
            timing: Timing {
                stages: StageLimits {
                    spawn_to_gateway: stage,
                    gateway_to_prompt_or_auth: stage,
                    otp_prompt_to_submit: stage,
                    otp_submit_to_auth: stage,
                    auth_to_tunnel: stage,
                },
                output_batch: Duration::from_millis(20),
                line_idle_flush: Duration::from_millis(250),
                pipe_drain: Duration::from_secs(1),
            },
            grace_period: Duration::from_secs(2),
            password: PASSWORD.to_owned(),
            otp: None,
            route_repair: RouteRepairOutcome::NotNeeded,
        }
    }
}

impl Setup {
    pub fn submitting_otp(self) -> Self {
        Self {
            otp: Some(OTP.to_owned()),
            ..self
        }
    }
}

pub fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.sh"))
}

/// Runs `/bin/sh <fixture> <record file> <args>`.
pub fn fixture(name: &str, args: &[&str], setup: Setup) -> Session {
    let record = RecordFile::new();
    let mut shell_args = vec![
        fixture_path(name).display().to_string(),
        record.path.display().to_string(),
    ];
    shell_args.extend(args.iter().map(|arg| (*arg).to_owned()));
    Session::start(shell("/bin/sh", shell_args), record, setup)
}

/// Runs `/bin/sh -c <script>` with the record file as `$1`.
pub fn script(script: &str, setup: Setup) -> Session {
    let record = RecordFile::new();
    let args = vec![
        "-c".to_owned(),
        script.to_owned(),
        "sh".to_owned(),
        record.path.display().to_string(),
    ];
    Session::start(shell("/bin/sh", args), record, setup)
}

pub fn shell(program: &str, args: Vec<String>) -> LaunchSpec {
    LaunchSpec {
        program: PathBuf::from(program),
        args,
        env: vec![("PATH".to_owned(), PATH.to_owned())],
        config_file: String::new(),
    }
}

pub fn sha256(text: &str) -> String {
    let output = Command::new("/bin/sh")
        .args(["-c", "printf '%s' \"$1\" | /sbin/sha256 -q", "sh", text])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

// The session

/// A report as recorded; the route repair was answered as it arrived.
#[derive(Debug)]
pub enum Delivered {
    Output(Vec<OutputLine>),
    Event(HelperEventKind),
    RepairRoutes,
}

pub struct Session {
    pub run: Run,
    reports: Receiver<Report>,
    pub delivered: Vec<Delivered>,
    pub record: RecordFile,
    otp: Option<String>,
    route_repair: RouteRepairOutcome,
}

impl Session {
    pub fn start(launch: LaunchSpec, record: RecordFile, setup: Setup) -> Self {
        let backend = FakeOpenFortiVpn {
            stop_policy: StopPolicy {
                graceful: StopScope::Process,
                grace_period: setup.grace_period,
                forced: StopScope::ProcessThenGroup,
            },
        };
        let (sender, reports) = mpsc::channel();
        let run = Run::start(
            &backend,
            &launch,
            setup.password,
            setup.timing,
            move |report| {
                // A test that already ended no longer listens.
                let _ = sender.send(report);
            },
        );
        Self {
            run,
            reports,
            delivered: Vec::new(),
            record,
            otp: setup.otp,
            route_repair: setup.route_repair,
        }
    }

    /// Records reports until `condition` holds; false if it does not within `timeout`. Checks
    /// often, since a condition on the record file holds without a report.
    pub fn wait(&mut self, timeout: Duration, condition: impl Fn(&Self) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while !condition(self) {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let poll = (deadline - now).min(Duration::from_millis(5));
            match self.reports.recv_timeout(poll) {
                Ok(report) => self.record_report(report),
                Err(RecvTimeoutError::Timeout) => {}
                // The run ended and nothing more will arrive.
                Err(RecvTimeoutError::Disconnected) => return condition(self),
            }
        }
        true
    }

    pub fn wait_for(&mut self, milestone: Milestone) {
        assert!(
            self.wait(Duration::from_secs(5), |session| session.reached(milestone)),
            "{milestone:?} not reached: {:?}",
            self.texts()
        );
    }

    pub fn wait_for_exit(&mut self) -> ExitFacts {
        self.wait_for_exit_within(Duration::from_secs(5))
    }

    pub fn wait_for_exit_within(&mut self, timeout: Duration) -> ExitFacts {
        assert!(
            self.wait(timeout, |session| session.exit_facts().is_some()),
            "no exit within {timeout:?}: {:?}",
            self.texts()
        );
        self.exit_facts().unwrap()
    }

    /// Waits for whatever else the run sends within `duration`.
    pub fn settle(&mut self, duration: Duration) {
        self.wait(duration, |_| false);
    }

    fn record_report(&mut self, report: Report) {
        match report {
            Report::Output(lines) => self.delivered.push(Delivered::Output(lines)),
            Report::Event(kind) => {
                let prompted = matches!(
                    kind,
                    HelperEventKind::Milestone {
                        milestone: Milestone::OtpPrompted,
                        ..
                    }
                );
                self.delivered.push(Delivered::Event(kind));
                if prompted && let Some(code) = self.otp.take() {
                    self.run.submit_otp(code).unwrap();
                }
            }
            Report::RepairRoutes(repair) => {
                self.delivered.push(Delivered::RepairRoutes);
                repair.finish(self.route_repair);
            }
        }
    }

    pub fn events(&self) -> Vec<&HelperEventKind> {
        self.delivered
            .iter()
            .filter_map(|delivered| match delivered {
                Delivered::Event(kind) => Some(kind),
                _ => None,
            })
            .collect()
    }

    pub fn batches(&self) -> Vec<&Vec<OutputLine>> {
        self.delivered
            .iter()
            .filter_map(|delivered| match delivered {
                Delivered::Output(lines) => Some(lines),
                _ => None,
            })
            .collect()
    }

    pub fn lines(&self) -> Vec<&OutputLine> {
        self.batches().into_iter().flatten().collect()
    }

    pub fn texts(&self) -> Vec<&str> {
        self.lines().iter().map(|line| line.text.as_str()).collect()
    }

    pub fn has_text(&self, text: &str) -> bool {
        self.texts().contains(&text)
    }

    pub fn count_text(&self, text: &str) -> usize {
        self.texts().iter().filter(|&&shown| shown == text).count()
    }

    pub fn has_line(&self, source: OutputSource, text: &str) -> bool {
        self.lines()
            .iter()
            .any(|line| line.source == source && line.text == text)
    }

    pub fn milestones(&self) -> Vec<Milestone> {
        self.events()
            .into_iter()
            .filter_map(|kind| match kind {
                HelperEventKind::Milestone { milestone, .. } => Some(*milestone),
                HelperEventKind::Exited { .. } => None,
            })
            .collect()
    }

    pub fn reached(&self, milestone: Milestone) -> bool {
        self.milestones().contains(&milestone)
    }

    pub fn exit_facts(&self) -> Option<ExitFacts> {
        self.events().into_iter().find_map(|kind| match kind {
            HelperEventKind::Exited { facts } => Some(facts.clone()),
            HelperEventKind::Milestone { .. } => None,
        })
    }

    pub fn repair_requests(&self) -> usize {
        self.delivered
            .iter()
            .filter(|delivered| matches!(delivered, Delivered::RepairRoutes))
            .count()
    }

    /// Texts delivered before the first event that `matches`.
    pub fn texts_before(&self, matches: impl Fn(&HelperEventKind) -> bool) -> Vec<&str> {
        let mut texts = Vec::new();
        for delivered in &self.delivered {
            match delivered {
                Delivered::Output(lines) => {
                    texts.extend(lines.iter().map(|line| line.text.as_str()))
                }
                Delivered::Event(kind) if matches(kind) => return texts,
                _ => {}
            }
        }
        texts
    }

    /// The rules every finished run follows: batches are never empty, the exit is reported once and
    /// last, after all output, which ends with the exit or route repair line, and no secret is
    /// shown.
    pub fn assert_finished_cleanly(&self) {
        assert!(self.batches().iter().all(|batch| !batch.is_empty()));
        assert!(
            matches!(
                self.delivered.last(),
                Some(Delivered::Event(HelperEventKind::Exited { .. }))
            ),
            "the last report is not the exit: {:?}",
            self.delivered.last()
        );
        let exits = self
            .events()
            .iter()
            .filter(|kind| matches!(kind, HelperEventKind::Exited { .. }))
            .count();
        assert_eq!(exits, 1);
        let last = self.lines().last().copied().unwrap();
        assert_eq!(last.source, OutputSource::Helper);
        assert!(
            last.text.starts_with("‹exited: ") || last.text.starts_with("‹route"),
            "{}",
            last.text
        );
        assert!(
            !self
                .texts()
                .iter()
                .any(|text| text.contains(PASSWORD) || text.contains(OTP))
        );
    }
}

// The record file

static RECORD_NUMBER: AtomicU64 = AtomicU64::new(0);

/// A temporary file the fixtures write `key=value` lines to.
pub struct RecordFile {
    pub path: PathBuf,
}

impl RecordFile {
    pub fn new() -> Self {
        let number = RECORD_NUMBER.fetch_add(1, Ordering::Relaxed);
        let name = format!("fvm-watchdog-{}-{number}.txt", std::process::id());
        Self {
            path: std::env::temp_dir().join(name),
        }
    }

    pub fn values(&self) -> HashMap<String, String> {
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        text.lines()
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    pub fn value(&self, key: &str) -> Option<String> {
        self.values().remove(key)
    }
}

impl Drop for RecordFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
