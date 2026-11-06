//! When the Helper exits: on the app's request, on SIGTERM and after its executable was replaced,
//! and which app connections it admits meanwhile.

mod common;

use std::process::Command;
use std::time::{Duration, Instant};

use common::*;
use serde_json::json;

#[test]
fn exit_when_idle_exits_shortly_after_the_reply_and_admits_no_one_meanwhile() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    let id = swift.request(1, json!({"exitWhenIdle": {}}));
    assert!(swift.reply_flag(id));
    let replied = Instant::now();
    assert!(
        swift.has_log("notice helper: exiting on request so launchd starts the current binary")
    );

    assert!(!swift.connect_app(2, 501));
    assert_eq!(
        swift.connect(1, ATTEMPT),
        error("busy", "the Helper is shutting down; try again shortly")
    );
    let again = swift.request(1, json!({"exitWhenIdle": {}}));
    assert!(swift.reply_flag(again));
    swift.wait_until("the exit", |swift| {
        swift.position(is("exitAfterDraining")).is_some()
    });
    let waited = replied.elapsed();
    assert!(
        (Duration::from_millis(400)..Duration::from_secs(2)).contains(&waited),
        "{waited:?}"
    );
    assert_eq!(swift.count(is("exitAfterDraining")), 1);
    assert_eq!(swift.count(is("exitNow")), 0);
    assert_eq!(swift.events(1).len(), 0);
}

#[test]
fn exit_when_idle_is_declined_during_a_run() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    swift.wait_for_milestone(1, OTP_PROMPTED);
    let id = swift.request(1, json!({"exitWhenIdle": {}}));
    assert!(!swift.reply_flag(id));

    let id = swift.request(1, disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(swift.reply(id), None);
    swift.wait_for_exit(1);
    let later = swift.received_within(Duration::from_millis(800));
    assert!(
        !later
            .iter()
            .any(|message| is("exitAfterDraining")(message) || is("exitNow")(message)),
        "{later:#?}"
    );
    assert!(swift.connect_app(2, 501));
}

#[test]
fn sigterm_while_idle_exits_at_once() {
    let mut swift = Swift::start(Setup::fixture("success"));
    swift.send(json!({"terminationRequested": {}}));
    swift.wait_until("the exit", |swift| {
        swift.position(is("exitAfterDraining")).is_some()
    });
    assert!(!swift.connect_app(1, 501));
}

#[test]
fn sigterm_during_a_run_stops_it_and_exits_once_its_exit_is_reported() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    swift.wait_for_milestone(1, OTP_PROMPTED);
    let id = swift.request(1, submit_otp(ATTEMPT, OTP));
    assert_eq!(swift.reply(id), None);
    swift.wait_for_milestone(1, TUNNEL_UP);

    swift.send(json!({"terminationRequested": {}}));
    swift.send(json!({"terminationRequested": {}}));
    assert!(!swift.connect_app(2, 501));
    assert_eq!(
        swift.connect(1, OTHER_ATTEMPT),
        error("busy", "the Helper is shutting down; try again shortly")
    );
    swift.wait_until("the exit", |swift| {
        swift.position(is("exitAfterDraining")).is_some()
    });
    let facts = swift
        .exit_facts(1)
        .expect("the run's exit before the Helper's");
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "helperShutdown"}})
    );
    let exit_event = swift
        .position(|message| {
            message["sendEvent"]["json"]
                .as_str()
                .is_some_and(|json| json.contains("exited"))
        })
        .unwrap();
    assert!(exit_event < swift.position(is("exitAfterDraining")).unwrap());

    let later = swift.received_within(Duration::from_millis(300));
    assert!(later.is_empty(), "{later:#?}");
    assert_eq!(swift.count(is("exitAfterDraining")), 1);
    assert_eq!(swift.count(is("exitNow")), 0);
    assert!(swift.scratch.run_directory_names().is_empty());
}

#[test]
fn sigterm_exits_anyway_when_the_run_does_not_end_in_time() {
    // The client ignores SIGTERM, and its grace period outlasts the Helper's shutdown limit.
    let mut swift = Swift::start(Setup {
        grace_period: Duration::from_secs(60),
        ..Setup::fixture("hang").args(&["gateway", "ignore-term"])
    });
    assert!(swift.connect_app(1, 501));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    swift.wait_for_milestone(1, CONNECTED_TO_GATEWAY);
    swift.send(json!({"terminationRequested": {}}));
    let sent = Instant::now();
    swift.wait_until_within(Duration::from_secs(20), "the forced exit", |swift| {
        swift.position(is("exitNow")).is_some()
    });
    let waited = sent.elapsed();
    assert!(
        (Duration::from_secs(14)..Duration::from_secs(17)).contains(&waited),
        "{waited:?}"
    );
    assert!(swift.has_log("error helper: the run did not finish after SIGTERM; exiting anyway"));
    assert_eq!(swift.exit_facts(1), None);
    assert_eq!(swift.count(is("exitAfterDraining")), 0);

    // The fixture would outlive the test.
    let started = swift
        .output(1)
        .into_iter()
        .find_map(|text| {
            let rest = text.strip_prefix("‹attempt started: pid ")?;
            Some(rest.split_once(',')?.0.to_owned())
        })
        .unwrap();
    Command::new("/bin/kill")
        .args(["-s", "KILL", "--", &format!("-{started}")])
        .status()
        .unwrap();
    swift.wait_for_exit(1);
}

#[test]
fn a_replaced_helper_finishes_its_run_and_then_exits() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    swift.wait_for_milestone(1, OTP_PROMPTED);
    swift.send(json!({"executableReplaced": {}}));
    // A connection that comes during the run is still admitted.
    assert!(swift.connect_app(2, 501));
    let meanwhile = swift.received_within(Duration::from_millis(300));
    assert!(
        !meanwhile
            .iter()
            .any(|message| is("exitAfterDraining")(message)),
        "{meanwhile:#?}"
    );

    let id = swift.request(2, disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(swift.reply(id), None);
    swift.wait_until("the exit", |swift| {
        swift.position(is("exitAfterDraining")).is_some()
    });
    assert!(swift.exit_facts(1).is_some());
    let executable = std::env::current_exe().unwrap();
    assert!(swift.has_log(&format!(
        "notice helper: executable replaced at {}; exiting so launchd starts the new build",
        executable.display()
    )));
    assert!(!swift.connect_app(3, 501));
}

#[test]
fn a_replaced_helper_that_is_idle_refuses_new_connections_and_exits() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    swift.send(json!({"executableReplaced": {}}));
    assert!(!swift.connect_app(2, 501));
    swift.wait_until("the exit", |swift| {
        swift.position(is("exitAfterDraining")).is_some()
    });
    let replaced = swift
        .logs()
        .iter()
        .filter(|line| line.contains("executable replaced at "))
        .count();
    assert_eq!(replaced, 1);
    // The app that is still connected gets its answers.
    assert_eq!(
        swift.connect(1, ATTEMPT),
        error("busy", "the Helper is shutting down; try again shortly")
    );
}
