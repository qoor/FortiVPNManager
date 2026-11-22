//! The runtime's one queue: every input is taken in turn, the judgment core decides,
//! and its effects are carried out before the next input. Work that waits on the outside, such as a
//! port answer or a network reply, comes back as another input.

use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::SystemTime;

use fvm_contract::app_helper::{
    AttemptId, HelperError, HelperEvent, OutputBatch, OutputSource, RunRequest, SessionId,
};
use fvm_contract::app_internal::event::SystemEvent;
use fvm_contract::app_internal::port::{
    HelperCall, LogCategory, LogLevel, PortAction, PortRequest, PortResponse,
};
use fvm_contract::app_internal::section::Section;
use fvm_contract::app_internal::{
    AppFacts, EpochMillis, FromRuntime, LogLine, LogSource, SecretText, ToRuntime,
};
use fvm_core::config;
use fvm_core::connection::{self, ConnectionManager, Effect, Launch, Note, Startup};
use jiff::tz::TimeZone;

use crate::config_file::ConfigFile;
use crate::log_file::{self, LogFile};
use crate::os::{OsRequest, OsResponse, SecretItem};
use crate::state_file::StateFile;
use crate::timers::{Timer, Timers};

/// One thing for the runtime to take in turn.
pub(crate) enum Input {
    /// From the app's OS side: Swift on macOS.
    App(ToRuntime),
    Timer(Timer),
    /// Work that finished elsewhere, carried on in the queue.
    Done(Box<dyn FnOnce(&mut Runtime) + Send>),
    Os {
        id: u64,
        response: OsResponse,
    },
    /// The saved IPv4 settings of a network service or the primary IPv4 state changed; one change
    /// may be reported several times.
    NetworkSettingsChanged,
}

pub(crate) enum Outgoing {
    App(FromRuntime),
    Os { id: u64, request: OsRequest },
    WatchNetworkSettings,
}

type PortReply = Box<dyn FnOnce(&mut Runtime, PortResponse)>;
type OsReply = Box<dyn FnOnce(&mut Runtime, OsResponse)>;

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

/// Where the runtime keeps its files, and where the VPN client's settings are imported from.
pub(crate) struct Paths {
    pub(crate) config: PathBuf,
    pub(crate) state: PathBuf,
    pub(crate) log: PathBuf,
    pub(crate) update_downloads: PathBuf,
    pub(crate) client_settings: ClientSettings,
}

/// The settings "Import" and the first launch bring in.
#[derive(Clone)]
pub(crate) enum ClientSettings {
    /// openfortivpn's settings file, looked for where openfortivpn looks under `home` (macOS).
    #[cfg_attr(
        all(windows, not(test)),
        expect(dead_code, reason = "only the macOS entry and the tests choose it")
    )]
    OpenFortiVpn { home: PathBuf },
    /// The official `FortiClient`'s SSL VPN connection, which the OS integration reads (Windows):
    /// openconnect has no settings file of its own.
    #[cfg_attr(
        not(any(windows, test)),
        expect(dead_code, reason = "only the Windows entry and the tests choose it")
    )]
    FortiClient,
}

pub(crate) struct Runtime {
    pub(crate) facts: AppFacts,
    pub(crate) time_zone: TimeZone,
    pub(crate) client_settings: ClientSettings,
    pub(crate) cm: ConnectionManager,
    pub(crate) config: ConfigFile,
    pub(crate) state: StateFile,
    pub(crate) log: LogFile,
    out: Sender<Outgoing>,
    pub(crate) inputs: Sender<Input>,
    pub(crate) timers: Timers,
    pub(crate) tasks: tokio::runtime::Runtime,
    next_port_id: u64,
    port_replies: HashMap<u64, PortReply>,
    os_replies: HashMap<u64, OsReply>,
    /// Tunnel calls go one at a time; the next waits for the previous answer.
    helper_busy: bool,
    quit_approved: bool,
    helper_queue: VecDeque<(HelperCall, PortReply)>,
    pending_lines: Vec<LogLine>,
    /// An attempt the core began whose first log line is still to come. That line starts the
    /// attempt's block in the log, so a note that started the attempt or the Helper's refusal of it
    /// belongs to the block; the Helper knows of neither.
    unmarked_attempt: Option<AttemptId>,
    sessions: HashMap<AttemptId, SessionId>,
    sent: HashMap<&'static str, Section>,
    open_settings: bool,
    pub(crate) forms: crate::account_form::Forms,
    pub(crate) options: crate::commands::Options,
    pub(crate) gmail: crate::otp_session::Gmail,
    pub(crate) google: crate::google_account::Google,
    pub(crate) bypass: crate::bypass_sync::Bypass,
    pub(crate) updates: crate::auto_update::Updates,
}

