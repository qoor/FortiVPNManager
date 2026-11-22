//! App requests and the runs they start, driven through the Helper-internal protocol.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use fvm_helper_core::session::backend::SessionEvent;
use serde_json::{Value, json};

fn up() -> Script {
    common::up(settings("utun9", "10.10.0.5"))
}

fn failed_login() -> Script {
    script(|_, link| {
        trust_gateway(link);
        error_line(link, "Failed to complete authentication");
        ended(-EPERM)
    })
}

#[test]
fn a_run_goes_from_connect_through_the_otp_to_the_tunnel_and_back() {
    let received = Arc::new(Mutex::new(None));
    let script = {
        let received = Arc::clone(&received);
        script(move |request, link| {
            *received.lock().unwrap() = Some(request.password.to_string());
            trust_gateway(link);
            let otp = ask_otp(link).unwrap();
            assert_eq!(otp.as_str(), OTP);
            open(link);
            tunnel_ready(link, settings("utun9", "10.10.0.5"));
            wait_for_cancel(link);
            ended(-EINTR)
        })
    };
    let mut os_side = OsSide::start(Setup::with([script]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.wait_for_milestone(1, OTP_PROMPTED);
    let id = os_side.request(1, &submit_otp(ATTEMPT, OTP));
    assert_eq!(os_side.reply(id), None);
    os_side.bring_up(1);
    os_side.wait_until("the tunnel log", |os_side| {
        os_side.has_log(&format!(
            "notice helper: 0277070470 run {ATTEMPT}: tunnel up"
        ))
    });
    assert_eq!(received.lock().unwrap().as_deref(), Some(PASSWORD));

    // The run record lives in the run directory for the run, readable only by its owner.
    assert_eq!(os_side.scratch.run_directory_names(), ["run.json"]);
    let file = os_side.scratch.run_directory().join("run.json");
    let mode = fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let record: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        record,
        json!({"outsideChanges": {"dnsServiceID": "svc", "routes": outside_routes()}})
    );
    let status = os_side.request(1, &json!({"status": {}}));
    os_side.answer_network("check", &json!({"checked": {"complete": true}}));
    assert_eq!(
        os_side.reply_json(status),
        json!({
            "sessionRunning": true,
            "backend": {"version": VERSION},
            "routingComplete": true,
        })
    );

    let id = os_side.request(1, &disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(os_side.reply(id), None);
    os_side.answer_network("revert", &reverted());
    let facts = os_side.wait_for_exit(1);
    assert_eq!(facts["furthestMilestone"], TUNNEL_UP);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "userDisconnect"}})
    );
    assert_eq!(facts["endCode"], -EINTR);

    assert_eq!(os_side.milestones(1), [0, 1, 2, 3, 4, 5]);
    let tunnel_up = os_side.events(1)[5].clone();
    assert_eq!(tunnel_up["attemptID"], ATTEMPT);
    assert_eq!(tunnel_up["kind"]["milestone"]["tunnelIP"], "10.10.0.5");
    assert_eq!(os_side.output(1)[0], "‹VPN client: openconnect 9.21›");
    assert!(
        os_side
            .output(1)
            .contains(&"‹session cancelled (reason: userDisconnect)›".to_owned())
    );
    os_side.wait_until("the exit log", |os_side| {
        os_side.logs().iter().any(|line| line.contains(" exited: "))
    });
    assert_eq!(os_side.scratch.run_directory_names(), Vec::<String>::new());
    assert_eq!(os_side.secrets.deletes(), [ATTEMPT]);
    for line in [
        format!(
            "notice helper: 0277070470 run {ATTEMPT} started for uid 501 with openconnect 9.21"
        ),
        format!("notice helper: 0277070470 run {ATTEMPT}: tunnel up"),
        format!("notice helper: 0277070470 run {ATTEMPT}: stop requested (userDisconnect)"),
        format!(
            "notice helper: 0277070470 run {ATTEMPT} exited: code -4, initiator \
             Helper {{ reason: UserDisconnect }}, furthest TunnelUp"
        ),
    ] {
        assert!(
            os_side.has_log(&line),
            "{line} not in {:#?}",
            os_side.logs()
        );
    }
    // The exit is the run's last message to the app, and its revert line is the last Log tab line.
    let exit = os_side.position(|message| {
        message["sendEvent"]["json"]
            .as_str()
            .is_some_and(|json| json.contains("exited"))
    });
    let last_to_app = os_side.received.iter().rposition(|message| {
        message
            .get("sendEvent")
            .or(message.get("sendOutput"))
            .is_some()
    });
    assert_eq!(exit, last_to_app);
    assert_eq!(
        os_side.output(1).last().unwrap(),
        "‹network settings reverted›"
    );
    os_side.assert_no_secret();
}

