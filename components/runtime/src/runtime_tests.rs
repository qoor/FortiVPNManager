//! The runtime as the app drives it: a fake app side answers the port requests from an in-memory
//! secret store, a fake Helper and a fake network, with the runtime's files in a scratch folder.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::app_helper::{AdditionalRoute, BypassRoutesSnapshot, HelperStatus};
use fvm_contract::app_internal::command::{AccountForm, Command};
use fvm_contract::app_internal::port::{
    HelperCall, PortRequest, PortResponse, RegistrationStatus, SecretItem,
};
use fvm_contract::app_internal::section::{
    AccountFeedback, BypassStatus, ClientSettingsImport, ConfigSection, ConnectionStatus,
    HelperState, ImportedPassword, Section,
};
use fvm_contract::app_internal::{AppFacts, FromRuntime, SecretText, SignInPages, ToRuntime};
use serde_json::json;

use crate::orchestration::{self, Input, Paths};

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
    /// The primary network service; `None` when there is no network.
    service: Option<BypassRoutesSnapshot>,
    requests: Vec<PortRequest>,
    sections: Vec<Section>,
    open_settings: u32,
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
            requests: Vec::new(),
            sections: Vec::new(),
            open_settings: 0,
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
            log: self.folder.join("logs/openfortivpn.log"),
            home: self.folder.join("home"),
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
                let response = self.answer(request);
                self.send(ToRuntime::PortResponse { id, response });
            }
            FromRuntime::OpenSettings {} => self.open_settings += 1,
            FromRuntime::LogLines { .. } | FromRuntime::PortAction { .. } => {}
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
        }
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
                    client_path: Some("/opt/homebrew/bin/openfortivpn".into()),
                    client_version: Some("1.23.1".into()),
                    client_problem: None,
                    untested_version: None,
                };
                reply(serde_json::to_value(status).unwrap())
            }
            HelperCall::ExitWhenIdle {} => {
                // The system starts the app's build next time.
                self.helper_build = APP_BUILD.into();
                PortResponse::HelperReplyFlag { value: true }
            }
            HelperCall::ReadBypassRoutes {} => match &self.service {
                Some(service) => reply(json!({"success": {"_0": service}})),
                None => reply(
                    json!({"failure": {"_0": {"code": "wrongPhase", "message": "no primary network service"}}}),
                ),
            },
            HelperCall::WriteBypassRoutes { json } => {
                let request: serde_json::Value = serde_json::from_str(&json).unwrap();
                let service = self
                    .service
                    .as_mut()
                    .expect("routes are written only on a network");
                service.routes = serde_json::from_value(request["routes"].clone()).unwrap();
                reply(json!({"success": {"_0": service}}))
            }
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

#[test]
fn exception_rules_write_the_routes_and_remember_them() {
    let mut app = App::new();
    app.write_config(&format!(
        "{ACCOUNT}routes:\n  networks: [192.168.0.0/24]\n  bypass_vpn: [10.30.0.0/16]\n"
    ));
    app.service = Some(BypassRoutesSnapshot {
        service_id: "SERVICE-1".into(),
        service_name: "Wi-Fi".into(),
        router: "192.168.0.1".into(),
        address: "192.168.0.10".into(),
        subnet_mask: "255.255.255.0".into(),
        routes: Vec::new(),
    });
    app.launch();
    app.settle_until("the applied rules", |app| {
        matches!(app.bypass_status(), Some(BypassStatus::Applied { .. }))
    });
    let route = AdditionalRoute {
        destination: "10.30.0.0".into(),
        subnet_mask: "255.255.0.0".into(),
        gateway: "192.168.0.1".into(),
    };
    assert_eq!(app.service.as_ref().unwrap().routes, vec![route]);
    let state = fs::read_to_string(app.folder.join("state/state.json")).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state).unwrap();
    assert_eq!(
        state["appliedExceptionRoutes"]["SERVICE-1"][0]["destination"],
        "10.30.0.0"
    );
}

#[test]
fn a_rule_removed_from_the_file_takes_its_route_away() {
    let mut app = App::new();
    let with_rule =
        format!("{ACCOUNT}routes:\n  networks: [192.168.0.0/24]\n  bypass_vpn: [10.30.0.0/16]\n");
    app.write_config(&with_rule);
    let other = AdditionalRoute {
        destination: "172.16.0.0".into(),
        subnet_mask: "255.255.0.0".into(),
        gateway: "192.168.0.254".into(),
    };
    app.service = Some(BypassRoutesSnapshot {
        service_id: "SERVICE-1".into(),
        service_name: "Wi-Fi".into(),
        router: "192.168.0.1".into(),
        address: "192.168.0.10".into(),
        subnet_mask: "255.255.255.0".into(),
        routes: vec![other.clone()],
    });
    app.launch();
    app.settle_until("the applied rules", |app| {
        app.service.as_ref().unwrap().routes.len() == 2
    });
    app.write_config(&with_rule.replace("bypass_vpn: [10.30.0.0/16]", "bypass_vpn: []"));
    app.send(ToRuntime::Event {
        event: fvm_contract::app_internal::event::SystemEvent::AppActivated {},
    });
    app.settle_until("the route removed", |app| {
        app.service.as_ref().unwrap().routes.len() == 1
    });
    // A route through another gateway was never the rules' to take.
    assert_eq!(app.service.as_ref().unwrap().routes, vec![other]);
}
