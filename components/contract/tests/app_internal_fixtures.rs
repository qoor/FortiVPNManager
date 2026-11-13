//! `fixtures/app-internal/` pins the JSON of the messages between the app's Swift parts and the
//! runtime. What Rust sends is generated here: run with `FVM_WRITE_FIXTURES=1` to rewrite those
//! files after a deliberate change. What Swift sends is written by hand and decoded here. In both
//! cases `tests/macos/AppInternalTests.swift` checks the Swift types the opposite way against the
//! same files. Every case of every enum appears at least once.

use std::fs;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fvm_contract::app_helper::{AttemptId, HelperErrorCode, StopReason};
use fvm_contract::app_internal::command::*;
use fvm_contract::app_internal::event::*;
use fvm_contract::app_internal::port::*;
use fvm_contract::app_internal::problem::*;
use fvm_contract::app_internal::section::*;
use fvm_contract::app_internal::*;
use fvm_contract::backend::{ClientProblem, ClientProblemKind, UntestedVersion};
use serde::Serialize;
use serde_json::Value;

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/app-internal")
        .join(name)
}

/// Rust-sent: `value` encodes to the fixture `name`.
fn pin(name: &str, value: impl Serialize) {
    let encoded = serde_json::to_value(value).unwrap();
    if std::env::var_os("FVM_WRITE_FIXTURES").is_some() {
        let mut text = serde_json::to_string_pretty(&encoded).unwrap();
        text.push('\n');
        fs::write(path(name), text).unwrap();
    }
    let fixture: Value = serde_json::from_str(&fs::read_to_string(path(name)).unwrap()).unwrap();
    assert_eq!(encoded, fixture, "{name}");
}

/// Swift-sent: the fixture `name` decodes to `expected`.
fn decodes(name: &str, expected: &[ToRuntime]) {
    let decoded: Vec<ToRuntime> =
        serde_json::from_str(&fs::read_to_string(path(name)).unwrap()).unwrap();
    assert_eq!(decoded, expected, "{name}");
}

const ATTEMPT: &str = "E621E1F8-C36C-495A-93FC-0C247A3E6E5F";

fn attempt() -> AttemptId {
    ATTEMPT.parse().unwrap()
}

fn at(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}

const T0: u64 = 1_762_412_345_250;

fn minutes(value: u32, minimum: u32, maximum: u32) -> MinutesSetting {
    MinutesSetting {
        value,
        minimum,
        maximum,
    }
}

fn time(hour: u8, minute: u8) -> TimeOfDay {
    TimeOfDay { hour, minute }
}

fn secret(text: &str) -> SecretText {
    SecretText(text.into())
}

fn store_failure() -> SecretStoreFailure {
    SecretStoreFailure {
        description: "User interaction is not allowed.".into(),
    }
}

fn unreadable_file() -> ConfigFileError {
    ConfigFileError::UnreadableFile {
        path: "/Users/alice/.config/FortiVPNManager/config.yaml".into(),
        problem: ConfigProblem {
            kind: ConfigProblemKind::DuplicateKey { key: "vpn".into() },
            location: Some(LineColumn { line: 7, column: 1 }),
        },
    }
}

fn connection_section(status: ConnectionStatus) -> ConnectionSection {
    ConnectionSection {
        icon: ConnectionIcon::Disconnected,
        status,
        wait_reason: None,
        connect: ButtonState::Enabled,
        disconnect: ButtonState::Hidden,
        reconnect_now: ButtonState::Disabled,
        account_applies_next_connection: false,
    }
}

// MARK: Rust-sent

#[test]
fn messages_from_the_runtime() {
    pin(
        "from_runtime.json",
        [
            FromRuntime::Section {
                section: Section::Connection {
                    section: Box::new(connection_section(ConnectionStatus::NotConnected {})),
                },
            },
            FromRuntime::Section {
                section: Section::Schedule {
                    section: Box::new(schedule_section(false, None, None)),
                },
            },
            FromRuntime::Section {
                section: Section::Readiness {
                    section: Box::new(ReadinessSection {
                        helper: HelperState::Checking {},
                        register_helper: ButtonState::Hidden,
                        register_error: None,
                        client: None,
                    }),
                },
            },
            FromRuntime::Section {
                section: Section::Gmail {
                    section: Box::new(gmail_section(GmailTest::Idle {})),
                },
            },
            FromRuntime::Section {
                section: Section::BypassRules {
                    section: Box::new(bypass_section(BypassStatus::Checking {}, None)),
                },
            },
            FromRuntime::Section {
                section: Section::Config {
                    section: Box::new(config_section(account_form(0), None)),
                },
            },
            FromRuntime::LogLines {
                lines: vec![
                    LogLine {
                        time: at(T0),
                        source: LogSource::Stdout,
                        text: "INFO:   Connected to gateway.".into(),
                    },
                    LogLine {
                        time: at(T0 + 750),
                        source: LogSource::Stderr,
                        text: "Using interface ppp0".into(),
                    },
                    LogLine {
                        time: at(T0 + 1_000),
                        source: LogSource::Note,
                        text: "‹schedule: scheduled hours started, connecting›".into(),
                    },
                ],
            },
            FromRuntime::PortRequest {
                id: 7,
                request: PortRequest::ScreenState {},
            },
            FromRuntime::PortAction {
                action: PortAction::ApproveQuit {},
            },
            FromRuntime::OpenSettings {},
        ],
    );
}

#[test]
fn connection_sections() {
    pin(
        "connection_sections.json",
        [
            ConnectionSection {
                icon: ConnectionIcon::Connected,
                status: ConnectionStatus::Connected {
                    since: at(T0),
                    tunnel_ip: Some(Ipv4Addr::new(10, 211, 0, 7)),
                    proactive_reconnect: ProactiveReconnect::At {
                        at: at(T0 + 10_500_000),
                    },
                },
                wait_reason: None,
                connect: ButtonState::Hidden,
                disconnect: ButtonState::Enabled,
                reconnect_now: ButtonState::Enabled,
                account_applies_next_connection: true,
            },
            connection_section(ConnectionStatus::Connected {
                since: at(T0),
                tunnel_ip: None,
                proactive_reconnect: ProactiveReconnect::PausedBySchedule {},
            }),
            connection_section(ConnectionStatus::Connected {
                since: at(T0),
                tunnel_ip: None,
                proactive_reconnect: ProactiveReconnect::Off {},
            }),
            ConnectionSection {
                icon: ConnectionIcon::InProgress,
                status: ConnectionStatus::WaitingToReconnect {
                    until: at(T0 + 90_000),
                },
                wait_reason: Some(WaitReason::AuthCooldown { delay_seconds: 90 }),
                connect: ButtonState::Hidden,
                disconnect: ButtonState::Enabled,
                reconnect_now: ButtonState::Enabled,
                account_applies_next_connection: false,
            },
            ConnectionSection {
                icon: ConnectionIcon::Error,
                status: ConnectionStatus::SetupRequired {
                    items: vec![
                        SetupItem::VpnPassword {},
                        SetupItem::GmailCredential {
                            method: OtpMethod::GmailApi,
                        },
                    ],
                },
                wait_reason: None,
                connect: ButtonState::Enabled,
                disconnect: ButtonState::Enabled,
                reconnect_now: ButtonState::Disabled,
                account_applies_next_connection: false,
            },
            ConnectionSection {
                icon: ConnectionIcon::Error,
                status: ConnectionStatus::Failed {
                    reason: FailureReason::VpnLoginRejected {},
                },
                wait_reason: None,
                connect: ButtonState::Enabled,
                disconnect: ButtonState::Hidden,
                reconnect_now: ButtonState::Disabled,
                account_applies_next_connection: false,
            },
            connection_section(ConnectionStatus::Connecting {
                reconnecting: true,
                progress: ConnectProgress::WaitingForOtp,
            }),
            ConnectionSection {
                wait_reason: Some(WaitReason::UserAway {}),
                ..connection_section(ConnectionStatus::ReconnectOnHold {
                    until: at(T0 + 1_800_000),
                })
            },
            connection_section(ConnectionStatus::WaitingForNetwork {}),
            connection_section(ConnectionStatus::Disconnecting {
                then: AfterDisconnect::Reconnect,
            }),
        ],
    );
}

