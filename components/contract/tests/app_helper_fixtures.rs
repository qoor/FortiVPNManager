//! The files in `fixtures/app-helper/` were captured from the phase-1 Swift app's XPC encoder
//! (FortiShared `XPCCoding`); phase 2 replaces them. Never edit them by hand. The Helper receives
//! the run request and sends everything else, so the run request is decoded and the rest encoded.

use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fvm_contract::app_helper::*;
use serde_json::{Value, to_value};

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

const ATTEMPT: &str = "E621E1F8-C36C-495A-93FC-0C247A3E6E5F";
const DIGEST_A: &str = "4f7a0c2b9d1e8f3a6b5c4d3e2f1a0b9c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a";
const DIGEST_B: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const CELLAR_PATH: &str = "/opt/homebrew/Cellar/openfortivpn/1.24.1/bin/openfortivpn";

fn attempt() -> AttemptId {
    ATTEMPT.parse().unwrap()
}

fn full_facts() -> ExitFacts {
    ExitFacts {
        furthest_milestone: Some(Milestone::TunnelUp),
        stop_initiator: StopInitiator::Helper {
            reason: StopReason::ProactiveReconnect,
        },
        cert_rejected_digests: vec![DIGEST_A.into()],
        cert_rejected_at: Some(Milestone::ConnectedToGateway),
        tunnel_mode_rejected: false,
        up_duration: Some(Duration::from_millis(10_498_750)),
        exit_status: ProcessExitStatus::Exited { code: 0 },
    }
}

fn at(seconds: f64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs_f64(seconds)
}

#[test]
fn run_request() {
    let request: RunRequest =
        serde_json::from_str(include_str!("fixtures/app-helper/run_request.json")).unwrap();
    assert_eq!(
        request,
        RunRequest {
            attempt_id: attempt(),
            host: "vpn.example.com".into(),
            port: 443,
            username: "alice".into(),
            realm: Some("staff".into()),
            trusted_certs: vec![DIGEST_A.into(), DIGEST_B.into()],
            password: "p@ss/w\"rd\\ ‹ü›".into(),
        }
    );
}

#[test]
fn run_request_without_realm() {
    let request: RunRequest = serde_json::from_str(include_str!(
        "fixtures/app-helper/run_request_without_realm.json"
    ))
    .unwrap();
    assert_eq!(
        request,
        RunRequest {
            attempt_id: attempt(),
            host: "192.0.2.10".into(),
            port: 10443,
            username: "bob".into(),
            realm: None,
            trusted_certs: vec![],
            password: "secret".into(),
        }
    );
}

#[test]
fn milestones_in_order() {
    use Milestone::*;
    let milestones = [
        Spawned,
        ConnectedToGateway,
        OtpPrompted,
        OtpSubmitted,
        Authenticated,
        TunnelUp,
    ];
    assert_eq!(
        to_value(milestones).unwrap(),
        fixture(include_str!("fixtures/app-helper/milestones.json"))
    );
    assert!(milestones.is_sorted());
}

#[test]
fn stop_reasons() {
    use StopReason::*;
    let reasons = [
        UserDisconnect,
        ProactiveReconnect,
        Reconnect,
        OtpTimeout,
        OtpUnavailable,
        AttemptTimeout,
        Sleep,
        AppQuit,
        AppDisconnected,
        Watchdog,
        HelperShutdown,
    ];
    assert_eq!(
        to_value(reasons).unwrap(),
        fixture(include_str!("fixtures/app-helper/stop_reasons.json"))
    );
}

#[test]
fn stop_reason_parses_the_raw_value_of_a_disconnect_request() {
    assert_eq!("clientInvalidated".parse(), Ok(StopReason::AppDisconnected));
    assert!("AppDisconnected".parse::<StopReason>().is_err());
}

#[test]
fn stop_initiators() {
    let initiators = [
        StopInitiator::Client {},
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect,
        },
        StopInitiator::Helper {
            reason: StopReason::Watchdog,
        },
    ];
    assert_eq!(
        to_value(initiators).unwrap(),
        fixture(include_str!("fixtures/app-helper/stop_initiators.json"))
    );
}

#[test]
fn process_exit_statuses() {
    let statuses = [
        ProcessExitStatus::Exited { code: 1 },
        ProcessExitStatus::Signaled { signal: 9 },
        ProcessExitStatus::SpawnFailed { errno: 13 },
        ProcessExitStatus::Unknown {},
    ];
    assert_eq!(
        to_value(statuses).unwrap(),
        fixture(include_str!(
            "fixtures/app-helper/process_exit_statuses.json"
        ))
    );
}

#[test]
fn exit_facts() {
    assert_eq!(
        to_value(full_facts()).unwrap(),
        fixture(include_str!("fixtures/app-helper/exit_facts.json"))
    );
}

#[test]
fn exit_facts_spawn_failed() {
    let facts = ExitFacts {
        furthest_milestone: None,
        stop_initiator: StopInitiator::Client {},
        cert_rejected_digests: vec![],
        cert_rejected_at: None,
        tunnel_mode_rejected: false,
        up_duration: None,
        exit_status: ProcessExitStatus::SpawnFailed { errno: 2 },
    };
    assert_eq!(
        to_value(facts).unwrap(),
        fixture(include_str!(
            "fixtures/app-helper/exit_facts_spawn_failed.json"
        ))
    );
}

