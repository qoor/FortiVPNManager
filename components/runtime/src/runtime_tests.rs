//! The runtime as the app drives it: a fake app side answers the port requests from an in-memory
//! secret store, a fake Helper and a fake network, with the runtime's files in a scratch folder.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use fvm_contract::app_helper::{
    AttemptId, BackendStatus, BypassRoute, BypassRoutesReply, BypassRoutesRequest,
    BypassRoutesSnapshot, EndFacts, HelperError, HelperErrorCode, HelperEvent, HelperEventKind,
    HelperStatus, Milestone, OutputBatch, OutputLine, OutputSource, PreparedUpdateReply,
    RunRequest, StopInitiator, StopReason, UpdateDiscardReply, UpdateInstallReply,
    UpdateInstallRequest, UpdatePrepareReply, UpdatePrepareRequest, UpdateRefusal,
};
use fvm_contract::app_internal::command::{AccountForm, BypassList, Command};
use fvm_contract::app_internal::event::SystemEvent;
use fvm_contract::app_internal::port::{
    FortiClientConnection, HelperCall, HelperChannelFailure, LogCategory, PortAction, PortRequest,
    PortResponse, RegistrationStatus, SecretItem, SecretStoreFailure, UserNotification,
};
use fvm_contract::app_internal::problem::AccountValueProblem;
use fvm_contract::app_internal::section::{
    AccountSaveError, AutoUpdateSection, BypassFailure, BypassRulesSection, BypassStatus,
    ClientSettingsImport, ConfigSection, ConnectProgress, ConnectionStatus, HelperState,
    ImportError, ImportedPassword, InstallFailure, RemainingTime, Section, UpdateCheckFailure,
    UpdateProgress, UpdateStatus,
};
use fvm_contract::app_internal::{
    AppFacts, ButtonState, FromRuntime, LogLine, SecretText, SignInPages, ToRuntime,
};
use fvm_core::auto_update::{Check, Release, Version};
use serde_json::{Value, json};

use crate::net::Reply;
use crate::orchestration::{self, ClientSettings, Input, Paths, Runtime};
use crate::timers::Timer;

const APP_BUILD: &str = "100";
const APP_VERSION: &str = "26.1.0";
/// No message for this long: the runtime has nothing more to do on its own.
const QUIET: Duration = Duration::from_millis(400);
const PATIENCE: Duration = Duration::from_secs(10);

const ACCOUNT: &str = r#"schema_version: 1
vpn:
  host: "vpn.example.com"
  port: 443
  username: "alice"
  realm: ""
  trusted_cert_pins: []
otp:
  gmail_address: "alice@example.com"
  method: "imap"
  google_client_id: ""
connection:
  auto_connect_on_launch: false
"#;

struct App {
    folder: PathBuf,
    inputs: Sender<Input>,
    out: Receiver<FromRuntime>,
    secrets: Vec<(SecretItem, SecretText)>,
    secret_writes_fail: bool,
    helper_registration: RegistrationStatus,
    helper_build: String,
    /// The primary network; `None` when there is no network.
    service: Option<BypassRoutesSnapshot>,
    /// The app's exception routes the fake Helper keeps in the routing table.
    exception_routes: Vec<BypassRoute>,
    /// Whether written routes stay in the routing table.
    keeps_routes: bool,
    /// The route calls find no Helper.
    routes_unreachable: bool,
    /// The route calls wait for `answer_held_route_calls`.
    holds_route_calls: bool,
    held_route_calls: Vec<(u64, HelperCall)>,
    holds_update_calls: bool,
    held_update_calls: Vec<(u64, HelperCall)>,
    holds_gmail_reads: bool,
    held_gmail_reads: usize,
    /// The app's exception routes when the quit was approved, once it was.
    routes_at_quit: Option<Vec<BypassRoute>>,
    requests: Vec<PortRequest>,
    actions: Vec<PortAction>,
    sections: Vec<Section>,
    log: Vec<LogLine>,
    open_settings: u32,
    /// `FortiClient`'s SSL VPN connections, which make the runtime import from `FortiClient` as on
    /// Windows; `None` imports openfortivpn's settings file as on macOS.
    forticlient: Option<Vec<FortiClientConnection>>,
    waiter_failure: Option<String>,
    prepare_answer: PortResponse,
    install_answer: PortResponse,
    prepared_answer: PortResponse,
    update_url: String,
    update_sections: Vec<(Instant, AutoUpdateSection)>,
    log_at_waiter: Option<String>,
}

fn scratch() -> PathBuf {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).unwrap();
    let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let folder = std::env::temp_dir().join(format!("fvm-runtime-app-{name}"));
    fs::create_dir_all(&folder).unwrap();
    folder
}

fn vpn_password_item() -> SecretItem {
    SecretItem::VpnPassword {
        username: "alice".into(),
        host: "vpn.example.com".into(),
    }
}

fn app_password_item() -> SecretItem {
    SecretItem::GmailAppPassword {
        address: "alice@example.com".into(),
    }
}

impl App {
    /// A Mac with a registered Helper of the app's build, before launch.
    fn new() -> Self {
        let (inputs, _) = mpsc::channel();
        let (_, out) = mpsc::channel();
        Self {
            folder: scratch(),
            inputs,
            out,
            secrets: Vec::new(),
            helper_registration: RegistrationStatus::Enabled,
            helper_build: APP_BUILD.into(),
            service: None,
            exception_routes: Vec::new(),
            keeps_routes: true,
            routes_unreachable: false,
            holds_route_calls: false,
            held_route_calls: Vec::new(),
            holds_update_calls: false,
            held_update_calls: Vec::new(),
            holds_gmail_reads: false,
            held_gmail_reads: 0,
            routes_at_quit: None,
            requests: Vec::new(),
            actions: Vec::new(),
            sections: Vec::new(),
            log: Vec::new(),
            open_settings: 0,
            forticlient: None,
            secret_writes_fail: false,
            waiter_failure: None,
            prepare_answer: helper_reply(&UpdatePrepareReply::Prepared {
                version: version("26.2.0"),
            }),
            install_answer: helper_reply(&UpdateInstallReply::Started {
                installer_process_id: None,
            }),
            prepared_answer: helper_reply(&PreparedUpdateReply::NotPrepared {}),
            update_url: String::new(),
            update_sections: Vec::new(),
            log_at_waiter: None,
        }
    }

    fn config_path(&self) -> PathBuf {
        self.folder.join("config/config.yaml")
    }

    fn write_config(&self, text: &str) {
        fs::create_dir_all(self.config_path().parent().unwrap()).unwrap();
        fs::write(self.config_path(), text).unwrap();
    }

    fn config_text(&self) -> String {
        fs::read_to_string(self.config_path()).unwrap()
    }

    fn launch(&mut self) {
        let (inputs, received) = mpsc::channel();
        let (out, from_runtime) = mpsc::channel();
        let paths = Paths {
            config: self.config_path(),
            state: self.folder.join("state/state.json"),
            log: self.folder.join("logs/vpn_backend.log"),
            update_downloads: self.downloads(),
            client_settings: match self.forticlient {
                Some(_) => ClientSettings::FortiClient,
                None => ClientSettings::OpenFortiVpn {
                    home: self.folder.join("home"),
                },
            },
        };
        let runtime_inputs = inputs.clone();
        let update_url = self.update_url.clone();
        thread::spawn(move || orchestration::run(paths, update_url, runtime_inputs, received, out));
        self.inputs = inputs;
        self.out = from_runtime;
        self.send(ToRuntime::Launched {
            facts: AppFacts {
                app_build: APP_BUILD.into(),
                app_version: APP_VERSION.into(),
                installed_copy: false,
                built_in_oauth_client: None,
                time_zone: "Asia/Seoul".into(),
                sign_in_pages: SignInPages {
                    completed: "done".into(),
                    not_completed: "no".into(),
                    invalid: "?".into(),
                },
            },
        });
    }

    fn send(&self, message: ToRuntime) {
        self.inputs.send(Input::App(message)).unwrap();
    }

    fn command(&self, command: Command) {
        self.send(ToRuntime::Command { command });
    }

    /// A line the Helper writes itself.
    fn output_of(&self, attempt: &AttemptId, text: &str) {
        let batch = OutputBatch {
            attempt_id: attempt.clone(),
            lines: vec![OutputLine {
                date: SystemTime::now(),
                source: OutputSource::Helper,
                text: text.into(),
            }],
        };
        self.send(ToRuntime::Event {
            event: SystemEvent::HelperOutput {
                json: serde_json::to_string(&batch).unwrap(),
            },
        });
    }

    fn helper_event(&self, attempt: &AttemptId, kind: HelperEventKind) {
        let event = HelperEvent {
            attempt_id: attempt.clone(),
            kind,
        };
        self.send(ToRuntime::Event {
            event: SystemEvent::HelperEvent {
                json: serde_json::to_string(&event).unwrap(),
            },
        });
    }

