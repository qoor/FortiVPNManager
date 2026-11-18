//! Cleaning up after an earlier Helper before "ready". The tests do not run as root, so no recorded
//! process can match and none is ever stopped here.

mod common;

use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::process::{Child, Command};
use std::time::Duration;

use common::*;
use serde_json::{Value, json};

/// A run directory as an earlier Helper left it: the pidfile, a config file and an unfinished
/// pidfile.
fn left_behind(record: &str) -> Scratch {
    let scratch = Scratch::new();
    let directory = scratch.run_directory();
    DirBuilder::new().mode(0o700).create(&directory).unwrap();
    fs::write(directory.join("run.json"), record).unwrap();
    fs::write(directory.join("config.7"), "host = vpn.example.com\n").unwrap();
    fs::write(directory.join("run.json.tmp"), "{").unwrap();
    fs::write(directory.join("unrelated"), "").unwrap();
    scratch
}

fn sleeper() -> Child {
    Command::new("/bin/sleep").arg("30").spawn().unwrap()
}

#[test]
fn routes_are_repaired_and_leftovers_removed_before_ready() {
    let mut sleeper = sleeper();
    // The sleeper's pid and path, but not its start time.
    let record = json!({
        "processes": [{
            "pid": sleeper.id(),
            "startSeconds": 1,
            "startMicroseconds": 0,
            "path": "/bin/sleep",
        }],
        "gatewayIP": GATEWAY,
    });
    let mut swift = Swift::launch(left_behind(&record.to_string()), Setup::fixture("success"));

    let first = swift.receive_within(Duration::from_secs(10));
    assert_eq!(first, Some(json!({"repairRoutes": {"gateway": GATEWAY}})));
    let before_answer = swift.received_within(Duration::from_millis(300));
    assert!(before_answer.is_empty(), "{before_answer:#?}");

    swift.send(json!({"routesRepaired": {"outcome": "repaired"}}));
    swift.wait_until("ready", |swift| swift.position(is("ready")).is_some());
    let logged = swift.position(is("log")).unwrap();
    assert_eq!(
        swift.received[logged]["log"]["message"],
        "cleaned up after an unfinished run (route repair: Repaired)"
    );
    assert_eq!(swift.received.len(), 3, "{:#?}", swift.received);
    assert_eq!(swift.scratch.run_directory_names(), ["unrelated"]);

    assert_eq!(sleeper.try_wait().unwrap(), None);
    sleeper.kill().unwrap();
    sleeper.wait().unwrap();
}

#[test]
fn recorded_network_changes_are_reverted_before_the_route_repair_and_ready() {
    let changes = json!({
        "dnsServiceID": "svc",
        "routes": [{
            "destination": {"address": GATEWAY, "prefixLength": 32},
            "router": "192.0.2.1",
            "serviceID": "ethernet",
        }],
    });
    let record = json!({"processes": [], "gatewayIP": GATEWAY, "outsideChanges": changes});
    let scratch = left_behind(&record.to_string());
    let values = scratch.run_directory().join("values");
    DirBuilder::new().mode(0o700).create(&values).unwrap();
    fs::write(values.join("values.7"), "reason=connect\0").unwrap();
    fs::write(values.join("values.8.tmp"), "reason=").unwrap();
    let mut swift = Swift::launch(scratch, Setup::fixture("success"));

    let first = swift.receive_within(Duration::from_secs(10));
    assert_eq!(
        first,
        Some(json!({"changeNetwork": {"change": {"revert": {"changes": changes}}}}))
    );
    let before_answer = swift.received_within(Duration::from_millis(300));
    assert!(before_answer.is_empty(), "{before_answer:#?}");
    swift.send(json!({"networkChanged": {"outcome": {"reverted": {"left": []}}}}));
    swift.wait_until("the route repair", |swift| {
        swift.position(is("repairRoutes")).is_some()
    });
    assert!(swift.has_log(
        "notice routes: reverted the network settings an unfinished run left (0 of 1 route(s) still in place)"
    ));
    swift.send(json!({"routesRepaired": {"outcome": "notNeeded"}}));
    swift.wait_until("ready", |swift| swift.position(is("ready")).is_some());
    assert_eq!(swift.scratch.run_directory_names(), ["unrelated", "values"]);
    assert_eq!(fs::read_dir(&values).unwrap().count(), 0);
}

