mod common;

use std::cell::RefCell;
use std::fs;
use std::sync::Arc;

use common::*;
use fvm_contract::helper_internal::{AddedRoute, LogCategory, LogLevel, OutsideChanges};
use fvm_helper_core::session::SessionManager;
use serde_json::json;

struct CleanedUp {
    logs: Vec<(LogLevel, String)>,
    reverted: Vec<OutsideChanges>,
    left: Vec<AddedRoute>,
}

fn clean_up(directory: &RunDirectory, secrets: &Arc<FakeSecrets>) -> CleanedUp {
    let sessions = SessionManager::new(
        Arc::new(FakeBackend::default()),
        Arc::clone(secrets) as _,
        directory.path.clone(),
        timing(),
    );
    let logs = RefCell::new(Vec::new());
    let mut reverted = Vec::new();
    let left = sessions.clean_up(
        |level, category, message| {
            assert_eq!(category, LogCategory::Helper);
            logs.borrow_mut().push((level, message));
        },
        |changes| reverted.push(changes),
        |recorded| recorded,
    );
    CleanedUp {
        logs: logs.into_inner(),
        reverted,
        left,
    }
}

fn changes(dns_service_id: &str) -> OutsideChanges {
    OutsideChanges {
        dns_service_id: dns_service_id.to_owned(),
        routes: vec![gateway_route("192.0.2.1")],
    }
}

#[test]
fn every_session_an_earlier_helper_left_is_logged_out_and_deleted() {
    let gateway = FakeGateway::start();
    let secrets = Arc::new(FakeSecrets::holding([
        (
            ATTEMPT.to_owned(),
            Ok(secret_json("127.0.0.1", gateway.port)),
        ),
        (
            OTHER_ATTEMPT.to_owned(),
            Ok(secret_json("127.0.0.1", gateway.port)),
        ),
        ("unreadable".to_owned(), Err("decryption failed".to_owned())),
        ("garbled".to_owned(), Ok(b"{".to_vec())),
    ]));
    let directory = RunDirectory::new();
    let cleaned = clean_up(&directory, &secrets);

    assert_eq!(
        gateway.wait_for_connections(2),
        [TLS_HANDSHAKE.to_vec(), TLS_HANDSHAKE.to_vec()]
    );
    assert!(secrets.stored().is_empty());
    let mut deleted = secrets.deletes();
    deleted.sort();
    let mut names = vec![ATTEMPT, OTHER_ATTEMPT, "unreadable", "garbled"];
    names.sort_unstable();
    assert_eq!(deleted, names);
    let logged_out: Vec<&String> = cleaned
        .logs
        .iter()
        .filter(|(level, message)| {
            *level == LogLevel::Notice && message.contains("logout request failed: HTTP 000")
        })
        .map(|(_, message)| message)
        .collect();
    assert_eq!(logged_out.len(), 2, "{:?}", cleaned.logs);
    assert!(cleaned.logs.contains(&(
        LogLevel::Error,
        "cannot read the session unreadable an earlier helper left: decryption failed".to_owned()
    )));
    assert!(cleaned.logs.iter().any(|(level, message)| {
        *level == LogLevel::Error
            && message.starts_with("cannot read the session garbled an earlier helper left: ")
    }));
}

#[test]
fn without_sessions_left_nothing_is_logged_out() {
    let gateway = FakeGateway::start();
    let secrets = Arc::new(FakeSecrets::default());
    let directory = RunDirectory::new();
    let cleaned = clean_up(&directory, &secrets);
    assert!(cleaned.logs.is_empty(), "{:?}", cleaned.logs);
    assert!(secrets.deletes().is_empty());
    assert!(gateway.connections().is_empty());
}

#[test]
fn both_run_records_are_reverted_and_removed() {
    let secrets = Arc::new(FakeSecrets::default());
    let directory = RunDirectory::new();
    fs::create_dir(&directory.path).unwrap();
    let record = |changes: &OutsideChanges| {
        json!({"processes": [], "gatewayIP": GATEWAY, "outsideChanges": changes}).to_string()
    };
    fs::write(directory.file("run.json"), record(&changes("first"))).unwrap();
    fs::write(directory.file("run.json.2"), record(&changes("second"))).unwrap();
    fs::write(directory.file("run.json.tmp"), "{").unwrap();
    fs::write(directory.file("config.7"), "host = vpn.example.com\n").unwrap();
    let cleaned = clean_up(&directory, &secrets);
    assert_eq!(cleaned.reverted, [changes("first"), changes("second")]);
    assert!(cleaned.logs.is_empty(), "{:?}", cleaned.logs);
    assert!(cleaned.left.is_empty());
    assert_eq!(directory.names(), ["config.7", "run.json.tmp"]);
}

