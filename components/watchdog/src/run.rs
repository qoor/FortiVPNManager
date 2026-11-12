//! One run: a thread that owns everything about the client and handles, in order, its output, its
//! exit, the caller's requests and the time limits. Reader threads and an exit thread only forward
//! what they see. A client that logs in and opens the tunnel in separate runs has two processes, one
//! after the other; their inputs carry which one they come from. When the Helper had to end the
//! tunnel run by force, or never started it, the run sends the client's logout request before it
//! reports the exit.

use std::io::{self, PipeReader, PipeWriter, Read, Write};
use std::iter;
use std::mem;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant, SystemTime};

use fvm_contract::app_helper::{
    ExitFacts, HelperError, HelperErrorCode, HelperEventKind, Milestone, OutputLine, OutputSource,
    ProcessExitStatus, StopInitiator, StopReason,
};
use fvm_contract::backend::{
    Backend, Finding, LaunchSpec, OutputInterpreter, Prompt, RouteRepairOutcome, SessionInputs,
    SessionLaunch,
};
use zeroize::Zeroizing;

use crate::lines::{LineAssembler, MAX_LINE_BYTES};
use crate::os::{self, Child, Spawned};
use crate::{Report, RouteRepair, Timing};

/// Controls a started run. Dropping it does not stop the run.
pub struct Run {
    inputs: Sender<Input>,
    client_name: &'static str,
    /// 0 once the client is about to be reaped.
    process_id: Arc<AtomicU32>,
    /// The thread that calls `on_report`.
    run_thread: ThreadId,
}

impl Run {
    /// Starts the client as `launch` says, for one run, and calls `on_report` in order on the run's
    /// own thread until the `Exited` event, which is also how a failed start is reported.
    /// `on_report` must never wait for anything that may be held while `submit_otp` runs, since
    /// `submit_otp` waits for that thread; calling `submit_otp` or `stop` on it panics.
    ///
    /// `password` goes to stdin at the client's first password prompt, if the interpreter expects
    /// one there, and is then zeroed. The process must ignore SIGPIPE (see the crate documentation).
    pub fn start<B>(
        backend: &B,
        launch: &LaunchSpec,
        password: String,
        timing: Timing,
        on_report: impl FnMut(Report) + Send + 'static,
    ) -> Run
    where
        B: Backend,
        B::Interpreter: 'static,
    {
        // The caller's copy is zeroed when this returns; the line is zeroed once written.
        let password = Zeroizing::new(password);
        let mut password_line = Zeroizing::new(Vec::with_capacity(password.len() + 1));
        password_line.extend_from_slice(password.as_bytes());
        password_line.push(b'\n');

        let (inputs, received) = mpsc::channel();
        let client_name = backend.client_name();
        let spawned = match os::spawn(launch) {
            Ok(spawned) => spawned,
            Err(errno) => {
                let text = format!(
                    "‹spawn failed: {}: {}›",
                    launch.program.display(),
                    describe_errno(errno)
                );
                let reporter = thread::Builder::new()
                    .name("fvm-watchdog".to_owned())
                    .spawn(move || report_spawn_failure(text, errno, on_report))
                    .expect("starting the watchdog thread");
                return Run {
                    inputs,
                    client_name,
                    process_id: Arc::new(AtomicU32::new(0)),
                    run_thread: reporter.thread().id(),
                };
            }
        };

        let process_id = Arc::new(AtomicU32::new(spawned.child.id()));
        let command = command_line(launch);
        let (child, stdin, wake_write) = watch_process(spawned, 0, &inputs);
        let watch = Watch {
            client_name,
            interpreter: backend.interpreter(),
            grace_period: backend.stop_grace_period(),
            timing,
            on_report,
            inputs: inputs.clone(),
            process_id: Arc::clone(&process_id),
            phase: Phase::Running(child),
            generation: 0,
            later_runs: launch.session.clone(),
            session: None,
            tunnel_started: false,
            forced: false,
            stdin: Some(stdin),
            password: Some(password_line),
            otp_submitted: false,
            furthest: Milestone::Spawned,
            tunnel_ip: None,
            tunnel_up_at: None,
            rejected_certificates: Vec::new(),
            certificate_rejected_at: None,
            tunnel_mode_rejected: false,
            stop: None,
            stdout: Stream::new(timing.line_idle_flush),
            stderr: Stream::new(timing.line_idle_flush),
            wake_readers: Some(wake_write),
            batch: Vec::new(),
            batch_deadline: None,
            stage: None,
            kill_deadline: None,
        };
        let watcher = thread::Builder::new()
            .name("fvm-watchdog".to_owned())
            .spawn(move || watch.run(command, &received))
            .expect("starting the watchdog thread");
        Run {
            inputs,
            client_name,
            process_id,
            run_thread: watcher.thread().id(),
        }
    }

