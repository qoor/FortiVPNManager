//! Cleaning up after an earlier Helper before "ready". The tests do not run as root, so no recorded
//! process can match and none is ever stopped here.

mod common;

use std::fs::{self, DirBuilder};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::process::{Child, Command};
use std::time::Duration;

use common::*;
use serde_json::json;

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
        "cleaned up after an unfinished run (route repair: repaired)"
    );
    assert_eq!(swift.received.len(), 3, "{:#?}", swift.received);
    assert_eq!(swift.scratch.run_directory_names(), ["unrelated"]);

    assert_eq!(sleeper.try_wait().unwrap(), None);
    sleeper.kill().unwrap();
    sleeper.wait().unwrap();
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
            .has_log("notice helper: cleaned up after an unfinished run (route repair: notNeeded)")
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
                 Operation not permitted (errno 1)",
                path.display()
            ),
        })
    );

    // A connect then fails the same way, before anything starts.
    assert!(swift.connect_app(1, 501));
    let message = format!(
        "could not create the openfortivpn config file: {} is not a directory owned by the \
         helper: Operation not permitted (errno 1)",
        path.display()
    );
    assert_eq!(swift.connect(1, ATTEMPT), error("internalError", &message));
    assert!(
        swift
            .logs()
            .iter()
            .any(|line| line.starts_with("error helper: cannot write the openfortivpn config: "))
    );
    assert!(swift.events(1).is_empty());
}
