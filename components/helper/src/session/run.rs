//! One run: a session thread that runs the backend's session, and a run thread that owns everything
//! else about the run and handles, in order, what the session reports, its end, the caller's
//! requests and the time limits. When the session opened, its secret is saved off the session
//! thread; once the session ends, the session thread deletes the secret and only then reports the
//! end. The run keeps its run record up to date and removes it before it reports the exit.

mod network;

use std::mem;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant, SystemTime};

use fvm_contract::app_helper::{
    EndFacts, HelperError, HelperErrorCode, HelperEventKind, Milestone, OutputLine, OutputSource,
    RunRequest, StopInitiator, StopReason,
};
use fvm_contract::helper_internal::{LogCategory, LogLevel, NetworkChange, NetworkOutcome};
use fvm_contract::validation;
use zeroize::Zeroizing;

use crate::recovery::logout::SessionSecret;
use crate::recovery::run_directory::{self, RunFiles, RunRecord};
use crate::secrets::SecretStore;
use crate::session::backend::{
    Backend, FormKind, LineLevel, Link, SessionEnd, SessionEvent, SessionRequest,
};
use crate::session::{Report, Timing};
use crate::watchdog::{Beat, Watchdog};

pub use network::{NotReplaceable, Replaced};

const LOCK: &str = "the saving slot is never held across a panic";

/// Controls a started run. Dropping it does not stop the run.
pub struct Run {
    client_name: &'static str,
    inputs: Sender<Input>,
    returned: Arc<AtomicBool>,
    /// The thread that calls `on_report`.
    run_thread: ThreadId,
    name: String,
}

impl Run {
    /// Starts a session for `request` and calls `on_report` in order on the run's own thread until
    /// the `Exited` event. The run's first line names the client and its version. `on_report` must
    /// never wait for anything that may be held while `submit_otp` runs, since `submit_otp` waits
    /// for that thread; calling `submit_otp` or `stop` on it panics.
    #[expect(clippy::too_many_arguments)]
    pub(crate) fn start(
        backend: Arc<dyn Backend>,
        secrets: Arc<dyn SecretStore>,
        request: &mut RunRequest,
        files: RunFiles,
        replaced: Option<Replaced>,
        start_after: Option<Receiver<()>>,
        watchdog: &Watchdog,
        timing: Timing,
        on_report: impl FnMut(Report) + Send + 'static,
    ) -> Self {
        let name = format!("{} run {}", request.session_id, request.attempt_id);
        let client_name = backend.name();
        let client_line = OutputLine {
            date: SystemTime::now(),
            source: OutputSource::Helper,
            text: format!("‹VPN client: {client_name} {}›", backend.version()),
        };
        let port = u16::try_from(request.port).expect("the backend checked the port");
        let attempt = request.attempt_id.to_string();
        let (inputs, received) = mpsc::channel();
        let saving: Arc<Mutex<Option<_>>> = Arc::default();
        let link = Arc::new(Link::new(
            {
                let inputs = inputs.clone();
                move |event| {
                    let _ = inputs.send(Input::Session(event));
                }
            },
            {
                let saving = Arc::clone(&saving);
                let secrets = Arc::clone(&secrets);
                let attempt = attempt.clone();
                let host = request.host.clone();
                let pins = request.trusted_certs.clone();
                move |mut cookie: Zeroizing<String>| {
                    let secret = SessionSecret {
                        host,
                        port,
                        pins,
                        cookie: mem::take(&mut *cookie),
                    };
                    let saver = thread::Builder::new()
                        .name("fvm-session-secret".to_owned())
                        .spawn(move || secrets.save(&attempt, &secret.encode()))
                        .expect("starting the session secret thread");
                    *saving.lock().expect(LOCK) = Some(saver);
                }
            },
        ));
        let session = SessionRequest {
            host: request.host.clone(),
            port,
            realm: request.realm.clone(),
            username: request.username.clone(),
            password: Zeroizing::new(mem::take(&mut request.password)),
            pins: request.trusted_certs.clone(),
            dtls: request.dtls,
        };
        {
            let link = Arc::clone(&link);
            let inputs = inputs.clone();
            thread::Builder::new()
                .name("fvm-session".to_owned())
                .spawn(move || {
                    if let Some(start_after) = start_after {
                        start_after
                            .recv()
                            .expect("the entry starts the session once the revert is answered");
                    }
                    let end = backend.run(session, &link);
                    let _ = inputs.send(Input::Returned(end));
                    let saver = saving.lock().expect(LOCK).take();
                    let saved = saver
                        .map(|saver| saver.join().expect("saving a session secret never panics"));
                    let ended = finish_session(saved, secrets.as_ref(), &attempt);
                    let _ = inputs.send(Input::Ended(ended));
                })
                .expect("starting the session thread");
        }
        let returned = Arc::new(AtomicBool::new(false));
        let watch = Watch {
            client_name,
            link,
            name: name.clone(),
            files,
            record: RunRecord::default(),
            network: network::Network::default(),
            replaced,
            timing,
            beat: watchdog.watch(name.clone()),
            on_report,
            inputs: inputs.clone(),
            returned: Arc::clone(&returned),
            phase: Phase::Running,
            otp_submitted: false,
            furthest: Milestone::Started,
            tunnel_ip: None,
            rejected_certificates: Vec::new(),
            certificate_rejected_at: None,
            tunnel_setup_failed: false,
            tunnel_address_in_use: false,
            stop: None,
            detach_at: None,
            stuck_at: None,
            batch: Vec::new(),
            batch_deadline: None,
            stage: None,
        };
        let watcher = thread::Builder::new()
            .name("fvm-run".to_owned())
            .spawn(move || watch.run(client_line, &received))
            .expect("starting the run thread");
        Run {
            client_name,
            inputs,
            returned,
            run_thread: watcher.thread().id(),
            name,
        }
    }

