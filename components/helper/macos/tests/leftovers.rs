//! Cleaning up after an earlier Helper before "ready".

mod common;

use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use serde_json::{Value, json};

/// A run directory as an earlier Helper left it: the run record, a config file of an older build
/// and an unfinished run record.
fn left_behind(record: &str) -> Scratch {
    let scratch = Scratch::new();
    let directory = scratch.run_directory();
    DirBuilder::new().mode(0o700).create(&directory).unwrap();
    fs::write(directory.join("run.json"), record).unwrap();
    fs::write(directory.join("config.7"), "host = vpn.example.com\n").unwrap();
    fs::write(directory.join("run.json.tmp"), "{").unwrap();
    scratch
}

fn changes(dns: &str, destination: &str) -> Value {
    json!({
        "dnsServiceID": dns,
        "routes": [{
            "destination": {"address": destination, "prefixLength": 32},
            "router": "192.0.2.1",
            "serviceID": "ethernet",
        }],
    })
}

fn requests(os_side: &OsSide) -> Vec<Value> {
    let logged = is("log");
    os_side
        .received
        .iter()
        .filter(|message| !logged(message))
        .cloned()
        .collect()
}

#[test]
fn recorded_network_changes_are_reverted_before_ready() {
    let changes = changes("svc", GATEWAY);
    let record = json!({"processes": [], "gatewayIP": GATEWAY, "outsideChanges": changes});
    let mut os_side = OsSide::launch(left_behind(&record.to_string()), Setup::default());

    let first = os_side.receive_within(Duration::from_secs(10));
    assert_eq!(
        first,
        Some(json!({"changeNetwork": {"change": {"revert": {"changes": changes}}}}))
    );
    let before_answer = os_side.received_within(Duration::from_millis(300));
    assert!(before_answer.is_empty(), "{before_answer:#?}");
    os_side.send(json!({"networkChanged": {"outcome": {"reverted": {"left": []}}}}));
    os_side.wait_until("ready", |os_side| os_side.position(is("ready")).is_some());
    assert!(os_side.has_log(
        "notice routes: reverted the network settings an unfinished run left (0 of 1 route(s) still in place)"
    ));
    assert_eq!(
        os_side.scratch.run_directory_names(),
        ["config.7", "run.json.tmp"]
    );
}

#[test]
fn every_run_record_left_is_cleaned_up_before_ready() {
    let first = changes("first", GATEWAY);
    let second = changes("second", "198.51.100.7");
    let record = json!({"outsideChanges": first});
    let scratch = left_behind(&record.to_string());
    let record = json!({"outsideChanges": second});
    fs::write(
        scratch.run_directory().join("run.json.2"),
        record.to_string(),
    )
    .unwrap();
    let mut os_side = OsSide::launch(scratch, Setup::default());

    for (number, changes) in [first, second].into_iter().enumerate() {
        os_side.wait_until("the next cleanup request", |os_side| {
            requests(os_side).len() > number
        });
        assert_eq!(
            requests(&os_side)[number],
            json!({"changeNetwork": {"change": {"revert": {"changes": changes}}}}),
            "{:#?}",
            os_side.received
        );
        os_side.send(json!({"networkChanged": {"outcome": {"reverted": {"left": []}}}}));
    }
    os_side.wait_until("ready", |os_side| os_side.position(is("ready")).is_some());
    assert_eq!(
        os_side.scratch.run_directory_names(),
        ["config.7", "run.json.tmp"]
    );
}

#[test]
fn an_unreadable_run_record_is_logged_and_removed() {
    let mut os_side = OsSide::launch(left_behind("not JSON"), Setup::default());
    os_side.wait_until("ready", |os_side| os_side.position(is("ready")).is_some());
    let record = os_side.scratch.run_directory().join("run.json");
    assert_eq!(
        os_side.received,
        [
            json!({"log": {
                "level": "error",
                "category": "helper",
                "message": format!("ignoring unreadable run record {}", record.display()),
            }}),
            json!({"ready": {}}),
        ]
    );
    assert!(!record.exists());
}