    fn ask<T: Send + 'static>(
        &self,
        question: impl FnOnce(&mut Runtime) -> T + Send + 'static,
    ) -> T {
        let (answer, answered) = mpsc::channel();
        self.inputs
            .send(Input::Done(Box::new(move |runtime| {
                answer.send(question(runtime)).unwrap();
            })))
            .unwrap();
        answered.recv_timeout(PATIENCE).unwrap()
    }

    fn run_requests(&self) -> Vec<RunRequest> {
        self.requests
            .iter()
            .filter_map(|request| match request {
                PortRequest::CallHelper {
                    call: HelperCall::Connect { json },
                } => Some(serde_json::from_str(&json.0).unwrap()),
                _ => None,
            })
            .collect()
    }

    fn start_run(&mut self, command: Command) -> AttemptId {
        let (reads, runs) = (self.held_gmail_reads, self.run_requests().len());
        self.command(command);
        self.settle_until("the OTP session's start", |app| {
            app.held_gmail_reads > reads
        });
        let attempt = self
            .ask(|runtime| runtime.cm.current_attempt().cloned())
            .unwrap();
        let ready = attempt.clone();
        self.ask(move |runtime| {
            runtime.feed(fvm_core::connection::Input::OtpSessionReady(ready));
        });
        self.settle_until("the run request", |app| app.run_requests().len() > runs);
        attempt
    }

    fn bring_up(&mut self, attempt: &AttemptId, tunnel_ip: std::net::Ipv4Addr) {
        for milestone in [Milestone::Started, Milestone::ConnectedToGateway] {
            self.helper_event(
                attempt,
                HelperEventKind::Milestone {
                    milestone,
                    tunnel_ip: None,
                },
            );
        }
        self.helper_event(
            attempt,
            HelperEventKind::Milestone {
                milestone: Milestone::TunnelUp,
                tunnel_ip: Some(tunnel_ip),
            },
        );
        self.settle_until("the tunnel", |app| {
            matches!(
                app.connection_status(),
                Some(ConnectionStatus::Connected { tunnel_ip: Some(ip), .. }) if *ip == tunnel_ip
            )
        });
    }

    /// Takes the runtime's messages, answering its requests, until it has been quiet for a while.
    fn settle(&mut self) {
        loop {
            match self.out.recv_timeout(QUIET) {
                Ok(message) => self.take(message),
                Err(RecvTimeoutError::Timeout) => return,
                Err(RecvTimeoutError::Disconnected) => panic!("the runtime stopped"),
            }
        }
    }

    /// Takes the runtime's messages until `done` holds.
    fn settle_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(self) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or_else(|| panic!("still waiting for {what}"));
            match self.out.recv_timeout(left) {
                Ok(message) => self.take(message),
                Err(RecvTimeoutError::Timeout) => panic!("still waiting for {what}"),
                Err(RecvTimeoutError::Disconnected) => panic!("the runtime stopped"),
            }
        }
    }

    fn take(&mut self, message: FromRuntime) {
        match message {
            FromRuntime::Section { section } => {
                if let Section::AutoUpdate { section } = &section {
                    self.update_sections
                        .push((Instant::now(), section.as_ref().clone()));
                }
                self.sections.retain(|other| {
                    std::mem::discriminant(other) != std::mem::discriminant(&section)
                });
                self.sections.push(section);
            }
            FromRuntime::PortRequest { id, request } => {
                self.requests.push(request.clone());
                if let PortRequest::ReadSecret {
                    item: SecretItem::GmailAppPassword { .. },
                } = &request
                    && self.holds_gmail_reads
                {
                    self.held_gmail_reads += 1;
                    return;
                }
                if let PortRequest::CallHelper { call } = &request
                    && is_route_call(call)
                    && self.holds_route_calls
                {
                    self.held_route_calls.push((id, call.clone()));
                    return;
                }
                if let PortRequest::CallHelper { call } = &request
                    && is_update_call(call)
                    && self.holds_update_calls
                {
                    self.held_update_calls.push((id, call.clone()));
                    return;
                }
                let response = self.answer(request);
                self.send(ToRuntime::PortResponse { id, response });
            }
            FromRuntime::OpenSettings {} => self.open_settings += 1,
            FromRuntime::LogLines { lines } => self.log.extend(lines),
            FromRuntime::PortAction { action } => {
                if action == (PortAction::ApproveQuit {}) {
                    self.routes_at_quit = Some(self.exception_routes.clone());
                }
                self.actions.push(action);
            }
        }
    }

    fn answer(&mut self, request: PortRequest) -> PortResponse {
        match request {
            PortRequest::ReadSecret { item } => PortResponse::Secret {
                value: self.secret(&item),
            },
            PortRequest::HasSecret { item } => PortResponse::HasSecret {
                exists: self.secret(&item).is_some(),
            },
            PortRequest::WriteSecret { .. } if self.secret_writes_fail => {
                PortResponse::SecretStoreFailed {
                    failure: SecretStoreFailure {
                        description: "denied".into(),
                    },
                }
            }
            PortRequest::WriteSecret { item, value } => {
                self.secrets.retain(|(other, _)| *other != item);
                self.secrets.push((item, value));
                PortResponse::SecretChanged {}
            }
            PortRequest::DeleteSecret { item } => {
                self.secrets.retain(|(other, _)| *other != item);
                PortResponse::SecretChanged {}
            }
            PortRequest::HelperRegistration {} => PortResponse::Registration {
                status: self.helper_registration,
            },
            PortRequest::RegisterHelper {} => {
                self.helper_registration = RegistrationStatus::Enabled;
                PortResponse::Registered {}
            }
            PortRequest::HelperBuild {} => PortResponse::HelperBuild {
                build: self.helper_build.clone(),
            },
            PortRequest::CallHelper { call } => self.helper(call),
            PortRequest::RouteInterface { .. } => PortResponse::RouteInterface {
                interface: Some("en0".into()),
            },
            PortRequest::InterfaceAddresses {} => PortResponse::InterfaceAddresses {
                addresses: Vec::new(),
            },
            PortRequest::PrimaryRouter {} => {
                let router = self
                    .service
                    .as_ref()
                    .map(|service| service.router.parse().unwrap());
                PortResponse::PrimaryRouter { router }
            }
            PortRequest::ScreenState {} => PortResponse::ScreenState {
                locked: false,
                displays_asleep: false,
            },
            PortRequest::OpenBrowser { .. } => PortResponse::BrowserOpened { opened: false },
            PortRequest::LoginItemRegistration {} => PortResponse::Registration {
                status: RegistrationStatus::NotRegistered,
            },
            PortRequest::SetLoginItem { .. } => PortResponse::Registered {},
            PortRequest::FortiClientConnections {} => PortResponse::FortiClientConnections {
                connections: self.forticlient.clone().unwrap_or_default(),
            },
            PortRequest::StartUpdateWaiter { .. } => {
                self.log_at_waiter =
                    fs::read_to_string(self.folder.join("logs/vpn_backend.log")).ok();
                match &self.waiter_failure {
                    Some(description) => PortResponse::UpdateWaiterFailed {
                        description: description.clone(),
                    },
                    None => PortResponse::UpdateWaiterStarted {},
                }
            }
        }
    }

    fn answer_held_route_calls(&mut self) {
        self.holds_route_calls = false;
        for (id, call) in std::mem::take(&mut self.held_route_calls) {
            let response = self.helper(call);
            self.send(ToRuntime::PortResponse { id, response });
        }
    }

    fn route_calls(&self) -> usize {
        self.requests
            .iter()
            .filter(|request| matches!(request, PortRequest::CallHelper { call } if is_route_call(call)))
            .count()
    }

    fn secret(&self, item: &SecretItem) -> Option<SecretText> {
        self.secrets
            .iter()
            .find(|(other, _)| other == item)
            .map(|(_, value)| value.clone())
    }

    fn helper(&mut self, call: HelperCall) -> PortResponse {
        let reply = |value: serde_json::Value| PortResponse::HelperReply {
            json: Some(value.to_string()),
        };
        match call {
            HelperCall::Status {} => {
                let status = HelperStatus {
                    session_running: false,
                    backend: BackendStatus {
                        version: Some("9.21".into()),
                    },
                    routing_complete: None,
                };
                reply(serde_json::to_value(status).unwrap())
            }
            HelperCall::ExitWhenIdle {} => {
                // The system starts the app's build next time.
                self.helper_build = APP_BUILD.into();
                PortResponse::HelperReplyFlag { value: true }
            }
            HelperCall::ReadBypassRoutes {} | HelperCall::WriteBypassRoutes { .. }
                if self.routes_unreachable =>
            {
                PortResponse::HelperUnreachable {
                    failure: HelperChannelFailure::ConnectionLost {},
                }
            }
            HelperCall::ReadBypassRoutes {} => match &self.service {
                Some(service) => reply(json!({"success": {
                    "snapshot": service,
                    "routes": self.exception_routes,
                }})),
                None => reply(
                    json!({"failure": {"_0": {"code": "wrongPhase", "message": "no active network service"}}}),
                ),
            },
            HelperCall::WriteBypassRoutes { json } => match serde_json::from_str(&json).unwrap() {
                BypassRoutesRequest::Write { expected, routes } => {
                    if self.service.as_ref() != Some(&expected) {
                        return reply(json!({"failure": {"_0": {
                            "code": "wrongPhase",
                            "message": "the network changed since it was read; read it again",
                        }}}));
                    }
                    self.exception_routes = if self.keeps_routes {
                        routes
                    } else {
                        Vec::new()
                    };
                    let reply_value = BypassRoutesReply::Success {
                        snapshot: expected,
                        routes: self.exception_routes.clone(),
                    };
                    reply(serde_json::to_value(reply_value).unwrap())
                }
                BypassRoutesRequest::Remove {} => {
                    self.exception_routes.clear();
                    reply(serde_json::to_value(BypassRoutesReply::Removed {}).unwrap())
                }
            },
            HelperCall::Connect { .. }
            | HelperCall::SubmitOtp { .. }
            | HelperCall::Disconnect { .. } => PortResponse::HelperReply { json: None },
            HelperCall::PrepareUpdate { .. } => self.prepare_answer.clone(),
            HelperCall::InstallUpdate { .. } => self.install_answer.clone(),
            HelperCall::ReadPreparedUpdate {} => self.prepared_answer.clone(),
            HelperCall::DiscardPreparedUpdate {} => helper_reply(&UpdateDiscardReply::Discarded {}),
        }
    }

    fn config_section(&self) -> &ConfigSection {
        self.sections
            .iter()
            .find_map(|section| match section {
                Section::Config { section } => Some(section.as_ref()),
                _ => None,
            })
            .expect("a config section")
    }

    #[cfg(not(windows))]
    fn vpn_backend_info(&self) -> Option<&fvm_contract::app_internal::section::VpnBackendInfo> {
        self.sections.iter().find_map(|section| match section {
            Section::Readiness { section } => section.backend.as_ref(),
            _ => None,
        })
    }

    fn helper_state(&self) -> Option<&HelperState> {
        self.sections.iter().find_map(|section| match section {
            Section::Readiness { section } => Some(&section.helper),
            _ => None,
        })
    }

    fn connection_status(&self) -> Option<&ConnectionStatus> {
        self.sections.iter().find_map(|section| match section {
            Section::Connection { section } => Some(&section.status),
            _ => None,
        })
    }

    fn bypass_status(&self) -> Option<&BypassStatus> {
        self.sections.iter().find_map(|section| match section {
            Section::BypassRules { section } => Some(&section.status),
            _ => None,
        })
    }

    fn asked_helper(&self, wanted: impl Fn(&HelperCall) -> bool) -> bool {
        self.requests
            .iter()
            .any(|request| matches!(request, PortRequest::CallHelper { call } if wanted(call)))
    }
}

#[test]
fn a_launch_with_everything_in_place_shows_the_helper_running_and_waits_for_connect() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.secrets
        .push((vpn_password_item(), SecretText("vpn secret".into())));
    app.secrets
        .push((app_password_item(), SecretText("abcdabcdabcdabcd".into())));
    app.launch();
    app.settle_until("the Helper's build", |app| {
        matches!(app.helper_state(), Some(HelperState::Running { build }) if build == APP_BUILD)
    });
    app.settle();
    assert_eq!(
        app.connection_status(),
        Some(&ConnectionStatus::NotConnected {})
    );
    assert_eq!(app.open_settings, 0);
}

const NO_GMAIL_CREDENTIAL: &str =
    "‹Gmail API selected but not signed in with Google, and no IMAP app password is stored›";

/// Stored, so setup is complete, but empty: an attempt ends before the VPN client starts, as the
/// OTP session has nothing to sign in with.
fn app_without_a_usable_gmail_credential(method: &str) -> App {
    let mut app = App::new();
    app.write_config(&ACCOUNT.replace("method: \"imap\"", &format!("method: \"{method}\"")));
    app.secrets
        .push((vpn_password_item(), SecretText("vpn secret".into())));
    app.secrets
        .push((app_password_item(), SecretText(String::new())));
    app.launch();
    app.settle();
    app
}

fn log_marks(app: &App) -> Vec<(&str, bool)> {
    app.log
        .iter()
        .map(|line| (line.text.split_once(' ').unwrap().1, line.starts_attempt))
        .collect()
}

fn log_sessions(app: &App) -> Vec<&str> {
    app.log
        .iter()
        .map(|line| line.text.split_once(' ').unwrap().0)
        .collect()
}

#[test]
fn each_attempt_starts_its_block_with_its_first_log_line() {
    let mut app = app_without_a_usable_gmail_credential("gmail_api");
    let mut attempts = Vec::new();
    for count in 1..=2 {
        app.command(Command::Connect {});
        attempts.push(
            app.ask(|runtime| runtime.cm.current_attempt().cloned())
                .unwrap(),
        );
        app.settle_until("the attempt's note", |app| {
            app.log
                .iter()
                .filter(|line| line.text.ends_with(NO_GMAIL_CREDENTIAL))
                .count()
                == count
        });
    }
    app.output_of(&attempts[0], "‹after›");
    app.settle();
    assert_eq!(
        log_marks(&app),
        [
            (NO_GMAIL_CREDENTIAL, true),
            (NO_GMAIL_CREDENTIAL, true),
            ("‹after›", false)
        ]
    );
    let sessions = log_sessions(&app);
    assert!(sessions[0] < sessions[1], "{sessions:?}");
    assert_eq!(sessions[2], sessions[0]);
}

#[test]
fn the_helpers_lines_of_another_attempt_start_no_block() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.secrets
        .push((vpn_password_item(), SecretText("vpn secret".into())));
    app.secrets
        .push((app_password_item(), SecretText("abcdabcdabcdabcd".into())));
    app.holds_gmail_reads = true;
    app.launch();
    app.settle();
    let other = app.start_run(Command::Connect {});
    app.bring_up(&other, "10.212.134.5".parse().unwrap());
    let attempt = app.start_run(Command::ReconnectNow {});
    app.output_of(&other, "‹b›");
    app.output_of(&attempt, "‹a›");
    app.output_of(&other, "‹b again›");
    app.output_of(&attempt, "‹a again›");
    app.settle();
    assert_eq!(
        log_marks(&app),
        [
            ("‹b›", false),
            ("‹a›", true),
            ("‹b again›", false),
            ("‹a again›", false)
        ]
    );
}

#[test]
fn an_attempt_that_ended_without_a_log_line_leaves_the_next_line_unmarked() {
    let mut app = app_without_a_usable_gmail_credential("imap");
    app.command(Command::Connect {});
    let attempt = app
        .ask(|runtime| runtime.cm.current_attempt().cloned())
        .unwrap();
    app.settle_until("the failure", |app| {
        matches!(
            app.connection_status(),
            Some(ConnectionStatus::Failed { .. })
        )
    });
    app.output_of(&attempt, "‹after›");
    app.settle();
    assert_eq!(log_marks(&app), [("‹after›", false)]);
}

#[test]
fn a_hand_edit_that_breaks_or_fixes_the_file_updates_the_setup_list() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.secrets
        .push((vpn_password_item(), SecretText("vpn secret".into())));
    app.secrets
        .push((app_password_item(), SecretText("abcdabcdabcdabcd".into())));
    app.launch();
    app.settle_until("the Helper's build", |app| {
        matches!(app.helper_state(), Some(HelperState::Running { build }) if build == APP_BUILD)
    });
    app.settle();
    assert_eq!(
        app.connection_status(),
        Some(&ConnectionStatus::NotConnected {})
    );

    app.write_config(&ACCOUNT.replace("port: 443", "port: 0443"));
    app.settle_until("the broken file in the setup list", |app| {
        matches!(
            app.connection_status(),
            Some(ConnectionStatus::SetupRequired { items })
                if items.contains(&fvm_contract::app_internal::section::SetupItem::ConfigFile {})
        )
    });
    app.write_config(ACCOUNT);
    app.settle_until("the fixed file", |app| {
        app.connection_status() == Some(&ConnectionStatus::NotConnected {})
    });
}

/// The certificate list belongs to the gateway: a hand edit of the server address empties it.
#[test]
fn a_new_server_address_empties_the_certificate_list() {
    let pin = format!("pin-sha256:{}=", "A".repeat(43));
    let with_values = ACCOUNT.replace(
        "  trusted_cert_pins: []\n",
        &format!("  trusted_cert_pins: [\"{pin}\"]\n"),
    );
    let mut app = App::new();
    app.write_config(&with_values);
    app.launch();
    app.settle();
    assert!(app.config_text().contains(&pin));
    app.write_config(&with_values.replace("vpn.example.com", "vpn2.example.com"));
    app.settle_until("the emptied list", |app| {
        app.config_text().contains("  trusted_cert_pins: []\n")
    });
    assert!(app.config_text().contains("vpn2.example.com"));
}

#[cfg(not(windows))]
#[test]
fn the_helper_status_shows_the_vpn_client() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle_until("the Helper's status", |app| {
        app.vpn_backend_info().is_some()
    });
    assert_eq!(
        app.vpn_backend_info().unwrap().version.as_deref(),
        Some("9.21")
    );
}

#[test]
fn a_launch_with_a_missing_password_asks_for_setup_and_opens_settings_once() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.secrets
        .push((app_password_item(), SecretText("abcdabcdabcdabcd".into())));
    app.launch();
    app.settle_until("the setup list", |app| {
        matches!(
            app.connection_status(),
            Some(ConnectionStatus::SetupRequired { .. })
        )
    });
    app.settle();
    let Some(ConnectionStatus::SetupRequired { items }) = app.connection_status() else {
        unreachable!()
    };
    assert_eq!(
        items,
        &vec![fvm_contract::app_internal::section::SetupItem::VpnPassword {}]
    );
    assert_eq!(app.open_settings, 1);
}

#[test]
fn a_helper_of_another_build_is_asked_to_exit_and_checked_again() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.helper_build = "99".into();
    app.launch();
    app.settle_until("the restarted Helper", |app| {
        matches!(app.helper_state(), Some(HelperState::Running { build }) if build == APP_BUILD)
    });
    assert!(app.asked_helper(|call| matches!(call, HelperCall::ExitWhenIdle {})));
}

#[test]
fn the_first_launch_imports_the_openfortivpn_config_with_its_password() {
    let mut app = App::new();
    let client_config = app.folder.join("home/.config/openfortivpn/config");
    fs::create_dir_all(client_config.parent().unwrap()).unwrap();
    fs::write(
        &client_config,
        "host = vpn.example.com\nport = 10443\nusername = alice\npassword = vpn secret\n",
    )
    .unwrap();
    app.launch();
    app.settle_until("the import report", |app| {
        app.sections.iter().any(|section| matches!(section, Section::Config { section } if section.account.import_report.is_some()))
    });
    app.settle();
    let Some(ClientSettingsImport::Imported { report }) =
        &app.config_section().account.import_report
    else {
        panic!("an import report")
    };
    assert_eq!(report.password, Some(ImportedPassword::Saved {}));
    let text = app.config_text();
    for line in [
        "  host: \"vpn.example.com\"\n",
        "  port: 10443\n",
        "  username: \"alice\"\n",
    ] {
        assert!(text.contains(line), "{text}");
    }
    assert_eq!(
        app.secret(&vpn_password_item()),
        Some(SecretText("vpn secret".into()))
    );
    // The client's file is only read.
    assert!(
        fs::read_to_string(&client_config)
            .unwrap()
            .contains("password = vpn secret")
    );
}