    /// Checks `code`, then hands it to the session; accepted once, while the session waits for the
    /// OTP. The error is the Helper's answer to the app.
    pub fn submit_otp(&self, code: String) -> Result<(), HelperError> {
        assert_ne!(
            thread::current().id(),
            self.run_thread,
            "submit_otp from on_report would wait for its own thread"
        );
        let code = Zeroizing::new(code);
        validation::check_otp(&code).map_err(|message| HelperError {
            code: HelperErrorCode::InvalidRequest,
            message,
        })?;
        let (reply, answer) = mpsc::sync_channel(1);
        let not_running = || HelperError {
            code: HelperErrorCode::WrongPhase,
            message: format!("{} is not running", self.client_name),
        };
        if self.inputs.send(Input::SubmitOtp { code, reply }).is_err() {
            return Err(not_running());
        }
        // No answer when the run ended before it got to the request.
        answer.recv().unwrap_or_else(|_| Err(not_running()))
    }

    /// Cancels the session. Only the first reason counts; nothing happens once the session ended.
    pub fn stop(&self, reason: StopReason) {
        assert_ne!(
            thread::current().id(),
            self.run_thread,
            "stop is not for on_report"
        );
        let _ = self.inputs.send(Input::Stop(reason));
    }

    pub fn replaceable(&self) -> Result<Replaced, NotReplaceable> {
        assert_ne!(
            thread::current().id(),
            self.run_thread,
            "replaceable from on_report would wait for its own thread"
        );
        let (reply, answer) = mpsc::sync_channel(1);
        if self.inputs.send(Input::Replaceable(reply)).is_err() {
            return Err(NotReplaceable::NotUp);
        }
        answer.recv().unwrap_or(Err(NotReplaceable::NotUp))
    }

    pub fn routing_check(&self) -> Option<NetworkChange> {
        assert_ne!(
            thread::current().id(),
            self.run_thread,
            "routing_check from on_report would wait for its own thread"
        );
        let (reply, answer) = mpsc::sync_channel(1);
        if self.inputs.send(Input::RoutingCheck(reply)).is_err() {
            return None;
        }
        answer.recv().ok().flatten()
    }

    pub fn replaced_run_exited(&self) {
        let _ = self.inputs.send(Input::ReplacedExited);
    }

    pub fn first_slot_freed(&self) {
        let _ = self.inputs.send(Input::FirstSlotFreed);
    }

    /// The client's name as the Helper's English errors and log lines show it.
    pub fn client_name(&self) -> &'static str {
        self.client_name
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn session_running(&self) -> bool {
        !self.returned.load(Ordering::Relaxed)
    }
}

pub(crate) struct Ended {
    failures: Vec<String>,
}