#[test]
fn connection_kinds() {
    use ConnectProgress::*;
    pin(
        "connect_progress.json",
        [
            CheckingReadiness,
            ConnectingToGmail,
            ContactingServer,
            LoggingIn,
            WaitingForOtp,
            VerifyingOtp,
            OpeningTunnel,
        ],
    );
    use AfterDisconnect::*;
    pin("after_disconnect.json", [Idle, Reconnect, Sleep, Quit]);
    pin(
        "failure_reasons.json",
        [
            FailureReason::VpnLoginRejected {},
            FailureReason::OtpNotReceived {
                in_spam: false,
                waited_seconds: 50,
            },
            FailureReason::OtpRejected {},
            FailureReason::TunnelSetupFailed {},
            FailureReason::TunnelModeRejected {},
            FailureReason::DroppedEarly {
                within_seconds: 120,
            },
            FailureReason::CertificateChangedAfterLogin {},
            FailureReason::GmailRejected {
                method: OtpMethod::Imap,
            },
            FailureReason::TooManyReconnects {
                window_seconds: 3600,
            },
            FailureReason::Network {},
            FailureReason::Helper {
                code: HelperErrorCode::Busy,
            },
            FailureReason::Helper {
                code: HelperErrorCode::InvalidRequest,
            },
            FailureReason::Helper {
                code: HelperErrorCode::UnknownAttempt,
            },
            FailureReason::Helper {
                code: HelperErrorCode::WrongPhase,
            },
            FailureReason::Helper {
                code: HelperErrorCode::ClientUnavailable,
            },
            FailureReason::Helper {
                code: HelperErrorCode::NotAuthorized,
            },
            FailureReason::Helper {
                code: HelperErrorCode::InternalError,
            },
        ],
    );
    pin(
        "wait_reasons.json",
        [
            WaitReason::Network {},
            WaitReason::ServerEndedSession {},
            WaitReason::ForcedTermination {},
            WaitReason::AuthFailure {
                reason: FailureReason::OtpNotReceived {
                    in_spam: true,
                    waited_seconds: 50,
                },
            },
            WaitReason::AuthCooldown { delay_seconds: 90 },
            WaitReason::Sleep {},
            WaitReason::UserAway {},
        ],
    );
    pin(
        "setup_items.json",
        [
            SetupItem::ConfigFile {},
            SetupItem::AccountDetails {},
            SetupItem::VpnPassword {},
            SetupItem::GmailCredential {
                method: OtpMethod::Imap,
            },
            SetupItem::HelperApproval {},
            SetupItem::HelperVersion {},
        ],
    );
}

fn schedule_section(
    enabled: bool,
    outlook: Option<ScheduleOutlook>,
    hours_error: Option<ScheduleHoursError>,
) -> ScheduleSection {
    use Weekday::*;
    ScheduleSection {
        enabled,
        days: vec![Mon, Tue, Wed, Thu, Fri, Sat, Sun],
        start: time(9, 0),
        end: time(18, 30),
        away_minutes: minutes(60, 1, 1440),
        outlook,
        hours_error,
    }
}

#[test]
fn schedule_sections() {
    pin(
        "schedule_sections.json",
        [
            schedule_section(
                true,
                Some(ScheduleOutlook::Connects {
                    at: at(T0 + 3_600_000),
                }),
                None,
            ),
            schedule_section(
                true,
                Some(ScheduleOutlook::NoDays {}),
                Some(ScheduleHoursError::EndNotAfterStart {}),
            ),
            schedule_section(
                true,
                Some(ScheduleOutlook::WithinHours {
                    end: at(T0 + 7_200_000),
                }),
                Some(ScheduleHoursError::NotSaved {
                    error: unreadable_file(),
                }),
            ),
            schedule_section(true, Some(ScheduleOutlook::OutsideHoursPresent {}), None),
            schedule_section(
                true,
                Some(ScheduleOutlook::HoldingWhileAway {
                    until: at(T0 + 1_800_000),
                }),
                None,
            ),
            schedule_section(true, Some(ScheduleOutlook::Suspended {}), None),
        ],
    );
}

fn readiness_section(helper: HelperState) -> ReadinessSection {
    ReadinessSection {
        helper,
        register_helper: ButtonState::Hidden,
        register_error: None,
        client: None,
    }
}

#[test]
fn readiness_sections() {
    pin(
        "readiness_sections.json",
        [
            ReadinessSection {
                helper: HelperState::Running {
                    build: "20261109120000".into(),
                },
                register_helper: ButtonState::Hidden,
                register_error: None,
                client: Some(VpnClientInfo {
                    path: Some("/opt/homebrew/Cellar/openfortivpn/1.25.0/bin/openfortivpn".into()),
                    version: Some("1.25.0".into()),
                    untested_version: Some(UntestedVersion {
                        tested_series: "1.24".into(),
                    }),
                    problem: None,
                }),
            },
            ReadinessSection {
                helper: HelperState::NotRegistered {},
                register_helper: ButtonState::Enabled,
                register_error: Some(RegistrationError::NotInstalledCopy {}),
                client: None,
            },
            ReadinessSection {
                helper: HelperState::Running {
                    build: "20261109120000".into(),
                },
                register_helper: ButtonState::Hidden,
                register_error: Some(RegistrationError::Failed {
                    description: "Operation not permitted".into(),
                }),
                client: Some(VpnClientInfo {
                    path: None,
                    version: None,
                    untested_version: None,
                    problem: Some(ClientProblem {
                        kind: ClientProblemKind::NotFound {},
                        path: "/opt/homebrew/bin/openfortivpn".into(),
                    }),
                }),
            },
            readiness_section(HelperState::RequiresApproval {}),
            readiness_section(HelperState::VersionMismatch {
                helper_build: "20261108090000".into(),
                app_build: "20261109120000".into(),
            }),
            readiness_section(HelperState::Unreachable {
                failure: HelperChannelFailure::NoReply {},
            }),
        ],
    );
    let cellar = "/opt/homebrew/Cellar/openfortivpn/1.24.1/bin/openfortivpn";
    let problem = |kind| ClientProblem {
        kind,
        path: cellar.into(),
    };
    pin(
        "client_problems.json",
        [
            problem(ClientProblemKind::NotFound {}),
            problem(ClientProblemKind::OutsideAllowedLocation {}),
            problem(ClientProblemKind::NotRegularFile {}),
            problem(ClientProblemKind::WritableByOthers { mode: 0o775 }),
            problem(ClientProblemKind::NotExecutable {}),
        ],
    );
    pin(
        "helper_channel_failures.json",
        [
            HelperChannelFailure::AppNotSigned {},
            HelperChannelFailure::Connection {
                description: "Couldn’t communicate with a helper application.".into(),
            },
            HelperChannelFailure::ConnectionLost {},
            HelperChannelFailure::NoReply {},
            HelperChannelFailure::UnreadableReply {
                description: "The data couldn’t be read because it isn’t in the correct format."
                    .into(),
            },
        ],
    );
}

