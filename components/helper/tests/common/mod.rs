#![allow(dead_code)]

pub mod fake;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use std::{env, fs, process};

use fvm_contract::app_helper::{
    ExitFacts, HelperEventKind, Milestone, OutputLine, OutputSource, RunRequest, SessionId,
    StopReason,
};
use fvm_contract::helper_internal::{LogCategory, LogLevel, NetworkOutcome, OutsideRoute};
use fvm_helper_core::recovery::RecordSlot;
use fvm_helper_core::session::{
    NetworkJob, Replaced, Report, Run, SessionManager, StageLimits, Timing,
};

pub use fake::*;

pub const ATTEMPT: &str = "E621E1F8-C36C-495A-93FC-0C247A3E6E5F";
pub const OTHER_ATTEMPT: &str = "0B6C3D51-2F4A-4E8B-9C7D-1A2B3C4D5E6F";
pub const PASSWORD: &str = " pässwörd 🔑 ";
pub const OTP: &str = "042137";
pub const RETRY: Duration = Duration::from_millis(200);
const WAIT_LIMIT: Duration = Duration::from_secs(10);

pub const FULL_FLOW: [Milestone; 6] = [
    Milestone::Spawned,
    Milestone::ConnectedToGateway,
    Milestone::OtpPrompted,
    Milestone::OtpSubmitted,
    Milestone::Authenticated,
    Milestone::TunnelUp,
];

pub fn timing() -> Timing {
    let stage = Duration::from_secs(5);
    Timing {
        stages: StageLimits {
            spawn_to_gateway: stage,
            gateway_to_prompt_or_auth: stage,
            otp_prompt_to_submit: stage,
            otp_submit_to_auth: stage,
            auth_to_tunnel: stage,
        },
        output_batch: Duration::from_millis(20),
        network_retry: RETRY,
        detach_after: Duration::from_secs(10),
        stuck_after: Duration::from_secs(30),
    }
}

pub struct RunDirectory {
    pub path: PathBuf,
}

impl RunDirectory {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "fvm-session-run-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = env::temp_dir().join(name);
        let _ = fs::remove_dir_all(&path);
        Self { path }
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.path)
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

impl Drop for RunDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn request(attempt: &str) -> RunRequest {
    RunRequest {
        attempt_id: attempt.parse().unwrap(),
        replacing: None,
        session_id: SessionId(277_070_470),
        host: GATEWAY.to_owned(),
        port: 443,
        username: "alice".to_owned(),
        realm: None,
        trusted_certs: vec![PIN.to_owned()],
        dtls: false,
        password: PASSWORD.to_owned(),
    }
}

pub struct Setup {
    pub timing: Timing,
    pub secrets: Arc<FakeSecrets>,
    pub request: RunRequest,
    pub otp: Option<String>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            timing: timing(),
            secrets: Arc::default(),
            request: request(ATTEMPT),
            otp: None,
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

    pub fn logging_out_to(mut self, gateway: &FakeGateway) -> Self {
        self.request.host = "127.0.0.1".to_owned();
        self.request.port = i64::from(gateway.port);
        self
    }
}

#[derive(Debug)]
pub enum Delivered {
    Output(Vec<OutputLine>),
    Event(HelperEventKind),
    Log(LogLevel, LogCategory, String),
    Stuck,
}

pub struct Session {
    pub run: Run,
    reports: Receiver<Report>,
    pub delivered: Vec<Delivered>,
    pub jobs: VecDeque<NetworkJob>,
    pub secrets: Arc<FakeSecrets>,
    pub directory: RunDirectory,
    otp: Option<String>,
}

impl Session {
    pub fn start(script: Script, setup: Setup) -> Self {
        Self::start_replacing(script, setup, None)
    }

    pub fn start_replacing(script: Script, setup: Setup, replaced: Option<Replaced>) -> Self {
        let directory = RunDirectory::new();
        let sessions = SessionManager::new(
            Arc::new(FakeBackend::new([script])),
            Arc::clone(&setup.secrets) as _,
            directory.path.clone(),
            setup.timing,
        );
        let (sender, reports) = mpsc::channel();
        let mut request = setup.request;
        let slot = if replaced.is_some() {
            RecordSlot::Second
        } else {
            RecordSlot::First
        };
        let run = sessions
            .start(&mut request, slot, replaced, move |report| {
                let _ = sender.send(report);
            })
            .unwrap_or_else(|failure| panic!("the run did not start: {failure:?}"));
        assert!(request.password.is_empty(), "the run took the password");
        Self {
            run,
            reports,
            delivered: Vec::new(),
            jobs: VecDeque::new(),
            secrets: setup.secrets,
            directory,
            otp: setup.otp,
        }
    }