fn finish_session(
    saved: Option<Result<(), String>>,
    secrets: &dyn SecretStore,
    attempt: &str,
) -> Ended {
    let Some(saved) = saved else {
        return Ended {
            failures: Vec::new(),
        };
    };
    let failure = match saved {
        Ok(()) => secrets
            .delete(attempt)
            .err()
            .map(|error| format!("cannot delete the session secret: {error}")),
        Err(error) => Some(format!("cannot save the session secret: {error}")),
    };
    Ended {
        failures: failure.into_iter().collect(),
    }
}

pub(crate) enum Input {
    Session(SessionEvent),
    Returned(SessionEnd),
    Ended(Ended),
    SubmitOtp {
        code: Zeroizing<String>,
        reply: SyncSender<Result<(), HelperError>>,
    },
    Stop(StopReason),
    NetworkChanged(NetworkOutcome),
    Replaceable(SyncSender<Result<Replaced, NotReplaceable>>),
    RoutingCheck(SyncSender<Option<NetworkChange>>),
    ReplacedExited,
    FirstSlotFreed,
    HandOverAsked(Sender<Input>),
    HandedOver(Option<network::HandOver>),
    SwitchEnded(network::SwitchEnd),
}

struct Watch<R> {
    client_name: &'static str,
    link: Arc<Link>,
    /// Names the run in the Helper's log.
    name: String,
    files: RunFiles,
    /// What the run record holds once written.
    record: RunRecord,
    network: network::Network,
    replaced: Option<Replaced>,
    timing: Timing,
    beat: Beat,
    on_report: R,
    /// For the answers to the network changes.
    inputs: Sender<Input>,
    returned: Arc<AtomicBool>,
    phase: Phase,
    otp_submitted: bool,

    furthest: Milestone,
    tunnel_ip: Option<Ipv4Addr>,
    rejected_certificates: Vec<String>,
    certificate_rejected_at: Option<Milestone>,
    tunnel_setup_failed: bool,
    tunnel_address_in_use: bool,
    /// The first stop reason.
    stop: Option<StopReason>,
    detach_at: Option<Instant>,
    stuck_at: Option<Instant>,

    batch: Vec<OutputLine>,
    batch_deadline: Option<Instant>,
    stage: Option<Stage>,
}

enum Phase {
    Running,
    Returned(SessionEnd),
    AwaitingSwitch(EndFacts),
    RevertingNetwork(EndFacts),
    Reported,
}

struct Stage {
    name: &'static str,
    limit: Duration,
    deadline: Instant,
}

impl<R: FnMut(Report)> Watch<R> {
    fn run(mut self, client_line: OutputLine, inputs: &Receiver<Input>) {
        self.batch.push(client_line);
        self.reach(Milestone::Started);
        while !matches!(self.phase, Phase::Reported) {
            self.beat.beat();
            let period = self.beat.period();
            let wait = self.next_deadline().map_or(period, |deadline| {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(period)
            });
            let input = match inputs.recv_timeout(wait) {
                Ok(input) => Some(input),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => unreachable!("the run holds a sender"),
            };
            if let Some(input) = input {
                self.handle(input);
            }
            self.deadlines_passed(Instant::now());
            self.settle_hand_over();
        }
    }

    fn running(&self) -> bool {
        matches!(self.phase, Phase::Running)
    }