fn edited_form(app: &App, edit: impl FnOnce(&mut AccountForm)) -> (u64, AccountForm) {
    let account = &app.config_section().account;
    let mut form = AccountForm {
        server: account.server.clone(),
        username: account.username.clone(),
        realm: account.realm.clone(),
        gmail_address: account.gmail_address.clone(),
        vpn_password: SecretText(String::new()),
        app_password: SecretText(String::new()),
    };
    edit(&mut form);
    (account.revision, form)
}

fn commit_account(app: &mut App, edit: impl FnOnce(&mut AccountForm)) {
    let (revision, form) = edited_form(app, edit);
    app.command(Command::EditAccount { revision, form });
    app.command(Command::SaveAccount {});
    app.settle();
}

#[test]
fn saving_the_account_writes_the_changed_field_and_the_password_under_the_saved_account() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    let (revision, form) = edited_form(&app, |form| {
        form.username = " bob ".into();
        form.vpn_password = SecretText("new secret".into());
    });
    app.command(Command::EditAccount { revision, form });
    // Edited elsewhere meanwhile: the field the user did not change keeps the edit.
    app.write_config(&ACCOUNT.replace("method: \"imap\"", "method: \"gmail_api\""));
    app.command(Command::SaveAccount {});
    let saved_under = SecretItem::VpnPassword {
        username: "bob".into(),
        host: "vpn.example.com".into(),
    };
    app.settle_until("the saved password", |app| {
        app.secret(&saved_under) == Some(SecretText("new secret".into()))
    });
    app.settle();
    let text = app.config_text();
    assert!(text.contains("  username: \"bob\"\n"), "{text}");
    assert!(text.contains("  method: \"gmail_api\"\n"), "{text}");
    let account = &app.config_section().account;
    assert_eq!(account.username, "bob");
    assert!(!account.vpn_password_entered);
    assert_eq!(account.save_error, None);
}

#[test]
fn a_commit_while_passwords_are_written_saves_after_them() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    let (revision, form) = edited_form(&app, |form| {
        form.vpn_password = SecretText("new secret".into());
    });
    app.command(Command::EditAccount { revision, form });
    app.command(Command::SaveAccount {});
    let (revision, form) = edited_form(&app, |form| form.realm = "staff".into());
    app.command(Command::EditAccount {
        revision: revision + 1,
        form,
    });
    app.command(Command::SaveAccount {});
    app.settle_until("the saved realm", |app| {
        app.config_text().contains("  realm: \"staff\"\n")
    });
    assert_eq!(
        app.secret(&vpn_password_item()),
        Some(SecretText("new secret".into()))
    );
}

#[test]
fn a_failed_password_write_stays_shown_after_a_commit_made_meanwhile() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    app.secret_writes_fail = true;
    let (revision, form) = edited_form(&app, |form| {
        form.vpn_password = SecretText("new secret".into());
    });
    app.command(Command::EditAccount { revision, form });
    app.command(Command::SaveAccount {});
    let (revision, form) = edited_form(&app, |form| form.realm = "staff".into());
    app.command(Command::EditAccount {
        revision: revision + 1,
        form,
    });
    app.command(Command::SaveAccount {});
    app.settle_until("the saved realm", |app| {
        app.config_text().contains("  realm: \"staff\"\n")
    });
    app.settle();
    assert!(matches!(
        app.config_section().account.save_error,
        Some(AccountSaveError::SecretStore { .. })
    ));
}

#[test]
fn the_server_field_holds_the_port_unless_it_is_the_default() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    assert_eq!(app.config_section().account.server, "vpn.example.com");

    commit_account(&mut app, |form| {
        form.server = " vpn.example.com:8443 ".into();
    });
    let text = app.config_text();
    assert!(text.contains("  host: \"vpn.example.com\"\n"), "{text}");
    assert!(text.contains("  port: 8443\n"), "{text}");
    assert_eq!(app.config_section().account.server, "vpn.example.com:8443");

    commit_account(&mut app, |form| form.server = "vpn.example.com".into());
    assert!(app.config_text().contains("  port: 443\n"));

    commit_account(&mut app, |form| form.server = "vpn.example.com:443".into());
    assert!(app.config_text().contains("  port: 443\n"));
    assert_eq!(app.config_section().account.server, "vpn.example.com");
}

#[test]
fn a_file_port_shows_in_the_server_field() {
    let mut app = App::new();
    app.write_config(&ACCOUNT.replace("port: 443", "port: 10443"));
    app.launch();
    app.settle();
    assert_eq!(app.config_section().account.server, "vpn.example.com:10443");
    app.write_config(ACCOUNT);
    app.settle_until("the default port", |app| {
        app.config_section().account.server == "vpn.example.com"
    });
}

#[test]
fn a_server_field_that_is_no_host_and_port_saves_nothing() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    for (server, error) in [
        ("vpn.example.com:", AccountSaveError::InvalidPort {}),
        ("vpn.example.com:0", AccountSaveError::InvalidPort {}),
        ("vpn.example.com:abc", AccountSaveError::InvalidPort {}),
        ("vpn.example.com:70000", AccountSaveError::InvalidPort {}),
        (
            "https://vpn.example.com:443",
            AccountSaveError::AccountValue {
                problem: AccountValueProblem::InvalidHost {},
            },
        ),
    ] {
        commit_account(&mut app, |form| form.server = server.into());
        let account = &app.config_section().account;
        assert_eq!(account.save_error, Some(error), "{server}");
        assert_eq!(account.server, server);
        assert_eq!(app.config_text(), ACCOUNT, "{server}");
    }
}

#[test]
fn a_save_that_keeps_the_typed_fields_does_not_replace_them() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    let before = app.config_section().account.revision;
    commit_account(&mut app, |form| form.username = "bob".into());
    assert!(app.config_text().contains("  username: \"bob\"\n"));
    assert_eq!(app.config_section().account.revision, before);

    commit_account(&mut app, |form| form.username = " carol ".into());
    let account = &app.config_section().account;
    assert_eq!(account.username, "carol");
    assert!(account.revision > before);
}

#[test]
fn a_typed_password_survives_a_file_change_but_not_closing_settings() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    let account = app.config_section().account.clone();
    let (revision, form) = edited_form(&app, |form| {
        form.vpn_password = SecretText("typed".into());
    });
    app.command(Command::EditAccount { revision, form });
    app.settle();
    // The file changes a field the user did not touch: the UI replaces its fields, the password
    // stays typed.
    app.write_config(&ACCOUNT.replace("alice\"\n  realm", "carol\"\n  realm"));
    app.settle_until("the merged username", |app| {
        app.config_section().account.username == "carol"
    });
    let merged = app.config_section().account.clone();
    assert!(merged.revision > account.revision);
    assert!(merged.vpn_password_entered);

    app.command(Command::SettingsClosed {});
    app.settle();
    let closed = &app.config_section().account;
    assert!(closed.revision > merged.revision);
    assert!(!closed.vpn_password_entered);
}

#[test]
fn a_draft_typed_against_replaced_fields_is_dropped() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    let (revision, form) = edited_form(&app, |form| form.username = "mallory".into());
    app.write_config(&ACCOUNT.replace("alice\"\n  realm", "carol\"\n  realm"));
    app.settle_until("the merged username", |app| {
        app.config_section().account.username == "carol"
    });
    app.command(Command::EditAccount { revision, form });
    app.settle();
    assert_eq!(app.config_section().account.username, "carol");
}

const WITH_RULE: &str = "routes:\n  networks: [192.168.0.0/24]\n  bypass_vpn: [10.30.0.0/16]\n";

fn office_wifi() -> BypassRoutesSnapshot {
    BypassRoutesSnapshot {
        service_id: "SERVICE-1".into(),
        service_name: "Wi-Fi".into(),
        router: "192.168.0.1".into(),
        address: "192.168.0.10".into(),
        subnet_mask: "255.255.255.0".into(),
    }
}

/// The rule's route through the office router.
fn rule_route() -> BypassRoute {
    BypassRoute {
        destination: "10.30.0.0".into(),
        subnet_mask: "255.255.0.0".into(),
        gateway: "192.168.0.1".into(),
    }
}

/// Has the app want the VPN up without reaching anything: with the account set up, it connects
/// while offline, so the attempt waits for the network. The path report takes a few seconds to
/// settle.
fn want_the_vpn(app: &mut App) {
    app.secrets
        .push((vpn_password_item(), SecretText("secret".into())));
    app.secrets
        .push((app_password_item(), SecretText("app password".into())));
    app.send(ToRuntime::Event {
        event: SystemEvent::NetworkPathChanged {
            satisfied: false,
            interfaces: Vec::new(),
        },
    });
    thread::sleep(Duration::from_millis(3500));
    app.command(Command::Connect {});
    app.settle_until("the wait for the network", |app| {
        app.connection_status() == Some(&ConnectionStatus::WaitingForNetwork {})
    });
}

/// An app on the office Wi-Fi with one exception rule, launched.
fn app_on_the_office_wifi() -> App {
    let mut app = App::new();
    app.write_config(&format!("{ACCOUNT}{WITH_RULE}"));
    app.service = Some(office_wifi());
    app.launch();
    app
}

/// An app on the office Wi-Fi with one exception rule, its route written while it wants the VPN.
fn app_with_the_rule_applied() -> App {
    let mut app = app_on_the_office_wifi();
    want_the_vpn(&mut app);
    app.settle_until("the applied rules", |app| {
        matches!(app.bypass_status(), Some(BypassStatus::Applied { .. }))
    });
    assert_eq!(app.exception_routes, vec![rule_route()]);
    app
}

fn bypass_section(app: &App) -> Option<&BypassRulesSection> {
    app.sections.iter().find_map(|section| match section {
        Section::BypassRules { section } => Some(section.as_ref()),
        _ => None,
    })
}

#[test]
fn an_ended_row_edit_saves_the_lists_unless_a_row_has_a_problem() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    app.command(Command::SetBypassRows {
        list: BypassList::Targets,
        rows: vec!["10.30.0.0/16".into(), String::new()],
    });
    app.settle();
    let saved = app.config_text();
    assert!(saved.contains("10.30.0.0/16"), "{saved}");

    app.command(Command::SetBypassRows {
        list: BypassList::Networks,
        rows: vec!["192.168.0".into()],
    });
    app.settle();
    assert_eq!(app.config_text(), saved);
    let section = bypass_section(&app).unwrap();
    assert_eq!(section.networks.rows, vec!["192.168.0".to_owned()]);
    assert_eq!(section.networks.problem_rows, vec![0]);

    app.command(Command::SetBypassRows {
        list: BypassList::Networks,
        rows: vec!["192.168.0.0/24".into()],
    });
    app.settle();
    let text = app.config_text();
    assert!(text.contains("192.168.0.0/24"), "{text}");
    assert!(text.contains("10.30.0.0/16"), "{text}");
}

#[test]
fn adding_the_current_network_saves_it() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.service = Some(office_wifi());
    app.launch();
    app.settle_until("the current network", |app| {
        bypass_section(app)
            .is_some_and(|section| section.add_current_network == ButtonState::Enabled)
    });
    app.command(Command::AddCurrentNetwork {});
    app.settle();
    let text = app.config_text();
    assert!(text.contains("192.168.0.0/24"), "{text}");
}

fn is_route_call(call: &HelperCall) -> bool {
    matches!(
        call,
        HelperCall::ReadBypassRoutes {} | HelperCall::WriteBypassRoutes { .. }
    )
}

fn network_changed(app: &App) {
    app.send(ToRuntime::Event {
        event: SystemEvent::NetworkSettingsChanged {},
    });
}

#[test]
fn exception_rules_write_the_routes_and_keep_no_record_of_them() {
    let mut app = app_with_the_rule_applied();
    app.settle();
    // The Helper keeps the record; the app's state file has none.
    let state = fs::read_to_string(app.folder.join("state/state.json")).unwrap_or_default();
    assert!(!state.contains("appliedExceptionRoutes"), "{state}");
}

#[test]
fn a_rule_removed_from_the_file_takes_its_route_away() {
    let mut app = app_with_the_rule_applied();
    app.write_config(&format!("{ACCOUNT}{WITH_RULE}").replace("[10.30.0.0/16]", "[]"));
    app.send(ToRuntime::Event {
        event: SystemEvent::AppActivated {},
    });
    app.settle_until("the route removed", |app| app.exception_routes.is_empty());
}

/// Only the settings window and the file name targets: a route the Helper reports that the rules
/// do not plan is removed, never added to the list.
#[test]
fn a_route_the_rules_do_not_plan_is_removed_not_imported() {
    let mut app = App::new();
    app.write_config(&format!("{ACCOUNT}{WITH_RULE}"));
    app.service = Some(office_wifi());
    let unplanned = BypassRoute {
        destination: "10.40.0.0".into(),
        subnet_mask: "255.255.0.0".into(),
        gateway: "192.168.0.1".into(),
    };
    app.exception_routes = vec![unplanned];
    app.launch();
    want_the_vpn(&mut app);
    app.settle_until("the planned route only", |app| {
        app.exception_routes == vec![rule_route()]
    });
    assert!(!app.config_text().contains("10.40.0.0"));
}

#[test]
fn the_routes_follow_the_network() {
    let mut app = app_with_the_rule_applied();
    // At home: the rules do not apply, so the route goes.
    app.service = Some(BypassRoutesSnapshot {
        router: "10.0.0.1".into(),
        address: "10.0.0.20".into(),
        ..office_wifi()
    });
    network_changed(&app);
    app.settle_until("the route removed away from the office", |app| {
        app.bypass_status() == Some(&BypassStatus::OutsideNetworks {})
    });
    assert_eq!(app.exception_routes, []);
    // Back at the office, it comes back.
    app.service = Some(office_wifi());
    network_changed(&app);
    app.settle_until("the route back", |app| {
        app.exception_routes == vec![rule_route()]
    });
}