#[test]
fn every_pidfile_left_is_cleaned_up_before_ready() {
    let changes = |dns: &str, destination: &str| {
        json!({
            "dnsServiceID": dns,
            "routes": [{
                "destination": {"address": destination, "prefixLength": 32},
                "router": "192.0.2.1",
                "serviceID": "ethernet",
            }],
        })
    };
    let first = changes("first", GATEWAY);
    let second = changes("second", "198.51.100.7");
    let record = json!({"processes": [], "gatewayIP": GATEWAY, "outsideChanges": first});
    let scratch = left_behind(&record.to_string());
    let record = json!({"processes": [], "outsideChanges": second});
    fs::write(
        scratch.run_directory().join("run.json.2"),
        record.to_string(),
    )
    .unwrap();
    fs::write(scratch.run_directory().join("run.json.2.tmp"), "{").unwrap();
    let mut swift = Swift::launch(scratch, Setup::fixture("success"));

    let requests = |swift: &Swift| -> Vec<Value> {
        let logged = is("log");
        swift
            .received
            .iter()
            .filter(|message| !logged(message))
            .cloned()
            .collect()
    };
    for (number, (asked, answer)) in [
        (
            json!({"changeNetwork": {"change": {"revert": {"changes": first}}}}),
            json!({"networkChanged": {"outcome": {"reverted": {"left": []}}}}),
        ),
        (
            json!({"repairRoutes": {"gateway": GATEWAY}}),
            json!({"routesRepaired": {"outcome": "notNeeded"}}),
        ),
        (
            json!({"changeNetwork": {"change": {"revert": {"changes": second}}}}),
            json!({"networkChanged": {"outcome": {"reverted": {"left": []}}}}),
        ),
        (
            json!({"repairRoutes": {}}),
            json!({"routesRepaired": {"outcome": "notNeeded"}}),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        swift.wait_until("the next cleanup request", |swift| {
            requests(swift).len() > number
        });
        assert_eq!(requests(&swift)[number], asked, "{:#?}", swift.received);
        swift.send(answer);
    }
    swift.wait_until("ready", |swift| swift.position(is("ready")).is_some());
    assert_eq!(swift.scratch.run_directory_names(), ["unrelated"]);
}

#[test]
fn both_records_of_a_switch_left_halfway_are_reverted_before_ready() {
    let route = |destination: &str| {
        json!({
            "destination": {"address": destination, "prefixLength": 32},
            "router": "192.0.2.1",
            "serviceID": "ethernet",
        })
    };
    let changes = json!({
        "dnsServiceID": "svc",
        "routes": [route(GATEWAY), route("10.20.161.16")],
    });
    let record = json!({"processes": [], "gatewayIP": GATEWAY, "outsideChanges": changes});
    let scratch = left_behind(&record.to_string());
    fs::write(
        scratch.run_directory().join("run.json.2"),
        record.to_string(),
    )
    .unwrap();
    let mut swift = Swift::launch(scratch, Setup::fixture("success"));

    let requests = |swift: &Swift| -> Vec<Value> {
        let logged = is("log");
        swift
            .received
            .iter()
            .filter(|message| !logged(message))
            .cloned()
            .collect()
    };
    let revert = (
        json!({"changeNetwork": {"change": {"revert": {"changes": changes}}}}),
        json!({"networkChanged": {"outcome": {"reverted": {"left": []}}}}),
    );
    let repair = (
        json!({"repairRoutes": {"gateway": GATEWAY}}),
        json!({"routesRepaired": {"outcome": "notNeeded"}}),
    );
    for (number, (asked, answer)) in [revert.clone(), repair.clone(), revert, repair]
        .into_iter()
        .enumerate()
    {
        swift.wait_until("the next cleanup request", |swift| {
            requests(swift).len() > number
        });
        assert_eq!(requests(&swift)[number], asked, "{:#?}", swift.received);
        swift.send(answer);
    }
    swift.wait_until("ready", |swift| swift.position(is("ready")).is_some());
    assert_eq!(swift.scratch.run_directory_names(), ["unrelated"]);
}

#[test]
fn a_second_pidfile_left_alone_is_cleaned_up_before_ready() {
    let changes = json!({
        "dnsServiceID": "svc",
        "routes": [{
            "destination": {"address": GATEWAY, "prefixLength": 32},
            "router": "192.0.2.1",
            "serviceID": "ethernet",
        }],
    });
    let record = json!({"processes": [], "gatewayIP": GATEWAY, "outsideChanges": changes});
    let scratch = Scratch::new();
    let directory = scratch.run_directory();
    DirBuilder::new().mode(0o700).create(&directory).unwrap();
    fs::write(directory.join("run.json.2"), record.to_string()).unwrap();
    let mut swift = Swift::launch(scratch, Setup::fixture("success"));

    let first = swift.receive_within(Duration::from_secs(10));
    assert_eq!(
        first,
        Some(json!({"changeNetwork": {"change": {"revert": {"changes": changes}}}}))
    );
    swift.send(json!({"networkChanged": {"outcome": {"reverted": {"left": []}}}}));
    swift.wait_until("the route repair", |swift| {
        swift.position(is("repairRoutes")).is_some()
    });
    assert_eq!(
        swift.received[swift.position(is("repairRoutes")).unwrap()],
        json!({"repairRoutes": {"gateway": GATEWAY}})
    );
    swift.send(json!({"routesRepaired": {"outcome": "notNeeded"}}));
    swift.wait_until("ready", |swift| swift.position(is("ready")).is_some());
    assert!(swift.scratch.run_directory_names().is_empty());
}

#[test]
fn an_unreadable_pidfile_still_leads_to_a_route_repair() {
    let mut swift = Swift::launch(left_behind("not JSON"), Setup::fixture("success"));
    swift.wait_until("the route repair", |swift| {
        swift.position(is("repairRoutes")).is_some()
    });
    let pidfile = swift.scratch.run_directory().join("run.json");
    assert_eq!(
        swift.received[..2],
        [
            json!({"log": {
                "level": "error",
                "category": "helper",
                "message": format!("ignoring unreadable pidfile {}", pidfile.display()),
            }}),
            json!({"repairRoutes": {}}),
        ]
    );
    swift.send(json!({"routesRepaired": {"outcome": "notNeeded"}}));
    swift.wait_until("ready", |swift| swift.position(is("ready")).is_some());
    assert!(
        swift
            .has_log("notice helper: cleaned up after an unfinished run (route repair: NotNeeded)")
    );
}

#[test]
fn without_a_pidfile_the_entry_is_ready_at_once() {
    let scratch = Scratch::new();
    let directory = scratch.run_directory();
    DirBuilder::new().mode(0o755).create(&directory).unwrap();
    fs::write(directory.join("config.3"), "").unwrap();
    let mut swift = Swift::launch(scratch, Setup::fixture("success"));
    let first = swift.receive_within(Duration::from_secs(10));
    assert_eq!(first, Some(json!({"ready": {}})));
    assert!(swift.scratch.run_directory_names().is_empty());
    // The directory is made private again.
    let mode = fs::metadata(&directory).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn a_run_directory_that_cannot_be_used_is_logged_and_left_alone() {
    let scratch = Scratch::new();
    // A file where the directory belongs.
    fs::write(scratch.run_directory(), "").unwrap();
    let mut swift = Swift::launch(scratch, Setup::fixture("success"));
    swift.wait_until("ready", |swift| swift.position(is("ready")).is_some());
    let path = swift.scratch.run_directory();
    assert_eq!(
        swift.received[0]["log"],
        json!({
            "level": "error",
            "category": "helper",
            "message": format!(
                "cannot prepare the run directory: {} is not a directory owned by the helper: \
                 Operation not permitted (os error 1)",
                path.display()
            ),
        })
    );

    // A connect then fails the same way, before anything starts.
    assert!(swift.connect_app(1, 501));
    let message = format!(
        "could not create the openconnect config file: {} is not a directory owned by the \
         helper: Operation not permitted (os error 1)",
        path.display()
    );
    assert_eq!(swift.connect(1, ATTEMPT), error("internalError", &message));
    assert!(
        swift
            .logs()
            .iter()
            .any(|line| line.starts_with("error helper: cannot write the openconnect config: "))
    );
    assert!(swift.events(1).is_empty());
}