fn gmail_section(test: GmailTest) -> GmailSection {
    GmailSection {
        method: OtpMethod::GmailApi,
        method_error: None,
        google_account: GoogleAccountStatus::NotConnected {},
        connect_google: ButtonState::Enabled,
        disconnect_google: ButtonState::Hidden,
        cancel_sign_in: ButtonState::Hidden,
        connect_blocker: None,
        google_error: None,
        oauth_client: OAuthClientSource::BuiltIn {
            client_id: "123-abc.apps.googleusercontent.com".into(),
        },
        imported_client_secret_missing: false,
        use_built_in_client: ButtonState::Hidden,
        import_client: ButtonState::Enabled,
        test,
        test_button: ButtonState::Enabled,
    }
}

#[test]
fn gmail_sections() {
    pin(
        "gmail_sections.json",
        [
            GmailSection {
                method: OtpMethod::GmailApi,
                method_error: None,
                google_account: GoogleAccountStatus::SigningIn {},
                connect_google: ButtonState::Hidden,
                disconnect_google: ButtonState::Hidden,
                cancel_sign_in: ButtonState::Enabled,
                connect_blocker: None,
                google_error: None,
                oauth_client: OAuthClientSource::Imported {
                    client_id: "987-xyz.apps.googleusercontent.com".into(),
                },
                imported_client_secret_missing: false,
                use_built_in_client: ButtonState::Disabled,
                import_client: ButtonState::Disabled,
                test: GmailTest::Idle {},
                test_button: ButtonState::Disabled,
            },
            GmailSection {
                method: OtpMethod::Imap,
                method_error: Some(unreadable_file()),
                google_account: GoogleAccountStatus::NotConnected {},
                connect_google: ButtonState::Disabled,
                disconnect_google: ButtonState::Hidden,
                cancel_sign_in: ButtonState::Hidden,
                connect_blocker: Some(GoogleConnectBlocker::SaveAddressFirst),
                google_error: Some(GoogleError::WrongAccount {
                    expected: "alice@example.com".into(),
                    actual: "bob@example.com".into(),
                }),
                oauth_client: OAuthClientSource::NotConfigured {},
                imported_client_secret_missing: true,
                use_built_in_client: ButtonState::Hidden,
                import_client: ButtonState::Enabled,
                test: GmailTest::Running {},
                test_button: ButtonState::Disabled,
            },
            GmailSection {
                google_account: GoogleAccountStatus::Disconnecting {},
                ..gmail_section(GmailTest::ApiSucceeded {
                    address: "alice@example.com".into(),
                })
            },
            GmailSection {
                google_account: GoogleAccountStatus::Connected {
                    address: "alice@example.com".into(),
                },
                ..gmail_section(GmailTest::ImapSucceeded {
                    archive: "[Gmail]/All Mail".into(),
                    trash: "[Gmail]/Trash".into(),
                    spam: Some("[Gmail]/Spam".into()),
                })
            },
        ],
    );
    use GoogleConnectBlocker::*;
    pin(
        "google_connect_blockers.json",
        [NoOAuthClient, EnterAddress, SaveAddressFirst],
    );
}

#[test]
fn google_errors() {
    pin(
        "google_errors.json",
        [
            GoogleError::ClientFileUnreadable {
                problem: FileProblem::TooLarge { limit_bytes: 65536 },
            },
            GoogleError::ClientFileInvalid {
                problem: OAuthClientFileProblem::NotJson {},
            },
            GoogleError::ClientFileInvalid {
                problem: OAuthClientFileProblem::WebClient {},
            },
            GoogleError::ClientFileInvalid {
                problem: OAuthClientFileProblem::NoDesktopClient {},
            },
            GoogleError::ClientFileInvalid {
                problem: OAuthClientFileProblem::MissingField {
                    name: "client_secret".into(),
                },
            },
            GoogleError::ClientFileInvalid {
                problem: OAuthClientFileProblem::InvalidField {
                    name: "client_id".into(),
                },
            },
            GoogleError::ListenerFailed {
                detail: "Address already in use (os error 48)".into(),
            },
            GoogleError::BrowserUnavailable {},
            GoogleError::TimedOut { after_seconds: 300 },
            GoogleError::CancelledInBrowser {},
            GoogleError::Denied {
                code: "invalid_scope".into(),
            },
            GoogleError::TokenRejected {
                detail: "Bad Request".into(),
            },
            GoogleError::ClientRejected {},
            GoogleError::NoRefreshToken {},
            GoogleError::ScopeNotGranted {},
            GoogleError::WrongAccount {
                expected: "alice@example.com".into(),
                actual: "bob@example.com".into(),
            },
            GoogleError::GmailApiRefused {
                reason: "accessNotConfigured".into(),
            },
            GoogleError::Network {
                detail: "connection refused".into(),
            },
            GoogleError::SecretStore {
                failure: store_failure(),
            },
            GoogleError::Config {
                error: ConfigFileError::KeptChanging {
                    path: "/Users/alice/.config/FortiVPNManager/config.yaml".into(),
                },
            },
        ],
    );
}

