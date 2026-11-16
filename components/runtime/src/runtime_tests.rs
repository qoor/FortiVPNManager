//! The runtime as the app drives it: a fake app side answers the port requests from an in-memory
//! secret store, a fake Helper and a fake network, with the runtime's files in a scratch folder.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use fvm_contract::app_helper::{
    BypassRoute, BypassRoutesReply, BypassRoutesRequest, BypassRoutesSnapshot, ClientStatus,
    HelperStatus, OutputLine, OutputSource, VpnClient,
};
use fvm_contract::app_internal::command::{AccountForm, Command};
use fvm_contract::app_internal::event::SystemEvent;
use fvm_contract::app_internal::port::{
    FortiClientConnection, HelperCall, HelperChannelFailure, PortAction, PortRequest, PortResponse,
    RegistrationStatus, SecretItem,
};
use fvm_contract::app_internal::section::{
    AccountFeedback, BypassFailure, BypassStatus, ClientSettingsImport, ConfigSection,
    ConnectionStatus, HelperState, ImportError, ImportedPassword, Section,
};
use fvm_contract::app_internal::{
    AppFacts, FromRuntime, LogLine, SecretText, SignInPages, ToRuntime,
};
use fvm_contract::backend::{ClientProblem, ClientProblemKind};
use serde_json::json;

use crate::orchestration::{self, ClientSettings, Input, Paths};

const APP_BUILD: &str = "100";
/// No message for this long: the runtime has nothing more to do on its own.
const QUIET: Duration = Duration::from_millis(400);
const PATIENCE: Duration = Duration::from_secs(10);