#[test]
fn routes_gone_with_the_helper_connection_are_written_again() {
    let mut app = app_with_the_rule_applied();
    // The Helper removes the routes of a connection that goes away.
    app.exception_routes.clear();
    app.send(ToRuntime::Event {
        event: SystemEvent::HelperConnectionLost {},
    });
    app.settle_until("the route written again", |app| {
        app.exception_routes == vec![rule_route()]
    });
}

#[test]
fn routes_the_routing_table_did_not_keep_are_reported_and_retried() {
    let mut app = App::new();
    app.write_config(&format!("{ACCOUNT}{WITH_RULE}"));
    app.service = Some(office_wifi());
    app.keeps_routes = false;
    app.launch();
    want_the_vpn(&mut app);
    app.settle_until("the failure", |app| {
        matches!(app.bypass_status(), Some(BypassStatus::Failed { .. }))
    });
    assert_eq!(
        app.bypass_status(),
        Some(&BypassStatus::Failed {
            failure: BypassFailure::RoutesNotKept {},
            retry_in_seconds: Some(10),
            stopped_after_retries: None,
        })
    );
}

#[test]
fn quitting_removes_the_routes_before_the_quit_is_approved() {
    let mut app = app_with_the_rule_applied();
    app.send(ToRuntime::Event {
        event: SystemEvent::QuitRequested {},
    });
    app.settle_until("the approved quit", |app| app.routes_at_quit.is_some());
    assert_eq!(app.routes_at_quit, Some(Vec::new()));
    // Nothing writes them again afterwards.
    network_changed(&app);
    app.settle();
    assert_eq!(app.exception_routes, []);
}

/// A quit approved while a sync's read is under way, here with no Helper to remove the routes:
/// whether the read succeeds or fails, its answer changes nothing, since the app may end at any
/// moment and its log was written out. No route call, log line, status or notification follows.
#[test]
fn nothing_follows_an_approved_quit() {
    // With no network the read fails, which is logged and shown.
    for network_when_answered in [Some(office_wifi()), None] {
        let mut app = App::new();
        app.write_config(&format!("{ACCOUNT}{WITH_RULE}"));
        app.service = Some(office_wifi());
        app.helper_registration = RegistrationStatus::NotRegistered;
        app.holds_route_calls = true;
        app.launch();
        app.settle_until("the read under way", |app| !app.held_route_calls.is_empty());
        app.send(ToRuntime::Event {
            event: SystemEvent::QuitRequested {},
        });
        app.settle_until("the approved quit", |app| app.routes_at_quit.is_some());
        let route_calls = app.route_calls();
        let actions = app.actions.len();
        let status = app.bypass_status().cloned();

        app.service = network_when_answered;
        app.answer_held_route_calls();
        app.settle();
        assert_eq!(app.route_calls(), route_calls);
        let after = &app.actions[actions..];
        assert!(
            !after.iter().any(|action| matches!(
                action,
                PortAction::Log { .. } | PortAction::PostNotification { .. }
            )),
            "{after:?}"
        );
        assert_eq!(app.bypass_status().cloned(), status);
    }
}

/// Without a VPN the routes mean nothing: they are in place only while the app wants it up.
#[test]
fn the_routes_are_in_place_only_while_the_vpn_is_wanted() {
    let mut app = app_on_the_office_wifi();
    app.settle_until("the rules not applied", |app| {
        app.bypass_status() == Some(&BypassStatus::VpnOff {})
    });
    assert_eq!(app.exception_routes, []);
    want_the_vpn(&mut app);
    app.settle_until("the route written", |app| {
        app.exception_routes == vec![rule_route()]
    });
    app.command(Command::Disconnect {});
    app.settle_until("the route removed", |app| {
        app.bypass_status() == Some(&BypassStatus::VpnOff {})
    });
    assert_eq!(app.exception_routes, []);
}

/// The VPN server always goes out directly: a file naming its address as a target is refused.
#[test]
fn a_file_naming_the_vpn_servers_address_as_a_target_is_refused() {
    let mut app = App::new();
    let config = format!("{ACCOUNT}{WITH_RULE}")
        .replace("vpn.example.com", "10.30.0.1")
        .replace("[10.30.0.0/16]", "[10.31.0.0/16, 10.30.0.1]");
    app.write_config(&config);
    app.service = Some(office_wifi());
    app.launch();
    app.settle_until("the file refused", |app| {
        app.bypass_status() == Some(&BypassStatus::ConfigInvalid {})
    });
}

#[test]
fn a_read_that_fails_after_the_helper_connection_was_lost_is_retried() {
    let mut app = app_with_the_rule_applied();
    app.routes_unreachable = true;
    app.send(ToRuntime::Event {
        event: SystemEvent::HelperConnectionLost {},
    });
    app.settle_until("the retry", |app| {
        app.bypass_status()
            == Some(&BypassStatus::Failed {
                failure: BypassFailure::HelperUnreachable {
                    failure: HelperChannelFailure::ConnectionLost {},
                },
                retry_in_seconds: Some(10),
                stopped_after_retries: None,
            })
    });
}

#[test]
fn a_read_that_still_fails_after_the_helper_connection_was_lost_stops_quietly() {
    let mut app = app_with_the_rule_applied();
    app.routes_unreachable = true;
    app.send(ToRuntime::Event {
        event: SystemEvent::HelperConnectionLost {},
    });
    app.settle_until("the first failure", |app| {
        matches!(app.bypass_status(), Some(BypassStatus::Failed { .. }))
    });
    for _ in 0..3 {
        let calls = app.route_calls();
        network_changed(&app);
        app.settle_until("another try", |app| app.route_calls() > calls);
        app.settle();
    }
    assert_eq!(
        app.bypass_status(),
        Some(&BypassStatus::Failed {
            failure: BypassFailure::HelperUnreachable {
                failure: HelperChannelFailure::ConnectionLost {},
            },
            retry_in_seconds: None,
            stopped_after_retries: Some(3),
        })
    );
    let notifications = app
        .actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                PortAction::PostNotification {
                    notification: UserNotification::ExceptionRulesNotApplied { .. },
                }
            )
        })
        .count();
    assert_eq!(notifications, 0);
}

#[test]
fn routes_the_routing_table_still_did_not_keep_are_reported_once() {
    let mut app = App::new();
    app.write_config(&format!("{ACCOUNT}{WITH_RULE}"));
    app.service = Some(office_wifi());
    app.keeps_routes = false;
    app.launch();
    want_the_vpn(&mut app);
    app.settle_until("the first failure", |app| {
        matches!(app.bypass_status(), Some(BypassStatus::Failed { .. }))
    });
    for _ in 0..4 {
        let calls = app.route_calls();
        network_changed(&app);
        app.settle_until("another try", |app| app.route_calls() > calls);
        app.settle();
    }
    assert_eq!(
        app.bypass_status(),
        Some(&BypassStatus::Failed {
            failure: BypassFailure::RoutesNotKept {},
            retry_in_seconds: None,
            stopped_after_retries: Some(3),
        })
    );
    let notifications = app
        .actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                PortAction::PostNotification {
                    notification: UserNotification::ExceptionRulesNotApplied { .. },
                }
            )
        })
        .count();
    assert_eq!(notifications, 1);
}

#[test]
fn writes_that_fail_once_the_helper_is_back_are_reported_after_reads_stopped_quietly() {
    let mut app = app_with_the_rule_applied();
    app.exception_routes.clear();
    app.routes_unreachable = true;
    app.keeps_routes = false;
    app.send(ToRuntime::Event {
        event: SystemEvent::HelperConnectionLost {},
    });
    app.settle_until("the first failure", |app| {
        matches!(app.bypass_status(), Some(BypassStatus::Failed { .. }))
    });
    for _ in 0..3 {
        let calls = app.route_calls();
        network_changed(&app);
        app.settle_until("another try", |app| app.route_calls() > calls);
        app.settle();
    }
    assert!(matches!(
        app.bypass_status(),
        Some(BypassStatus::Failed {
            stopped_after_retries: Some(3),
            ..
        })
    ));
    let notifications = app
        .actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                PortAction::PostNotification {
                    notification: UserNotification::ExceptionRulesNotApplied { .. },
                }
            )
        })
        .count();
    assert_eq!(notifications, 0);
    app.routes_unreachable = false;
    for _ in 0..4 {
        let calls = app.route_calls();
        network_changed(&app);
        app.settle_until("another try", |app| app.route_calls() > calls);
        app.settle();
    }
    assert_eq!(
        app.bypass_status(),
        Some(&BypassStatus::Failed {
            failure: BypassFailure::RoutesNotKept {},
            retry_in_seconds: None,
            stopped_after_retries: Some(3),
        })
    );
    let notifications = app
        .actions
        .iter()
        .filter(|action| {
            matches!(
                action,
                PortAction::PostNotification {
                    notification: UserNotification::ExceptionRulesNotApplied { .. },
                }
            )
        })
        .count();
    assert_eq!(notifications, 1);
}

/// A sync that failed and stopped is tried again once a readiness check finds the Helper.
#[test]
fn a_failed_sync_is_tried_again_when_the_helper_answers() {
    let mut app = App::new();
    app.write_config(&format!("{ACCOUNT}{WITH_RULE}"));
    app.service = Some(office_wifi());
    app.routes_unreachable = true;
    app.launch();
    app.settle_until("the failure", |app| {
        matches!(
            app.bypass_status(),
            Some(BypassStatus::Failed {
                retry_in_seconds: None,
                ..
            })
        )
    });
    app.routes_unreachable = false;
    // Registering again follows the approval with readiness checks.
    app.command(Command::RegisterHelper {});
    app.settle_until("the sync", |app| {
        app.bypass_status() == Some(&BypassStatus::VpnOff {})
    });
}

#[test]
fn the_helpers_output_reaches_the_log_file_as_it_came() {
    let marker = "FortiVPNManager: route script finished connecting";
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.secrets
        .push((vpn_password_item(), SecretText("vpn secret".into())));
    app.secrets
        .push((app_password_item(), SecretText("abcdabcdabcdabcd".into())));
    app.holds_gmail_reads = true;
    app.launch();
    app.settle();
    let attempt = app.start_run(Command::Connect {});
    let session = app.run_requests()[0].session_id;
    let batch = OutputBatch {
        attempt_id: attempt,
        lines: vec![
            fvm_contract::app_helper::OutputLine {
                date: std::time::SystemTime::now(),
                source: fvm_contract::app_helper::OutputSource::Helper,
                text: "‹password sent›".into(),
            },
            fvm_contract::app_helper::OutputLine {
                date: std::time::SystemTime::now(),
                source: fvm_contract::app_helper::OutputSource::Info,
                text: marker.into(),
            },
        ],
    };
    app.send(ToRuntime::Event {
        event: fvm_contract::app_internal::event::SystemEvent::HelperOutput {
            json: serde_json::to_string(&batch).unwrap(),
        },
    });
    let ends = [
        format!(" [▸] {session} ‹password sent›"),
        format!(" [info] {session} {marker}"),
    ];
    app.settle_until("the lines in the log file", |app| {
        let file = fs::read_to_string(app.folder.join("logs/vpn_backend.log")).unwrap_or_default();
        let lines: Vec<&str> = file.lines().collect();
        lines.len() == 2
            && lines
                .iter()
                .zip(&ends)
                .all(|(line, end)| line.ends_with(end.as_str()))
    });
}

#[test]
fn reconnect_now_runs_the_next_tunnel_beside_the_connected_one() {
    let first_ip = "10.212.134.5".parse().unwrap();
    let next_ip = "10.212.134.9".parse().unwrap();
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.secrets
        .push((vpn_password_item(), SecretText("vpn secret".into())));
    app.secrets
        .push((app_password_item(), SecretText("abcdabcdabcdabcd".into())));
    app.holds_gmail_reads = true;
    app.launch();
    app.settle();
    let old = app.start_run(Command::Connect {});
    assert_eq!(app.run_requests()[0].attempt_id, old);
    assert_eq!(app.run_requests()[0].replacing, None);
    let old_session = app.run_requests()[0].session_id;
    app.bring_up(&old, first_ip);
    app.output_of(&old, "‹old: up›");

    let next = app.start_run(Command::ReconnectNow {});
    assert_ne!(next, old);
    let request = &app.run_requests()[1];
    assert_eq!(request.attempt_id, next);
    assert_eq!(request.replacing, Some(old.clone()));
    let next_session = request.session_id;
    assert!(next_session.0 > old_session.0);
    assert_eq!(
        app.connection_status(),
        Some(&ConnectionStatus::Connecting {
            reconnecting: true,
            progress: ConnectProgress::ContactingServer,
        })
    );
    app.output_of(&old, "‹old: still up›");
    app.output_of(&next, "‹next: starting›");
    app.output_of(&old, "‹old: rekey›");
    app.output_of(&next, "‹next: logged in›");
    app.bring_up(&next, next_ip);
    app.helper_event(
        &old,
        HelperEventKind::Exited {
            facts: EndFacts {
                furthest_milestone: Milestone::TunnelUp,
                stop_initiator: StopInitiator::Helper {
                    reason: StopReason::Replaced,
                },
                cert_rejected_digests: Vec::new(),
                cert_rejected_at: None,
                tunnel_setup_failed: false,
                tunnel_address_in_use: false,
                end_code: -4,
            },
        },
    );
    app.output_of(&old, "‹old: exited›");
    app.settle();
    assert!(matches!(
        app.connection_status(),
        Some(ConnectionStatus::Connected { tunnel_ip: Some(ip), .. }) if *ip == next_ip
    ));
    assert_eq!(app.run_requests().len(), 2);
    assert!(!app.asked_helper(|call| matches!(call, HelperCall::Disconnect { .. })));
    assert_eq!(
        log_marks(&app),
        [
            ("‹old: up›", true),
            ("‹old: still up›", false),
            ("‹next: starting›", true),
            ("‹old: rekey›", false),
            ("‹next: logged in›", false),
            ("‹old: exited›", false),
        ]
    );
    let (old_id, next_id) = (old_session.to_string(), next_session.to_string());
    assert_eq!(
        log_sessions(&app),
        [&old_id, &old_id, &next_id, &old_id, &next_id, &old_id]
    );
}

fn forticlient_connection(name: &str, server: &str) -> FortiClientConnection {
    FortiClientConnection {
        name: name.into(),
        server: server.into(),
    }
}

impl App {
    fn import_report(&self) -> Option<ClientSettingsImport> {
        self.config_section().account.import_report.clone()
    }
}