#[test]
fn gmail_tests() {
    let failed = |failure| GmailTest::Failed { failure };
    let unreachable = |failure| failed(GmailTestFailure::ImapUnreachable { failure });
    pin(
        "gmail_tests.json",
        [
            GmailTest::Idle {},
            GmailTest::Running {},
            GmailTest::ImapSucceeded {
                archive: "[Gmail]/All Mail".into(),
                trash: "[Gmail]/Trash".into(),
                spam: None,
            },
            GmailTest::ApiSucceeded {
                address: "alice@example.com".into(),
            },
            failed(GmailTestFailure::EnterAddress {}),
            failed(GmailTestFailure::EnterAppPassword {}),
            failed(GmailTestFailure::AppPasswordUnreadable {
                failure: store_failure(),
            }),
            failed(GmailTestFailure::ImapLoginRejected {
                server_text: Some("[AUTHENTICATIONFAILED] Invalid credentials (Failure)".into()),
            }),
            failed(GmailTestFailure::ImapLoginRejected { server_text: None }),
            failed(GmailTestFailure::MailboxesMissing {
                all_mail: true,
                trash: false,
            }),
            unreachable(ImapFailure::Network {
                detail: "dns error: failed to lookup address information".into(),
            }),
            unreachable(ImapFailure::TimedOut {
                step: ImapStep::Connecting {},
            }),
            unreachable(ImapFailure::TimedOut {
                step: ImapStep::Command {
                    name: "UID SEARCH".into(),
                },
            }),
            unreachable(ImapFailure::ServerClosed {
                text: Some("System error".into()),
            }),
            unreachable(ImapFailure::ServerClosed { text: None }),
            unreachable(ImapFailure::Protocol {
                detail: "unexpected server greeting".into(),
            }),
            unreachable(ImapFailure::Rejected {
                command: "SELECT".into(),
                status: "NO".into(),
                text: "[NONEXISTENT] Unknown Mailbox".into(),
            }),
            failed(GmailTestFailure::NoOAuthClient {}),
            failed(GmailTestFailure::NotSignedIn {}),
            failed(GmailTestFailure::AccessRevoked {}),
            failed(GmailTestFailure::ClientRejected {}),
            failed(GmailTestFailure::SecretStore {
                failure: store_failure(),
            }),
            failed(GmailTestFailure::OtherAccount {
                authorized: "bob@example.com".into(),
                expected: "alice@example.com".into(),
            }),
            failed(GmailTestFailure::AccessTokenRejected {}),
            failed(GmailTestFailure::GmailApiRefused {
                reason: "accessNotConfigured".into(),
            }),
            failed(GmailTestFailure::RateLimited {}),
            failed(GmailTestFailure::GoogleUnreachable {
                detail: "connection reset by peer".into(),
            }),
            failed(GmailTestFailure::GoogleReturnedError {
                detail: "HTTP 500".into(),
            }),
        ],
    );
}

fn bypass_section(status: BypassStatus, notice: Option<BypassNotice>) -> BypassRulesSection {
    BypassRulesSection {
        current_network: None,
        networks: RuleList {
            rows: vec![],
            problem_rows: vec![],
            add_row: ButtonState::Enabled,
        },
        targets: RuleList {
            rows: vec![],
            problem_rows: vec![],
            add_row: ButtonState::Enabled,
        },
        notice,
        status,
        has_changes: false,
        add_current_network: ButtonState::Disabled,
        discard: ButtonState::Disabled,
        apply: ButtonState::Disabled,
    }
}

#[test]
fn bypass_rules_sections() {
    let failed = |failure, retry_in_seconds, stopped_after_retries| BypassStatus::Failed {
        failure,
        retry_in_seconds,
        stopped_after_retries,
    };
    pin(
        "bypass_rules_sections.json",
        [
            BypassRulesSection {
                current_network: Some(CurrentNetwork {
                    service_name: "Ethernet".into(),
                    address: "10.20.161.20".into(),
                    prefix_length: Some(24),
                    within: Some("10.20.161.0/24".into()),
                }),
                networks: RuleList {
                    rows: vec!["10.20.161.0/24".into()],
                    problem_rows: vec![],
                    add_row: ButtonState::Enabled,
                },
                targets: RuleList {
                    rows: vec![
                        "10.30.161.16".into(),
                        "10.30.161".into(),
                        "10.30.161.16/32".into(),
                    ],
                    problem_rows: vec![1, 2],
                    add_row: ButtonState::Disabled,
                },
                notice: Some(BypassNotice::RowProblem {
                    row: "10.30.161".into(),
                    problem: RowProblem::Invalid {
                        problem: AddressProblem::Malformed {},
                    },
                }),
                status: BypassStatus::Applied {
                    service_name: "Ethernet".into(),
                    router: "10.20.161.1".into(),
                    on_link_targets: 1,
                    not_imported: 2,
                    kept_through_other_gateways: 3,
                },
                has_changes: true,
                add_current_network: ButtonState::Disabled,
                discard: ButtonState::Enabled,
                apply: ButtonState::Disabled,
            },
            BypassRulesSection {
                current_network: Some(CurrentNetwork {
                    service_name: "Wi-Fi".into(),
                    address: "192.168.0.12".into(),
                    prefix_length: None,
                    within: None,
                }),
                ..bypass_section(
                    BypassStatus::OutsideNetworks {
                        kept_through_other_gateways: 0,
                    },
                    Some(BypassNotice::RowProblem {
                        row: "10.30.161.16/32".into(),
                        problem: RowProblem::Duplicate {
                            entry: "10.30.161.16".into(),
                        },
                    }),
                )
            },
            bypass_section(
                BypassStatus::NoNetworksListed {
                    kept_through_other_gateways: 1,
                },
                Some(BypassNotice::RowProblem {
                    row: "10.30.170.9".into(),
                    problem: RowProblem::Overlap {
                        other: "10.30.170.0/24".into(),
                    },
                }),
            ),
            bypass_section(
                BypassStatus::NoConfigFile {},
                Some(BypassNotice::ApplyFailed {
                    error: unreadable_file(),
                }),
            ),
            bypass_section(BypassStatus::ConfigInvalid {}, None),
            bypass_section(BypassStatus::NoNetwork {}, None),
            bypass_section(failed(BypassFailure::RoutesNotKept {}, Some(10), None), None),
            bypass_section(
                failed(BypassFailure::NetworkChanged {}, None, Some(3)),
                None,
            ),
            bypass_section(
                failed(
                    BypassFailure::HelperRefused {
                        message: "the route list has repeats, too many routes, or routes this helper does not write".into(),
                    },
                    None,
                    None,
                ),
                None,
            ),
            bypass_section(
                failed(
                    BypassFailure::SettingsNotChanged {
                        message: "** Error: The parameters were not valid.".into(),
                    },
                    None,
                    None,
                ),
                None,
            ),
            bypass_section(
                failed(
                    BypassFailure::ImportNotSaved {
                        error: unreadable_file(),
                    },
                    None,
                    None,
                ),
                None,
            ),
            bypass_section(
                failed(
                    BypassFailure::HelperUnreachable {
                        failure: HelperChannelFailure::ConnectionLost {},
                    },
                    None,
                    None,
                ),
                None,
            ),
            bypass_section(
                failed(
                    BypassFailure::HelperBuildMismatch {
                        helper_build: "20261108090000".into(),
                        app_build: "20261109120000".into(),
                    },
                    None,
                    None,
                ),
                None,
            ),
        ],
    );
}