/// Takes the inputs until the process ends. The first message from the app is always the launch.
pub(crate) fn run(
    paths: Paths,
    update_url: String,
    inputs: Sender<Input>,
    received: Receiver<Input>,
    out: Sender<Outgoing>,
) {
    let Input::App(ToRuntime::Launched { facts }) =
        received.recv().expect("the input queue stays open")
    else {
        panic!("the first message from the app is the launch");
    };
    let mut runtime = Runtime::launch(facts, paths, update_url, inputs, out);
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
        update_url: String,
        inputs: Sender<Input>,
        out: Sender<Outgoing>,
    ) -> Self {
        let time_zone = TimeZone::get(&facts.time_zone).unwrap_or(TimeZone::UTC);
        let mut config = ConfigFile::new(paths.config);
        config.poll();
        let (state, state_problem) = StateFile::open(paths.state);
        let mut attempt_seed = [0u8; 16];
        getrandom::fill(&mut attempt_seed).expect("random bytes from the OS");
        let settings = connection::Settings::new(config.settings());
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
            client_settings: paths.client_settings,
            cm,
            config,
            state,
            log: LogFile::new(paths.log),
            out,
            inputs,
            timers,
            tasks,
            next_port_id: 0,
            port_replies: HashMap::new(),
            os_replies: HashMap::new(),
            helper_busy: false,
            quit_approved: false,
            helper_queue: VecDeque::new(),
            pending_lines: Vec::new(),
            unmarked_attempt: None,
            sessions: HashMap::new(),
            sent: HashMap::new(),
            open_settings: false,
            forms: crate::account_form::Forms::default(),
            options: crate::commands::Options::new(facts.installed_copy),
            gmail: crate::otp_session::Gmail::default(),
            google: crate::google_account::Google::default(),
            bypass: crate::bypass_sync::Bypass::default(),
            updates: crate::auto_update::Updates::new(
                update_url,
                &facts.app_version,
                paths.update_downloads,
            ),
            facts,
        };
        if let Some(problem) = state_problem {
            runtime.system_log(LogLevel::Error, LogCategory::State, problem);
        }
        let _ = runtime.out.send(Outgoing::WatchNetworkSettings);
        let connected_before_update = runtime.take_update_record();
        runtime.check_for_updates();
        runtime.start_polling_config();
        runtime.register_login_item_once();
        runtime.import_on_first_launch(move |runtime, import_report_shown| {
            runtime
                .forms
                .reset_account(&runtime.config.settings().clone());
            runtime.look_up_stored_secrets();
            let launch = Launch {
                auto_connect: runtime.config.settings().connection.auto_connect_on_launch,
                restored_ledger: runtime.state.state().ledger.clone(),
                import_report_shown,
                connected_before_update,
            };
            runtime.feed(connection::Input::Launched(launch));
            runtime.sync_bypass_routes();
        });
        runtime
    }

    fn take(&mut self, input: Input) {
        match input {
            Input::App(ToRuntime::Launched { .. }) => panic!("the app launches the runtime once"),
            Input::App(ToRuntime::Command { command }) => self.command(command),
            Input::App(ToRuntime::PortResponse { id, response }) => {
                let reply = self
                    .port_replies
                    .remove(&id)
                    .expect("an answer to a request the runtime sent");
                reply(self, response);
            }
            Input::App(ToRuntime::Event { event }) => self.system_event(event),
            Input::Timer(Timer::Connection(timer)) => {
                self.feed(connection::Input::TimerFired(timer));
            }
            Input::Timer(Timer::UpdateCheck) => self.check_for_updates(),
            Input::Done(work) => work(self),
            Input::Os { id, response } => {
                let reply = self
                    .os_replies
                    .remove(&id)
                    .expect("an answer to a request the runtime sent");
                reply(self, response);
            }
            Input::NetworkSettingsChanged => self.sync_bypass_routes_soon(),
        }
    }

    // MARK: Talking to the app

    pub(crate) fn send(&self, message: FromRuntime) {
        // The writer thread lives as long as the process.
        let _ = self.out.send(Outgoing::App(message));
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

    pub(crate) fn ask_os(
        &mut self,
        request: OsRequest,
        reply: impl FnOnce(&mut Runtime, OsResponse) + 'static,
    ) {
        self.next_port_id += 1;
        let id = self.next_port_id;
        self.os_replies.insert(id, Box::new(reply));
        let _ = self.out.send(Outgoing::Os { id, request });
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

    /// A line for the Log pane and the log file, sent after the current input.
    pub(crate) fn log_line(&mut self, mut line: LogLine) {
        if let Some(attempt) = self.unmarked_attempt.take() {
            // An attempt that ended without a line has no block.
            line.starts_attempt = self.cm.current_attempt() == Some(&attempt);
        }
        self.pending_lines.push(line);
    }

    pub(crate) fn note(&mut self, text: impl Into<String>) {
        self.log_line(log_file::note(text));
    }

    pub(crate) fn attempt_note(&mut self, attempt: &AttemptId, text: impl Display) {
        let text = format!("{} {text}", self.sessions[attempt]);
        self.note(text);
    }

    // MARK: Helper calls

    /// Calls the Helper. Tunnel calls wait for the previous one's answer;
    /// route and update calls go at once.
    pub(crate) fn call_helper(
        &mut self,
        call: HelperCall,
        reply: impl FnOnce(&mut Runtime, PortResponse) + 'static,
    ) {
        if matches!(
            call,
            HelperCall::ReadBypassRoutes {}
                | HelperCall::WriteBypassRoutes { .. }
                | HelperCall::PrepareUpdate { .. }
                | HelperCall::InstallUpdate { .. }
                | HelperCall::ReadPreparedUpdate {}
                | HelperCall::DiscardPreparedUpdate {}
        ) {
            // An answer after the quit's approval changes nothing: the app may end at any moment,
            // and its log was written out.
            self.request(
                PortRequest::CallHelper { call },
                move |runtime, response| {
                    if !runtime.quit_approved {
                        reply(runtime, response);
                    }
                },
            );
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
        let vpn_wanted = self.cm.wants_vpn();
        let effects = self.cm.handle(input, SystemTime::now());
        for effect in effects {
            self.carry_out(effect);
        }
        if self.cm.wants_vpn() != vpn_wanted {
            self.sync_bypass_routes();
        }
    }

    fn carry_out(&mut self, effect: Effect) {
        match effect {
            Effect::AttemptBegun { attempt, session } => {
                self.sessions.insert(attempt.clone(), session);
                self.unmarked_attempt = Some(attempt);
            }
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
            Effect::TrashStaleOtp { attempt, since } => self.trash_stale_otp(&attempt, since),
            Effect::StartRun {
                attempt,
                session,
                account,
                replacing,
            } => self.start_run(attempt, session, account, replacing),
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
            Effect::SaveTrustedCerts { certs } => {
                self.save_trusted_certs(move |settings| {
                    settings.vpn.trusted_cert_pins.clone_from(&certs);
                });
            }
            Effect::ClearTrustedCerts => self.save_trusted_certs(|settings| {
                settings.vpn.trusted_cert_pins.clear();
            }),
            Effect::SaveLedger(ledger) => {
                if let Err(problem) = self.state.update(|state| state.ledger = Some(ledger)) {
                    self.system_log(LogLevel::Error, LogCategory::State, problem);
                }
            }
            Effect::SetTimer { timer, at } => self.timers.set(Timer::Connection(timer), at),
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
                });
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
                });
            }
            Effect::Note { attempt, note } => {
                let text = note_text(note);
                match attempt {
                    Some(attempt) => self.attempt_note(&attempt, text),
                    None => self.note(text),
                }
            }
            Effect::OpenSettings => self.open_settings = true,
            Effect::StoppedForUpdate => self.update_vpn_stopped(),
            Effect::ApproveQuit => self.quit_after_removing_bypass_routes(),
        }
    }

    /// Finishes the log file and lets the app quit, once.
    pub(crate) fn approve_quit(&mut self) {
        if std::mem::replace(&mut self.quit_approved, true) {
            return;
        }
        self.flush_log_lines();
        self.log.flush();
        self.action(PortAction::ApproveQuit {});
    }

    fn start_run(
        &mut self,
        attempt: fvm_contract::app_helper::AttemptId,
        session: SessionId,
        account: connection::VpnAccount,
        replacing: Option<AttemptId>,
    ) {
        let item = SecretItem::VpnPassword {
            username: account.username.clone(),
            host: account.host.clone(),
        };
        self.ask_os(OsRequest::ReadSecret { item }, move |runtime, response| {
            if !runtime.cm.wants_run(&attempt) {
                // Settled like a run the Helper could not start: the stop already decided,
                // or else the network's failure.
                runtime.feed(connection::Input::RunRejected(
                    attempt,
                    fvm_contract::app_helper::HelperErrorCode::InternalError,
                ));
                return;
            }
            let OsResponse::Secret {
                value: Some(SecretText(password)),
            } = response
            else {
                // The readiness check found it moments ago; treat it like a Helper refusal.
                runtime.feed(connection::Input::RunRejected(
                    attempt,
                    fvm_contract::app_helper::HelperErrorCode::InternalError,
                ));
                return;
            };
            let request: RunRequest =
                account.run_request(attempt.clone(), session, replacing, password);
            let json = SecretText(serde_json::to_string(&request).expect("a run request encodes"));
            runtime.call_helper(HelperCall::Connect { json }, move |runtime, response| {
                if let Some(code) = helper_failure(runtime, &attempt, "connect", &response) {
                    runtime.feed(connection::Input::RunRejected(attempt, code));
                }
            });
        });
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

    fn save_trusted_certs(&mut self, change: impl Fn(&mut config::Settings)) {
        if !self.config.exists() {
            self.system_log(
                LogLevel::Error,
                LogCategory::Config,
                "not pinning certificates: config.yaml does not exist",
            );
            return;
        }
        match self.config.update(change) {
            // The next input: this runs among the effects of the current one.
            Ok(_) => self.queue().run(Runtime::settings_changed),
            // The core keeps the values it learned; the file's older ones would undo that.
            Err(error) => self.system_log(
                LogLevel::Error,
                LogCategory::Config,
                format!("saving the server certificate failed: {error:?}"),
            ),
        }
    }

    /// Hands the settings in use to the core after the file changed.
    pub(crate) fn settings_changed(&mut self) {
        let settings = connection::Settings::new(self.config.settings());
        self.feed(connection::Input::SettingsChanged(settings));
    }

    // MARK: System events

    fn system_event(&mut self, event: SystemEvent) {
        let input = match event {
            SystemEvent::WillSleep { at } => connection::Input::WillSleep { at },
            SystemEvent::DidWake {} => {
                self.timers.nudge();
                self.check_for_updates_if_due();
                connection::Input::DidWake
            }
            SystemEvent::ScreenLocked { at } => connection::Input::ScreenLocked { at },
            SystemEvent::ScreenUnlocked {} => connection::Input::ScreenUnlocked,
            SystemEvent::DisplaysSlept { at } => connection::Input::DisplaysSlept { at },
            SystemEvent::DisplaysWoke {} => connection::Input::DisplaysWoke,
            SystemEvent::NetworkPathChanged {
                satisfied,
                interfaces,
            } => {
                if satisfied {
                    self.check_for_updates_after_network_failure();
                }
                connection::Input::NetworkPathChanged {
                    satisfied,
                    interfaces,
                }
            }
            SystemEvent::ClockChanged {} => {
                self.timers.nudge();
                self.check_for_updates_if_due();
                return;
            }
            SystemEvent::TimeZoneChanged { time_zone } => {
                self.time_zone = TimeZone::get(&time_zone).unwrap_or(TimeZone::UTC);
                self.timers.nudge();
                self.check_for_updates_if_due();
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
                let batch: OutputBatch =
                    serde_json::from_str(&json).expect("Helper output of this build");
                let session = self.sessions[&batch.attempt_id];
                for line in batch.lines {
                    let source = match line.source {
                        OutputSource::Info => LogSource::Info,
                        OutputSource::Error => LogSource::Error,
                        OutputSource::Helper => LogSource::Note,
                    };
                    let starts_attempt =
                        if self.unmarked_attempt.as_ref() == Some(&batch.attempt_id) {
                            self.unmarked_attempt = None;
                            self.cm.current_attempt() == Some(&batch.attempt_id)
                        } else {
                            false
                        };
                    self.pending_lines.push(LogLine {
                        time: EpochMillis(line.date.0),
                        source,
                        text: format!("{session} {}", line.text),
                        starts_attempt,
                    });
                }
                return;
            }
            SystemEvent::HelperConnectionLost {} => {
                self.bypass_connection_lost();
                connection::Input::HelperConnectionLost
            }
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
                    queue.run(|runtime| {
                        // The core checks again for the changes it sees; a hand edit that makes
                        // the file invalid or valid again, or changes the Gmail address, shows
                        // only in the readiness facts.
                        let facts = |runtime: &Runtime| {
                            let address = runtime.config.settings().otp.gmail_address.clone();
                            (runtime.config.is_valid(), address)
                        };
                        let before = facts(runtime);
                        if runtime.poll_config() && facts(runtime) != before {
                            runtime.feed(connection::Input::RecheckReadiness);
                        }
                    });
                }
            })
            .expect("starting the config poll thread");
    }

    /// Whether the file changed outside the app since it was last read.
    pub(crate) fn poll_config(&mut self) -> bool {
        let changed = self.config.poll();
        if changed {
            self.config_reloaded();
        }
        changed
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

    pub(crate) fn flush_log_lines(&mut self) {
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
        Section::AutoUpdate { .. } => "update",
    }
}

