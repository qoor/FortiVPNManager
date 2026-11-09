//! The runtime's one queue: every input is taken in turn, the judgment core decides,
//! and its effects are carried out before the next input. Work that waits on the outside, such as a
//! port answer or a network reply, comes back as another input.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::SystemTime;

use fvm_contract::app_helper::{HelperError, HelperEvent, OutputLine, OutputSource, RunRequest};
use fvm_contract::app_internal::event::SystemEvent;
use fvm_contract::app_internal::port::{
    HelperCall, LogCategory, LogLevel, PortAction, PortRequest, PortResponse, SecretItem,
};
use fvm_contract::app_internal::section::Section;
use fvm_contract::app_internal::{
    AppFacts, FromRuntime, LogLine, LogSource, SecretText, ToRuntime,
};
use fvm_core::connection::{self, ConnectionManager, Effect, Launch, Note, Startup, TimerId};
use jiff::tz::TimeZone;

use crate::config_file::ConfigFile;
use crate::log_file::{self, LogFile};
use crate::state_file::StateFile;
use crate::timers::Timers;

/// One thing for the runtime to take in turn.
pub(crate) enum Input {
    Swift(ToRuntime),
    Timer(TimerId),
    /// Work that finished elsewhere, carried on in the queue.
    Done(Box<dyn FnOnce(&mut Runtime) + Send>),
}

type PortReply = Box<dyn FnOnce(&mut Runtime, PortResponse)>;

/// Puts work on the runtime's queue from any thread or task.
#[derive(Clone)]
pub(crate) struct Queue(Sender<Input>);

impl Queue {
    pub(crate) fn run(&self, work: impl FnOnce(&mut Runtime) + Send + 'static) {
        // The queue lives as long as the process.
        let _ = self.0.send(Input::Done(Box::new(work)));
    }

    /// Runs `work` on the queue and waits for the answer it hands to its reply.
    pub(crate) async fn ask<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Runtime, Box<dyn FnOnce(T) + Send>) + Send + 'static,
    ) -> T {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.run(move |runtime| {
            work(
                runtime,
                Box::new(move |value| {
                    let _ = reply.send(value);
                }),
            );
        });
        answer.await.expect("the runtime answers every question")
    }
}

/// Where the runtime keeps its files.
pub(crate) struct Paths {
    pub(crate) config: PathBuf,
    pub(crate) state: PathBuf,
    pub(crate) log: PathBuf,
    pub(crate) home: PathBuf,
}

pub(crate) struct Runtime {
    pub(crate) facts: AppFacts,
    pub(crate) time_zone: TimeZone,
    pub(crate) home: PathBuf,
    pub(crate) cm: ConnectionManager,
    /// The account the core was last given.
    applied_account: connection::VpnAccount,
    pub(crate) config: ConfigFile,
    pub(crate) state: StateFile,
    log: LogFile,
    out: Sender<FromRuntime>,
    pub(crate) inputs: Sender<Input>,
    timers: Timers,
    pub(crate) tasks: tokio::runtime::Runtime,
    next_port_id: u64,
    port_replies: HashMap<u64, PortReply>,
    /// Tunnel calls go one at a time; the next waits for the previous answer.
    helper_busy: bool,
    helper_queue: VecDeque<(HelperCall, PortReply)>,
    pending_lines: Vec<LogLine>,
    sent: HashMap<&'static str, Section>,
    open_settings: bool,
    pub(crate) forms: crate::account_form::Forms,
    pub(crate) options: crate::commands::Options,
    pub(crate) gmail: crate::otp_session::Gmail,
    pub(crate) google: crate::google_account::Google,
    pub(crate) bypass: crate::bypass_sync::Bypass,
}