fn account_form(revision: u64) -> AccountFormState {
    AccountFormState {
        revision,
        host: "vpn.example.com".into(),
        port: "443".into(),
        username: "alice".into(),
        realm: String::new(),
        gmail_address: "alice@example.com".into(),
        vpn_password_entered: false,
        app_password_entered: false,
        vpn_password_stored: true,
        app_password_stored: false,
        saved_account_problem: None,
        feedback: None,
        save: ButtonState::Disabled,
        revert: ButtonState::Disabled,
        import_report: None,
        default_import_file: None,
    }
}

fn config_section(
    account: AccountFormState,
    options_error: Option<ConfigFileError>,
) -> ConfigSection {
    ConfigSection {
        file: ConfigFileState {
            path: "/Users/alice/.config/FortiVPNManager/config.yaml".into(),
            exists: true,
            load_error: None,
            warnings: vec![],
        },
        account,
        options: ConnectionOptions {
            auto_connect_on_launch: true,
            session_max_minutes: minutes(180, 10, 1440),
            proactive_reconnect: true,
            margin_minutes: minutes(5, 1, 60),
            reconnect_after_sleep: false,
            min_sleep_minutes: minutes(30, 1, 1440),
        },
        options_error,
        open_at_login: OpenAtLogin {
            available: true,
            enabled: true,
            requires_approval: false,
            error: None,
        },
        log_file: "/Users/alice/Library/Logs/FortiVPNManager/openfortivpn.log".into(),
    }
}

fn import_report(password: Option<ImportedPassword>) -> ImportReport {
    ImportReport {
        source: "/Users/alice/.config/openfortivpn/config".into(),
        host: "vpn.example.com".into(),
        port: 10443,
        username: "alice".into(),
        realm: "staff".into(),
        trusted_certs: 1,
        password,
        unknown_keys: vec!["insecure-cert".into()],
        ignored_keys: vec!["set-dns".into()],
        unreadable_lines: vec![4, 9],
    }
}