#[test]
fn a_failed_login_is_reported_as_the_session_ending_the_run() {
    let mut os_side = OsSide::start(Setup::with([failed_login()]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    let facts = os_side.wait_for_exit(1);
    assert_eq!(facts["furthestMilestone"], CONNECTED_TO_GATEWAY);
    assert_eq!(facts["stopInitiator"], json!({"none": {}}));
    assert_eq!(facts["endCode"], -EPERM);
    os_side.wait_until("the exit log", |os_side| {
        os_side.has_log(&format!(
            "notice helper: 0277070470 run {ATTEMPT} exited: code -1, initiator Session, furthest \
             ConnectedToGateway"
        ))
    });
    assert_eq!(os_side.scratch.run_directory_names(), Vec::<String>::new());
    assert_eq!(os_side.secrets.saves(), []);
    os_side.assert_no_secret();
}

#[test]
fn every_output_batch_names_its_attempt() {
    let mut os_side = OsSide::start(Setup::with([failed_login()]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.wait_for_exit(1);
    let attempts: Vec<Value> = os_side
        .received
        .iter()
        .filter_map(|message| message.get("sendOutput"))
        .map(|output| serde_json::from_str::<Value>(output["json"].as_str().unwrap()).unwrap())
        .map(|batch| batch["attemptID"].clone())
        .collect();
    assert_ne!(attempts, Vec::<Value>::new());
    assert!(
        attempts.iter().all(|attempt| attempt == ATTEMPT),
        "{attempts:?}"
    );
}

#[test]
fn a_rejected_certificate_is_reported_with_its_value() {
    let script = script(|_, link| {
        link.report(SessionEvent::CertificateRejected(OTHER_PIN.to_owned()));
        ended(-EIO)
    });
    let mut os_side = OsSide::start(Setup::with([script]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    let facts = os_side.wait_for_exit(1);
    assert_eq!(facts["certRejectedDigests"], json!([OTHER_PIN]));
    assert_eq!(facts["certRejectedAt"], STARTED);
    assert_eq!(facts["furthestMilestone"], STARTED);
    assert_eq!(facts["stopInitiator"], json!({"none": {}}));
}

#[test]
fn a_second_run_is_refused_and_only_the_owner_controls_the_run() {
    let mut os_side = OsSide::start(Setup::with([up()]));
    assert!(os_side.connect_app(1, 501));
    assert!(os_side.connect_app(2, 501));
    assert!(os_side.connect_app(3, 502));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.wait_for_milestone(1, OTP_PROMPTED);

    assert_eq!(
        os_side.connect(2, OTHER_ATTEMPT),
        error("busy", "openconnect is already running")
    );
    assert_eq!(
        os_side.connect(3, OTHER_ATTEMPT),
        error(
            "notAuthorized",
            "a VPN run started by another user is active"
        )
    );
    let refused = [
        (
            3,
            submit_otp(ATTEMPT, OTP),
            error("notAuthorized", "the VPN run belongs to another user"),
        ),
        (
            3,
            disconnect(ATTEMPT, "userDisconnect"),
            error("notAuthorized", "the VPN run belongs to another user"),
        ),
        (
            1,
            disconnect(OTHER_ATTEMPT, "userDisconnect"),
            error("unknownAttempt", "not the active connection attempt"),
        ),
        (
            1,
            submit_otp("not a UUID", OTP),
            error("unknownAttempt", "not the active connection attempt"),
        ),
        (
            1,
            submit_otp(ATTEMPT, "12345"),
            error("invalidRequest", "OTP must be 6 to 8 ASCII digits"),
        ),
    ];
    for (app, request, expected) in refused {
        let id = os_side.request(app, &request);
        assert_eq!(os_side.reply(id), expected);
    }

    // Any connection of the owner's user may control the run; attempt IDs ignore case.
    let id = os_side.request(2, &submit_otp(&ATTEMPT.to_lowercase(), OTP));
    assert_eq!(os_side.reply(id), None);
    let id = os_side.request(1, &submit_otp(ATTEMPT, OTP));
    assert_eq!(
        os_side.reply(id),
        error(
            "wrongPhase",
            "an OTP was already submitted for this attempt"
        )
    );
    os_side.bring_up(1);
    // A stop names the run it is for.
    let id = os_side.request(2, &disconnect("", "userDisconnect"));
    assert_eq!(
        os_side.reply(id),
        error("unknownAttempt", "not the active connection attempt")
    );
    let id = os_side.request(2, &disconnect(ATTEMPT, "notAReason"));
    assert_eq!(os_side.reply(id), None);
    os_side.answer_network("revert", &reverted());
    let facts = os_side.wait_for_exit(1);
    // An unknown reason counts as the user's.
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "userDisconnect"}})
    );
    assert_eq!(os_side.events(2), Vec::<Value>::new());

    // With no run, every attempt is unknown.
    let id = os_side.request(1, &disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(
        os_side.reply(id),
        error("unknownAttempt", "not the active connection attempt")
    );
    let id = os_side.request(1, &submit_otp("", OTP));
    assert_eq!(
        os_side.reply(id),
        error("unknownAttempt", "not the active connection attempt")
    );
}

#[test]
fn a_bad_connect_request_starts_nothing() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    let undecodable = os_side.request(1, &json!({"connect": {"json": "{\"host\": 1}"}}));
    let invalid = os_side.request(
        1,
        &connect_request_with(&json!({
            "attemptID": ATTEMPT,
            "sessionID": 277_070_470,
            "host": GATEWAY,
            "port": 0,
            "username": "alice",
            "trustedCerts": [],
            "dtls": false,
            "password": PASSWORD,
        })),
    );
    assert_eq!(
        os_side.reply(undecodable),
        error("invalidRequest", "could not decode the connect request")
    );
    assert_eq!(
        os_side.reply(invalid),
        error("invalidRequest", "port 0 is refused")
    );
    assert_eq!(os_side.status(1)["sessionRunning"], false);
    assert_eq!(os_side.events(1), Vec::<Value>::new());
    assert_eq!(os_side.scratch.run_directory_names(), Vec::<String>::new());
    os_side.assert_no_secret();
}

#[test]
fn the_status_carries_the_backends_version() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    assert_eq!(
        os_side.status(1),
        json!({
            "sessionRunning": false,
            "backend": {"version": VERSION},
        })
    );
}

#[test]
fn the_status_of_an_up_run_waits_for_its_routing_check() {
    let mut os_side = OsSide::start(Setup::with([up_without_otp(settings(
        "utun9",
        "10.10.0.5",
    ))]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.bring_up(1);
    let outcomes = [
        (json!({"checked": {"complete": true}}), Some(true)),
        (json!({"checked": {"complete": false}}), Some(false)),
        (
            json!({"failed": {"step": "routes", "error": "the routing table could not be read", "added": [], "left": []}}),
            Some(false),
        ),
    ];
    for (outcome, routing_complete) in outcomes {
        let status = os_side.request(1, &json!({"status": {}}));
        let late = os_side.received_within(Duration::from_millis(200));
        assert!(
            !late.iter().any(|message| message["reply"]["id"] == status),
            "{late:#?}"
        );
        let check = os_side.answer_network("check", &outcome);
        assert_eq!(check["settings"]["tunnel"]["name"], "utun9");
        assert_eq!(
            check["outside"],
            json!([
                {"address": GATEWAY, "prefixLength": 32},
                {"address": "10.20.161.16", "prefixLength": 32},
            ])
        );
        let reply = os_side.reply_json(status);
        assert_eq!(reply["sessionRunning"], true);
        assert_eq!(
            reply.get("routingComplete").and_then(Value::as_bool),
            routing_complete,
            "{reply}"
        );
    }
}

#[test]
fn the_status_of_a_run_that_is_not_up_needs_no_routing_check() {
    let mut os_side = OsSide::start(Setup::with([up()]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.wait_for_milestone(1, OTP_PROMPTED);
    let reply = os_side.status(1);
    assert_eq!(reply["sessionRunning"], true);
    assert!(reply.get("routingComplete").is_none(), "{reply}");
    assert_eq!(os_side.network_changes(), Vec::<Value>::new());
}

#[test]
fn the_owner_going_away_stops_its_run_and_its_queued_requests_are_answered() {
    let mut os_side = OsSide::start(Setup::with([up()]));
    assert!(os_side.connect_app(1, 501));
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.wait_for_milestone(1, OTP_PROMPTED);
    let status = os_side.request(1, &json!({"status": {}}));
    os_side.disconnect_app(1);

    assert_eq!(os_side.reply_json(status)["sessionRunning"], true);
    let facts = os_side.wait_for_exit(1);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "clientInvalidated"}})
    );
    assert!(os_side.has_log(&format!(
        "notice helper: 0277070470 run {ATTEMPT}: owning connection invalidated"
    )));
    os_side.wait_until("the exit log", |os_side| {
        os_side.logs().iter().any(|line| line.contains(" exited: "))
    });
    assert_eq!(os_side.scratch.run_directory_names(), Vec::<String>::new());
}

#[test]
fn every_request_of_a_gone_app_gets_exactly_one_reply_and_starts_no_run() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    os_side.disconnect_app(1);
    let gone = error("notAuthorized", "the app connection is closed");
    let connect = os_side.request(1, &connect_request(ATTEMPT));
    let otp = os_side.request(1, &submit_otp(ATTEMPT, OTP));
    let stop = os_side.request(1, &disconnect(ATTEMPT, "userDisconnect"));
    let status = os_side.request(1, &json!({"status": {}}));
    assert_eq!(os_side.reply(connect), gone);
    assert_eq!(os_side.reply(otp), gone);
    assert_eq!(os_side.reply(stop), gone);
    assert_eq!(os_side.reply_json(status)["sessionRunning"], false);

    let late = os_side.received_within(Duration::from_millis(300));
    assert!(late.is_empty(), "{late:#?}");
    for id in [connect, otp, stop, status] {
        assert_eq!(os_side.replies(id).len(), 1, "request {id}");
    }
    assert_eq!(os_side.events(1), Vec::<Value>::new());
    assert_eq!(os_side.scratch.run_directory_names(), Vec::<String>::new());
}

#[test]
fn replies_carry_the_request_id_they_answer() {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    let ids: Vec<u64> = (0..5)
        .map(|_| os_side.request(1, &json!({"status": {}})))
        .collect();
    os_side.wait_until("all replies", |os_side| {
        ids.iter().all(|&id| !os_side.replies(id).is_empty())
    });
    let answered: Vec<Value> = os_side
        .received
        .iter()
        .filter_map(|message| message["reply"].get("id").cloned())
        .collect();
    assert_eq!(
        answered,
        ids.iter().map(|&id| json!(id)).collect::<Vec<_>>()
    );
}