/// Takes the inputs until the process ends. The first message from Swift is always the launch.
pub(crate) fn run(
    paths: Paths,
    inputs: Sender<Input>,
    received: Receiver<Input>,
    out: Sender<FromRuntime>,
) {
    let facts = match received.recv().expect("the input queue stays open") {
        Input::Swift(ToRuntime::Launched { facts }) => facts,
        _ => panic!("the first message from Swift is the launch"),
    };
    let mut runtime = Runtime::launch(facts, paths, inputs, out);
    runtime.publish();
    for input in received {
        runtime.take(input);
        runtime.publish();
    }
}

impl Runtime {
    fn launch(
        facts: AppFacts,
        paths: Paths,
        inputs: Sender<Input>,
        out: Sender<FromRuntime>,
    ) -> Self {
        let time_zone = TimeZone::get(&facts.time_zone).unwrap_or(TimeZone::UTC);
        let mut config = ConfigFile::new(paths.config);
        config.poll();
        let (state, state_problem) = StateFile::open(paths.state);
        let mut attempt_seed = [0u8; 16];
        getrandom::fill(&mut attempt_seed).expect("random bytes from the OS");
        let settings = core_settings(config.settings());
        let applied_account = settings.account.clone();
        let cm = ConnectionManager::new(Startup {
            app_build: facts.app_build.clone(),
            attempt_seed,
            settings,
            time_zone: time_zone.clone(),
        });
        let timer_inputs = inputs.clone();
        let timers = Timers::start(move |timer| {
            let _ = timer_inputs.send(Input::Timer(timer));
        });
        let tasks = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("fvm-runtime-io")
            .enable_all()
            .build()
            .expect("starting the runtime's I/O threads");
        let mut runtime = Self {
            time_zone,
            home: paths.home,
            cm,
            applied_account,
            config,
            state,
            log: LogFile::new(paths.log),
            out,
            inputs,
            timers,
            tasks,
            next_port_id: 0,
            port_replies: HashMap::new(),
            helper_busy: false,
            helper_queue: VecDeque::new(),
            pending_lines: Vec::new(),
            sent: HashMap::new(),
            open_settings: false,
            forms: crate::account_form::Forms::default(),
            options: crate::commands::Options::new(facts.installed_copy),
            gmail: crate::otp_session::Gmail::default(),
            google: crate::google_account::Google::default(),
            bypass: crate::bypass_sync::Bypass::default(),
            facts,
        };
        if let Some(problem) = state_problem {
            runtime.system_log(LogLevel::Error, LogCategory::State, problem);
        }
        runtime.start_polling_config();
        runtime.register_login_item_once();
        runtime.import_on_first_launch(|runtime, import_report_shown| {
            runtime
                .forms
                .reset_account(&runtime.config.settings().clone());
            runtime.look_up_stored_secrets();
            let launch = Launch {
                auto_connect: runtime.config.settings().connection.auto_connect_on_launch,
                restored_ledger: runtime.state.state().ledger.clone(),
                import_report_shown,
            };
            runtime.feed(connection::Input::Launched(launch));
            runtime.sync_bypass_routes();
        });
        runtime
    }

    fn take(&mut self, input: Input) {
        match input {
            Input::Swift(ToRuntime::Launched { .. }) => panic!("Swift launches the runtime once"),
            Input::Swift(ToRuntime::Command { command }) => self.command(command),
            Input::Swift(ToRuntime::PortResponse { id, response }) => {
                let reply = self
                    .port_replies
                    .remove(&id)
                    .expect("an answer to a request the runtime sent");
                reply(self, response);
            }
            Input::Swift(ToRuntime::Event { event }) => self.system_event(event),
            Input::Timer(timer) => self.feed(connection::Input::TimerFired(timer)),
            Input::Done(work) => work(self),
        }
    }

    // MARK: Talking to Swift

    pub(crate) fn send(&self, message: FromRuntime) {
        // The writer thread lives as long as the process.
        let _ = self.out.send(message);
    }