#[test]
fn the_sessions_an_earlier_helper_left_are_logged_out_and_deleted_before_ready() {
    let gateway = FakeGateway::start();
    let secrets = Arc::new(FakeSecrets::holding([
        (
            ATTEMPT.to_owned(),
            Ok(secret_json("127.0.0.1", gateway.port)),
        ),
        ("unreadable".to_owned(), Err("decryption failed".to_owned())),
    ]));
    let changes = changes("svc", GATEWAY);
    let record = json!({"outsideChanges": changes});
    let setup = Setup {
        secrets: Arc::clone(&secrets),
        ..Setup::default()
    };
    let mut os_side = OsSide::launch(left_behind(&record.to_string()), setup);
    let first = os_side.receive_within(Duration::from_secs(10));
    assert_eq!(
        first,
        Some(json!({"changeNetwork": {"change": {"revert": {"changes": changes}}}}))
    );
    assert_eq!(gateway.connections(), Vec::<Vec<u8>>::new());
    os_side.send(json!({"networkChanged": {"outcome": {"reverted": {"left": []}}}}));
    os_side.wait_until("ready", |os_side| os_side.position(is("ready")).is_some());
    assert_eq!(gateway.connections(), [TLS_HANDSHAKE.to_vec()]);
    assert!(secrets.stored().is_empty());
    let logs = os_side.logs();
    let logged_out = format!("notice helper: session {ATTEMPT} an earlier helper left: ");
    assert!(
        logs.iter().any(|line| line.starts_with(&logged_out)),
        "{logs:#?}"
    );
    assert!(os_side.has_log(
        "error helper: cannot read the session unreadable an earlier helper left: decryption failed"
    ));
    let ready = os_side.position(is("ready")).unwrap();
    let last_log = os_side.received.iter().rposition(is("log")).unwrap();
    assert!(last_log < ready);
}

#[test]
fn a_session_left_with_a_bad_pin_or_host_is_deleted_without_a_logout_before_ready() {
    let gateway = FakeGateway::start();
    let secret = |host: &str, pin: &str| {
        serde_json::to_vec(
            &json!({"host": host, "port": gateway.port, "pins": [pin], "cookie": COOKIE}),
        )
        .unwrap()
    };
    let secrets = Arc::new(FakeSecrets::holding([
        (
            "bad-pin".to_owned(),
            Ok(secret(
                "127.0.0.1",
                "sha256//VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=",
            )),
        ),
        ("bad-host".to_owned(), Ok(secret("127.0.0.1/x", PIN))),
    ]));
    let setup = Setup {
        secrets: Arc::clone(&secrets),
        ..Setup::default()
    };
    let mut os_side = OsSide::launch(Scratch::new(), setup);
    os_side.wait_until("ready", |os_side| os_side.position(is("ready")).is_some());
    assert_eq!(gateway.connections(), Vec::<Vec<u8>>::new());
    assert!(secrets.stored().is_empty());
    let mut deleted = secrets.deletes();
    deleted.sort();
    assert_eq!(deleted, ["bad-host", "bad-pin"]);
    assert!(os_side.has_log(
        "error helper: cannot read the session bad-pin an earlier helper left: pin 0 is not a \
         pin-sha256: value"
    ));
    assert!(os_side.has_log(
        "error helper: cannot read the session bad-host an earlier helper left: host must be a DNS \
         hostname (letters, digits and hyphens, labels of 1-63) or a dotted-quad IPv4 address"
    ));
    let ready = os_side.position(is("ready")).unwrap();
    assert_eq!(os_side.received.len(), ready + 1, "{:#?}", os_side.received);
    assert_eq!(ready, 2, "{:#?}", os_side.received);
}

#[test]
fn without_a_run_record_the_entry_is_ready_at_once() {
    let scratch = Scratch::new();
    let directory = scratch.run_directory();
    DirBuilder::new().mode(0o755).create(&directory).unwrap();
    let mut os_side = OsSide::launch(scratch, Setup::default());
    let first = os_side.receive_within(Duration::from_secs(10));
    assert_eq!(first, Some(json!({"ready": {}})));
    // The directory is made private again.
    let mode = fs::metadata(&directory).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn a_run_directory_that_cannot_be_used_is_logged_and_left_alone() {
    let scratch = Scratch::new();
    // A file where the directory belongs.
    fs::write(scratch.run_directory(), "").unwrap();
    let mut os_side = OsSide::launch(scratch, Setup::default());
    os_side.wait_until("ready", |os_side| os_side.position(is("ready")).is_some());
    let path = os_side.scratch.run_directory();
    let unusable = format!(
        "{} is not a directory owned by the helper: Operation not permitted (os error 1)",
        path.display()
    );
    assert_eq!(
        os_side.received[0]["log"],
        json!({
            "level": "error",
            "category": "helper",
            "message": format!("cannot prepare the run directory: {unusable}"),
        })
    );

    // A connect then fails the same way, before anything starts.
    assert!(os_side.connect_app(1, 501));
    assert_eq!(
        os_side.connect(1, ATTEMPT),
        error(
            "internalError",
            &format!("could not prepare the run directory: {unusable}")
        )
    );
    assert!(os_side.logs().iter().any(|line| {
        line == &format!("error helper: cannot prepare the run directory: {unusable}")
    }));
    assert_eq!(os_side.events(1), Vec::<Value>::new());
}