#[test]
fn an_unreadable_run_record_is_logged_and_removed() {
    let secrets = Arc::new(FakeSecrets::default());
    let directory = RunDirectory::new();
    fs::create_dir(&directory.path).unwrap();
    fs::write(directory.file("run.json"), "not JSON").unwrap();
    let cleaned = clean_up(&directory, &secrets);
    assert!(cleaned.reverted.is_empty());
    assert_eq!(
        cleaned.logs,
        [(
            LogLevel::Error,
            format!(
                "ignoring unreadable run record {}",
                directory.file("run.json").display()
            )
        )]
    );
    assert!(directory.names().is_empty());
}

#[test]
fn a_session_left_with_a_value_that_fails_the_check_is_deleted_without_a_logout() {
    let gateway = FakeGateway::start();
    let secret = |host: &str, port: u16, pin: &str| {
        serde_json::to_vec(
            &json!({"host": host, "port": port, "pins": [PIN, pin], "cookie": COOKIE}),
        )
        .unwrap()
    };
    let secrets = Arc::new(FakeSecrets::holding([
        (
            "bad-pin".to_owned(),
            Ok(secret(
                "127.0.0.1",
                gateway.port,
                "sha256//VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=",
            )),
        ),
        (
            "bad-host".to_owned(),
            Ok(secret("127.0.0.1/x", gateway.port, PIN)),
        ),
        ("bad-port".to_owned(), Ok(secret("127.0.0.1", 0, PIN))),
        (
            ATTEMPT.to_owned(),
            Ok(secret_json("127.0.0.1", gateway.port)),
        ),
    ]));
    let directory = RunDirectory::new();
    let cleaned = clean_up(&directory, &secrets);

    assert_eq!(gateway.wait_for_connections(1), [TLS_HANDSHAKE.to_vec()]);
    assert!(secrets.stored().is_empty());
    let mut deleted = secrets.deletes();
    deleted.sort();
    let mut names = vec![ATTEMPT, "bad-host", "bad-pin", "bad-port"];
    names.sort_unstable();
    assert_eq!(deleted, names);
    let unreadable = |name: &str, error: &str| {
        (
            LogLevel::Error,
            format!("cannot read the session {name} an earlier helper left: {error}"),
        )
    };
    for expected in [
        unreadable("bad-pin", "pin 1 is not a pin-sha256: value"),
        unreadable(
            "bad-host",
            "host must be a DNS hostname (letters, digits and hyphens, labels of 1-63) or a \
             dotted-quad IPv4 address",
        ),
        unreadable("bad-port", "port must be between 1 and 65535"),
    ] {
        assert!(cleaned.logs.contains(&expected), "{:?}", cleaned.logs);
    }
    assert_eq!(
        cleaned
            .logs
            .iter()
            .filter(|(level, _)| *level == LogLevel::Notice)
            .count(),
        1,
        "{:?}",
        cleaned.logs
    );
}

#[test]
fn the_sessions_left_are_logged_out_even_when_the_run_directory_cannot_be_used() {
    let gateway = FakeGateway::start();
    let secrets = Arc::new(FakeSecrets::holding([(
        ATTEMPT.to_owned(),
        Ok(secret_json("127.0.0.1", gateway.port)),
    )]));
    let directory = RunDirectory::new();
    fs::write(&directory.path, "").unwrap();
    let cleaned = clean_up(&directory, &secrets);
    fs::remove_file(&directory.path).unwrap();

    assert_eq!(gateway.wait_for_connections(1), [TLS_HANDSHAKE.to_vec()]);
    assert_eq!(secrets.deletes(), [ATTEMPT]);
    assert!(secrets.stored().is_empty());
    assert!(cleaned.left.is_empty());
    assert_eq!(cleaned.logs.len(), 2, "{:?}", cleaned.logs);
    assert_eq!(cleaned.logs[0].0, LogLevel::Error);
    assert!(
        cleaned.logs[0]
            .1
            .starts_with("cannot prepare the run directory: "),
        "{:?}",
        cleaned.logs
    );
    assert_eq!(cleaned.logs[1].0, LogLevel::Notice);
    assert!(
        cleaned.logs[1].1.starts_with(&format!(
            "session {ATTEMPT} an earlier helper left: logout request failed: HTTP 000"
        )),
        "{:?}",
        cleaned.logs
    );
}
