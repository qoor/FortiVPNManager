//! When the Helper exits: on the app's request, on SIGTERM and after its executable was replaced,
//! and which app connections it admits meanwhile.

mod common;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::*;
use serde_json::json;

fn up() -> Script {
    common::up(settings("utun9", "10.10.0.5"))
}

const KEPT_OPEN: &str =
    "notice helper: an app that asked the Helper to exit kept its connection open; exiting anyway";

const WELL_BEFORE_THE_LIMIT: Duration = Duration::from_millis(650);

#[test]
fn exit_when_idle_exits_once_the_asking_app_disconnects_and_admits_no_one_meanwhile() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    let asked = Instant::now();
    let id = os_side.request(1, &json!({"exitWhenIdle": {}}));
    assert!(os_side.reply_flag(id));
    assert!(
        os_side.has_log("notice helper: exiting on request so launchd starts the current binary")
    );

    assert!(!os_side.connect_app(2, 501));
    assert_eq!(
        os_side.connect(1, ATTEMPT),
        Some(error(
            "busy",
            "the Helper is shutting down; try again shortly"
        ))
    );
    let again = os_side.request(1, &json!({"exitWhenIdle": {}}));
    assert!(os_side.reply_flag(again));
    os_side
        .received_within((asked + WELL_BEFORE_THE_LIMIT).saturating_duration_since(Instant::now()));
    assert_eq!(os_side.count(is("exitAfterDraining")), 0);

    os_side.disconnect_app(1);
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    assert!(!os_side.has_log(KEPT_OPEN));
    assert_eq!(os_side.count(is("exitAfterDraining")), 1);
    assert_eq!(os_side.count(is("exitNow")), 0);
    assert_eq!(os_side.events(1).len(), 0);
}

#[test]
fn exit_when_idle_exits_at_the_limit_when_the_asking_app_keeps_its_connection() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    let asked = Instant::now();
    let id = os_side.request(1, &json!({"exitWhenIdle": {}}));
    assert!(os_side.reply_flag(id));
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    let waited = asked.elapsed();
    assert!(
        (Duration::from_secs(1)..Duration::from_secs(3)).contains(&waited),
        "{waited:?}"
    );
    assert!(os_side.has_log(KEPT_OPEN));
    assert_eq!(os_side.count(is("exitAfterDraining")), 1);
    assert_eq!(os_side.count(is("exitNow")), 0);
}

#[test]
fn exit_when_idle_counts_the_limit_from_the_first_request() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    assert!(os_side.connect_app(2, 502));
    let asked = Instant::now();
    let first = os_side.request(1, &json!({"exitWhenIdle": {}}));
    assert!(os_side.reply_flag(first));
    os_side.received_within(
        (asked + Duration::from_millis(600)).saturating_duration_since(Instant::now()),
    );
    let second = os_side.request(2, &json!({"exitWhenIdle": {}}));
    assert!(os_side.reply_flag(second));

    os_side.disconnect_app(1);
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    let waited = asked.elapsed();
    assert!(
        (Duration::from_secs(1)..Duration::from_millis(1500)).contains(&waited),
        "{waited:?}"
    );
    assert!(os_side.has_log(KEPT_OPEN));
}

#[test]
fn exit_when_idle_waits_for_every_app_that_asked() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    assert!(os_side.connect_app(2, 502));
    let asked = Instant::now();
    let first = os_side.request(1, &json!({"exitWhenIdle": {}}));
    assert!(os_side.reply_flag(first));
    let second = os_side.request(2, &json!({"exitWhenIdle": {}}));
    assert!(os_side.reply_flag(second));

    os_side.disconnect_app(1);
    os_side
        .received_within((asked + WELL_BEFORE_THE_LIMIT).saturating_duration_since(Instant::now()));
    assert_eq!(os_side.count(is("exitAfterDraining")), 0);
    os_side.disconnect_app(2);
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    assert!(!os_side.has_log(KEPT_OPEN));
}

#[test]
fn exit_when_idle_does_not_wait_for_apps_that_did_not_ask() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    assert!(os_side.connect_app(2, 502));
    assert!(os_side.connect_app(3, 503));
    let asked = Instant::now();
    let id = os_side.request(1, &json!({"exitWhenIdle": {}}));
    assert!(os_side.reply_flag(id));

    os_side.disconnect_app(3);
    os_side
        .received_within((asked + WELL_BEFORE_THE_LIMIT).saturating_duration_since(Instant::now()));
    assert_eq!(os_side.count(is("exitAfterDraining")), 0);
    os_side.disconnect_app(1);
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    assert!(!os_side.has_log(KEPT_OPEN));
}

#[test]
fn exit_when_idle_from_a_closed_connection_exits_right_after_the_reply() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    os_side.disconnect_app(1);
    let id = os_side.request(1, &json!({"exitWhenIdle": {}}));
    assert!(os_side.reply_flag(id));
    let replied = Instant::now();
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    assert!(replied.elapsed() < Duration::from_millis(400));
    let reply = os_side
        .position(|message| message["replyFlag"]["id"] == id)
        .unwrap();
    assert!(reply < os_side.position(is("exitAfterDraining")).unwrap());
    assert!(!os_side.has_log(KEPT_OPEN));
}