    fn handle(&mut self, input: Input) {
        match input {
            Input::Session(event) => self.session_event(event),
            Input::Returned(end) => self.returned(end),
            Input::Ended(ended) => self.ended(ended),
            Input::SubmitOtp { code, reply } => {
                let _ = reply.send(self.submit_otp(code));
            }
            Input::Stop(reason) => self.stop(reason),
            Input::NetworkChanged(outcome) => self.network_changed(outcome),
            Input::Replaceable(reply) => {
                let _ = reply.send(self.replaceable());
            }
            Input::RoutingCheck(reply) => {
                let _ = reply.send(self.routing_check());
            }
            Input::ReplacedExited => self.replaced_exited(),
            Input::FirstSlotFreed => self.take_first_slot(),
            Input::HandOverAsked(to) => self.network.hand_over_to = Some(to),
            Input::HandedOver(hand_over) => self.handed_over(hand_over),
            Input::SwitchEnded(end) => self.switch_ended(end),
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        [
            self.batch_deadline,
            self.stage.as_ref().map(|stage| stage.deadline),
            self.detach_at,
            self.stuck_at,
            self.network.deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn deadlines_passed(&mut self, now: Instant) {
        if self
            .stage
            .as_ref()
            .is_some_and(|stage| stage.deadline <= now)
        {
            self.stage_expired();
        }
        if self.detach_at.is_some_and(|deadline| deadline <= now) {
            self.detach_at = None;
            if self.link.detach() {
                self.meta("‹session still running: detach sent›");
            }
        }
        if self.stuck_at.is_some_and(|deadline| deadline <= now) {
            self.stuck_at = None;
            self.stuck();
        }
        self.network_deadlines_passed(now);
        if self.batch_deadline.is_some_and(|deadline| deadline <= now) {
            self.flush_batch();
        }
    }

    // The session

    fn session_event(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::GatewayTrusted => self.advance(Milestone::ConnectedToGateway),
            SessionEvent::CertificateRejected(value) => {
                self.meta(format!("‹gateway certificate not trusted: {value}›"));
                self.certificate_rejected_at.get_or_insert(self.furthest);
                if !self.rejected_certificates.contains(&value) {
                    self.rejected_certificates.push(value);
                }
            }
            SessionEvent::OtpPrompted => self.advance(Milestone::OtpPrompted),
            SessionEvent::FormRepeated(_) if self.stop.is_some() => {}
            SessionEvent::FormRepeated(FormKind::Login) => {
                self.meta("‹password asked again: the gateway rejected the login›");
                self.stop(StopReason::PromptRepeated);
            }
            SessionEvent::FormRepeated(FormKind::Otp) => {
                self.meta("‹OTP asked again: the gateway rejected the OTP›");
                self.stop(StopReason::PromptRepeated);
            }
            SessionEvent::Authenticated => self.advance(Milestone::Authenticated),
            SessionEvent::Line(level, text) => {
                let source = match level {
                    LineLevel::Error => OutputSource::Error,
                    LineLevel::Info => OutputSource::Info,
                };
                self.output(source, text);
            }
            SessionEvent::TunnelReady(settings) => self.tunnel_ready(settings),
            SessionEvent::TunnelFailed(address, reason) => self.tunnel_failed(address, reason),
            SessionEvent::Reconnected => self.reconnected(),
        }
    }

    // Output

    fn output(&mut self, source: OutputSource, text: String) {
        self.batch.push(OutputLine {
            date: SystemTime::now(),
            source,
            text,
        });
        self.batch_deadline
            .get_or_insert_with(|| Instant::now() + self.timing.output_batch);
    }

    fn meta(&mut self, text: impl Into<String>) {
        self.output(OutputSource::Helper, text.into());
    }

    fn flush_batch(&mut self) {
        self.batch_deadline = None;
        if !self.batch.is_empty() {
            (self.on_report)(Report::Output(mem::take(&mut self.batch)));
        }
    }

    fn event(&mut self, kind: HelperEventKind) {
        self.flush_batch();
        (self.on_report)(Report::Event(kind));
    }

    // Milestones and input

    fn advance(&mut self, milestone: Milestone) {
        if milestone > self.furthest {
            self.reach(milestone);
        }
    }

    fn reach(&mut self, milestone: Milestone) {
        self.furthest = milestone;
        let tunnel_ip = if milestone == Milestone::TunnelUp {
            self.tunnel_ip
        } else {
            None
        };
        self.event(HelperEventKind::Milestone {
            milestone,
            tunnel_ip,
        });
        self.stage = self
            .timing
            .stages
            .after(milestone)
            .filter(|_| self.running() && self.stop.is_none())
            .map(|(name, limit)| Stage {
                name,
                limit,
                deadline: Instant::now() + limit,
            });
    }

    fn submit_otp(&mut self, code: Zeroizing<String>) -> Result<(), HelperError> {
        let client = self.client_name;
        let refusal = if !self.running() || self.stop.is_some() {
            Some(format!("{client} is not running"))
        } else if self.otp_submitted {
            Some("an OTP was already submitted for this attempt".to_owned())
        } else if self.furthest != Milestone::OtpPrompted {
            Some(format!("{client} is not waiting for an OTP"))
        } else {
            None
        };
        if let Some(message) = refusal {
            return Err(HelperError {
                code: HelperErrorCode::WrongPhase,
                message,
            });
        }
        self.otp_submitted = true;
        self.link.submit_otp(code);
        self.meta("‹OTP sent›");
        self.advance(Milestone::OtpSubmitted);
        Ok(())
    }

    // Time limits and stopping

    fn stage_expired(&mut self) {
        let Some(stage) = self.stage.take() else {
            return;
        };
        let text = format!(
            "‹stage limit: {} did not finish within {}›",
            stage.name,
            seconds(stage.limit)
        );
        if self.network.watching() {
            self.network_setup_failed(text);
        } else {
            self.meta(text);
            self.stop(StopReason::StageLimit);
        }
    }

    fn stop(&mut self, reason: StopReason) {
        if self.stop.is_some() || !self.running() {
            return;
        }
        self.stop = Some(reason);
        self.stage = None;
        let raw = serde_json::to_value(reason).expect("a stop reason serializes");
        let raw = raw
            .as_str()
            .expect("a stop reason serializes as its raw value");
        self.meta(format!("‹session cancelled (reason: {raw})›"));
        if self.link.cancel() {
            self.detach_at = Some(Instant::now() + self.timing.detach_after);
        }
        self.stuck_at = Some(Instant::now() + self.timing.stuck_after);
    }

    fn stuck(&mut self) {
        let limit = seconds(self.timing.stuck_after);
        let what = if self.running() {
            format!("session did not end within {limit} after the cancel")
        } else {
            format!(
                "saving and deleting the session secret did not finish within {limit} after the session ended"
            )
        };
        self.meta(format!("‹{what}: restarting the Helper›"));
        self.flush_batch();
        let message = format!(
            "{}: {what}; aborting so the OS restarts the Helper",
            self.name
        );
        self.log(LogLevel::Error, LogCategory::Helper, message);
        (self.on_report)(Report::Stuck);
    }

    // The end

    fn returned(&mut self, end: SessionEnd) {
        let Phase::Running = mem::replace(&mut self.phase, Phase::Returned(end)) else {
            unreachable!("a session returns once")
        };
        self.returned.store(true, Ordering::Relaxed);
        self.stage = None;
        self.detach_at = None;
        self.stuck_at = Some(Instant::now() + self.timing.stuck_after);
        self.network.stop_watching();
    }

    fn ended(&mut self, ended: Ended) {
        let Phase::Returned(end) = mem::replace(&mut self.phase, Phase::Reported) else {
            unreachable!("a session ends once, after it returned")
        };
        self.stuck_at = None;
        for failure in ended.failures {
            let message = format!("{}: {failure}", self.name);
            self.log(LogLevel::Error, LogCategory::Helper, message);
        }
        self.meta(format!("‹session ended: code {}›", end.code));
        let facts = EndFacts {
            furthest_milestone: self.furthest,
            stop_initiator: match self.stop {
                Some(reason) => StopInitiator::Helper { reason },
                None => StopInitiator::Session {},
            },
            cert_rejected_digests: mem::take(&mut self.rejected_certificates),
            cert_rejected_at: self.certificate_rejected_at,
            tunnel_setup_failed: self.tunnel_setup_failed,
            tunnel_address_in_use: self.tunnel_address_in_use,
            end_code: end.code,
        };
        self.revert_network_or_continue(facts);
    }

    // The run record

    fn take_first_slot(&mut self) {
        let Some(second) = self.files.take_first_slot() else {
            return;
        };
        self.save_record();
        if let Err(line) = run_directory::remove(&second) {
            self.log(LogLevel::Error, LogCategory::Helper, line);
        }
    }

    fn save_record(&mut self) {
        if let Err(failure) = self.files.write_record(&self.record) {
            self.log(
                LogLevel::Error,
                LogCategory::Helper,
                format!("cannot write the run record: {failure}"),
            );
        }
    }

    /// Never with a secret: the line goes to the system log.
    fn log(&mut self, level: LogLevel, category: LogCategory, message: String) {
        (self.on_report)(Report::Log(level, category, message));
    }

    fn report_exit(&mut self, facts: EndFacts) {
        self.phase = Phase::Reported;
        if let Err(line) = self.files.remove() {
            self.log(LogLevel::Error, LogCategory::Helper, line);
        }
        self.event(HelperEventKind::Exited { facts });
    }
}

fn tunnel_ipv4(address: IpAddr) -> Ipv4Addr {
    let IpAddr::V4(address) = address else {
        unreachable!("the tunnel's IPv4 settings hold an IPv4 address")
    };
    address
}

/// "30s", "0.25s".
fn seconds(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        let text = format!("{:.3}", duration.as_secs_f64());
        format!("{}s", text.trim_end_matches('0'))
    }
}