#[test]
fn the_first_launch_imports_the_server_of_forticlients_one_connection() {
    let mut app = App::new();
    app.forticlient = Some(vec![forticlient_connection(
        "Office",
        "vpn.example.com:10443",
    )]);
    app.launch();
    app.settle_until("the import report", |app| {
        app.sections.iter().any(|section| matches!(section, Section::Config { section } if section.account.import_report.is_some()))
    });
    app.settle();
    let Some(ClientSettingsImport::Imported { report }) = app.import_report() else {
        panic!("an import report")
    };
    assert_eq!(report.source, "Office");
    assert_eq!(
        (report.host.as_str(), report.port),
        ("vpn.example.com", 10443)
    );
    assert_eq!(report.username, "");
    let text = app.config_text();
    for line in ["  host: \"vpn.example.com\"\n", "  port: 10443\n"] {
        assert!(text.contains(line), "{text}");
    }
}

#[test]
fn importing_from_forticlient_keeps_the_account_and_takes_only_the_server() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.forticlient = Some(vec![forticlient_connection("Office", "gw.example.com")]);
    app.launch();
    app.settle();
    app.command(Command::ImportClientSettings { file: None });
    app.settle();
    let Some(ClientSettingsImport::Imported { report }) = app.import_report() else {
        panic!("an import report")
    };
    assert_eq!((report.host.as_str(), report.port), ("gw.example.com", 443));
    assert_eq!(report.username, "alice");
    let text = app.config_text();
    for line in [
        "  host: \"gw.example.com\"\n",
        "  port: 443\n",
        "  username: \"alice\"\n",
    ] {
        assert!(text.contains(line), "{text}");
    }
}

#[test]
fn importing_from_forticlient_tells_why_nothing_was_taken() {
    for (connections, expected) in [
        (vec![], ImportError::NoFortiClientConnection {}),
        (
            vec![forticlient_connection("Office", "https://vpn.example.com/")],
            ImportError::InvalidFortiClientServer {
                connection: "Office".into(),
            },
        ),
    ] {
        let mut app = App::new();
        app.write_config(ACCOUNT);
        app.forticlient = Some(connections);
        app.launch();
        app.settle();
        app.command(Command::ImportClientSettings { file: None });
        app.settle();
        assert_eq!(
            app.import_report(),
            Some(ClientSettingsImport::Failed { error: expected })
        );
        assert_eq!(app.config_text(), ACCOUNT);
    }
}

fn is_update_call(call: &HelperCall) -> bool {
    matches!(
        call,
        HelperCall::PrepareUpdate { .. }
            | HelperCall::InstallUpdate { .. }
            | HelperCall::ReadPreparedUpdate {}
            | HelperCall::DiscardPreparedUpdate {}
    )
}

fn helper_reply(reply: &impl serde::Serialize) -> PortResponse {
    PortResponse::HelperReply {
        json: Some(serde_json::to_string(reply).unwrap()),
    }
}

const INSTALLER_BYTES: usize = 4_000;
fn installer_name() -> String {
    let release = Release::from_json(&full_release("26.2.0")).unwrap();
    match fvm_core::auto_update::check(version(APP_VERSION), &release, crate::auto_update::PLATFORM)
    {
        Check::Available(update) => update.installer_file_name,
        other => panic!("26.2.0 is not available: {other:?}"),
    }
}

fn installer() -> Vec<u8> {
    (0..INSTALLER_BYTES)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect()
}

fn release(version: &str, assets: &[&str]) -> Vec<u8> {
    let assets: Vec<Value> = assets
        .iter()
        .map(|name| json!({"name": name, "size": INSTALLER_BYTES}))
        .collect();
    json!({"tag_name": format!("v{version}"), "body": "", "assets": assets})
        .to_string()
        .into_bytes()
}

fn full_release(version: &str) -> Vec<u8> {
    release(
        version,
        &[
            &format!("FortiVPNManager-{version}.pkg"),
            &format!("FortiVPNManager-{version}.msi"),
            "SHA256SUMS.txt",
            "SHA256SUMS.txt.sig",
        ],
    )
}

fn update_available(version: &str) -> UpdateStatus {
    UpdateStatus::Available {
        version: version.into(),
        release_page_url: format!(
            "https://github.com/qoor/FortiVPNManager/releases/tag/v{version}"
        ),
        installer_bytes: INSTALLER_BYTES as u64,
        release_note_titles: Default::default(),
    }
}

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn ready_to_restart() -> UpdateProgress {
    UpdateProgress::ReadyToRestart {
        version: "26.2.0".into(),
    }
}

#[derive(Clone)]
struct Route {
    status: u16,
    body: Vec<u8>,
    held_after: Option<usize>,
    closes: bool,
}

impl Route {
    fn ok(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            body,
            held_after: None,
            closes: false,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::ok(Vec::new())
        }
    }

    fn held_after(bytes: usize, body: Vec<u8>) -> Self {
        Self {
            held_after: Some(bytes),
            ..Self::ok(body)
        }
    }

    fn closed() -> Self {
        Self {
            closes: true,
            ..Self::ok(Vec::new())
        }
    }
}

struct Server {
    url: String,
    routes: Arc<Mutex<HashMap<String, Route>>>,
    hits: Arc<Mutex<Vec<String>>>,
    release: Sender<()>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let hits = Arc::new(Mutex::new(Vec::new()));
        let (release, released) = mpsc::channel();
        let released = Arc::new(Mutex::new(released));
        let (served_routes, served_hits) = (Arc::clone(&routes), Arc::clone(&hits));
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let (routes, hits, released) = (
                    Arc::clone(&served_routes),
                    Arc::clone(&served_hits),
                    Arc::clone(&released),
                );
                thread::spawn(move || serve(stream, &routes, &hits, &released));
            }
        });
        Self {
            url,
            routes,
            hits,
            release,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }

    fn route(&self, path: &str, route: Route) {
        self.routes.lock().unwrap().insert(path.into(), route);
    }

    fn hits(&self, path: &str) -> usize {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .filter(|hit| *hit == path)
            .count()
    }

    fn release(&self) {
        self.release.send(()).unwrap();
    }
}

fn serve(
    mut stream: TcpStream,
    routes: &Mutex<HashMap<String, Route>>,
    hits: &Mutex<Vec<String>>,
    released: &Mutex<Receiver<()>>,
) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if line == "\r\n" => break,
            Ok(_) => {}
        }
    }
    let path = request
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    hits.lock().unwrap().push(path.clone());
    let route = routes
        .lock()
        .unwrap()
        .get(&path)
        .cloned()
        .unwrap_or_else(|| Route::status(404));
    if route.closes {
        return;
    }
    let head = format!(
        "HTTP/1.1 {} Reply\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        route.status,
        route.body.len()
    );
    let first = route.held_after.unwrap_or(route.body.len());
    let sent = stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(&route.body[..first]))
        .and_then(|()| stream.flush());
    if sent.is_err() || first == route.body.len() {
        return;
    }
    if released.lock().unwrap().recv().is_ok() {
        let _ = stream.write_all(&route.body[first..]);
    }
}

fn update_server() -> Server {
    let server = Server::start();
    server.route("/latest", Route::ok(full_release("26.2.0")));
    server.route("/installer", Route::ok(installer()));
    server.route("/sums", Route::ok(b"checksums".to_vec()));
    server.route("/sig", Route::ok(b"signature".to_vec()));
    server
}

impl App {
    fn downloads(&self) -> PathBuf {
        self.folder.join("downloads")
    }

    fn state_path(&self) -> PathBuf {
        self.folder.join("state/state.json")
    }

    fn write_state(&self, state: &Value) {
        fs::create_dir_all(self.state_path().parent().unwrap()).unwrap();
        fs::write(self.state_path(), state.to_string()).unwrap();
    }

    fn state(&self) -> Value {
        serde_json::from_slice(&fs::read(self.state_path()).unwrap()).unwrap()
    }

    fn update_section(&self) -> &AutoUpdateSection {
        self.sections
            .iter()
            .find_map(|section| match section {
                Section::AutoUpdate { section } => Some(section.as_ref()),
                _ => None,
            })
            .expect("an auto update section")
    }

    fn update_progress(&self) -> &UpdateProgress {
        &self.update_section().progress
    }

    fn settle_until_progress(&mut self, what: &str, wanted: impl Fn(&UpdateProgress) -> bool) {
        self.settle_until(what, |app| wanted(app.update_progress()));
    }

    fn settle_for(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.out.recv_timeout(left) {
                Ok(message) => self.take(message),
                Err(RecvTimeoutError::Timeout) => return,
                Err(RecvTimeoutError::Disconnected) => panic!("the runtime stopped"),
            }
        }
    }

    fn checked(&mut self, status: u16, body: Vec<u8>) {
        self.ask(move |runtime| runtime.update_checked(Ok(Reply { status, body })));
        self.settle();
    }

    fn update_notifications(&self) -> Vec<&UserNotification> {
        self.actions
            .iter()
            .filter_map(|action| match action {
                PortAction::PostNotification {
                    notification: notification @ UserNotification::UpdateAvailable { .. },
                } => Some(notification),
                _ => None,
            })
            .collect()
    }

    fn update_logs(&self) -> usize {
        self.actions
            .iter()
            .filter(|action| {
                matches!(
                    action,
                    PortAction::Log {
                        category: LogCategory::Update,
                        ..
                    }
                )
            })
            .count()
    }

    fn logged_update(&self, text: &str) -> bool {
        self.actions.iter().any(|action| {
            matches!(
                action,
                PortAction::Log { category: LogCategory::Update, message, .. } if message.contains(text)
            )
        })
    }

    fn stopped_update_waiters(&self) -> usize {
        self.actions
            .iter()
            .filter(|action| **action == PortAction::StopUpdateWaiter {})
            .count()
    }

    fn helper_calls(&self, wanted: impl Fn(&HelperCall) -> bool) -> usize {
        self.requests
            .iter()
            .filter(|request| matches!(request, PortRequest::CallHelper { call } if wanted(call)))
            .count()
    }

    fn prepare_requests(&self) -> Vec<UpdatePrepareRequest> {
        self.requests
            .iter()
            .filter_map(|request| match request {
                PortRequest::CallHelper {
                    call: HelperCall::PrepareUpdate { json },
                } => Some(serde_json::from_str(json).unwrap()),
                _ => None,
            })
            .collect()
    }

    fn discards(&self) -> usize {
        self.helper_calls(|call| matches!(call, HelperCall::DiscardPreparedUpdate {}))
    }

    fn update_sections_since(&self, since: usize) -> Vec<&AutoUpdateSection> {
        self.update_sections[since..]
            .iter()
            .map(|(_, section)| section)
            .collect()
    }

    fn download_from(&self, server: &Server) {
        let (installer, checksums, signature) = (
            server.url("/installer"),
            server.url("/sums"),
            server.url("/sig"),
        );
        self.ask(move |runtime| {
            let Ok(Some(update)) = &mut runtime.updates.found else {
                panic!("an update was found")
            };
            update.installer_url = installer;
            update.checksums_url = checksums;
            update.checksums_signature_url = signature;
        });
    }

    fn launch_until_helper_runs(&mut self) {
        self.launch();
        self.settle_until("the Helper's build", |app| {
            matches!(app.helper_state(), Some(HelperState::Running { .. }))
        });
        self.settle();
    }

    fn answer_held_update_calls(&mut self) {
        self.holds_update_calls = false;
        for (id, call) in std::mem::take(&mut self.held_update_calls) {
            let response = self.helper(call);
            self.send(ToRuntime::PortResponse { id, response });
        }
    }

    fn prepared_questions(&self) -> usize {
        self.helper_calls(|call| matches!(call, HelperCall::ReadPreparedUpdate {}))
    }

    fn update_now_until_ready(&mut self) {
        self.command(Command::UpdateNow {});
        self.settle_until_progress("the prepared update", |progress| {
            *progress == ready_to_restart()
        });
    }

    fn quit_approved(&self) -> bool {
        self.actions.contains(&PortAction::ApproveQuit {})
    }

    fn position(&self, wanted: impl Fn(&PortRequest) -> bool) -> usize {
        self.requests
            .iter()
            .position(wanted)
            .expect("the request was made")
    }

    fn update_stops(&self) -> Vec<(AttemptId, StopReason)> {
        self.requests
            .iter()
            .filter_map(|request| match request {
                PortRequest::CallHelper {
                    call: HelperCall::Disconnect { attempt, reason },
                } => Some((attempt.clone(), *reason)),
                _ => None,
            })
            .collect()
    }

    fn connection_buttons(&self) -> [ButtonState; 3] {
        let section = self
            .sections
            .iter()
            .find_map(|section| match section {
                Section::Connection { section } => Some(section.as_ref()),
                _ => None,
            })
            .expect("a connection section");
        [section.connect, section.disconnect, section.reconnect_now]
    }

    fn exited(&self, attempt: &AttemptId, furthest: Milestone) {
        self.helper_event(
            attempt,
            HelperEventKind::Exited {
                facts: EndFacts {
                    furthest_milestone: furthest,
                    stop_initiator: StopInitiator::Helper {
                        reason: StopReason::AppQuit,
                    },
                    cert_rejected_digests: Vec::new(),
                    cert_rejected_at: None,
                    tunnel_setup_failed: false,
                    tunnel_address_in_use: false,
                    end_code: -4,
                },
            },
        );
    }
}

fn app_with_a_served_update(server: &Server) -> App {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.update_url = server.url("/latest");
    app.launch_until_helper_runs();
    assert_eq!(app.update_section().status, update_available("26.2.0"));
    app.download_from(server);
    app
}

fn app_ready_to_restart(server: &Server) -> App {
    let mut app = app_with_a_served_update(server);
    app.update_now_until_ready();
    app
}