    pub fn wait(&mut self, timeout: Duration, condition: impl Fn(&Self) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while !condition(self) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            match self
                .reports
                .recv_timeout(left.min(Duration::from_millis(5)))
            {
                Ok(report) => self.record(report),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return condition(self),
            }
        }
        true
    }

    fn record(&mut self, report: Report) {
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
            Report::ChangeNetwork(job) => self.jobs.push_back(job),
            Report::Log(level, category, message) => {
                self.delivered
                    .push(Delivered::Log(level, category, message));
            }
            Report::Stuck => self.delivered.push(Delivered::Stuck),
        }
    }

    pub fn wait_for(&mut self, milestone: Milestone) {
        assert!(
            self.wait(WAIT_LIMIT, |session| session.reached(milestone)),
            "{milestone:?} not reached: {:#?}",
            self.texts()
        );
    }

    pub fn wait_for_exit(&mut self) -> ExitFacts {
        assert!(
            self.wait(WAIT_LIMIT, |session| session.exit_facts().is_some()),
            "no exit: {:#?}",
            self.texts()
        );
        self.exit_facts().unwrap()
    }

    pub fn settle(&mut self, duration: Duration) {
        self.wait(duration, |_| false);
    }

    pub fn next_job(&mut self) -> NetworkJob {
        assert!(
            self.wait(WAIT_LIMIT, |session| !session.jobs.is_empty()),
            "no network change: {:#?}",
            self.texts()
        );
        self.jobs.pop_front().unwrap()
    }

    pub fn bring_up(&mut self, routes: Vec<OutsideRoute>) {
        self.next_job().finish(looked_up(routes.clone()));
        self.next_job().finish(applied(routes));
        self.wait_for(Milestone::TunnelUp);
    }

    pub fn stop_and_revert(&mut self, reason: StopReason) -> ExitFacts {
        self.run.stop(reason);
        self.next_job()
            .finish(NetworkOutcome::Reverted { left: Vec::new() });
        self.wait_for_exit()
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

    pub fn lines(&self) -> Vec<&OutputLine> {
        self.delivered
            .iter()
            .filter_map(|delivered| match delivered {
                Delivered::Output(lines) => Some(lines),
                _ => None,
            })
            .flatten()
            .collect()
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

    pub fn logs(&self) -> Vec<&str> {
        self.delivered
            .iter()
            .filter_map(|delivered| match delivered {
                Delivered::Log(_, _, message) => Some(message.as_str()),
                _ => None,
            })
            .collect()
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

    pub fn texts_before(&self, matches: impl Fn(&HelperEventKind) -> bool) -> Vec<&str> {
        let mut texts = Vec::new();
        for delivered in &self.delivered {
            match delivered {
                Delivered::Output(lines) => {
                    texts.extend(lines.iter().map(|line| line.text.as_str()));
                }
                Delivered::Event(kind) if matches(kind) => return texts,
                _ => {}
            }
        }
        texts
    }

    pub fn assert_finished_cleanly(&self) {
        assert!(matches!(
            self.delivered
                .iter()
                .rev()
                .find(|delivered| !matches!(delivered, Delivered::Log(..))),
            Some(Delivered::Event(HelperEventKind::Exited { .. }))
        ));
        let exits = self
            .events()
            .iter()
            .filter(|kind| matches!(kind, HelperEventKind::Exited { .. }))
            .count();
        assert_eq!(exits, 1);
        assert!(!self.run.session_running());
        let shown = format!("{:?} {:?}", self.texts(), self.logs());
        for secret in [PASSWORD, OTP, COOKIE] {
            assert!(!shown.contains(secret), "{secret} shown in {shown}");
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.run.stop(StopReason::AppQuit);
    }
}

pub fn outside(destination: &str, prefix_length: u8, router: &str) -> OutsideRoute {
    OutsideRoute {
        destination: network(destination, prefix_length),
        router: router.parse().unwrap(),
        service_id: "ethernet".to_owned(),
    }
}

pub fn gateway_route(router: &str) -> OutsideRoute {
    outside(GATEWAY, 32, router)
}

pub fn excluded_route() -> OutsideRoute {
    outside("10.20.161.16", 32, "192.0.2.1")
}

pub fn looked_up(missing: Vec<OutsideRoute>) -> NetworkOutcome {
    NetworkOutcome::LookedUp {
        dns_service_id: "svc".to_owned(),
        missing,
    }
}

pub fn applied(added: Vec<OutsideRoute>) -> NetworkOutcome {
    NetworkOutcome::Applied {
        added,
        left: Vec::new(),
    }
}
