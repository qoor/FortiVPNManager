mod common;

use std::cell::RefCell;
use std::fs;
use std::sync::Arc;

use common::*;
use fvm_contract::helper_internal::{AddedRoute, LogCategory, LogLevel, OutsideChanges};
use fvm_helper_core::session::SessionManager;
use serde_json::{Value, json};

struct CleanedUp {
    logs: Vec<(LogLevel, String)>,
    reverted: Vec<OutsideChanges>,
    reverted_again: Vec<OutsideChanges>,
    left: Vec<AddedRoute>,
    unreverted: Vec<OutsideChanges>,
}

fn clean_up(directory: &RunDirectory, secrets: &Arc<FakeSecrets>) -> CleanedUp {
    clean_up_leaving(directory, secrets, |_| None)
}

fn clean_up_leaving(
    directory: &RunDirectory,
    secrets: &Arc<FakeSecrets>,
    left: impl Fn(&OutsideChanges) -> Option<OutsideChanges>,
) -> CleanedUp {
    let sessions = SessionManager::new(
        Arc::new(FakeBackend::default()),
        Arc::clone(secrets) as _,
        directory.path.clone(),
        timing(),
    );
    let logs = RefCell::new(Vec::new());
    let mut reverted = Vec::new();
    let mut reverted_again = Vec::new();
    let (left_routes, unreverted) = sessions.clean_up(
        |level, category, message| {
            assert_eq!(category, LogCategory::Helper);
            logs.borrow_mut().push((level, message));
        },
        |changes| {
            let remaining = left(&changes);
            reverted.push(changes);
            remaining
        },
        |kept| {
            reverted_again.extend(kept.iter().cloned());
            kept.iter().filter_map(&left).collect()
        },
        |recorded| recorded,
    );
    CleanedUp {
        logs: logs.into_inner(),
        reverted,
        reverted_again,
        left: left_routes,
        unreverted,
    }
}

fn unreverted_record(changes: &[OutsideChanges]) -> String {
    json!({"changes": changes}).to_string()
}

fn recorded_unreverted(directory: &RunDirectory) -> Value {
    serde_json::from_slice(&fs::read(directory.file("unreverted-changes.json")).unwrap()).unwrap()
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
    assert_eq!(secrets.deletes(), Vec::<String>::new());
    assert_eq!(gateway.connections(), Vec::<Vec<u8>>::new());
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
    assert_eq!(cleaned.left, []);
    assert_eq!(directory.names(), ["config.7", "run.json.tmp"]);
}

#[test]
fn an_unreadable_run_record_is_logged_and_removed() {
    let secrets = Arc::new(FakeSecrets::default());
    let directory = RunDirectory::new();
    fs::create_dir(&directory.path).unwrap();
    fs::write(directory.file("run.json"), "not JSON").unwrap();
    let cleaned = clean_up(&directory, &secrets);
    assert_eq!(cleaned.reverted, []);
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
    assert_eq!(directory.names(), Vec::<String>::new());
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
    assert_eq!(cleaned.left, []);
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

#[test]
fn the_network_settings_a_revert_left_are_reverted_again_and_their_record_removed() {
    let secrets = Arc::new(FakeSecrets::default());
    let directory = RunDirectory::new();
    fs::create_dir(&directory.path).unwrap();
    fs::write(
        directory.file("unreverted-changes.json"),
        unreverted_record(&[changes("first"), changes("second")]),
    )
    .unwrap();
    let cleaned = clean_up(&directory, &secrets);
    assert_eq!(cleaned.reverted, []);
    assert_eq!(
        cleaned.reverted_again,
        [changes("first"), changes("second")]
    );
    assert_eq!(cleaned.unreverted, []);
    assert!(cleaned.logs.is_empty(), "{:?}", cleaned.logs);
    assert_eq!(directory.names(), Vec::<String>::new());
}

#[test]
fn what_a_revert_at_the_start_leaves_stays_recorded_and_is_not_reverted_again_at_once() {
    let secrets = Arc::new(FakeSecrets::default());
    let directory = RunDirectory::new();
    fs::create_dir(&directory.path).unwrap();
    let record = json!({"outsideChanges": changes("run")}).to_string();
    fs::write(directory.file("run.json"), record).unwrap();
    fs::write(
        directory.file("unreverted-changes.json"),
        unreverted_record(&[changes("kept"), changes("gone")]),
    )
    .unwrap();
    let dns_left = |changes: &OutsideChanges| {
        (changes.dns_service_id != "gone").then(|| dns_only(&changes.dns_service_id))
    };
    let cleaned = clean_up_leaving(&directory, &secrets, dns_left);
    assert_eq!(cleaned.reverted, [changes("run")]);
    assert_eq!(cleaned.reverted_again, [changes("kept"), changes("gone")]);
    let left = [dns_only("kept"), dns_only("run")];
    assert_eq!(cleaned.unreverted, left);
    assert_eq!(recorded_unreverted(&directory), json!({"changes": left}));
    assert_eq!(directory.names(), ["unreverted-changes.json"]);
    assert!(cleaned.logs.is_empty(), "{:?}", cleaned.logs);
}

fn dns_only(dns_service_id: &str) -> OutsideChanges {
    OutsideChanges {
        dns_service_id: dns_service_id.to_owned(),
        routes: Vec::new(),
    }
}

#[test]
fn an_unreadable_unreverted_change_record_is_logged_and_removed() {
    let secrets = Arc::new(FakeSecrets::default());
    let directory = RunDirectory::new();
    fs::create_dir(&directory.path).unwrap();
    fs::write(directory.file("unreverted-changes.json"), "not JSON").unwrap();
    let cleaned = clean_up(&directory, &secrets);
    assert_eq!(cleaned.reverted_again, []);
    assert_eq!(cleaned.unreverted, []);
    assert_eq!(
        cleaned.logs,
        [(
            LogLevel::Error,
            format!(
                "ignoring unreadable unreverted change record {}",
                directory.file("unreverted-changes.json").display()
            )
        )]
    );
    assert_eq!(directory.names(), Vec::<String>::new());
}