#[test]
fn without_a_url_the_section_shows_the_running_version_and_nothing_is_checked() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    let initial = AutoUpdateSection {
        current_version: APP_VERSION.into(),
        status: UpdateStatus::UpToDate {},
        failed_install_version: None,
        progress: UpdateProgress::Idle {},
        check: ButtonState::Enabled,
        update_now: ButtonState::Hidden,
        cancel: ButtonState::Hidden,
        restart: ButtonState::Hidden,
        card_dot: false,
        menu_badge: false,
    };
    assert_eq!(*app.update_section(), initial);
    for command in [
        Command::CheckForUpdates {},
        Command::UpdatePageOpened {},
        Command::UpdateNow {},
        Command::UpdateNowFromNotification {},
        Command::CancelUpdateDownload {},
        Command::RestartToUpdate {},
    ] {
        app.command(command);
    }
    app.send(ToRuntime::Event {
        event: SystemEvent::DidWake {},
    });
    app.inputs.send(Input::Timer(Timer::UpdateCheck)).unwrap();
    app.settle();
    assert_eq!(*app.update_section(), initial);
    assert!(
        !app.requests
            .iter()
            .any(|request| matches!(request, PortRequest::StartUpdateWaiter { .. }))
    );
    assert!(!app.asked_helper(|call| matches!(
        call,
        HelperCall::PrepareUpdate { .. } | HelperCall::InstallUpdate { .. }
    )));

    app.ask(|runtime| runtime.update_checked(Err("dns error".into())));
    app.send(ToRuntime::Event {
        event: SystemEvent::NetworkPathChanged {
            satisfied: true,
            interfaces: Vec::new(),
        },
    });
    app.settle();
    assert_eq!(
        app.update_section().status,
        UpdateStatus::CheckFailed {
            reason: UpdateCheckFailure::Network {
                detail: "dns error".into()
            },
        }
    );
    assert!(!app.update_section().card_dot);
    assert!(!app.update_section().menu_badge);
}

#[test]
fn a_launch_after_an_install_of_a_newer_version_shows_it_failed_and_clears_the_record() {
    for (target, failed) in [
        ("26.2.0", Some("26.2.0")),
        ("26.1.0", None),
        ("26.0.9", None),
    ] {
        let mut app = App::new();
        app.write_config(ACCOUNT);
        app.write_state(&json!({"loginItemRegistered": true, "updateTargetVersion": target}));
        app.launch();
        app.settle();
        assert_eq!(
            app.update_section().failed_install_version.as_deref(),
            failed,
            "{target}"
        );
        assert_eq!(
            app.state(),
            json!({"loginItemRegistered": true}),
            "{target}"
        );
    }
}

#[test]
fn a_launch_after_an_install_that_began_connected_connects_where_a_launch_would_not() {
    for connected_before_update in [true, false] {
        let mut app = App::new();
        app.write_config(ACCOUNT);
        app.secrets
            .push((vpn_password_item(), SecretText("vpn secret".into())));
        app.secrets
            .push((app_password_item(), SecretText("abcdabcdabcdabcd".into())));
        app.holds_gmail_reads = true;
        let mut state = json!({"loginItemRegistered": false, "updateTargetVersion": "26.1.0"});
        if connected_before_update {
            state["connectedBeforeUpdate"] = json!(true);
        }
        app.write_state(&state);
        app.launch();
        if connected_before_update {
            app.settle_until("the attempt's OTP session", |app| app.held_gmail_reads > 0);
        } else {
            app.settle();
            assert_eq!(app.held_gmail_reads, 0);
            assert_eq!(
                app.connection_status(),
                Some(&ConnectionStatus::NotConnected {})
            );
        }
        let state = app.state();
        assert_eq!(state.get("connectedBeforeUpdate"), None, "{state}");
        assert_eq!(state.get("updateTargetVersion"), None, "{state}");
    }
}

#[test]
fn each_check_outcome_shows_in_the_section_and_failures_are_logged() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    for (status, body, shown) in [
        (
            403,
            Vec::new(),
            UpdateStatus::CheckFailed {
                reason: UpdateCheckFailure::HttpStatus { code: 403 },
            },
        ),
        (
            200,
            b"{}".to_vec(),
            UpdateStatus::CheckFailed {
                reason: UpdateCheckFailure::InvalidReply {},
            },
        ),
        (
            200,
            release("26.2.0", &["SHA256SUMS.txt", "SHA256SUMS.txt.sig"]),
            UpdateStatus::CheckFailed {
                reason: UpdateCheckFailure::InstallerMissing {},
            },
        ),
        (200, release(APP_VERSION, &[]), UpdateStatus::UpToDate {}),
    ] {
        let logs = app.update_logs();
        app.checked(status, body);
        assert_eq!(app.update_section().status, shown);
        assert_eq!(app.update_section().update_now, ButtonState::Hidden);
        let failed = matches!(shown, UpdateStatus::CheckFailed { .. });
        assert_eq!(app.update_logs(), logs + usize::from(failed), "{shown:?}");
    }
    app.ask(|runtime| runtime.update_checked(Err("dns error".into())));
    app.settle();
    assert_eq!(
        app.update_section().status,
        UpdateStatus::CheckFailed {
            reason: UpdateCheckFailure::Network {
                detail: "dns error".into()
            },
        }
    );
    assert_eq!(app.update_notifications(), Vec::<&UserNotification>::new());
}

#[test]
fn a_version_is_announced_once_even_across_launches() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.write_state(&json!({"loginItemRegistered": false, "lastNotifiedUpdateVersion": "26.2.0"}));
    app.launch();
    app.settle();
    app.checked(200, full_release("26.2.0"));
    assert_eq!(app.update_section().status, update_available("26.2.0"));
    assert_eq!(app.update_notifications(), Vec::<&UserNotification>::new());

    for _ in 0..2 {
        app.checked(200, full_release("26.3.0"));
    }
    assert_eq!(app.update_section().status, update_available("26.3.0"));
    assert_eq!(
        app.update_notifications(),
        [&UserNotification::UpdateAvailable {
            version: "26.3.0".into()
        }]
    );
    assert_eq!(app.state()["lastNotifiedUpdateVersion"], json!("26.3.0"));
}

#[test]
fn updating_needs_the_helper_running() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.helper_registration = RegistrationStatus::NotRegistered;
    app.launch();
    app.settle();
    app.checked(200, full_release("26.2.0"));
    let section = app.update_section();
    assert_eq!(section.update_now, ButtonState::Disabled);
    assert!(section.card_dot);
    assert!(section.menu_badge);
    app.command(Command::UpdateNow {});
    app.settle();
    assert_eq!(*app.update_progress(), UpdateProgress::Idle {});
}

#[test]
fn an_update_downloads_then_the_helper_prepares_it_for_a_restart() {
    let server = update_server();
    server.route(
        "/installer",
        Route::held_after(INSTALLER_BYTES / 2, installer()),
    );
    let mut app = app_with_a_served_update(&server);
    let section = app.update_section();
    assert_eq!(
        (
            section.check,
            section.update_now,
            section.cancel,
            section.restart
        ),
        (
            ButtonState::Enabled,
            ButtonState::Enabled,
            ButtonState::Hidden,
            ButtonState::Hidden
        )
    );
    assert!(section.card_dot && section.menu_badge);
    let before = app.update_sections.len();

    app.command(Command::UpdateNow {});
    app.settle_until_progress("half the installer", |progress| {
        matches!(progress, UpdateProgress::Downloading { received_bytes, .. } if *received_bytes == INSTALLER_BYTES as u64 / 2)
    });
    app.settle_for(Duration::from_millis(2_500));
    let section = app.update_section().clone();
    assert_eq!(
        section.progress,
        UpdateProgress::Downloading {
            received_bytes: INSTALLER_BYTES as u64 / 2,
            total_bytes: INSTALLER_BYTES as u64,
            percent: 30,
            remaining: RemainingTime::Seconds { seconds: 5 },
        }
    );
    assert_eq!(section.status, update_available("26.2.0"));
    assert_eq!(
        (
            section.check,
            section.update_now,
            section.cancel,
            section.restart
        ),
        (
            ButtonState::Disabled,
            ButtonState::Hidden,
            ButtonState::Enabled,
            ButtonState::Hidden
        )
    );
    assert!(!section.card_dot && section.menu_badge);
    let downloads: Vec<(Instant, &UpdateProgress)> = app.update_sections[before..]
        .iter()
        .map(|(at, section)| (*at, &section.progress))
        .collect();
    assert_eq!(
        downloads[0].1,
        &UpdateProgress::Downloading {
            received_bytes: 0,
            total_bytes: INSTALLER_BYTES as u64,
            percent: 0,
            remaining: RemainingTime::Calculating {},
        }
    );
    assert!(downloads.len() >= 3, "{downloads:?}");
    for pair in downloads.windows(2) {
        assert!(
            pair[1].0 - pair[0].0 >= Duration::from_millis(800),
            "{downloads:?}"
        );
    }

    let before = app.update_sections.len();
    app.holds_update_calls = true;
    server.release();
    app.settle_until("the held preparation", |app| {
        app.held_update_calls
            .iter()
            .any(|(_, call)| matches!(call, HelperCall::PrepareUpdate { .. }))
    });
    let downloads = app.downloads();
    let path = |name: &str| downloads.join(name).display().to_string();
    let prepared = app.prepare_requests();
    assert_eq!(
        prepared,
        [UpdatePrepareRequest {
            installer_path: path(&installer_name()),
            checksums_path: path("SHA256SUMS.txt"),
            signature_path: path("SHA256SUMS.txt.sig"),
        }]
    );
    assert_eq!(fs::read(&prepared[0].installer_path).unwrap(), installer());
    assert_eq!(fs::read(&prepared[0].checksums_path).unwrap(), b"checksums");
    assert_eq!(fs::read(&prepared[0].signature_path).unwrap(), b"signature");
    app.answer_held_update_calls();
    app.settle_until_progress("the prepared update", |progress| {
        *progress == ready_to_restart()
    });
    let preparing = app
        .update_sections_since(before)
        .into_iter()
        .find(|section| matches!(section.progress, UpdateProgress::Preparing { .. }))
        .expect("a preparing section")
        .clone();
    assert_eq!(
        preparing.progress,
        UpdateProgress::Preparing { percent: 60 }
    );
    assert_eq!(
        (
            preparing.check,
            preparing.update_now,
            preparing.cancel,
            preparing.restart
        ),
        (
            ButtonState::Disabled,
            ButtonState::Hidden,
            ButtonState::Hidden,
            ButtonState::Hidden
        )
    );
    assert!(!preparing.card_dot && preparing.menu_badge);
    let section = app.update_section();
    assert_eq!(
        (
            section.check,
            section.update_now,
            section.cancel,
            section.restart
        ),
        (
            ButtonState::Enabled,
            ButtonState::Hidden,
            ButtonState::Hidden,
            ButtonState::Enabled
        )
    );
    assert!(section.card_dot && section.menu_badge);
    assert!(!downloads.exists());
    assert!(app.logged_update("update 26.2.0 prepared"));
    assert!(!app.asked_helper(|call| matches!(call, HelperCall::InstallUpdate { .. })));
    assert!(
        !app.requests
            .iter()
            .any(|request| matches!(request, PortRequest::StartUpdateWaiter { .. }))
    );
}

#[test]
fn cancelling_the_download_deletes_it_and_shows_the_update_again() {
    let server = update_server();
    server.route(
        "/installer",
        Route::held_after(INSTALLER_BYTES / 2, installer()),
    );
    let mut app = app_with_a_served_update(&server);
    app.command(Command::UpdateNow {});
    app.settle_until_progress("half the installer", |progress| {
        matches!(progress, UpdateProgress::Downloading { received_bytes, .. } if *received_bytes == INSTALLER_BYTES as u64 / 2)
    });
    assert_eq!(
        fs::read(app.downloads().join(installer_name()))
            .unwrap()
            .len(),
        INSTALLER_BYTES / 2
    );
    app.ask(|runtime| runtime.updates.last_check = Some(SystemTime::UNIX_EPOCH));
    for command in [
        Command::CheckForUpdates {},
        Command::UpdatePageOpened {},
        Command::UpdateNowFromNotification {},
        Command::UpdateNow {},
        Command::RestartToUpdate {},
    ] {
        app.command(command);
    }
    app.settle();
    assert_eq!(server.hits("/latest"), 1);
    assert_eq!(server.hits("/installer"), 1);
    assert!(matches!(
        app.update_progress(),
        UpdateProgress::Downloading { .. }
    ));
    app.ask(|runtime| runtime.updates.last_check = Some(SystemTime::now()));

    app.command(Command::CancelUpdateDownload {});
    app.settle_until_progress("the cancelled download", |progress| {
        *progress == UpdateProgress::Idle {}
    });
    assert!(!app.downloads().exists());
    let section = app.update_section();
    assert_eq!(section.status, update_available("26.2.0"));
    assert_eq!(section.failed_install_version, None);
    assert_eq!(
        (section.check, section.update_now, section.cancel),
        (
            ButtonState::Enabled,
            ButtonState::Enabled,
            ButtonState::Hidden
        )
    );
    assert!(section.card_dot && section.menu_badge);
    assert_eq!(app.prepare_requests(), []);
    assert!(app.logged_update("downloading update 26.2.0 cancelled"));

    server.route("/installer", Route::ok(installer()));
    let before = app.update_sections.len();
    app.update_now_until_ready();
    assert_eq!(server.hits("/installer"), 2);
    assert!(matches!(
        app.update_sections_since(before)[0].progress,
        UpdateProgress::Downloading {
            received_bytes: 0,
            ..
        }
    ));
    assert_eq!(app.prepare_requests().len(), 1);
    assert!(!app.downloads().exists());
}

#[test]
fn a_failed_download_shows_why_and_offers_to_try_again() {
    let server = update_server();
    server.route("/sums", Route::status(404));
    let mut app = app_with_a_served_update(&server);
    let logs = app.update_logs();
    app.command(Command::UpdateNow {});
    app.settle_until_progress("the failed download", |progress| {
        matches!(progress, UpdateProgress::Failed { .. })
    });
    let section = app.update_section();
    assert_eq!(
        section.progress,
        UpdateProgress::Failed {
            reason: InstallFailure::DownloadFailed {
                detail: "SHA256SUMS.txt: HTTP 404".into()
            }
        }
    );
    assert_eq!(
        (
            section.check,
            section.update_now,
            section.cancel,
            section.restart
        ),
        (
            ButtonState::Enabled,
            ButtonState::Enabled,
            ButtonState::Hidden,
            ButtonState::Hidden
        )
    );
    assert!(section.card_dot && section.menu_badge);
    assert_eq!(app.update_logs(), logs + 1);
    assert_eq!(app.prepare_requests(), []);
    assert!(!app.downloads().exists());

    server.route("/sums", Route::ok(b"checksums".to_vec()));
    app.update_now_until_ready();
}