    pub(crate) fn request(
        &mut self,
        request: PortRequest,
        reply: impl FnOnce(&mut Runtime, PortResponse) + 'static,
    ) {
        self.next_port_id += 1;
        let id = self.next_port_id;
        self.port_replies.insert(id, Box::new(reply));
        self.send(FromRuntime::PortRequest { id, request });
    }

    pub(crate) fn action(&self, action: PortAction) {
        self.send(FromRuntime::PortAction { action });
    }

    pub(crate) fn system_log(
        &self,
        level: LogLevel,
        category: LogCategory,
        message: impl Into<String>,
    ) {
        self.action(PortAction::Log {
            level,
            category,
            message: message.into(),
        });
    }

    /// A handle that puts work on this queue from any thread.
    pub(crate) fn queue(&self) -> Queue {
        Queue(self.inputs.clone())
    }

    /// A line for the Log tab and the log file, sent after the current input.
    pub(crate) fn log_line(&mut self, line: LogLine) {
        self.pending_lines.push(line);
    }

    pub(crate) fn note(&mut self, text: impl Into<String>) {
        self.log_line(log_file::note(text));
    }

    // MARK: Helper calls

    /// Calls the Helper. Tunnel calls wait for the previous one's answer;
    /// route calls go at once.
    pub(crate) fn call_helper(
        &mut self,
        call: HelperCall,
        reply: impl FnOnce(&mut Runtime, PortResponse) + 'static,
    ) {
        if matches!(
            call,
            HelperCall::ReadBypassRoutes {} | HelperCall::WriteBypassRoutes { .. }
        ) {
            self.request(PortRequest::CallHelper { call }, reply);
            return;
        }
        self.helper_queue.push_back((call, Box::new(reply)));
        self.next_helper_call();
    }

    fn next_helper_call(&mut self) {
        if self.helper_busy {
            return;
        }
        let Some((call, reply)) = self.helper_queue.pop_front() else {
            return;
        };
        self.helper_busy = true;
        self.request(
            PortRequest::CallHelper { call },
            move |runtime, response| {
                runtime.helper_busy = false;
                reply(runtime, response);
                runtime.next_helper_call();
            },
        );
    }

    // MARK: The judgment core

    pub(crate) fn feed(&mut self, input: connection::Input) {
        let effects = self.cm.handle(input, SystemTime::now());
        for effect in effects {
            self.carry_out(effect);
        }
    }

    fn carry_out(&mut self, effect: Effect) {
        match effect {
            Effect::CheckReadiness => self.check_readiness(),
            Effect::OpenOtpSession { attempt } => self.open_otp_session(attempt),
            Effect::SearchOtp {
                attempt,
                prompted_at,
            } => self.search_otp(&attempt, prompted_at),
            Effect::CloseOtpSession {
                attempt,
                check_spam,
            } => self.close_otp_session(&attempt, check_spam),
            Effect::StartRun { attempt, account } => self.start_run(attempt, account),
            Effect::SubmitOtp { attempt, code } => {
                // The used mail go to the trash once the code is with the Helper.
                self.finish_otp_session(&attempt);
                self.call_helper(
                    HelperCall::SubmitOtp { attempt, code },
                    |runtime, response| {
                        runtime.log_helper_refusal("submitting the OTP", &response);
                    },
                );
            }
            Effect::StopRun { attempt, reason } => {
                self.call_helper(
                    HelperCall::Disconnect { attempt, reason },
                    |runtime, response| {
                        runtime.log_helper_refusal("stopping the run", &response);
                    },
                );
            }
            Effect::SaveTrustedCerts(certs) => self.save_trusted_certs(certs),
            Effect::SaveLedger(ledger) => {
                if let Err(problem) = self.state.update(|state| state.ledger = Some(ledger)) {
                    self.system_log(LogLevel::Error, LogCategory::State, problem);
                }
            }
            Effect::SetTimer { timer, at } => self.timers.set(timer, at),
            Effect::CancelTimer(kind) => self.timers.cancel(kind),
            Effect::HoldActivity(hold) => self.action(PortAction::HoldActivity { hold }),
            Effect::ReadRouter => {
                self.request(PortRequest::PrimaryRouter {}, |runtime, response| {
                    let PortResponse::PrimaryRouter { router } = response else {
                        panic!("a router answer")
                    };
                    runtime.feed(connection::Input::RouterRead {
                        present: router.is_some(),
                    });
                })
            }
            Effect::ProbeNetwork(probe) => self.probe_network(probe),
            Effect::ReadScreenState => {
                self.request(PortRequest::ScreenState {}, |runtime, response| {
                    let PortResponse::ScreenState {
                        locked,
                        displays_asleep,
                    } = response
                    else {
                        panic!("a screen state answer")
                    };
                    runtime.feed(connection::Input::ScreenState {
                        locked,
                        displays_asleep,
                    });
                })
            }
            Effect::Note(note) => {
                let text = match note {
                    Note::ScheduledHoursStarted => "‹schedule: scheduled hours started, connecting›".to_owned(),
                    Note::ReconnectHeldWhileAway => {
                        "‹schedule: outside the scheduled hours and away, the reconnect waits until you return›".to_owned()
                    }
                    Note::ReconnectsSuspended { away_minutes } => format!(
                        "‹schedule: away for {away_minutes} min outside the scheduled hours, automatic reconnects stopped›"
                    ),
                };
                self.note(text);
            }
            Effect::OpenSettings => self.open_settings = true,
            Effect::ApproveQuit => {
                self.flush_log_lines();
                self.log.flush();
                self.action(PortAction::ApproveQuit {});
            }
        }
    }