#[test]
fn exit_facts_tunnel_mode_rejected() {
    let facts = ExitFacts {
        furthest_milestone: Some(Milestone::Authenticated),
        stop_initiator: StopInitiator::Client {},
        cert_rejected_digests: vec![],
        cert_rejected_at: None,
        tunnel_mode_rejected: true,
        up_duration: None,
        exit_status: ProcessExitStatus::Signaled { signal: 15 },
    };
    assert_eq!(
        to_value(facts).unwrap(),
        fixture(include_str!(
            "fixtures/app-helper/exit_facts_tunnel_mode_rejected.json"
        ))
    );
}

#[test]
fn helper_events() {
    let tunnel_up = HelperEvent {
        attempt_id: attempt(),
        kind: HelperEventKind::Milestone {
            milestone: Milestone::TunnelUp,
            tunnel_ip: Some(Ipv4Addr::new(10, 211, 0, 7)),
        },
    };
    assert_eq!(
        to_value(tunnel_up).unwrap(),
        fixture(include_str!(
            "fixtures/app-helper/helper_event_milestone.json"
        ))
    );

    let gateway = HelperEvent {
        attempt_id: attempt(),
        kind: HelperEventKind::Milestone {
            milestone: Milestone::ConnectedToGateway,
            tunnel_ip: None,
        },
    };
    assert_eq!(
        to_value(gateway).unwrap(),
        fixture(include_str!(
            "fixtures/app-helper/helper_event_milestone_without_ip.json"
        ))
    );

    let exited = HelperEvent {
        attempt_id: attempt(),
        kind: HelperEventKind::Exited {
            facts: full_facts(),
        },
    };
    assert_eq!(
        to_value(exited).unwrap(),
        fixture(include_str!("fixtures/app-helper/helper_event_exited.json"))
    );
}

#[test]
fn output_sources() {
    let sources = [
        OutputSource::Stdout,
        OutputSource::Stderr,
        OutputSource::Helper,
    ];
    assert_eq!(
        to_value(sources).unwrap(),
        fixture(include_str!("fixtures/app-helper/output_sources.json"))
    );
}

#[test]
fn output_lines() {
    let line = |date, source, text: &str| OutputLine {
        date,
        source,
        text: text.into(),
    };
    let lines = [
        line(
            at(1_762_412_345.25),
            OutputSource::Stdout,
            "INFO:   Connected to gateway.",
        ),
        line(
            at(1_762_412_346.0),
            OutputSource::Stderr,
            "Using interface ppp0",
        ),
        line(
            at(1_762_412_346.125),
            OutputSource::Helper,
            "‹exited: code 0›",
        ),
        line(
            at(1_762_412_347.5),
            OutputSource::Stdout,
            "VPN account password: \\x1b[0m",
        ),
    ];
    assert_eq!(
        to_value(lines).unwrap(),
        fixture(include_str!("fixtures/app-helper/output_lines.json"))
    );
}

#[test]
fn helper_statuses() {
    let running = HelperStatus {
        client_running: true,
        client_path: Some(CELLAR_PATH.into()),
        client_version: Some("1.24.1".into()),
        client_problem: None,
    };
    assert_eq!(
        to_value(running).unwrap(),
        fixture(include_str!("fixtures/app-helper/helper_status.json"))
    );

    let missing = HelperStatus {
        client_running: false,
        client_path: None,
        client_version: None,
        client_problem: Some(
            "openfortivpn not found at /opt/homebrew/bin/openfortivpn; install it with: brew install openfortivpn"
                .into(),
        ),
    };
    assert_eq!(
        to_value(missing).unwrap(),
        fixture(include_str!(
            "fixtures/app-helper/helper_status_problem.json"
        ))
    );
}

#[test]
fn helper_errors() {
    use HelperErrorCode::*;
    let error = |code, message: &str| HelperError {
        code,
        message: message.into(),
    };
    let errors = [
        error(Busy, "openfortivpn is already running"),
        error(InvalidRequest, "could not decode the connect request"),
        error(UnknownAttempt, "not the active connection attempt"),
        error(WrongPhase, "openfortivpn is not waiting for an OTP"),
        error(
            ClientUnavailable,
            &format!("openfortivpn is not executable: {CELLAR_PATH}"),
        ),
        error(NotAuthorized, "the VPN run belongs to another user"),
        error(
            InternalError,
            "could not create the openfortivpn config file: permission denied",
        ),
    ];
    assert_eq!(
        to_value(errors).unwrap(),
        fixture(include_str!("fixtures/app-helper/helper_errors.json"))
    );
}

#[test]
fn attempt_id_keeps_the_apps_uppercase_form() {
    let lowercase: AttemptId = ATTEMPT.to_ascii_lowercase().parse().unwrap();
    assert_eq!(lowercase, attempt());
    assert_eq!(lowercase.to_string(), ATTEMPT);
    for bad in [
        "",
        "E621E1F8C36C495A93FC0C247A3E6E5F",
        "E621E1F8-C36C-495A-93FC-0C247A3E6E5G",
    ] {
        assert!(bad.parse::<AttemptId>().is_err(), "{bad}");
    }
}

#[test]
fn run_request_debug_redacts_the_password() {
    let request: RunRequest =
        serde_json::from_str(include_str!("fixtures/app-helper/run_request.json")).unwrap();
    let debug = format!("{request:?}");
    assert!(!debug.contains(&request.password), "{debug}");
    assert!(debug.contains("password: <redacted>"), "{debug}");
}