#[test]
fn a_failed_preparation_shows_why_and_offers_to_try_again() {
    let unreadable = serde_json::from_str::<UpdatePrepareReply>("{}").unwrap_err();
    let server = update_server();
    let mut app = app_with_a_served_update(&server);
    for (answer, expected) in [
        (
            PortResponse::HelperReply { json: None },
            InstallFailure::HelperFailed {
                message: "the Helper sent an empty reply".into(),
            },
        ),
        (
            PortResponse::HelperReply {
                json: Some("{}".into()),
            },
            InstallFailure::HelperFailed {
                message: format!("the Helper sent an unreadable reply: {unreadable}"),
            },
        ),
        (
            helper_reply(&UpdatePrepareReply::Refused {
                reason: UpdateRefusal::SignatureInvalid,
                message: "SHA256SUMS.txt.sig does not verify".into(),
            }),
            InstallFailure::HelperRefused {
                reason: UpdateRefusal::SignatureInvalid,
                message: "SHA256SUMS.txt.sig does not verify".into(),
            },
        ),
        (
            helper_reply(&UpdatePrepareReply::Failure {
                error: HelperError {
                    code: HelperErrorCode::InternalError,
                    message: "could not copy the installer".into(),
                },
            }),
            InstallFailure::HelperFailed {
                message: "could not copy the installer".into(),
            },
        ),
        (
            PortResponse::HelperUnreachable {
                failure: HelperChannelFailure::NoReply {},
            },
            InstallFailure::HelperUnreachable {
                failure: HelperChannelFailure::NoReply {},
            },
        ),
        (
            PortResponse::HelperBuildMismatch {
                helper_build: "99".into(),
            },
            InstallFailure::HelperBuildMismatch {
                helper_build: "99".into(),
                app_build: APP_BUILD.into(),
            },
        ),
    ] {
        app.prepare_answer = answer;
        let logs = app.update_logs();
        app.command(Command::UpdateNow {});
        let failed = UpdateProgress::Failed {
            reason: expected.clone(),
        };
        app.settle_until_progress("the failed preparation", |progress| *progress == failed);
        app.settle();
        let section = app.update_section();
        assert_eq!(section.update_now, ButtonState::Enabled, "{expected:?}");
        assert_eq!(section.restart, ButtonState::Hidden, "{expected:?}");
        assert_eq!(app.update_logs(), logs + 1, "{expected:?}");
        assert!(!app.downloads().exists(), "{expected:?}");
    }
    assert_eq!(app.stopped_update_waiters(), 0);
    assert_eq!(app.state().get("updateTargetVersion"), None);
    app.prepare_answer = helper_reply(&UpdatePrepareReply::Prepared {
        version: version("26.2.0"),
    });
    app.update_now_until_ready();
}

fn minutes_ago(minutes: u64, seconds: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(minutes * 60 + seconds)
}

#[test]
fn opening_the_page_checks_when_the_last_check_is_five_minutes_old() {
    let server = update_server();
    let mut app = app_with_a_served_update(&server);
    assert_eq!(server.hits("/latest"), 1);
    app.command(Command::UpdatePageOpened {});
    app.settle();
    assert_eq!(server.hits("/latest"), 1);
    app.ask(|runtime| runtime.updates.last_check = Some(minutes_ago(4, 50)));
    app.command(Command::UpdatePageOpened {});
    app.settle();
    assert_eq!(server.hits("/latest"), 1);
    app.ask(|runtime| runtime.updates.last_check = Some(minutes_ago(5, 0)));
    app.command(Command::UpdatePageOpened {});
    app.settle();
    assert_eq!(server.hits("/latest"), 2);

    server.route("/latest", Route::held_after(0, full_release("26.3.0")));
    app.command(Command::CheckForUpdates {});
    app.settle();
    assert_eq!(server.hits("/latest"), 3);
    assert_eq!(app.update_section().status, UpdateStatus::Checking {});
    assert_eq!(app.update_section().check, ButtonState::Disabled);
    app.ask(|runtime| runtime.updates.last_check = Some(minutes_ago(6, 0)));
    app.command(Command::UpdatePageOpened {});
    app.settle();
    assert_eq!(server.hits("/latest"), 3);
    server.release();
    app.settle_until("the check", |app| {
        app.update_section().status == update_available("26.3.0")
    });
}

#[test]
fn a_check_in_ready_to_restart_discards_the_files_when_the_latest_differs() {
    let server = update_server();
    let mut app = app_ready_to_restart(&server);
    app.ask(|runtime| runtime.updates.last_check = Some(minutes_ago(5, 0)));
    app.command(Command::UpdatePageOpened {});
    app.settle();
    assert_eq!(server.hits("/latest"), 2);
    assert_eq!(*app.update_progress(), ready_to_restart());
    assert_eq!(app.discards(), 0);

    server.route("/latest", Route::ok(full_release("26.3.0")));
    app.command(Command::UpdatePageOpened {});
    app.settle();
    assert_eq!(server.hits("/latest"), 2);
    app.ask(|runtime| runtime.updates.last_check = Some(minutes_ago(5, 0)));
    app.command(Command::UpdatePageOpened {});
    app.settle();
    assert_eq!(server.hits("/latest"), 3);
    assert_eq!(app.discards(), 1);
    assert!(app.logged_update("discarded the prepared update 26.2.0"));
    let section = app.update_section();
    assert_eq!(section.progress, UpdateProgress::Idle {});
    assert_eq!(section.status, update_available("26.3.0"));
    assert_eq!(section.update_now, ButtonState::Enabled);
    assert_eq!(section.restart, ButtonState::Hidden);
}

#[test]
fn update_now_from_the_notification_checks_and_starts_an_available_update() {
    let server = update_server();
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.update_url = server.url("/latest");
    fs::write(app.downloads(), b"").unwrap();
    app.launch_until_helper_runs();
    assert_eq!(server.hits("/latest"), 1);
    let before = app.update_sections.len();
    app.command(Command::UpdateNowFromNotification {});
    app.settle_until_progress("the started update", |progress| {
        matches!(progress, UpdateProgress::Failed { .. })
    });
    assert_eq!(server.hits("/latest"), 2);
    let sections = app.update_sections_since(before);
    assert_eq!(sections[0].status, UpdateStatus::Checking {});
    assert!(
        sections
            .iter()
            .any(|section| matches!(section.progress, UpdateProgress::Downloading { .. }))
    );
    assert!(matches!(
        app.update_progress(),
        UpdateProgress::Failed {
            reason: InstallFailure::DownloadFailed { detail }
        } if detail.starts_with(&app.downloads().display().to_string())
    ));
}

#[test]
fn update_now_from_the_notification_starts_nothing_without_an_update() {
    let server = update_server();
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.update_url = server.url("/latest");
    server.route("/latest", Route::ok(full_release(APP_VERSION)));
    app.launch_until_helper_runs();
    app.command(Command::UpdateNowFromNotification {});
    app.settle();
    assert_eq!(server.hits("/latest"), 2);
    assert_eq!(app.update_section().status, UpdateStatus::UpToDate {});

    server.route("/latest", Route::status(500));
    app.command(Command::UpdateNowFromNotification {});
    app.settle();
    assert_eq!(server.hits("/latest"), 3);
    assert_eq!(
        app.update_section().status,
        UpdateStatus::CheckFailed {
            reason: UpdateCheckFailure::HttpStatus { code: 500 }
        }
    );

    server.route("/latest", Route::ok(full_release("26.2.0")));
    app.command(Command::CheckForUpdates {});
    app.settle();
    assert_eq!(app.update_section().status, update_available("26.2.0"));
    assert!(
        !app.update_sections
            .iter()
            .any(|(_, section)| !matches!(section.progress, UpdateProgress::Idle {}))
    );
}

fn relaunch_with_prepared_files(server: &Server, route: Route) -> App {
    server.route("/latest", route);
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.update_url = server.url("/latest");
    app.prepared_answer = helper_reply(&PreparedUpdateReply::Prepared {
        version: version("26.2.0"),
    });
    app.launch_until_helper_runs();
    app
}

#[test]
fn a_relaunch_with_the_latest_update_prepared_is_ready_to_restart_after_a_check() {
    let server = update_server();
    let mut app =
        relaunch_with_prepared_files(&server, Route::held_after(0, full_release("26.2.0")));
    assert!(app.asked_helper(|call| matches!(call, HelperCall::ReadPreparedUpdate {})));
    let section = app.update_section();
    assert_eq!(section.status, UpdateStatus::Checking {});
    assert_eq!(section.progress, UpdateProgress::Idle {});
    assert_eq!(
        (section.update_now, section.restart),
        (ButtonState::Hidden, ButtonState::Hidden)
    );
    server.release();
    app.settle_until_progress("the restored update", |progress| {
        *progress == ready_to_restart()
    });
    let section = app.update_section();
    assert_eq!(section.status, update_available("26.2.0"));
    assert_eq!(section.restart, ButtonState::Enabled);
    assert!(section.card_dot && section.menu_badge);
    assert_eq!(server.hits("/latest"), 1);
    assert_eq!(app.discards(), 0);
    assert!(app.logged_update("update 26.2.0 was prepared before this launch"));

    app.ask(|runtime| runtime.feed(fvm_core::connection::Input::RecheckReadiness));
    app.settle();
    assert_eq!(app.prepared_questions(), 1);
}

#[test]
fn a_relaunch_with_an_outdated_update_prepared_discards_it() {
    for (latest, status) in [
        ("26.3.0", update_available("26.3.0")),
        (APP_VERSION, UpdateStatus::UpToDate {}),
    ] {
        let server = update_server();
        let mut app = relaunch_with_prepared_files(&server, Route::ok(full_release(latest)));
        app.settle_until("the discard", |app| app.discards() == 1);
        app.settle();
        let section = app.update_section();
        assert_eq!(section.progress, UpdateProgress::Idle {}, "{latest}");
        assert_eq!(section.status, status, "{latest}");
        assert_eq!(section.restart, ButtonState::Hidden, "{latest}");
    }
}

#[test]
fn a_relaunch_offline_keeps_the_prepared_update_until_a_check_finds_another() {
    for (latest, discards) in [("26.2.0", 0), ("26.3.0", 1)] {
        let server = update_server();
        let mut app = relaunch_with_prepared_files(&server, Route::closed());
        app.settle_until_progress("the restored update", |progress| {
            *progress == ready_to_restart()
        });
        assert!(matches!(
            app.update_section().status,
            UpdateStatus::CheckFailed {
                reason: UpdateCheckFailure::Network { .. }
            }
        ));
        assert_eq!(app.update_section().restart, ButtonState::Enabled);
        assert_eq!(app.discards(), 0);

        let hits = server.hits("/latest");
        server.route("/latest", Route::ok(full_release(latest)));
        app.send(ToRuntime::Event {
            event: SystemEvent::NetworkPathChanged {
                satisfied: true,
                interfaces: Vec::new(),
            },
        });
        app.settle_until("the recheck", |app| {
            app.update_section().status == update_available(latest)
        });
        app.settle();
        assert_eq!(server.hits("/latest"), hits + 1, "{latest}");
        assert_eq!(app.discards(), discards, "{latest}");
        let expected = if discards == 0 {
            ready_to_restart()
        } else {
            UpdateProgress::Idle {}
        };
        assert_eq!(*app.update_progress(), expected, "{latest}");
    }
}

#[test]
fn a_failed_question_for_prepared_files_is_logged_and_changes_nothing() {
    for answer in [
        helper_reply(&PreparedUpdateReply::Failure {
            error: HelperError {
                code: HelperErrorCode::InternalError,
                message: "could not read the update folder".into(),
            },
        }),
        PortResponse::HelperUnreachable {
            failure: HelperChannelFailure::ConnectionLost {},
        },
    ] {
        let mut app = App::new();
        app.write_config(ACCOUNT);
        app.prepared_answer = answer;
        app.launch_until_helper_runs();
        assert!(app.logged_update("asking for a prepared update failed"));
        assert_eq!(*app.update_progress(), UpdateProgress::Idle {});
        assert_eq!(app.discards(), 0);
        app.ask(|runtime| runtime.feed(fvm_core::connection::Input::RecheckReadiness));
        app.settle();
        assert_eq!(app.prepared_questions(), 1);
    }
}

#[test]
fn checking_by_hand_asks_again_for_prepared_files_until_answered() {
    let server = update_server();
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.update_url = server.url("/latest");
    app.prepared_answer = PortResponse::HelperUnreachable {
        failure: HelperChannelFailure::NoReply {},
    };
    app.launch_until_helper_runs();
    assert_eq!(app.prepared_questions(), 1);
    assert_eq!(app.update_section().status, update_available("26.2.0"));
    app.ask(|runtime| runtime.updates.last_check = Some(minutes_ago(5, 0)));
    app.command(Command::UpdatePageOpened {});
    app.settle();
    assert_eq!(server.hits("/latest"), 2);
    assert_eq!(app.prepared_questions(), 1);

    app.prepared_answer = helper_reply(&PreparedUpdateReply::Prepared {
        version: version("26.2.0"),
    });
    app.command(Command::CheckForUpdates {});
    app.settle_until_progress("the restored update", |progress| {
        *progress == ready_to_restart()
    });
    app.settle();
    assert_eq!(app.prepared_questions(), 2);
    assert_eq!(server.hits("/latest"), 3);

    app.command(Command::CheckForUpdates {});
    app.settle();
    assert_eq!(app.prepared_questions(), 2);
    assert_eq!(server.hits("/latest"), 4);
    assert_eq!(*app.update_progress(), ready_to_restart());
}

#[test]
fn update_calls_never_hold_up_the_connections_helper_calls() {
    let server = update_server();
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.secrets
        .push((vpn_password_item(), SecretText("vpn secret".into())));
    app.secrets
        .push((app_password_item(), SecretText("abcdabcdabcdabcd".into())));
    app.holds_gmail_reads = true;
    app.holds_update_calls = true;
    app.update_url = server.url("/latest");
    app.launch_until_helper_runs();
    app.download_from(&server);
    app.command(Command::UpdateNow {});
    app.settle_until("the held preparation", |app| {
        app.held_update_calls.len() == 2
    });
    app.settle();
    assert_eq!(
        *app.update_progress(),
        UpdateProgress::Preparing { percent: 60 }
    );

    app.start_run(Command::Connect {});
    app.command(Command::Disconnect {});
    app.settle_until("the disconnect", |app| {
        app.asked_helper(|call| matches!(call, HelperCall::Disconnect { .. }))
    });
    assert_eq!(app.held_update_calls.len(), 2);
    assert_eq!(
        *app.update_progress(),
        UpdateProgress::Preparing { percent: 60 }
    );

    app.answer_held_update_calls();
    app.settle_until_progress("the prepared update", |progress| {
        *progress == ready_to_restart()
    });
}