const ACCOUNT: &str = r#"schema_version: 1
vpn:
  host: "vpn.example.com"
  port: 443
  username: "alice"
  realm: ""
  trusted_certs: []
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
    /// The app's exception routes when the quit was approved, once it was.
    routes_at_quit: Option<Vec<BypassRoute>>,
    requests: Vec<PortRequest>,
    actions: Vec<PortAction>,
    sections: Vec<Section>,
    log: Vec<LogLine>,
    open_settings: u32,
    /// FortiClient's SSL VPN connections, which make the runtime import from FortiClient as on
    /// Windows; `None` imports openfortivpn's settings file as on macOS.
    forticlient: Option<Vec<FortiClientConnection>>,
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
            routes_at_quit: None,
            requests: Vec::new(),
            actions: Vec::new(),
            sections: Vec::new(),
            log: Vec::new(),
            open_settings: 0,
            forticlient: None,
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
            client_settings: match self.forticlient {
                Some(_) => ClientSettings::FortiClient,
                None => ClientSettings::OpenFortiVpn {
                    home: self.folder.join("home"),
                },
            },
        };
        let runtime_inputs = inputs.clone();
        thread::spawn(move || orchestration::run(paths, runtime_inputs, received, out));
        self.inputs = inputs;
        self.out = from_runtime;
        self.send(ToRuntime::Launched {
            facts: AppFacts {
                app_build: APP_BUILD.into(),
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
    fn helper_line(&self, text: &str) {
        let lines = [OutputLine {
            date: SystemTime::now(),
            source: OutputSource::Helper,
            text: text.into(),
        }];
        self.send(ToRuntime::Event {
            event: SystemEvent::HelperOutput {
                json: serde_json::to_string(&lines).unwrap(),
            },
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
                self.sections.retain(|other| {
                    std::mem::discriminant(other) != std::mem::discriminant(&section)
                });
                self.sections.push(section);
            }
            FromRuntime::PortRequest { id, request } => {
                self.requests.push(request.clone());
                if let PortRequest::CallHelper { call } = &request
                    && is_route_call(call)
                    && self.holds_route_calls
                {
                    self.held_route_calls.push((id, call.clone()));
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
                    client_running: false,
                    clients: vec![
                        ClientStatus {
                            client: VpnClient::Openfortivpn,
                            path: Some("/opt/homebrew/bin/openfortivpn".into()),
                            version: Some("1.23.1".into()),
                            problem: None,
                            untested_version: None,
                        },
                        ClientStatus {
                            client: VpnClient::Openconnect,
                            path: None,
                            version: None,
                            problem: Some(ClientProblem {
                                kind: ClientProblemKind::NotFound {},
                                path: "/opt/homebrew/bin/openconnect".into(),
                            }),
                            untested_version: None,
                        },
                    ],
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
    fn vpn_client_info(&self) -> Option<&fvm_contract::app_internal::section::VpnClientInfo> {
        self.sections.iter().find_map(|section| match section {
            Section::Readiness { section } => section.client.as_ref(),
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
        .map(|line| (line.text.as_str(), line.starts_attempt))
        .collect()
}

/// Each attempt the core begins starts its block in the log with its first line, here the note
/// that ends it; a line of no attempt starts none.
#[test]
fn each_attempt_starts_its_block_with_its_first_log_line() {
    let mut app = app_without_a_usable_gmail_credential("gmail_api");
    app.helper_line("‹before›");
    for attempts in 1..=2 {
        app.command(Command::Connect {});
        app.settle_until("the attempt's note", |app| {
            app.log
                .iter()
                .filter(|line| line.text == NO_GMAIL_CREDENTIAL)
                .count()
                == attempts
        });
    }
    app.helper_line("‹after›");
    app.settle();
    assert_eq!(
        log_marks(&app),
        [
            ("‹before›", false),
            (NO_GMAIL_CREDENTIAL, true),
            (NO_GMAIL_CREDENTIAL, true),
            ("‹after›", false)
        ]
    );
}

#[test]
fn an_attempt_that_ended_without_a_log_line_leaves_the_next_line_unmarked() {
    let mut app = app_without_a_usable_gmail_credential("imap");
    app.command(Command::Connect {});
    app.settle_until("the failure", |app| {
        matches!(
            app.connection_status(),
            Some(ConnectionStatus::Failed { .. })
        )
    });
    app.helper_line("‹after›");
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

/// Both certificate lists belong to the gateway: a hand edit of the server address empties them,
/// the other OS's list included.
#[test]
fn a_new_server_address_empties_both_certificate_lists() {
    let digest = "a".repeat(64);
    let pin = format!("pin-sha256:{}=", "A".repeat(43));
    let with_values = ACCOUNT.replace(
        "  trusted_certs: []\n",
        &format!("  trusted_certs: [\"{digest}\"]\n  trusted_cert_pins: [\"{pin}\"]\n"),
    );
    let mut app = App::new();
    app.write_config(&with_values);
    app.launch();
    app.settle();
    assert!(app.config_text().contains(&pin));
    app.write_config(&with_values.replace("vpn.example.com", "vpn2.example.com"));
    app.settle_until("the emptied lists", |app| {
        let text = app.config_text();
        text.contains("  trusted_certs: []\n") && text.contains("  trusted_cert_pins: []\n")
    });
    assert!(app.config_text().contains("vpn2.example.com"));
}

#[cfg(not(windows))]
#[test]
fn choosing_the_vpn_client_writes_it_and_shows_its_status() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle_until("the Helper's status", |app| app.vpn_client_info().is_some());
    assert_eq!(app.config_section().vpn_client, VpnClient::Openconnect);
    assert!(app.vpn_client_info().unwrap().problem.is_some());
    app.command(Command::SetVpnClient {
        client: VpnClient::Openfortivpn,
    });
    app.settle_until("openfortivpn's status", |app| {
        app.vpn_client_info()
            .is_some_and(|info| info.version.as_deref() == Some("1.23.1"))
    });
    app.settle();
    assert_eq!(app.config_section().vpn_client, VpnClient::Openfortivpn);
    assert_eq!(app.config_section().vpn_client_error, None);
    assert!(app.config_text().contains("\n  client: \"openfortivpn\"\n"));
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

#[test]
fn saving_the_account_writes_the_changed_field_and_the_password_under_the_saved_account() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    let account = &app.config_section().account;
    let form = AccountForm {
        host: account.host.clone(),
        port: account.port.clone(),
        username: " bob ".into(),
        realm: account.realm.clone(),
        gmail_address: account.gmail_address.clone(),
        vpn_password: SecretText("new secret".into()),
        app_password: SecretText(String::new()),
    };
    app.command(Command::EditAccount {
        revision: account.revision,
        form,
    });
    // Edited elsewhere meanwhile: the field the user did not change keeps the edit.
    app.write_config(&ACCOUNT.replace("method: \"imap\"", "method: \"gmail_api\""));
    app.command(Command::SaveAccount {});
    app.settle_until("the saved feedback", |app| {
        app.config_section().account.feedback == Some(AccountFeedback::Saved {})
    });
    let text = app.config_text();
    assert!(text.contains("  username: \"bob\"\n"), "{text}");
    assert!(text.contains("  method: \"gmail_api\"\n"), "{text}");
    let saved_under = SecretItem::VpnPassword {
        username: "bob".into(),
        host: "vpn.example.com".into(),
    };
    assert_eq!(
        app.secret(&saved_under),
        Some(SecretText("new secret".into()))
    );
    assert!(!app.config_section().account.vpn_password_entered);
}

#[test]
fn a_typed_password_survives_a_file_change_but_not_closing_settings() {
    let mut app = App::new();
    app.write_config(ACCOUNT);
    app.launch();
    app.settle();
    let account = app.config_section().account.clone();
    let form = AccountForm {
        host: account.host.clone(),
        port: account.port.clone(),
        username: account.username.clone(),
        realm: account.realm.clone(),
        gmail_address: account.gmail_address.clone(),
        vpn_password: SecretText("typed".into()),
        app_password: SecretText(String::new()),
    };
    app.command(Command::EditAccount {
        revision: account.revision,
        form,
    });
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
    let account = app.config_section().account.clone();
    app.command(Command::RevertAccount {});
    app.settle();
    let form = AccountForm {
        host: account.host.clone(),
        port: account.port.clone(),
        username: "mallory".into(),
        realm: account.realm.clone(),
        gmail_address: account.gmail_address.clone(),
        vpn_password: SecretText(String::new()),
        app_password: SecretText(String::new()),
    };
    app.command(Command::EditAccount {
        revision: account.revision,
        form,
    });
    app.settle();
    assert_eq!(app.config_section().account.username, "alice");
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
    assert!(app.exception_routes.is_empty());
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
    assert!(app.exception_routes.is_empty());
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
    assert!(app.exception_routes.is_empty());
    want_the_vpn(&mut app);
    app.settle_until("the route written", |app| {
        app.exception_routes == vec![rule_route()]
    });
    app.command(Command::Disconnect {});
    app.settle_until("the route removed", |app| {
        app.bypass_status() == Some(&BypassStatus::VpnOff {})
    });
    assert!(app.exception_routes.is_empty());
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
    app.launch();
    app.settle();
    let lines = vec![fvm_contract::app_helper::OutputLine {
        date: std::time::SystemTime::now(),
        source: fvm_contract::app_helper::OutputSource::Stdout,
        text: marker.into(),
    }];
    app.send(ToRuntime::Event {
        event: fvm_contract::app_internal::event::SystemEvent::HelperOutput {
            json: serde_json::to_string(&lines).unwrap(),
        },
    });
    app.settle_until("the line in the log file", |app| {
        fs::read_to_string(app.folder.join("logs/vpn_backend.log"))
            .unwrap_or_default()
            .lines()
            .any(|line| line.ends_with(&format!(" [out] {marker}")))
    });
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