#[test]
fn exit_when_idle_is_declined_during_a_run() {
    let mut os_side = OsSide::start(Setup::with([up()]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.wait_for_milestone(1, OTP_PROMPTED);
    let id = os_side.request(1, &json!({"exitWhenIdle": {}}));
    assert!(!os_side.reply_flag(id));

    let id = os_side.request(1, &disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(os_side.reply(id), None);
    os_side.wait_for_exit(1);
    let later = os_side.received_within(Duration::from_millis(800));
    assert!(
        !later
            .iter()
            .any(|message| is("exitAfterDraining")(message) || is("exitNow")(message)),
        "{later:#?}"
    );
    assert!(os_side.connect_app(2, 501));
}

#[test]
fn sigterm_while_idle_exits_at_once() {
    let mut os_side = OsSide::start(Setup::default());
    os_side.send(json!({"terminationRequested": {}}));
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    assert!(!os_side.connect_app(1, 501));
}

#[test]
fn sigterm_during_a_run_stops_it_and_exits_once_its_exit_is_reported() {
    let mut os_side = OsSide::start(Setup::with([up()]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.wait_for_milestone(1, OTP_PROMPTED);
    let id = os_side.request(1, &submit_otp(ATTEMPT, OTP));
    assert_eq!(os_side.reply(id), None);
    os_side.bring_up(1);

    os_side.send(json!({"terminationRequested": {}}));
    os_side.send(json!({"terminationRequested": {}}));
    assert!(!os_side.connect_app(2, 501));
    assert_eq!(
        os_side.connect(1, OTHER_ATTEMPT),
        Some(error(
            "busy",
            "the Helper is shutting down; try again shortly"
        ))
    );
    os_side.answer_network("revert", &reverted());
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    let facts = os_side
        .end_facts(1)
        .expect("the run's exit before the Helper's");
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "helperShutdown"}})
    );
    let exit_event = os_side
        .position(|message| {
            message["sendEvent"]["json"]
                .as_str()
                .is_some_and(|json| json.contains("exited"))
        })
        .unwrap();
    assert!(exit_event < os_side.position(is("exitAfterDraining")).unwrap());

    let later = os_side.received_within(Duration::from_millis(300));
    assert!(later.is_empty(), "{later:#?}");
    assert_eq!(os_side.count(is("exitAfterDraining")), 1);
    assert_eq!(os_side.count(is("exitNow")), 0);
    assert_eq!(os_side.scratch.run_directory_names(), Vec::<String>::new());
}

#[test]
fn sigterm_exits_anyway_when_the_run_does_not_end_in_time() {
    let (release, released) = mpsc::channel::<()>();
    let stuck = script(move |_, link| {
        trust_gateway(link);
        let _ = released.recv();
        ended(-EIO)
    });
    let mut os_side = OsSide::start(Setup::with([stuck]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.wait_for_milestone(1, CONNECTED_TO_GATEWAY);
    os_side.send(json!({"terminationRequested": {}}));
    let sent = Instant::now();
    os_side.wait_until_within(Duration::from_secs(20), "the forced exit", |os_side| {
        os_side.position(is("exitNow")).is_some()
    });
    let waited = sent.elapsed();
    assert!(
        (Duration::from_secs(14)..Duration::from_secs(17)).contains(&waited),
        "{waited:?}"
    );
    assert!(os_side.has_log(
        "error helper: the run did not finish after the termination request; exiting anyway"
    ));
    assert_eq!(os_side.end_facts(1), None);
    assert_eq!(os_side.count(is("exitAfterDraining")), 0);

    release.send(()).unwrap();
    os_side.wait_for_exit(1);
}

#[test]
fn a_replaced_helper_finishes_its_run_and_then_exits() {
    let mut os_side = OsSide::start(Setup::with([up()]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.wait_for_milestone(1, OTP_PROMPTED);
    os_side.send(json!({"executableReplaced": {}}));
    // A connection that comes during the run is still admitted.
    assert!(os_side.connect_app(2, 501));
    let meanwhile = os_side.received_within(Duration::from_millis(300));
    assert!(
        !meanwhile
            .iter()
            .any(|message| is("exitAfterDraining")(message)),
        "{meanwhile:#?}"
    );

    let id = os_side.request(2, &disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(os_side.reply(id), None);
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    assert!(os_side.end_facts(1).is_some());
    let executable = std::env::current_exe().unwrap();
    assert!(os_side.has_log(&format!(
        "notice helper: executable replaced at {}; exiting so launchd starts the new build",
        executable.display()
    )));
    assert!(!os_side.connect_app(3, 501));
}

#[test]
fn a_replaced_helper_that_is_idle_refuses_new_connections_and_exits() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    os_side.send(json!({"executableReplaced": {}}));
    assert!(!os_side.connect_app(2, 501));
    os_side.wait_until("the exit", |os_side| {
        os_side.position(is("exitAfterDraining")).is_some()
    });
    let replaced = os_side
        .logs()
        .iter()
        .filter(|line| line.contains("executable replaced at "))
        .count();
    assert_eq!(replaced, 1);
    // The app that is still connected gets its answers.
    assert_eq!(
        os_side.connect(1, ATTEMPT),
        Some(error(
            "busy",
            "the Helper is shutting down; try again shortly"
        ))
    );
}
