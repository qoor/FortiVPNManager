//! `fixtures/helper-internal/` pins the JSON of every message between the Helper's Swift side
//! and the Helper entry (Rust). Rust decodes what Swift sends and encodes what Swift
//! receives; `tests/macos/HelperInternalTests.swift` checks the Swift types the opposite way against
//! the same files.

use std::net::Ipv4Addr;

use fvm_contract::app_helper::{BypassRoute, BypassRoutesSnapshot, HelperError, HelperErrorCode};
use fvm_contract::backend::RouteRepairOutcome;
use fvm_contract::helper_internal::*;
use serde_json::Value;

const ATTEMPT: &str = "E621E1F8-C36C-495A-93FC-0C247A3E6E5F";

fn ethernet() -> BypassRoutesSnapshot {
    BypassRoutesSnapshot {
        service_id: "8A3F1C2E-5B6D-4E7F-9A0B-1C2D3E4F5A6B".into(),
        service_name: "Ethernet".into(),
        router: "10.20.161.1".into(),
        address: "10.20.161.20".into(),
        subnet_mask: "255.255.255.0".into(),
    }
}

fn route(destination: &str, subnet_mask: &str, gateway: &str) -> BypassRoute {
    BypassRoute {
        destination: destination.into(),
        subnet_mask: subnet_mask.into(),
        gateway: gateway.into(),
    }
}

/// A host target and a network target through the router.
fn routes() -> [BypassRoute; 2] {
    [
        route("10.30.161.16", "255.255.255.255", "10.20.161.1"),
        route("10.30.170.0", "255.255.255.0", "10.20.161.1"),
    ]
}

/// The host as the Helper added it through a former network.
fn stale() -> AddedRoute {
    AddedRoute {
        route: route("10.30.161.16", "255.255.255.255", "10.20.161.254"),
        service_id: "0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0".into(),
    }
}

#[test]
fn messages_to_the_entry() {
    let request = |id, request| ToEntry::Request {
        id,
        app: 1,
        request,
    };
    let host_and_network = routes();
    let messages: Vec<ToEntry> =
        serde_json::from_str(include_str!("fixtures/helper-internal/to_entry.json")).unwrap();
    assert_eq!(
        messages,
        [
            ToEntry::AppConnected { app: 1, uid: 501 },
            request(
                1,
                AppRequest::Connect {
                    json: format!(r#"{{"attemptID":"{ATTEMPT}","host":"vpn.example.com"}}"#),
                },
            ),
            request(
                2,
                AppRequest::SubmitOtp {
                    attempt: ATTEMPT.into(),
                    code: "123456".into(),
                },
            ),
            request(
                3,
                AppRequest::Disconnect {
                    attempt: ATTEMPT.into(),
                    reason: "userDisconnect".into(),
                },
            ),
            request(4, AppRequest::Status {}),
            request(5, AppRequest::ExitWhenIdle {}),
            request(6, AppRequest::ReadBypassRoutes {}),
            request(
                7,
                AppRequest::WriteBypassRoutes {
                    json: r#"{"remove":{}}"#.into(),
                },
            ),
            ToEntry::AppDisconnected { app: 1 },
            ToEntry::ExecutableReplaced {},
            ToEntry::TerminationRequested {},
            ToEntry::RoutesRepaired {
                outcome: RouteRepairOutcome::Repaired
            },
            ToEntry::RoutesRepaired {
                outcome: RouteRepairOutcome::NotNeeded
            },
            ToEntry::RoutesRepaired {
                outcome: RouteRepairOutcome::Failed
            },
            ToEntry::BypassRoutesChanged {
                outcome: BypassRoutesOutcome::Changed {
                    network: Some(ethernet()),
                    added: vec![host_and_network[1].clone()],
                    found: host_and_network.to_vec(),
                    left: Vec::new(),
                },
            },
            ToEntry::BypassRoutesChanged {
                outcome: BypassRoutesOutcome::Changed {
                    network: None,
                    added: Vec::new(),
                    found: Vec::new(),
                    left: vec![stale()],
                },
            },
            ToEntry::BypassRoutesChanged {
                outcome: BypassRoutesOutcome::Refused {
                    error: HelperError {
                        code: HelperErrorCode::WrongPhase,
                        message: "no active network service".into(),
                    },
                },
            },
        ]
    );
}

#[test]
fn messages_from_the_entry() {
    let [host, network] = routes();
    let log = |level, category, message: &str| FromEntry::Log {
        level,
        category,
        message: message.into(),
    };
    let messages = [
        FromEntry::Ready {},
        FromEntry::Admitted {
            app: 1,
            accept: true,
        },
        FromEntry::Admitted {
            app: 2,
            accept: false,
        },
        FromEntry::Reply { id: 1, json: None },
        FromEntry::Reply {
            id: 2,
            json: Some(r#"{"code":"busy","message":"openfortivpn is already running"}"#.into()),
        },
        FromEntry::ReplyFlag { id: 5, value: true },
        FromEntry::SendEvent {
            app: 1,
            json: format!(r#"{{"attemptID":"{ATTEMPT}","kind":{{"milestone":{{"_0":0}}}}}}"#),
        },
        FromEntry::SendOutput {
            app: 1,
            json: r#"[{"date":1762412345.25,"source":"stdout","text":"INFO:   Connected to gateway."}]"#
                .into(),
        },
        FromEntry::RepairRoutes {
            gateway: Some(Ipv4Addr::new(10, 30, 161, 16)),
        },
        FromEntry::RepairRoutes { gateway: None },
        FromEntry::ChangeBypassRoutes {
            change: BypassRoutesChange {
                network: NetworkCheck::Expect {
                    network: ethernet(),
                },
                remove: vec![stale()],
                add: vec![host.clone(), network.clone()],
                look_up: vec![host.clone(), network],
            },
        },
        FromEntry::ChangeBypassRoutes {
            change: BypassRoutesChange {
                network: NetworkCheck::Read {},
                remove: Vec::new(),
                add: Vec::new(),
                look_up: vec![host],
            },
        },
        FromEntry::ChangeBypassRoutes {
            change: BypassRoutesChange {
                network: NetworkCheck::Skip {},
                remove: vec![stale()],
                add: Vec::new(),
                look_up: Vec::new(),
            },
        },
        FromEntry::ExitAfterDraining {},
        FromEntry::ExitNow {},
        log(
            LogLevel::Info,
            LogCategory::Routes,
            "gateway has no IPv4 address; its host route will not be repaired",
        ),
        log(
            LogLevel::Notice,
            LogCategory::Helper,
            &format!("run {ATTEMPT}: tunnel up"),
        ),
        log(
            LogLevel::Error,
            LogCategory::Helper,
            "cannot write the pidfile: ‹denied›",
        ),
    ];
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/helper-internal/from_entry.json")).unwrap();
    assert_eq!(serde_json::to_value(messages).unwrap(), fixture);
}

#[test]
fn request_debug_redacts_the_password_and_the_otp() {
    let request = |request| ToEntry::Request {
        id: 1,
        app: 1,
        request,
    };
    let debug = format!(
        "{:?}",
        [
            request(AppRequest::Connect {
                json: r#"{"password":"hunter2"}"#.into(),
            }),
            request(AppRequest::SubmitOtp {
                attempt: ATTEMPT.into(),
                code: "987654".into(),
            }),
        ]
    );
    assert!(!debug.contains("hunter2"), "{debug}");
    assert!(!debug.contains("987654"), "{debug}");
    assert!(debug.contains("json: <redacted>"), "{debug}");
    assert!(debug.contains("code: <redacted>"), "{debug}");
}