fn removes_routes(request: &PortRequest) -> bool {
    matches!(
        request,
        PortRequest::CallHelper { call: HelperCall::WriteBypassRoutes { json } }
            if serde_json::from_str::<BypassRoutesRequest>(json).unwrap() == BypassRoutesRequest::Remove {}
    )
}

fn starts_waiter(request: &PortRequest) -> bool {
    matches!(request, PortRequest::StartUpdateWaiter { .. })
}

fn installs(request: &PortRequest) -> bool {
    matches!(
        request,
        PortRequest::CallHelper {
            call: HelperCall::InstallUpdate { .. }
        }
    )
}

fn app_on_the_office_wifi_with_a_served_update(server: &Server) -> App {
    let mut app = App::new();
    app.write_config(&format!("{ACCOUNT}{WITH_RULE}"));
    app.service = Some(office_wifi());
    app.secrets
        .push((vpn_password_item(), SecretText("vpn secret".into())));
    app.secrets
        .push((app_password_item(), SecretText("abcdabcdabcdabcd".into())));
    app.holds_gmail_reads = true;
    app.update_url = server.url("/latest");
    app.launch_until_helper_runs();
    app.download_from(server);
    app.update_now_until_ready();
    app
}

fn assert_quit_after_cleanup(app: &App) {
    let waiter = app.position(starts_waiter);
    assert!(app.quit_approved());
    assert!(app.position(removes_routes) < waiter);
    assert!(waiter < app.position(installs));
    assert_eq!(app.routes_at_quit, Some(Vec::new()));
    assert_eq!(app.actions.last(), Some(&PortAction::ApproveQuit {}));
}

fn releases_waiter(action: &PortAction) -> bool {
    matches!(action, PortAction::ReleaseUpdateWaiter { .. })
}

#[test]
fn an_installer_the_helper_names_is_passed_to_the_waiter_before_the_quit() {
    let server = update_server();
    let mut app = app_on_the_office_wifi_with_a_served_update(&server);
    app.install_answer = helper_reply(&UpdateInstallReply::Started {
        installer_process_id: Some(4321),
    });
    app.command(Command::RestartToUpdate {});
    app.settle_until("the approved quit", App::quit_approved);
    assert_quit_after_cleanup(&app);
    let released: Vec<usize> = app
        .actions
        .iter()
        .enumerate()
        .filter(|(_, action)| releases_waiter(action))
        .map(|(position, _)| position)
        .collect();
    assert_eq!(released.len(), 1);
    assert_eq!(
        app.actions[released[0]],
        PortAction::ReleaseUpdateWaiter {
            installer_process_id: 4321
        }
    );
    let quit = app
        .actions
        .iter()
        .position(|action| *action == PortAction::ApproveQuit {})
        .unwrap();
    assert!(released[0] < quit);
}

#[test]
fn an_installer_the_helper_does_not_name_releases_no_waiter() {
    let server = update_server();
    let mut app = app_on_the_office_wifi_with_a_served_update(&server);
    app.command(Command::RestartToUpdate {});
    app.settle_until("the approved quit", App::quit_approved);
    assert_quit_after_cleanup(&app);
    assert!(!app.actions.iter().any(releases_waiter));
}

#[test]
fn restarting_while_connected_stops_the_vpn_cleans_up_and_quits_once_the_install_started() {
    let server = update_server();
    let mut app = app_on_the_office_wifi_with_a_served_update(&server);
    let attempt = app.start_run(Command::Connect {});
    app.bring_up(&attempt, std::net::Ipv4Addr::new(10, 212, 134, 5));
    app.settle_until("the applied rules", |app| {
        app.exception_routes == vec![rule_route()]
    });
    app.output_of(&attempt, "‹before the update›");
    app.settle();
    let runs = app.run_requests().len();

    app.command(Command::RestartToUpdate {});
    app.settle();
    assert_eq!(app.update_stops(), [(attempt.clone(), StopReason::AppQuit)]);
    assert_eq!(
        *app.update_progress(),
        UpdateProgress::Installing {
            version: "26.2.0".into()
        }
    );
    assert_eq!(app.update_section().restart, ButtonState::Disabled);
    assert!(
        app.connection_buttons()
            .iter()
            .all(|button| *button != ButtonState::Enabled),
        "{:?}",
        app.connection_buttons()
    );
    assert!(!app.requests.iter().any(starts_waiter));
    for command in [
        Command::Connect {},
        Command::Disconnect {},
        Command::ReconnectNow {},
    ] {
        app.command(command);
    }
    app.settle();
    assert_eq!(app.update_stops().len(), 1);
    assert_eq!(app.run_requests().len(), runs);
    assert!(!app.quit_approved());

    app.exited(&attempt, Milestone::TunnelUp);
    app.settle_until("the approved quit", App::quit_approved);
    assert_quit_after_cleanup(&app);
    assert!(
        app.log_at_waiter
            .as_deref()
            .is_some_and(|log| log.contains("‹before the update›"))
    );
    assert_eq!(app.state()["updateTargetVersion"], json!("26.2.0"));
    assert_eq!(app.state()["connectedBeforeUpdate"], json!(true));
}

#[test]
fn restarting_during_an_attempt_stops_it_and_records_the_reconnect() {
    let server = update_server();
    let mut app = app_on_the_office_wifi_with_a_served_update(&server);
    let attempt = app.start_run(Command::Connect {});
    app.command(Command::RestartToUpdate {});
    app.settle();
    assert_eq!(app.update_stops(), [(attempt.clone(), StopReason::AppQuit)]);
    assert!(!app.requests.iter().any(starts_waiter));
    app.exited(&attempt, Milestone::Started);
    app.settle_until("the approved quit", App::quit_approved);
    assert_quit_after_cleanup(&app);
    assert_eq!(app.state()["connectedBeforeUpdate"], json!(true));
}

#[test]
fn restarting_while_disconnected_installs_at_once_without_a_reconnect() {
    let server = update_server();
    let mut app = app_on_the_office_wifi_with_a_served_update(&server);
    app.command(Command::RestartToUpdate {});
    app.settle_until("the approved quit", App::quit_approved);
    assert_eq!(app.update_stops(), []);
    assert_quit_after_cleanup(&app);
    let waiter = app
        .requests
        .iter()
        .find_map(|request| match request {
            PortRequest::StartUpdateWaiter { install_id } => Some(*install_id),
            _ => None,
        })
        .unwrap();
    assert!(app.asked_helper(|call| matches!(
        call,
        HelperCall::InstallUpdate { json }
            if serde_json::from_str::<UpdateInstallRequest>(json).unwrap() == UpdateInstallRequest { install_id: waiter }
    )));
    assert_eq!(
        app.state(),
        json!({"loginItemRegistered": false, "lastNotifiedUpdateVersion": "26.2.0", "updateTargetVersion": "26.2.0"})
    );
}

#[test]
fn a_quit_during_the_restart_is_a_normal_quit_and_records_nothing() {
    let server = update_server();
    let mut app = app_on_the_office_wifi_with_a_served_update(&server);
    let attempt = app.start_run(Command::Connect {});
    app.command(Command::RestartToUpdate {});
    app.settle();
    app.send(ToRuntime::Event {
        event: SystemEvent::QuitRequested {},
    });
    app.settle();
    assert!(!app.quit_approved());
    app.exited(&attempt, Milestone::Started);
    app.settle_until("the approved quit", App::quit_approved);
    assert!(!app.requests.iter().any(starts_waiter));
    assert_eq!(app.state().get("updateTargetVersion"), None);
    assert_eq!(app.state().get("connectedBeforeUpdate"), None);
}

fn failed_restarts() -> Vec<(Option<String>, PortResponse, InstallFailure)> {
    let unreachable = HelperChannelFailure::NoReply {};
    vec![
        (
            Some("The application could not be launched.".into()),
            helper_reply(&UpdateInstallReply::Started {
                installer_process_id: None,
            }),
            InstallFailure::WaiterFailed {
                description: "The application could not be launched.".into(),
            },
        ),
        (
            None,
            helper_reply(&UpdateInstallReply::Refused {
                reason: UpdateRefusal::NothingPrepared,
                message: "no update is prepared".into(),
            }),
            InstallFailure::HelperRefused {
                reason: UpdateRefusal::NothingPrepared,
                message: "no update is prepared".into(),
            },
        ),
        (
            None,
            helper_reply(&UpdateInstallReply::Failure {
                error: HelperError {
                    code: HelperErrorCode::InternalError,
                    message: "could not start the installer".into(),
                },
            }),
            InstallFailure::HelperFailed {
                message: "could not start the installer".into(),
            },
        ),
        (
            None,
            PortResponse::HelperUnreachable {
                failure: unreachable.clone(),
            },
            InstallFailure::HelperUnreachable {
                failure: unreachable,
            },
        ),
        (
            None,
            PortResponse::HelperBuildMismatch {
                helper_build: "99".into(),
            },
            InstallFailure::HelperBuildMismatch {
                helper_build: "99".into(),
                app_build: APP_BUILD.into(),
            },
        ),
    ]
}

#[test]
fn a_failed_restart_shows_why_and_reconnects_the_vpn_it_stopped() {
    let server = update_server();
    let mut app = App::new();
    app.write_config(&format!("{ACCOUNT}{WITH_RULE}"));
    app.service = Some(office_wifi());
    app.update_url = server.url("/latest");
    app.launch_until_helper_runs();
    app.download_from(&server);
    want_the_vpn(&mut app);
    app.settle_until("the applied rules", |app| {
        app.exception_routes == vec![rule_route()]
    });
    for (waiter_failure, install_answer, expected) in failed_restarts() {
        app.waiter_failure = waiter_failure.clone();
        app.install_answer = install_answer;
        app.update_now_until_ready();
        let (logs, stopped) = (app.update_logs(), app.stopped_update_waiters());
        app.command(Command::RestartToUpdate {});
        let failed = UpdateProgress::Failed {
            reason: expected.clone(),
        };
        app.settle_until_progress("the failed restart", |progress| *progress == failed);
        app.settle_until("the routes again", |app| {
            app.exception_routes == vec![rule_route()]
        });
        app.settle();
        assert!(!app.quit_approved(), "{expected:?}");
        assert_eq!(
            app.connection_status(),
            Some(&ConnectionStatus::WaitingForNetwork {}),
            "{expected:?}"
        );
        assert_eq!(
            app.connection_buttons()[1],
            ButtonState::Enabled,
            "{expected:?}"
        );
        let section = app.update_section();
        assert_eq!(section.update_now, ButtonState::Enabled, "{expected:?}");
        assert_eq!(section.restart, ButtonState::Hidden, "{expected:?}");
        assert_eq!(app.update_logs(), logs + 1, "{expected:?}");
        let installed = waiter_failure.is_none();
        assert_eq!(
            app.stopped_update_waiters(),
            stopped + usize::from(installed),
            "{expected:?}"
        );
        assert_eq!(app.state().get("updateTargetVersion"), None, "{expected:?}");
        assert_eq!(
            app.state().get("connectedBeforeUpdate"),
            None,
            "{expected:?}"
        );
    }
}

#[test]
fn a_failed_restart_without_a_vpn_leaves_it_off() {
    let server = update_server();
    let mut app = App::new();
    app.write_config(&format!("{ACCOUNT}{WITH_RULE}"));
    app.service = Some(office_wifi());
    app.update_url = server.url("/latest");
    app.launch_until_helper_runs();
    app.download_from(&server);
    let status = app.connection_status().cloned();
    for (waiter_failure, install_answer, expected) in failed_restarts() {
        app.waiter_failure = waiter_failure;
        app.install_answer = install_answer;
        app.update_now_until_ready();
        app.command(Command::RestartToUpdate {});
        let failed = UpdateProgress::Failed {
            reason: expected.clone(),
        };
        app.settle_until_progress("the failed restart", |progress| *progress == failed);
        app.settle();
        assert!(!app.quit_approved(), "{expected:?}");
        assert_eq!(app.connection_status(), status.as_ref(), "{expected:?}");
        assert_eq!(
            app.connection_buttons()[0],
            ButtonState::Enabled,
            "{expected:?}"
        );
        assert!(app.exception_routes.is_empty(), "{expected:?}");
        assert_eq!(app.update_section().update_now, ButtonState::Enabled);
        assert_eq!(app.state().get("updateTargetVersion"), None, "{expected:?}");
    }
    assert_eq!(app.run_requests(), []);
}

#[test]
fn a_reconnect_after_a_failed_restart_waits_for_the_stopped_attempts_late_code() {
    let server = update_server();
    let mut app = app_on_the_office_wifi_with_a_served_update(&server);
    app.install_answer = helper_reply(&UpdateInstallReply::Refused {
        reason: UpdateRefusal::NothingPrepared,
        message: "no update is prepared".into(),
    });
    let attempt = app.start_run(Command::Connect {});
    for milestone in [
        Milestone::Started,
        Milestone::ConnectedToGateway,
        Milestone::OtpPrompted,
    ] {
        app.helper_event(
            &attempt,
            HelperEventKind::Milestone {
                milestone,
                tunnel_ip: None,
            },
        );
    }
    app.settle();
    let (reads, runs) = (app.held_gmail_reads, app.run_requests().len());

    app.command(Command::RestartToUpdate {});
    app.settle();
    app.exited(&attempt, Milestone::OtpPrompted);
    app.settle_until_progress("the failed restart", |progress| {
        matches!(progress, UpdateProgress::Failed { .. })
    });
    app.settle();
    let waits = |app: &App| {
        app.log.iter().any(|line| {
            line.text
                .contains("waiting for the stopped attempt's OTP email")
        })
    };
    assert!(waits(&app));
    assert!(matches!(
        app.connection_status(),
        Some(ConnectionStatus::Connecting { .. })
    ));
    assert_eq!(app.held_gmail_reads, reads);
    assert_eq!(app.run_requests().len(), runs);

    let stopped = attempt.clone();
    app.ask(move |runtime| {
        runtime.feed(fvm_core::connection::Input::StaleOtpTrashed(stopped));
    });
    app.settle_until("the new attempt's OTP session", |app| {
        app.held_gmail_reads > reads
    });
    assert!(!app.quit_approved());
}