    fn start_run(
        &mut self,
        attempt: fvm_contract::app_helper::AttemptId,
        account: connection::VpnAccount,
    ) {
        let item = SecretItem::VpnPassword {
            username: account.username.clone(),
            host: account.host.clone(),
        };
        self.request(
            PortRequest::ReadSecret { item },
            move |runtime, response| {
                let password = match response {
                    PortResponse::Secret {
                        value: Some(SecretText(password)),
                    } => password,
                    _ => {
                        // The readiness check found it moments ago; treat it like a Helper refusal.
                        runtime.feed(connection::Input::RunRejected(
                            attempt,
                            fvm_contract::app_helper::HelperErrorCode::InternalError,
                        ));
                        return;
                    }
                };
                let request: RunRequest = account.run_request(attempt.clone(), password);
                let json =
                    SecretText(serde_json::to_string(&request).expect("a run request encodes"));
                runtime.call_helper(HelperCall::Connect { json }, move |runtime, response| {
                    if let Some(code) = helper_failure(runtime, "connect", &response) {
                        runtime.feed(connection::Input::RunRejected(attempt, code));
                    }
                });
            },
        );
    }

    fn log_helper_refusal(&mut self, what: &str, response: &PortResponse) {
        if let PortResponse::HelperReply { json: Some(json) } = response
            && let Ok(error) = serde_json::from_str::<HelperError>(json)
        {
            self.system_log(
                LogLevel::Error,
                LogCategory::Connection,
                format!("{what} refused: {}", error.message),
            );
        }
    }

    fn save_trusted_certs(&mut self, certs: Vec<String>) {
        if !self.config.exists() {
            self.system_log(
                LogLevel::Error,
                LogCategory::Config,
                "not pinning certificates: config.yaml does not exist",
            );
            return;
        }
        if let Err(error) = self
            .config
            .update(|settings| settings.vpn.trusted_certs = certs.clone())
        {
            self.system_log(
                LogLevel::Error,
                LogCategory::Config,
                format!("saving the server certificate failed: {error:?}"),
            );
        }
        self.settings_changed();
    }