#[test]
fn config_sections() {
    let feedback = |error| AccountFormState {
        feedback: Some(AccountFeedback::Failed { error }),
        ..account_form(3)
    };
    let imported = |report| AccountFormState {
        import_report: Some(report),
        ..account_form(4)
    };
    let file = |path: &str| path.to_owned();
    pin(
        "config_sections.json",
        [
            ConfigSection {
                file: ConfigFileState {
                    path: file("/Users/alice/.config/FortiVPNManager/config.yaml"),
                    exists: true,
                    load_error: Some(unreadable_file()),
                    warnings: vec![
                        ConfigWarning {
                            kind: ConfigWarningKind::UnknownKey {
                                path: "vpn.colour".into(),
                            },
                            location: Some(LineColumn { line: 3, column: 3 }),
                        },
                        ConfigWarning {
                            kind: ConfigWarningKind::InsecurePermissions { mode: 0o644 },
                            location: None,
                        },
                    ],
                },
                account: AccountFormState {
                    revision: 2,
                    host: "https://vpn.example.com".into(),
                    port: "4433".into(),
                    username: "alice".into(),
                    realm: "staff".into(),
                    gmail_address: "alice@example.com".into(),
                    vpn_password_entered: true,
                    app_password_entered: true,
                    vpn_password_stored: false,
                    app_password_stored: true,
                    saved_account_problem: Some(AccountValueProblem::InvalidHost {}),
                    feedback: Some(AccountFeedback::Saved {}),
                    save: ButtonState::Enabled,
                    revert: ButtonState::Enabled,
                    import_report: None,
                    default_import_file: Some(file("/Users/alice/.config/openfortivpn/config")),
                },
                options: ConnectionOptions {
                    auto_connect_on_launch: false,
                    session_max_minutes: minutes(180, 10, 1440),
                    proactive_reconnect: false,
                    margin_minutes: minutes(5, 1, 60),
                    reconnect_after_sleep: true,
                    min_sleep_minutes: minutes(30, 1, 1440),
                },
                options_error: None,
                open_at_login: OpenAtLogin {
                    available: false,
                    enabled: false,
                    requires_approval: true,
                    error: Some(RegistrationError::NotInstalledCopy {}),
                },
                log_file: file("/Users/alice/Library/Logs/FortiVPNManager/openfortivpn.log"),
            },
            config_section(
                feedback(AccountSaveError::InvalidPort {}),
                Some(ConfigFileError::InvalidSettings {
                    problem: ConfigProblem {
                        kind: ConfigProblemKind::MarginNotLessThanSession {
                            margin: 60,
                            session: 60,
                        },
                        location: None,
                    },
                }),
            ),
            config_section(feedback(AccountSaveError::PasswordNeedsAccount {}), None),
            config_section(feedback(AccountSaveError::AppPasswordNeedsAddress {}), None),
            config_section(
                feedback(AccountSaveError::AccountValue {
                    problem: AccountValueProblem::TooLong {
                        field: AccountField::Username,
                        max_bytes: 64,
                    },
                }),
                None,
            ),
            config_section(
                feedback(AccountSaveError::AccountValue {
                    problem: AccountValueProblem::LeadingOrTrailingSpace {
                        field: AccountField::Host,
                    },
                }),
                None,
            ),
            config_section(
                feedback(AccountSaveError::AccountValue {
                    problem: AccountValueProblem::DisallowedCharacters {
                        field: AccountField::Realm,
                    },
                }),
                None,
            ),
            config_section(
                feedback(AccountSaveError::InvalidVpnPassword { max_bytes: 255 }),
                None,
            ),
            config_section(
                feedback(AccountSaveError::Config {
                    error: unreadable_file(),
                }),
                None,
            ),
            config_section(
                feedback(AccountSaveError::SecretStore {
                    failure: store_failure(),
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Imported {
                    report: import_report(Some(ImportedPassword::Saved {})),
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Imported {
                    report: import_report(Some(ImportedPassword::NotSavedWithoutUsername {})),
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Imported {
                    report: import_report(Some(ImportedPassword::NotSaved {
                        failure: store_failure(),
                    })),
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Imported {
                    report: import_report(None),
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Failed {
                    error: ImportError::SourceNotFound {
                        path: file("/Users/alice/.config/openfortivpn/config"),
                    },
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Failed {
                    error: ImportError::Unreadable {
                        path: file("/Users/alice/vpn.conf"),
                        problem: FileProblem::NotRegularFile {},
                    },
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Failed {
                    error: ImportError::MissingHost {
                        path: file("/Users/alice/vpn.conf"),
                    },
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Failed {
                    error: ImportError::Config {
                        error: unreadable_file(),
                    },
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Failed {
                    error: ImportError::NoFortiClientConnection {},
                }),
                None,
            ),
            config_section(
                imported(ClientSettingsImport::Failed {
                    error: ImportError::InvalidFortiClientServer {
                        connection: "Office".into(),
                    },
                }),
                None,
            ),
        ],
    );
}

#[test]
fn settings_file_problems() {
    let config = "/Users/alice/.config/FortiVPNManager/config.yaml";
    pin(
        "config_file_errors.json",
        [
            ConfigFileError::InvalidSettings {
                problem: ConfigProblem {
                    kind: ConfigProblemKind::ScheduleEndNotAfterStart {
                        start: time(18, 0),
                        end: time(9, 0),
                    },
                    location: None,
                },
            },
            unreadable_file(),
            ConfigFileError::NotRegularFile {
                path: config.into(),
            },
            ConfigFileError::DanglingSymlink {
                path: config.into(),
            },
            ConfigFileError::KeptChanging {
                path: config.into(),
            },
            ConfigFileError::Io {
                operation: "rename".into(),
                path: config.into(),
                errno: 13,
            },
        ],
    );
    let problem = |kind| ConfigProblem {
        kind,
        location: Some(LineColumn { line: 2, column: 9 }),
    };
    use ConfigProblemKind::*;
    let path = |text: &str| text.to_owned();
    pin(
        "config_problems.json",
        vec![
            ConfigProblem {
                kind: FileTooLarge {
                    byte_count: 70_000,
                    limit: 65_536,
                },
                location: None,
            },
            problem(InvalidUtf8 {}),
            problem(DisallowedCharacter { scalar: 0x85 }),
            problem(Syntax {
                problem: "did not find expected key".into(),
            }),
            problem(MultipleDocuments {}),
            problem(NestingTooDeep {}),
            problem(DuplicateKey { key: "port".into() }),
            problem(RootNotMapping {}),
            problem(AnchorOrAlias {}),
            problem(MergeKey {}),
            problem(ExplicitTag {
                tag: "!!str".into(),
            }),
            problem(ComplexKey {}),
            problem(TypeMismatch {
                path: path("vpn"),
                expected: ValueKind::Mapping,
            }),
            problem(TypeMismatch {
                path: path("routes.networks"),
                expected: ValueKind::Sequence,
            }),
            problem(TypeMismatch {
                path: path("vpn.host"),
                expected: ValueKind::String,
            }),
            problem(TypeMismatch {
                path: path("vpn.port"),
                expected: ValueKind::Integer,
            }),
            problem(TypeMismatch {
                path: path("connection.auto_connect_on_launch"),
                expected: ValueKind::Boolean,
            }),
            problem(InvalidInteger {
                path: path("vpn.port"),
                raw: "0443".into(),
            }),
            problem(InvalidBoolean {
                path: path("connection.auto_connect_on_launch"),
                raw: "yes".into(),
            }),
            problem(QuotedNumber {
                path: path("vpn.port"),
                raw: "443".into(),
            }),
            problem(QuotedBoolean {
                path: path("connection.proactive_reconnect.enabled"),
                raw: "true".into(),
            }),
            problem(InvalidChoice {
                path: path("otp.method"),
                raw: "pop3".into(),
                allowed: vec!["imap".into(), "gmail_api".into()],
            }),
            problem(OutOfRange {
                path: path("vpn.port"),
                raw: "70000".into(),
                minimum: 1,
                maximum: 65535,
            }),
            problem(MarginNotLessThanSession {
                margin: 30,
                session: 20,
            }),
            problem(InvalidTime {
                path: path("connection.schedule.start"),
                raw: "9am".into(),
            }),
            problem(ScheduleEndNotAfterStart {
                start: time(18, 0),
                end: time(9, 0),
            }),
            problem(InvalidCharacter {
                path: path("vpn.username"),
                scalar: 0x2028,
            }),
            problem(NewerSchemaVersion { version: 2 }),
            problem(InvalidRouteEntry {
                path: path("routes.bypass_vpn"),
                index: 1,
                raw: "10.30.170.5/24".into(),
                problem: AddressProblem::HostBitsSet {
                    network: "10.30.170.0/24".into(),
                },
            }),
            problem(TooManyRouteEntries {
                path: path("routes.networks"),
                count: 101,
                limit: 100,
            }),
            problem(DuplicateRouteEntry {
                path: path("routes.bypass_vpn"),
                index: 2,
            }),
            problem(OverlappingRouteEntry {
                path: path("routes.networks"),
                index: 1,
                other: "10.20.0.0/16".into(),
            }),
        ],
    );
    let warning = |kind| ConfigWarning {
        kind,
        location: Some(LineColumn { line: 4, column: 5 }),
    };
    pin(
        "config_warnings.json",
        [
            warning(ConfigWarningKind::UnknownKey {
                path: "vpn.colour".into(),
            }),
            warning(ConfigWarningKind::InvalidTrustedCert { index: 1 }),
            warning(ConfigWarningKind::DuplicateTrustedCert { index: 2 }),
            warning(ConfigWarningKind::TooManyTrustedCerts { limit: 4 }),
            warning(ConfigWarningKind::DuplicateRouteEntry {
                path: "routes.networks".into(),
                index: 3,
            }),
            ConfigWarning {
                kind: ConfigWarningKind::MultipleHardLinks { count: 2 },
                location: None,
            },
            ConfigWarning {
                kind: ConfigWarningKind::InsecurePermissions { mode: 0o644 },
                location: None,
            },
            ConfigWarning {
                kind: ConfigWarningKind::PermissionsCorrected {
                    previous_mode: 0o640,
                },
                location: None,
            },
        ],
    );
    pin(
        "address_problems.json",
        [
            AddressProblem::Malformed {},
            AddressProblem::PrefixOutOfRange {
                length: 4,
                minimum: 8,
                maximum: 32,
            },
            AddressProblem::NonContiguousMask {},
            AddressProblem::Unroutable {},
            AddressProblem::HostBitsSet {
                network: "10.30.170.0/24".into(),
            },
        ],
    );
    pin(
        "file_problems.json",
        [
            FileProblem::NotFound {},
            FileProblem::NotRegularFile {},
            FileProblem::TooLarge { limit_bytes: 65536 },
            FileProblem::Io {
                operation: "open".into(),
                path: "/Users/alice/Downloads/client_secret.json".into(),
                errno: 1,
            },
        ],
    );
}

#[test]
fn port_requests() {
    let request = |id, request| FromRuntime::PortRequest { id, request };
    let call = |id, call| request(id, PortRequest::CallHelper { call });
    pin(
        "port_requests.json",
        [
            request(
                1,
                PortRequest::ReadSecret {
                    item: SecretItem::VpnPassword {
                        username: "alice".into(),
                        host: "vpn.example.com".into(),
                    },
                },
            ),
            request(
                2,
                PortRequest::HasSecret {
                    item: SecretItem::GmailAppPassword {
                        address: "alice@example.com".into(),
                    },
                },
            ),
            request(
                3,
                PortRequest::WriteSecret {
                    item: SecretItem::GoogleRefreshToken {
                        address: "Alice@Example.com".into(),
                    },
                    value: secret("1//0g-refresh"),
                },
            ),
            request(
                4,
                PortRequest::DeleteSecret {
                    item: SecretItem::GoogleClientSecret {
                        client_id: "987-xyz.apps.googleusercontent.com".into(),
                    },
                },
            ),
            request(5, PortRequest::HelperRegistration {}),
            request(6, PortRequest::RegisterHelper {}),
            request(7, PortRequest::HelperBuild {}),
            call(
                8,
                HelperCall::Connect {
                    json: secret(&format!(
                        r#"{{"attemptID":"{ATTEMPT}","host":"vpn.example.com","password":"p"}}"#
                    )),
                },
            ),
            call(
                9,
                HelperCall::SubmitOtp {
                    attempt: attempt(),
                    code: secret("123456"),
                },
            ),
            call(
                10,
                HelperCall::Disconnect {
                    attempt: attempt(),
                    reason: StopReason::UserDisconnect,
                },
            ),
            call(
                11,
                HelperCall::Disconnect {
                    attempt: attempt(),
                    reason: StopReason::AppDisconnected,
                },
            ),
            call(12, HelperCall::Status {}),
            call(13, HelperCall::ExitWhenIdle {}),
            call(14, HelperCall::ReadBypassRoutes {}),
            call(
                15,
                HelperCall::WriteBypassRoutes {
                    json: r#"{"expected":{},"routes":[]}"#.into(),
                },
            ),
            request(
                17,
                PortRequest::RouteInterface {
                    destination: Ipv4Addr::new(203, 0, 113, 10),
                },
            ),
            request(18, PortRequest::InterfaceAddresses {}),
            request(19, PortRequest::PrimaryRouter {}),
            request(20, PortRequest::ScreenState {}),
            request(
                21,
                PortRequest::OpenBrowser {
                    url: "https://accounts.google.com/o/oauth2/v2/auth?client_id=x&state=y".into(),
                },
            ),
            request(22, PortRequest::LoginItemRegistration {}),
            request(23, PortRequest::SetLoginItem { enabled: false }),
            request(24, PortRequest::FortiClientConnections {}),
        ],
    );
}

#[test]
fn port_actions() {
    let action = |action| FromRuntime::PortAction { action };
    let log = |level, category, message: &str| {
        action(PortAction::Log {
            level,
            category,
            message: message.into(),
        })
    };
    use LogCategory::*;
    pin(
        "port_actions.json",
        [
            log(LogLevel::Info, Connection, "state: connected"),
            log(LogLevel::Notice, Readiness, "readiness: ready"),
            log(LogLevel::Error, Connection, "presence: away"),
            log(LogLevel::Info, Config, "config changed"),
            log(LogLevel::Info, State, "state saved"),
            log(LogLevel::Info, Logfile, "log file rotated"),
            log(LogLevel::Info, Import, "imported"),
            log(LogLevel::Info, Oauth, "token refreshed"),
            log(LogLevel::Info, Otp, "code found"),
            log(LogLevel::Info, BypassRoutes, "routes written"),
            log(LogLevel::Info, LoginItem, "registered"),
            action(PortAction::PostNotification {
                notification: UserNotification::ExceptionRulesNotApplied {
                    retries: 3,
                    failure: BypassFailure::RoutesNotKept {},
                },
            }),
            action(PortAction::HoldActivity { hold: true }),
            action(PortAction::OpenHelperApprovalSettings {}),
            action(PortAction::ApproveQuit {}),
        ],
    );
}

// MARK: Swift-sent

#[test]
fn launched() {
    decodes(
        "launched.json",
        &[ToRuntime::Launched {
            facts: AppFacts {
                app_build: "20261109120000".into(),
                installed_copy: true,
                built_in_oauth_client: Some(secret(
                    r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com"}}"#,
                )),
                time_zone: "Asia/Seoul".into(),
                sign_in_pages: SignInPages {
                    completed: "FortiVPNManager: you can close this window.".into(),
                    not_completed:
                        "FortiVPNManager: Google sign-in was not completed. You can close this window."
                            .into(),
                    invalid: "FortiVPNManager: this sign-in response is not valid. Start the sign-in again in Settings.".into(),
                },
            },
        }],
    );
}

#[test]
fn commands() {
    let command = |command| ToRuntime::Command { command };
    let option = |option| command(Command::SetConnectionOption { option });
    use ConnectionOption::*;
    decodes(
        "commands.json",
        &[
            command(Command::Connect {}),
            command(Command::Disconnect {}),
            command(Command::ReconnectNow {}),
            command(Command::SettingsOpened {}),
            command(Command::SettingsClosed {}),
            command(Command::EditAccount {
                revision: 4,
                form: AccountForm {
                    host: "vpn.example.com".into(),
                    port: "443".into(),
                    username: "alice".into(),
                    realm: String::new(),
                    gmail_address: "alice@example.com".into(),
                    vpn_password: secret("p@ss"),
                    app_password: secret(""),
                },
            }),
            command(Command::SaveAccount {}),
            command(Command::RevertAccount {}),
            command(Command::ImportClientSettings { file: None }),
            command(Command::ImportClientSettings {
                file: Some("/Users/alice/vpn.conf".into()),
            }),
            command(Command::DismissImportReport {}),
            command(Command::SetOtpMethod {
                method: OtpMethod::GmailApi,
            }),
            command(Command::SetOtpMethod {
                method: OtpMethod::Imap,
            }),
            command(Command::TestGmail {}),
            command(Command::ConnectGoogleAccount {}),
            command(Command::CancelGoogleSignIn {}),
            command(Command::DisconnectGoogleAccount {}),
            command(Command::ImportOAuthClient {
                file: "/Users/alice/Downloads/client_secret_123.json".into(),
            }),
            command(Command::UseBuiltInOAuthClient {}),
            option(ScheduleEnabled { enabled: true }),
            option(ScheduleDay {
                day: Weekday::Mon,
                selected: false,
            }),
            option(ScheduleDay {
                day: Weekday::Sun,
                selected: true,
            }),
            option(AwayMinutes { minutes: 45 }),
            option(AutoConnectOnLaunch { enabled: false }),
            option(SessionMaxMinutes { minutes: -5 }),
            option(ProactiveReconnect { enabled: true }),
            option(ProactiveMarginMinutes { minutes: 10 }),
            option(ReconnectAfterSleep { enabled: true }),
            option(MinSleepMinutes { minutes: 30 }),
            command(Command::SetScheduleHours {
                start: time(9, 0),
                end: time(18, 30),
            }),
            command(Command::SetOpenAtLogin { enabled: true }),
            command(Command::RegisterHelper {}),
            command(Command::OpenHelperApprovalSettings {}),
            command(Command::SetBypassRows {
                list: BypassList::Networks,
                rows: vec!["10.20.161.0/24".into()],
            }),
            command(Command::SetBypassRows {
                list: BypassList::Targets,
                rows: vec!["10.30.161.16".into(), String::new()],
            }),
            command(Command::AddCurrentNetwork {}),
            command(Command::DiscardBypassChanges {}),
            command(Command::ApplyBypassRules {}),
        ],
    );
}

#[test]
fn port_responses() {
    let response = |id, response| ToRuntime::PortResponse { id, response };
    let unreachable = |id, failure| response(id, PortResponse::HelperUnreachable { failure });
    let registration = |id, status| response(id, PortResponse::Registration { status });
    use RegistrationStatus::*;
    decodes(
        "port_responses.json",
        &[
            response(
                1,
                PortResponse::Secret {
                    value: Some(secret("p@ss")),
                },
            ),
            response(1, PortResponse::Secret { value: None }),
            response(2, PortResponse::HasSecret { exists: true }),
            response(3, PortResponse::SecretChanged {}),
            response(
                4,
                PortResponse::SecretStoreFailed {
                    failure: store_failure(),
                },
            ),
            registration(5, NotRegistered),
            registration(5, Enabled),
            registration(5, RequiresApproval),
            registration(5, NotFound),
            response(6, PortResponse::Registered {}),
            response(
                6,
                PortResponse::RegistrationFailed {
                    error: RegistrationError::NotInstalledCopy {},
                },
            ),
            response(
                6,
                PortResponse::RegistrationFailed {
                    error: RegistrationError::Failed {
                        description: "Operation not permitted".into(),
                    },
                },
            ),
            response(
                7,
                PortResponse::HelperBuild {
                    build: "20261109120000".into(),
                },
            ),
            response(8, PortResponse::HelperReply { json: None }),
            response(
                12,
                PortResponse::HelperReply {
                    json: Some(r#"{"childAlive":false}"#.into()),
                },
            ),
            response(13, PortResponse::HelperReplyFlag { value: true }),
            unreachable(8, HelperChannelFailure::AppNotSigned {}),
            unreachable(
                8,
                HelperChannelFailure::Connection {
                    description: "Couldn’t communicate with a helper application.".into(),
                },
            ),
            unreachable(8, HelperChannelFailure::ConnectionLost {}),
            unreachable(8, HelperChannelFailure::NoReply {}),
            unreachable(
                7,
                HelperChannelFailure::UnreadableReply {
                    description: "The data couldn’t be read because it is missing.".into(),
                },
            ),
            response(
                8,
                PortResponse::HelperBuildMismatch {
                    helper_build: "20261108090000".into(),
                },
            ),
            response(
                17,
                PortResponse::RouteInterface {
                    interface: Some("en0".into()),
                },
            ),
            response(17, PortResponse::RouteInterface { interface: None }),
            response(
                18,
                PortResponse::InterfaceAddresses {
                    addresses: vec![
                        InterfaceAddress {
                            name: "en0".into(),
                            up: true,
                            ipv4: Some(Ipv4Addr::new(10, 20, 161, 20)),
                        },
                        InterfaceAddress {
                            name: "ppp0".into(),
                            up: true,
                            ipv4: None,
                        },
                    ],
                },
            ),
            response(
                19,
                PortResponse::PrimaryRouter {
                    router: Some(Ipv4Addr::new(10, 20, 161, 1)),
                },
            ),
            response(19, PortResponse::PrimaryRouter { router: None }),
            response(
                20,
                PortResponse::ScreenState {
                    locked: true,
                    displays_asleep: false,
                },
            ),
            response(21, PortResponse::BrowserOpened { opened: false }),
            response(
                22,
                PortResponse::FortiClientConnections {
                    connections: vec![FortiClientConnection {
                        name: "Office".into(),
                        server: "vpn.example.com:10443".into(),
                    }],
                },
            ),
            response(
                22,
                PortResponse::FortiClientConnections {
                    connections: vec![],
                },
            ),
        ],
    );
}

#[test]
fn system_events() {
    let event = |event| ToRuntime::Event { event };
    let interface = |name: &str, kind| PathInterface {
        name: name.into(),
        kind,
    };
    use InterfaceKind::*;
    decodes(
        "system_events.json",
        &[
            event(SystemEvent::WillSleep { at: at(T0) }),
            event(SystemEvent::DidWake {}),
            event(SystemEvent::ScreenLocked { at: at(T0 + 250) }),
            event(SystemEvent::ScreenUnlocked {}),
            event(SystemEvent::DisplaysSlept { at: at(T0 + 500) }),
            event(SystemEvent::DisplaysWoke {}),
            event(SystemEvent::NetworkPathChanged {
                satisfied: true,
                interfaces: vec![
                    interface("en0", WiredEthernet),
                    interface("en1", Wifi),
                    interface("pdp_ip0", Cellular),
                    interface("lo0", Loopback),
                    interface("ppp0", Other),
                ],
            }),
            event(SystemEvent::NetworkPathChanged {
                satisfied: false,
                interfaces: vec![],
            }),
            event(SystemEvent::NetworkSettingsChanged {}),
            event(SystemEvent::ClockChanged {}),
            event(SystemEvent::TimeZoneChanged {
                time_zone: "Europe/Berlin".into(),
            }),
            event(SystemEvent::AppActivated {}),
            event(SystemEvent::QuitRequested {}),
            event(SystemEvent::HelperEvent {
                json: format!(r#"{{"attemptID":"{ATTEMPT}","kind":{{"milestone":{{"_0":0}}}}}}"#),
            }),
            event(SystemEvent::HelperOutput {
                json: r#"[{"date":1762412345.25,"source":"stdout","text":"INFO:   Connected to gateway."}]"#.into(),
            }),
            event(SystemEvent::HelperConnectionLost {}),
        ],
    );
}

#[test]
fn swift_may_write_a_fraction_of_a_millisecond() {
    let event: ToRuntime =
        serde_json::from_str(r#"{"event":{"_0":{"willSleep":{"at":1762412345250.75}}}}"#).unwrap();
    assert_eq!(
        event,
        ToRuntime::Event {
            event: SystemEvent::WillSleep {
                at: UNIX_EPOCH + Duration::from_secs_f64(1_762_412_345_250.75 / 1000.0),
            },
        }
    );
}

#[test]
fn debug_output_redacts_secrets() {
    let debug = format!(
        "{:?}",
        (
            Command::EditAccount {
                revision: 0,
                form: AccountForm {
                    host: "vpn.example.com".into(),
                    port: "443".into(),
                    username: "alice".into(),
                    realm: String::new(),
                    gmail_address: "alice@example.com".into(),
                    vpn_password: secret("hunter2"),
                    app_password: secret("abcdefghijklmnop"),
                },
            },
            PortRequest::WriteSecret {
                item: SecretItem::GmailAppPassword {
                    address: "alice@example.com".into(),
                },
                value: secret("qrstuvwxyzabcdef"),
            },
            HelperCall::SubmitOtp {
                attempt: attempt(),
                code: secret("987654"),
            },
            PortResponse::Secret {
                value: Some(secret("token-xyz")),
            },
        )
    );
    for secret in [
        "hunter2",
        "abcdefghijklmnop",
        "qrstuvwxyzabcdef",
        "987654",
        "token-xyz",
    ] {
        assert!(!debug.contains(secret), "{debug}");
    }
    assert!(debug.contains("<redacted>"), "{debug}");
}