/// The Helper's refusal of a call, or `None` when it accepted it. A Helper that cannot be reached
/// or is another build counts as an internal error, which the core retries like a network failure.
pub(crate) fn helper_failure(
    runtime: &mut Runtime,
    attempt: &AttemptId,
    what: &str,
    response: &PortResponse,
) -> Option<fvm_contract::app_helper::HelperErrorCode> {
    use fvm_contract::app_helper::HelperErrorCode;
    match response {
        PortResponse::HelperReply { json: None } => None,
        PortResponse::HelperReply { json: Some(json) } => {
            match serde_json::from_str::<HelperError>(json) {
                Ok(error) => {
                    runtime.attempt_note(attempt, format!("‹{what} rejected: {}›", error.message));
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

fn note_text(note: Note) -> String {
    match note {
        Note::ScheduledHoursStarted => "‹schedule: scheduled hours started, connecting›".to_owned(),
        Note::ReconnectHeldWhileAway => {
            "‹schedule: outside the scheduled hours and away, the reconnect waits until you return›".to_owned()
        }
        Note::ReconnectsSuspended { away_minutes } => format!(
            "‹schedule: away for {away_minutes} min outside the scheduled hours, automatic reconnects stopped›"
        ),
        Note::WaitingForStaleOtp => {
            "‹waiting for the stopped attempt's OTP email before signing in again›".to_owned()
        }
        Note::StaleOtpTrashed => {
            "‹the stopped attempt's OTP email is in the trash, signing in again›".to_owned()
        }
        Note::StaleOtpSessionEnded => {
            "‹the stopped attempt's Gmail session ended before its OTP email was trashed, signing in again›".to_owned()
        }
        Note::StaleOtpWaitRanOut => format!(
            "‹the stopped attempt's OTP email was not trashed within {} s, signing in again›",
            connection::STALE_OTP_WAIT.as_secs()
        ),
    }
}