    /// Hands the settings in use to the core after the file changed. A new server address or port
    /// drops the certificate values the old gateway left, unless they changed with it.
    pub(crate) fn settings_changed(&mut self) {
        let mut settings = core_settings(self.config.settings());
        if connection::gateway_change_drops_certs(&self.applied_account, &settings.account)
            && !settings.account.trusted_certs.is_empty()
        {
            match self
                .config
                .update(|settings| settings.vpn.trusted_certs.clear())
            {
                Ok(_) => settings = core_settings(self.config.settings()),
                Err(error) => self.system_log(
                    LogLevel::Error,
                    LogCategory::Config,
                    format!("clearing the old gateway's certificates failed: {error:?}"),
                ),
            }
        }
        self.applied_account = settings.account.clone();
        self.feed(connection::Input::SettingsChanged(settings));
    }

    // MARK: System events

    fn system_event(&mut self, event: SystemEvent) {
        let input = match event {
            SystemEvent::WillSleep { at } => connection::Input::WillSleep { at },
            SystemEvent::DidWake {} => {
                self.timers.nudge();
                connection::Input::DidWake
            }
            SystemEvent::ScreenLocked { at } => connection::Input::ScreenLocked { at },
            SystemEvent::ScreenUnlocked {} => connection::Input::ScreenUnlocked,
            SystemEvent::DisplaysSlept { at } => connection::Input::DisplaysSlept { at },
            SystemEvent::DisplaysWoke {} => connection::Input::DisplaysWoke,
            SystemEvent::NetworkPathChanged {
                satisfied,
                interfaces,
            } => connection::Input::NetworkPathChanged {
                satisfied,
                interfaces,
            },
            SystemEvent::NetworkSettingsChanged {} => {
                self.sync_bypass_routes_soon();
                return;
            }
            SystemEvent::ClockChanged {} => {
                self.timers.nudge();
                return;
            }
            SystemEvent::TimeZoneChanged { time_zone } => {
                self.time_zone = TimeZone::get(&time_zone).unwrap_or(TimeZone::UTC);
                self.timers.nudge();
                connection::Input::TimeZoneChanged(self.time_zone.clone())
            }
            SystemEvent::AppActivated {} => {
                self.poll_config();
                connection::Input::AppActivated
            }
            SystemEvent::QuitRequested {} => connection::Input::QuitRequested,
            SystemEvent::HelperEvent { json } => connection::Input::Helper(
                serde_json::from_str::<HelperEvent>(&json).expect("a Helper event of this build"),
            ),
            SystemEvent::HelperOutput { json } => {
                let lines: Vec<OutputLine> =
                    serde_json::from_str(&json).expect("Helper output of this build");
                for line in lines {
                    let source = match line.source {
                        OutputSource::Stdout => LogSource::Stdout,
                        OutputSource::Stderr => LogSource::Stderr,
                        OutputSource::Helper => LogSource::Note,
                    };
                    self.log_line(LogLine {
                        time: line.date,
                        source,
                        text: line.text,
                    });
                }
                return;
            }
            SystemEvent::HelperConnectionLost {} => connection::Input::HelperConnectionLost,
        };
        self.feed(input);
    }

    // MARK: Config file

    fn start_polling_config(&self) {
        let queue = self.queue();
        std::thread::Builder::new()
            .name("fvm-runtime-config-poll".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(crate::config_file::POLL_INTERVAL);
                    queue.run(Runtime::poll_config);
                }
            })
            .expect("starting the config poll thread");
    }

    pub(crate) fn poll_config(&mut self) {
        if self.config.poll() {
            self.config_reloaded();
        }
    }

    /// The file changed outside the app, or the app wrote it.
    pub(crate) fn config_reloaded(&mut self) {
        self.forms.merge_account(self.config.settings());
        self.settings_changed();
        self.sync_bypass_routes();
    }

    // MARK: Output

    /// Sends the log lines of the current input, then the sections that changed, then a pending
    /// request to show Settings, which the UI answers from those sections.
    fn publish(&mut self) {
        self.flush_log_lines();
        for section in self.sections() {
            let key = section_key(&section);
            if self.sent.get(key) != Some(&section) {
                self.sent.insert(key, section.clone());
                self.send(FromRuntime::Section { section });
            }
        }
        if std::mem::take(&mut self.open_settings) {
            self.send(FromRuntime::OpenSettings {});
        }
    }

    fn flush_log_lines(&mut self) {
        if self.pending_lines.is_empty() {
            return;
        }
        let lines = std::mem::take(&mut self.pending_lines);
        if let Some(problem) = self.log.append(&lines, &self.time_zone) {
            self.system_log(LogLevel::Error, LogCategory::Logfile, problem);
        }
        self.send(FromRuntime::LogLines { lines });
    }

    pub(crate) fn log_file_path(&self) -> String {
        self.log.path().display().to_string()
    }
}