    /// Writes `code` and a line break to the client's stdin and closes stdin; accepted once, while
    /// the client waits for the OTP. The caller checks the code with `Backend::check_otp` first.
    /// The error is the Helper's answer to the app.
    pub fn submit_otp(&self, code: String) -> Result<(), HelperError> {
        assert_ne!(
            thread::current().id(),
            self.run_thread,
            "submit_otp from on_report would wait for its own thread"
        );
        let code = Zeroizing::new(code);
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

    /// Stops the client: the stop policy's graceful stop now, the forced one after its grace
    /// period. Only the first reason counts; nothing happens once the client has exited.
    pub fn stop(&self, reason: StopReason) {
        assert_ne!(
            thread::current().id(),
            self.run_thread,
            "stop is not for on_report"
        );
        let _ = self.inputs.send(Input::Stop(reason));
    }

    /// The client's process id until it exits and is reaped.
    pub fn process_id(&self) -> Option<u32> {
        match self.process_id.load(Ordering::Relaxed) {
            0 => None,
            id => Some(id),
        }
    }
}

/// Starts the threads that forward what process number `generation` of the run does, and returns
/// what the run itself keeps of it.
fn watch_process(
    spawned: Spawned,
    generation: u32,
    inputs: &Sender<Input>,
) -> (Child, PipeWriter, PipeWriter) {
    let exit = spawned.child.exit_watch();
    let exited = inputs.clone();
    thread::Builder::new()
        .name("fvm-watchdog-exit".to_owned())
        .spawn(move || {
            exit.wait();
            let _ = exited.send(Input::Exited(generation));
        })
        .expect("starting the watchdog's exit thread");
    let wake = Arc::new(spawned.wake_read);
    for (source, pipe) in [
        (OutputSource::Stdout, spawned.stdout),
        (OutputSource::Stderr, spawned.stderr),
    ] {
        let wake = Arc::clone(&wake);
        let forwarded = inputs.clone();
        thread::Builder::new()
            .name("fvm-watchdog-output".to_owned())
            .spawn(move || {
                os::read_output(pipe, &wake, |bytes| {
                    forwarded
                        .send(Input::Output(generation, source, bytes))
                        .is_ok()
                });
                let _ = forwarded.send(Input::End(generation, source));
            })
            .expect("starting a watchdog output thread");
    }
    (spawned.child, spawned.stdin, spawned.wake_write)
}

fn command_line(launch: &LaunchSpec) -> String {
    iter::once(launch.program.display().to_string())
        .chain(launch.args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) enum Input {
    /// From process number `.0` of the run, as all three of these.
    Output(u32, OutputSource, Vec<u8>),
    End(u32, OutputSource),
    Exited(u32),
    SubmitOtp {
        code: Zeroizing<String>,
        reply: SyncSender<Result<(), HelperError>>,
    },
    Stop(StopReason),
    /// The log line about the logout request.
    LoggedOut(String),
    RoutesRepaired(RouteRepairOutcome),
}

fn report_spawn_failure(text: String, errno: i32, mut on_report: impl FnMut(Report)) {
    on_report(Report::Output(vec![OutputLine {
        date: SystemTime::now(),
        source: OutputSource::Helper,
        text,
    }]));
    on_report(Report::Event(HelperEventKind::Exited {
        facts: ExitFacts {
            furthest_milestone: None,
            stop_initiator: StopInitiator::Client {},
            cert_rejected_digests: Vec::new(),
            cert_rejected_at: None,
            tunnel_mode_rejected: false,
            up_duration: None,
            exit_status: ProcessExitStatus::SpawnFailed { errno },
        },
    }));
}

struct Watch<I, R> {
    client_name: &'static str,
    interpreter: I,
    grace_period: Duration,
    timing: Timing,
    on_report: R,
    /// For the route repair's answer.
    inputs: Sender<Input>,
    process_id: Arc<AtomicU32>,
    phase: Phase,
    /// Which of the run's processes is the current one, counting from 0.
    generation: u32,
    /// What follows the login run of a client that logs in and opens the tunnel in separate runs.
    later_runs: Option<Box<SessionLaunch>>,
    /// The session the login run printed.
    session: Option<SessionInputs>,
    tunnel_started: bool,
    /// The current process could not log out: it was killed rather than asked to stop, or it never
    /// started.
    forced: bool,

    stdin: Option<PipeWriter>,
    /// The password and a line break, until written.
    password: Option<Zeroizing<Vec<u8>>>,
    otp_submitted: bool,

    furthest: Milestone,
    tunnel_ip: Option<Ipv4Addr>,
    /// On the clock that counts sleep: the app compares the up duration with how long a session
    /// counts as stable. The stage limits stay on `Instant`, which stops during sleep.
    tunnel_up_at: Option<Duration>,
    rejected_certificates: Vec<String>,
    certificate_rejected_at: Option<Milestone>,
    tunnel_mode_rejected: bool,
    /// The first stop reason.
    stop: Option<StopReason>,

    stdout: Stream,
    stderr: Stream,
    /// Closed when the run finishes, so the output threads return even while something the client
    /// started still holds a pipe.
    wake_readers: Option<PipeWriter>,
    batch: Vec<OutputLine>,
    batch_deadline: Option<Instant>,
    stage: Option<Stage>,
    kill_deadline: Option<Instant>,
}

enum Phase {
    Running(Child),
    /// Reaped; collecting the rest of the output until both pipes end or the deadline passes.
    Draining {
        status: ProcessExitStatus,
        /// On the clock of `tunnel_up_at`.
        exited_at: Duration,
        deadline: Instant,
    },
    /// The client could not log out; waiting for its logout request.
    LoggingOut(ExitFacts),
    RepairingRoutes(ExitFacts),
    Reported,
}

struct Stage {
    name: &'static str,
    limit: Duration,
    deadline: Instant,
}

struct Stream {
    lines: LineAssembler,
    /// The bytes after the last line break, for prompts; the last `MAX_LINE_BYTES` of them, like
    /// the lines, since a prompt is at the end.
    unfinished: Vec<u8>,
    open: bool,
}

impl Stream {
    fn new(idle_flush: Duration) -> Self {
        Self {
            lines: LineAssembler::new(idle_flush),
            unfinished: Vec::new(),
            open: true,
        }
    }
}

impl<I: OutputInterpreter, R: FnMut(Report)> Watch<I, R> {
    fn run(mut self, command: String, inputs: &Receiver<Input>) {
        let Phase::Running(child) = &self.phase else {
            unreachable!("a run starts with the client running")
        };
        let pid = child.id();
        self.meta(format!("‹attempt started: pid {pid}, {command}›"));
        self.client_started();
        self.reach(Milestone::Spawned);
        while !matches!(self.phase, Phase::Reported) {
            let input = match self.next_deadline() {
                None => Some(inputs.recv().expect("the run holds a sender")),
                Some(deadline) => {
                    match inputs.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(input) => Some(input),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => {
                            unreachable!("the run holds a sender")
                        }
                    }
                }
            };
            if let Some(input) = input {
                self.handle(input);
            }
            self.deadlines_passed(Instant::now(), inputs);
        }
    }

    fn running(&self) -> bool {
        matches!(self.phase, Phase::Running(_))
    }

    fn handle(&mut self, input: Input) {
        match input {
            Input::Output(generation, _, _)
            | Input::End(generation, _)
            | Input::Exited(generation)
                if generation != self.generation => {}
            Input::Output(_, source, bytes) => {
                if matches!(self.phase, Phase::Running(_) | Phase::Draining { .. }) {
                    self.read(source, &bytes);
                }
            }
            Input::End(_, source) => {
                self.stream(source).open = false;
                self.finish_if_drained();
            }
            Input::Exited(_) => self.exited(),
            Input::SubmitOtp { code, reply } => {
                let _ = reply.send(self.submit_otp(&code));
            }
            Input::Stop(reason) => self.stop(reason),
            Input::LoggedOut(text) => self.logged_out(text),
            Input::RoutesRepaired(outcome) => self.routes_repaired(outcome),
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        let draining = match self.phase {
            Phase::Draining { deadline, .. } => Some(deadline),
            _ => None,
        };
        [
            self.batch_deadline,
            self.stage.as_ref().map(|stage| stage.deadline),
            self.kill_deadline,
            self.stdout.lines.idle_deadline(),
            self.stderr.lines.idle_deadline(),
            draining,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn deadlines_passed(&mut self, now: Instant, inputs: &Receiver<Input>) {
        for source in [OutputSource::Stdout, OutputSource::Stderr] {
            if let Some(text) = self.stream(source).lines.flush_if_idle(now) {
                self.line(source, text);
            }
        }
        if self
            .stage
            .as_ref()
            .is_some_and(|stage| stage.deadline <= now)
        {
            self.stage_expired();
        }
        if self.kill_deadline.is_some_and(|deadline| deadline <= now) {
            self.grace_period_expired();
        }
        if matches!(self.phase, Phase::Draining { deadline, .. } if deadline <= now) {
            self.drain_expired(inputs);
        }
        if self.batch_deadline.is_some_and(|deadline| deadline <= now) {
            self.flush_batch();
        }
    }

    fn stream(&mut self, source: OutputSource) -> &mut Stream {
        match source {
            OutputSource::Stdout => &mut self.stdout,
            OutputSource::Stderr => &mut self.stderr,
            OutputSource::Helper => unreachable!("the Helper's own lines are not read"),
        }
    }

    // Output

    fn read(&mut self, source: OutputSource, bytes: &[u8]) {
        let stream = self.stream(source);
        let lines = stream.lines.append(bytes, Instant::now());
        match bytes.iter().rposition(|&byte| byte == b'\n') {
            Some(last_break) => {
                stream.unfinished.clear();
                stream
                    .unfinished
                    .extend_from_slice(&bytes[last_break + 1..]);
            }
            None => stream.unfinished.extend_from_slice(bytes),
        }
        let excess = stream.unfinished.len().saturating_sub(MAX_LINE_BYTES);
        stream.unfinished.drain(..excess);
        // A prompt follows the lines printed before it.
        for text in lines {
            self.line(source, text);
        }
        if bytes.last() != Some(&b'\n') {
            let unfinished = match source {
                OutputSource::Stdout => &self.stdout.unfinished,
                _ => &self.stderr.unfinished,
            };
            if let Some(prompt) = self.interpreter.prompt(source, unfinished) {
                self.prompt(prompt);
            }
        }
    }

    fn line(&mut self, source: OutputSource, text: String) {
        let finding = self.interpreter.line(source, &text);
        match finding {
            Some(Finding::Session(_)) => self.meta("‹session received; its line is not shown›"),
            Some(Finding::Hidden) => self.meta("‹a line holding a secret is not shown›"),
            _ => self.output(source, text),
        }
        match finding {
            Some(Finding::Milestone(milestone)) => self.advance(milestone),
            Some(Finding::TunnelIp(ip)) => self.tunnel_ip = Some(ip),
            Some(Finding::CertificateRejected(value)) => {
                self.certificate_rejected_at.get_or_insert(self.furthest);
                self.rejected_certificates.push(value);
            }
            Some(Finding::TunnelModeRejected) => self.tunnel_mode_rejected = true,
            Some(Finding::Session(session)) => {
                self.session = Some(session);
                self.advance(Milestone::Authenticated);
            }
            Some(Finding::Hidden) | None => {}
        }
    }

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
            self.tunnel_up_at = Some(os::time_including_sleep());
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

    /// A prompt for what was already sent means the gateway rejected it. The client would wait
    /// for an answer it never gets, so the run ends at once rather than at the stage's limit.
    fn prompt(&mut self, prompt: Prompt) {
        let answering = self.running() && self.stop.is_none();
        match prompt {
            Prompt::Password if answering && self.password.is_none() => {
                self.meta("‹password asked again: the gateway rejected the login›");
                self.stop(StopReason::PromptRepeated);
            }
            Prompt::Otp if answering && self.otp_submitted => {
                self.meta("‹OTP asked again: the gateway rejected the OTP›");
                self.stop(StopReason::PromptRepeated);
            }
            Prompt::Password => {
                let expected = answering && self.interpreter.password_expected(self.furthest);
                match self.password.take_if(|_| expected) {
                    Some(line) => {
                        if self.write_stdin(&line) {
                            self.meta("‹password sent›");
                        }
                    }
                    None => self.meta("‹password prompt ignored: not expected at this stage›"),
                }
            }
            Prompt::Otp => self.advance(Milestone::OtpPrompted),
        }
    }

    fn submit_otp(&mut self, code: &str) -> Result<(), HelperError> {
        let client = self.client_name;
        let refusal = if !self.running() || self.stop.is_some() {
            Some(format!("{client} is not running"))
        } else if self.otp_submitted {
            Some("an OTP was already submitted for this attempt".to_owned())
        } else if self.furthest != Milestone::OtpPrompted {
            Some(format!("{client} is not waiting for an OTP"))
        } else {
            self.otp_submitted = true;
            let mut line = Zeroizing::new(Vec::with_capacity(code.len() + 1));
            line.extend_from_slice(code.as_bytes());
            line.push(b'\n');
            let written = self.write_stdin(&line);
            // The client reads nothing after the OTP.
            self.stdin = None;
            (!written).then(|| format!("{client} exited before the OTP could be written"))
        };
        if let Some(message) = refusal {
            return Err(HelperError {
                code: HelperErrorCode::WrongPhase,
                message,
            });
        }
        self.meta("‹OTP sent›");
        self.advance(Milestone::OtpSubmitted);
        Ok(())
    }

    /// Writes all of `bytes`, a few hundred at most, so the pipe never fills. On failure closes
    /// stdin, logs why and returns false.
    fn write_stdin(&mut self, bytes: &[u8]) -> bool {
        let Some(stdin) = self.stdin.as_mut() else {
            return false;
        };
        let Err(error) = stdin.write_all(bytes) else {
            return true;
        };
        self.stdin = None;
        let text = if error.kind() == io::ErrorKind::BrokenPipe {
            format!(
                "‹input failed: {} exited before reading it (EPIPE)›",
                self.client_name
            )
        } else {
            let errno = error
                .raw_os_error()
                .expect("a pipe write fails with an errno");
            format!("‹input failed: {}›", describe_errno(errno))
        };
        self.meta(text);
        false
    }

    // Time limits and stopping

    fn stage_expired(&mut self) {
        let Some(stage) = self.stage.take() else {
            return;
        };
        self.meta(format!(
            "‹watchdog: {} did not finish within {}›",
            stage.name,
            seconds(stage.limit)
        ));
        self.stop(StopReason::Watchdog);
    }

    /// Between the login run and the tunnel run only the reason is kept, so the tunnel run does not
    /// start.
    fn stop(&mut self, reason: StopReason) {
        if self.stop.is_some() {
            return;
        }
        let done = match &self.phase {
            Phase::Running(child) => {
                let (done, forced) = child.terminate();
                self.forced = forced;
                self.kill_deadline = Some(Instant::now() + self.grace_period);
                done
            }
            Phase::Draining { .. } if self.tunnel_pending() => "tunnel run not started",
            _ => return,
        };
        self.stop = Some(reason);
        self.stage = None;
        let raw = serde_json::to_value(reason).expect("a stop reason serializes");
        let raw = raw
            .as_str()
            .expect("a stop reason serializes as its raw value");
        self.meta(format!("‹{done} (reason: {raw})›"));
    }

    fn grace_period_expired(&mut self) {
        self.kill_deadline = None;
        let Phase::Running(child) = &self.phase else {
            return;
        };
        let done = child.kill();
        self.forced = true;
        self.meta(format!(
            "‹{done}: still running {} after {}›",
            seconds(self.grace_period),
            os::STOP_REQUEST
        ));
    }

    // Exit

    fn exited(&mut self) {
        let Phase::Running(child) = mem::replace(&mut self.phase, Phase::Reported) else {
            unreachable!("the client exits once")
        };
        // Before the pid can be reused.
        self.process_id.store(0, Ordering::Relaxed);
        let status = child.reap();
        self.phase = Phase::Draining {
            status,
            exited_at: os::time_including_sleep(),
            deadline: Instant::now() + self.timing.pipe_drain,
        };
        self.stdin = None;
        self.password = None;
        // The tunnel run's stage goes on from the login run's.
        if !self.tunnel_pending() {
            self.stage = None;
        }
        self.kill_deadline = None;
        self.finish_if_drained();
    }

    /// A login run printed its session, or may still, and the tunnel run has not started.
    fn tunnel_pending(&self) -> bool {
        self.later_runs.is_some() && !self.tunnel_started
    }

    fn finish_if_drained(&mut self) {
        if matches!(self.phase, Phase::Draining { .. }) && !self.stdout.open && !self.stderr.open {
            self.finish(false);
        }
    }

    /// The client is gone, but something it started still holds a pipe. Takes the output already
    /// read and stops waiting.
    fn drain_expired(&mut self, inputs: &Receiver<Input>) {
        while let Ok(input) = inputs.try_recv() {
            self.handle(input);
        }
        if matches!(self.phase, Phase::Draining { .. }) {
            self.finish(true);
        }
    }

    fn finish(&mut self, pipes_still_open: bool) {
        let Phase::Draining {
            status, exited_at, ..
        } = self.phase
        else {
            unreachable!("a run finishes once, after the client exited")
        };
        self.wake_readers = None;
        for source in [OutputSource::Stdout, OutputSource::Stderr] {
            if let Some(text) = self.stream(source).lines.finish() {
                self.line(source, text);
            }
        }
        if pipes_still_open {
            self.meta(format!(
                "‹output pipes still open after {}; not waiting any longer›",
                seconds(self.timing.pipe_drain)
            ));
        }
        if !matches!(status, ProcessExitStatus::SpawnFailed { .. }) {
            self.meta(format!("‹exited: {}›", describe(status)));
        }
        if self.tunnel_pending() && self.session.is_some() && self.stop.is_none() {
            self.start_tunnel();
            return;
        }
        self.stage = None;
        let facts = ExitFacts {
            furthest_milestone: Some(self.furthest),
            stop_initiator: match self.stop {
                Some(reason) => StopInitiator::Helper { reason },
                None => StopInitiator::Client {},
            },
            cert_rejected_digests: mem::take(&mut self.rejected_certificates),
            cert_rejected_at: self.certificate_rejected_at,
            tunnel_mode_rejected: self.tunnel_mode_rejected,
            up_duration: self.tunnel_up_at.map(|up| exited_at.saturating_sub(up)),
            exit_status: status,
        };
        // A tunnel run that ended by itself or after the stop request logged out on its own.
        let logout_needed = self.session.is_some() && (!self.tunnel_started || self.forced);
        match (self.later_runs.take(), self.session.take()) {
            (Some(later_runs), Some(session)) if logout_needed => {
                self.flush_batch();
                self.phase = Phase::LoggingOut(facts);
                let inputs = self.inputs.clone();
                thread::Builder::new()
                    .name("fvm-watchdog-logout".to_owned())
                    .spawn(move || {
                        let text = send_logout(&later_runs.logout, &session.logout);
                        let _ = inputs.send(Input::LoggedOut(text));
                    })
                    .expect("starting the watchdog's logout thread");
            }
            _ => self.repair_routes_or_report(facts),
        }
    }

    /// Starts the tunnel run with the session on its stdin, as the run's next process.
    fn start_tunnel(&mut self) {
        let launch = self
            .later_runs
            .as_ref()
            .expect("a tunnel run follows a login run")
            .tunnel
            .clone();
        let input = self
            .session
            .as_ref()
            .expect("the login run printed a session")
            .tunnel
            .clone();
        let command = command_line(&launch);
        self.tunnel_started = true;
        self.forced = false;
        self.generation += 1;
        let spawned = match os::spawn(&launch) {
            Ok(spawned) => spawned,
            Err(errno) => {
                self.meta(format!(
                    "‹spawn failed: {}: {}›",
                    launch.program.display(),
                    describe_errno(errno)
                ));
                self.forced = true;
                self.phase = Phase::Draining {
                    status: ProcessExitStatus::SpawnFailed { errno },
                    exited_at: os::time_including_sleep(),
                    deadline: Instant::now(),
                };
                self.finish(false);
                return;
            }
        };
        self.process_id.store(spawned.child.id(), Ordering::Relaxed);
        let (child, stdin, wake_write) = watch_process(spawned, self.generation, &self.inputs);
        let pid = child.id();
        self.phase = Phase::Running(child);
        self.stdin = Some(stdin);
        self.stdout = Stream::new(self.timing.line_idle_flush);
        self.stderr = Stream::new(self.timing.line_idle_flush);
        self.wake_readers = Some(wake_write);
        self.meta(format!("‹tunnel run started: pid {pid}, {command}›"));
        self.client_started();
        // The client reads nothing after the session.
        self.write_stdin(&input);
        self.stdin = None;
    }

    fn client_started(&mut self) {
        self.flush_batch();
        (self.on_report)(Report::ClientStarted);
    }

    fn logged_out(&mut self, text: String) {
        let Phase::LoggingOut(facts) = mem::replace(&mut self.phase, Phase::Reported) else {
            unreachable!("the logout request answers only when sent")
        };
        self.meta(text);
        self.repair_routes_or_report(facts);
    }

    fn repair_routes_or_report(&mut self, facts: ExitFacts) {
        if self.interpreter.routes_left_unrestored() {
            self.flush_batch();
            self.phase = Phase::RepairingRoutes(facts);
            (self.on_report)(Report::RepairRoutes(RouteRepair(Some(self.inputs.clone()))));
        } else {
            self.report_exit(facts);
        }
    }

    fn routes_repaired(&mut self, outcome: RouteRepairOutcome) {
        let Phase::RepairingRoutes(facts) = mem::replace(&mut self.phase, Phase::Reported) else {
            unreachable!("routes are repaired only on request")
        };
        self.meta(match outcome {
            RouteRepairOutcome::Repaired => "‹routes repaired›",
            RouteRepairOutcome::NotNeeded => "‹routes checked: no repair needed›",
            RouteRepairOutcome::Failed => "‹route repair failed — see the Helper log›",
        });
        self.report_exit(facts);
    }

    fn report_exit(&mut self, facts: ExitFacts) {
        self.phase = Phase::Reported;
        self.event(HelperEventKind::Exited { facts });
    }
}

fn describe(status: ProcessExitStatus) -> String {
    match status {
        ProcessExitStatus::Exited { code } => format!("code {code}"),
        ProcessExitStatus::Signaled { signal } => match os::signal_name(signal) {
            Some(name) => format!("signal {signal} ({name})"),
            None => format!("signal {signal}"),
        },
        ProcessExitStatus::SpawnFailed { .. } => {
            unreachable!("a client that never started has no exit line")
        }
        ProcessExitStatus::Unknown {} => "unknown".to_owned(),
    }
}

/// How long the logout request may take before it is killed; longer than its own time limit.
const LOGOUT_LIMIT: Duration = Duration::from_secs(15);
/// Of the logout request's output, only so much of the first line goes to the log.
const LOGOUT_OUTPUT_BYTES: u64 = 256;

/// Runs the logout request with `input` on its stdin and returns the log line about it: its first
/// line of output, or why there is none.
fn send_logout(launch: &LaunchSpec, input: &[u8]) -> String {
    let spawned = match os::spawn(launch) {
        Ok(spawned) => spawned,
        Err(errno) => {
            return format!(
                "‹logout request not sent: {}: {}›",
                launch.program.display(),
                describe_errno(errno)
            );
        }
    };
    let Spawned {
        child,
        mut stdin,
        stdout,
        stderr,
        ..
    } = spawned;
    // A few hundred bytes fit in the pipe, so this never waits for the request to read them. A
    // request that exited at once is reported by its exit status.
    let _ = stdin.write_all(input);
    drop(stdin);
    let output = thread::spawn(move || read_some(stdout));
    let errors = thread::spawn(move || read_some(stderr));
    let (exited, exit_seen) = mpsc::channel();
    let exit = child.exit_watch();
    thread::spawn(move || {
        exit.wait();
        let _ = exited.send(());
    });
    let timed_out = exit_seen.recv_timeout(LOGOUT_LIMIT).is_err();
    if timed_out {
        child.kill();
    }
    let status = child.reap();
    let output = output
        .join()
        .expect("the logout output reader never panics");
    drop(errors);
    let output = String::from_utf8_lossy(&output);
    let answer = output.lines().next().map_or("", str::trim);
    if timed_out {
        format!(
            "‹logout request did not finish within {}; killed›",
            seconds(LOGOUT_LIMIT)
        )
    } else if status == (ProcessExitStatus::Exited { code: 0 }) {
        format!("‹logout request sent: {answer}›")
    } else {
        format!("‹logout request failed: {answer} ({})›", describe(status))
    }
}

/// The start of what `pipe` carries until it ends.
fn read_some(pipe: PipeReader) -> Vec<u8> {
    let mut bytes = Vec::new();
    let _ = pipe.take(LOGOUT_OUTPUT_BYTES).read_to_end(&mut bytes);
    bytes
}

/// The OS's text and the number, as the Log tab shows OS errors: "No such file or directory
/// (errno 2)". On Windows the number is a Windows error code.
fn describe_errno(errno: i32) -> String {
    let shown = io::Error::from_raw_os_error(errno).to_string();
    let text = shown
        .strip_suffix(&format!(" (os error {errno})"))
        .expect("std shows an OS error as its text and the code");
    format!("{text} (errno {errno})")
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