fn section_key(section: &Section) -> &'static str {
    match section {
        Section::Connection { .. } => "connection",
        Section::Schedule { .. } => "schedule",
        Section::Readiness { .. } => "readiness",
        Section::Gmail { .. } => "gmail",
        Section::BypassRules { .. } => "bypass",
        Section::Config { .. } => "config",
    }
}

/// The Helper's refusal of a call, or `None` when it accepted it. A Helper that cannot be reached
/// or is another build counts as an internal error, which the core retries like a network failure.
pub(crate) fn helper_failure(
    runtime: &mut Runtime,
    what: &str,
    response: &PortResponse,
) -> Option<fvm_contract::app_helper::HelperErrorCode> {
    use fvm_contract::app_helper::HelperErrorCode;
    match response {
        PortResponse::HelperReply { json: None } => None,
        PortResponse::HelperReply { json: Some(json) } => {
            match serde_json::from_str::<HelperError>(json) {
                Ok(error) => {
                    runtime.note(format!("‹{what} rejected: {}›", error.message));
                    Some(error.code)
                }
                Err(_) => Some(HelperErrorCode::InternalError),
            }
        }
        PortResponse::HelperUnreachable { failure } => {
            runtime.system_log(
                LogLevel::Error,
                LogCategory::Connection,
                format!("Helper unreachable: {failure:?}"),
            );
            Some(HelperErrorCode::InternalError)
        }
        PortResponse::HelperBuildMismatch { helper_build } => {
            runtime.system_log(
                LogLevel::Error,
                LogCategory::Connection,
                format!("Helper build {helper_build} differs from the app's"),
            );
            Some(HelperErrorCode::InternalError)
        }
        other => panic!("a Helper call answered with {other:?}"),
    }
}

/// What the core needs from the settings file.
pub(crate) fn core_settings(settings: &fvm_core::config::Settings) -> connection::Settings {
    let connection = &settings.connection;
    connection::Settings {
        account: connection::VpnAccount {
            host: settings.vpn.host.clone(),
            port: u16::try_from(settings.vpn.port)
                .expect("the settings file checks the port range"),
            username: settings.vpn.username.clone(),
            realm: settings.vpn.realm.clone(),
            trusted_certs: settings.vpn.trusted_certs.clone(),
        },
        connection: connection::ConnectionSettings {
            session_max_minutes: minutes(connection.session_max_minutes),
            proactive_reconnect: connection.proactive_reconnect.enabled,
            proactive_margin_minutes: minutes(connection.proactive_reconnect.margin_minutes),
            reconnect_after_sleep: connection.reconnect_after_sleep.enabled,
            min_sleep_minutes: minutes(connection.reconnect_after_sleep.min_sleep_minutes),
            schedule: connection.schedule.enabled.then(|| connection::Schedule {
                days: connection.schedule.days.clone(),
                start: connection.schedule.start,
                end: connection.schedule.end,
                away_minutes: minutes(connection.schedule.away_minutes),
            }),
        },
        otp_method: settings.otp.method,
    }
}

fn minutes(value: i64) -> u32 {
    u32::try_from(value).expect("the settings file checks the minute ranges")
}
