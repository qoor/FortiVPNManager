//! App requests and the runs they start, driven through the Helper-internal protocol.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use common::*;
use fvm_contract::backend::{ClientProblem, ClientProblemKind};
use serde_json::{Value, json};

#[test]
fn a_run_goes_from_connect_through_the_otp_to_the_tunnel_and_back() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    swift.wait_for_milestone(1, OTP_PROMPTED);
    let id = swift.request(1, submit_otp(ATTEMPT, OTP));
    assert_eq!(swift.reply(id), None);
    // Logged once the pidfile is written.
    swift.wait_until("the tunnel", |swift| {
        swift.has_log(&format!("notice helper: run {ATTEMPT}: tunnel up"))
    });

    // The config file and the pidfile live in the run directory for the run, readable only by
    // their owner. The test's client is not root, so the pidfile records no process.
    assert_eq!(
        swift.scratch.run_directory_names(),
        ["config.1", "run.json"]
    );
    let file = |name: &str| swift.scratch.run_directory().join(name);
    for name in ["config.1", "run.json"] {
        let mode = fs::metadata(file(name)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{name}");
    }
    let config = fs::read_to_string(file("config.1")).unwrap();
    assert!(config.contains(&format!("host = {GATEWAY}\n")), "{config}");
    assert!(!config.contains(PASSWORD.trim()), "{config}");
    let record: Value = serde_json::from_slice(&fs::read(file("run.json")).unwrap()).unwrap();
    assert_eq!(record, json!({"processes": [], "gatewayIP": GATEWAY}));
    assert_eq!(
        swift.status(1),
        json!({"childAlive": true, "openfortivpnPath": "/bin/sh", "openfortivpnVersion": "1.24.1"})
    );

    let id = swift.request(1, disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(swift.reply(id), None);
    let facts = swift.wait_for_exit(1);
    assert_eq!(facts["furthestMilestone"], TUNNEL_UP);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "userDisconnect"}})
    );
    assert_eq!(facts["exitStatus"], json!({"exited": {"code": 0}}));
    assert!(facts["upDuration"].is_number(), "{facts}");

    assert_eq!(swift.milestones(1), [0, 1, 2, 3, 4, 5]);
    let tunnel_up = swift.events(1)[5].clone();
    assert_eq!(tunnel_up["attemptID"], ATTEMPT);
    assert_eq!(tunnel_up["kind"]["milestone"]["tunnelIP"], "10.212.134.200");
    assert!(swift.output(1).contains(&"‹password sent›".to_owned()));
    assert!(
        swift
            .output(1)
            .contains(&"‹sent SIGTERM (reason: userDisconnect)›".to_owned())
    );
    assert_eq!(
        swift.scratch.record_value("password_sha256"),
        Some(sha256(PASSWORD))
    );
    assert_eq!(swift.scratch.record_value("otp_sha256"), Some(sha256(OTP)));
    // Logged once the run's files are removed.
    swift.wait_until("the exit log", |swift| {
        swift.logs().iter().any(|line| line.contains(" exited: "))
    });
    assert!(swift.scratch.run_directory_names().is_empty());
    for line in [
        format!("notice helper: run {ATTEMPT} started for uid 501 with openfortivpn 1.24.1"),
        format!("notice helper: run {ATTEMPT}: tunnel up"),
        format!("notice helper: run {ATTEMPT}: stop requested (userDisconnect)"),
        format!(
            "notice helper: run {ATTEMPT} exited: exited(code: 0), initiator \
             helper(FortiShared.StopReason.userDisconnect), furthest \
             Optional(FortiShared.Milestone.tunnelUp)"
        ),
    ] {
        assert!(swift.has_log(&line), "{line} not in {:#?}", swift.logs());
    }
    // The exit is the run's last message to the app, and its exit line is the last Log tab line.
    let exit = swift.position(|message| {
        message["sendEvent"]["json"]
            .as_str()
            .is_some_and(|json| json.contains("exited"))
    });
    let last_to_app = swift.received.iter().rposition(|message| {
        message
            .get("sendEvent")
            .or(message.get("sendOutput"))
            .is_some()
    });
    assert_eq!(exit, last_to_app);
    assert_eq!(swift.output(1).last().unwrap(), "‹exited: code 0›");
    swift.assert_no_secret();
}

#[test]
fn a_failed_login_is_reported_as_the_client_ending_the_run() {
    let mut swift = Swift::start(Setup::fixture("auth-failed"));
    assert!(swift.connect_app(1, 501));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    let facts = swift.wait_for_exit(1);
    assert_eq!(facts["furthestMilestone"], CONNECTED_TO_GATEWAY);
    assert_eq!(facts["stopInitiator"], json!({"none": {}}));
    assert_eq!(facts["exitStatus"], json!({"exited": {"code": 1}}));
    swift.wait_until("the exit log", |swift| {
        swift.has_log(&format!(
            "notice helper: run {ATTEMPT} exited: exited(code: 1), initiator none, furthest \
             Optional(FortiShared.Milestone.connectedToGateway)"
        ))
    });
    assert!(swift.scratch.run_directory_names().is_empty());
    swift.assert_no_secret();
}

#[test]
fn a_rejected_certificate_is_reported_with_its_value() {
    let mut swift = Swift::start(Setup::fixture("cert-rejected"));
    assert!(swift.connect_app(1, 501));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    let facts = swift.wait_for_exit(1);
    assert_eq!(facts["certRejectedDigests"], json!([REJECTED_DIGEST]));
    assert_eq!(facts["certRejectedAt"], SPAWNED);
    assert_eq!(facts["furthestMilestone"], SPAWNED);
    assert_eq!(facts["stopInitiator"], json!({"none": {}}));
}

#[test]
fn a_run_whose_client_left_the_routes_broken_waits_for_the_route_repair() {
    let mut swift = Swift::start(Setup::fixture("server-cut").args(&["crash"]));
    assert!(swift.connect_app(1, 501));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    swift.wait_for_milestone(1, OTP_PROMPTED);
    let id = swift.request(1, submit_otp(ATTEMPT, OTP));
    assert_eq!(swift.reply(id), None);
    swift.wait_until("the route repair", |swift| {
        swift.position(is("repairRoutes")).is_some()
    });
    let repair = swift.position(is("repairRoutes")).unwrap();
    assert_eq!(
        swift.received[repair],
        json!({"repairRoutes": {"gateway": GATEWAY}})
    );
    assert_eq!(swift.exit_facts(1), None);
    let meanwhile = swift.received_within(Duration::from_millis(300));
    assert!(meanwhile.is_empty(), "{meanwhile:#?}");

    swift.send(json!({"routesRepaired": {"outcome": "repaired"}}));
    let facts = swift.wait_for_exit(1);
    assert_eq!(facts["exitStatus"], json!({"signaled": {"signal": 9}}));
    assert!(swift.output(1).contains(&"‹routes repaired›".to_owned()));
}

#[test]
fn a_second_run_is_refused_and_only_the_owner_controls_the_run() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    assert!(swift.connect_app(2, 501));
    assert!(swift.connect_app(3, 502));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    swift.wait_for_milestone(1, OTP_PROMPTED);

    assert_eq!(
        swift.connect(2, OTHER_ATTEMPT),
        error("busy", "openfortivpn is already running")
    );
    assert_eq!(
        swift.connect(3, OTHER_ATTEMPT),
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
            disconnect("", "userDisconnect"),
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
        let id = swift.request(app, request);
        assert_eq!(swift.reply(id), expected);
    }

    // Any connection of the owner's user may control the run; attempt IDs ignore case.
    let id = swift.request(2, submit_otp(&ATTEMPT.to_lowercase(), OTP));
    assert_eq!(swift.reply(id), None);
    let id = swift.request(1, submit_otp(ATTEMPT, OTP));
    assert_eq!(
        swift.reply(id),
        error(
            "wrongPhase",
            "an OTP was already submitted for this attempt"
        )
    );
    swift.wait_for_milestone(1, TUNNEL_UP);
    let id = swift.request(2, disconnect("", "notAReason"));
    assert_eq!(swift.reply(id), None);
    let facts = swift.wait_for_exit(1);
    // An unknown reason counts as the user's.
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "userDisconnect"}})
    );
    assert!(swift.events(2).is_empty());

    // With no run, an empty attempt is fine and a named one is unknown.
    let id = swift.request(1, disconnect("", "userDisconnect"));
    assert_eq!(swift.reply(id), None);
    let id = swift.request(1, disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(
        swift.reply(id),
        error("unknownAttempt", "not the active connection attempt")
    );
    let id = swift.request(1, submit_otp("", OTP));
    assert_eq!(
        swift.reply(id),
        error("unknownAttempt", "not the active connection attempt")
    );
}

#[test]
fn a_bad_connect_request_starts_nothing() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    let undecodable = swift.request(1, json!({"connect": {"json": "{\"host\": 1}"}}));
    let invalid = swift.request(
        1,
        connect_request_with(json!({
            "attemptID": ATTEMPT,
            "host": GATEWAY,
            "port": 0,
            "username": "alice",
            "trustedCerts": [],
            "password": PASSWORD,
        })),
    );
    assert_eq!(
        swift.reply(undecodable),
        error("invalidRequest", "could not decode the connect request")
    );
    assert_eq!(
        swift.reply(invalid),
        error("invalidRequest", "port must be between 1 and 65535")
    );
    assert_eq!(swift.status(1)["childAlive"], false);
    assert!(swift.events(1).is_empty());
    assert!(swift.scratch.run_directory_names().is_empty());
    swift.assert_no_secret();
}

#[test]
fn a_client_that_is_not_found_has_no_path_in_the_status() {
    let not_found = ClientProblem {
        kind: ClientProblemKind::NotFound,
        path: PathBuf::from("/opt/homebrew/bin/openfortivpn"),
    };
    let message = "openfortivpn not found at /opt/homebrew/bin/openfortivpn; install it with: brew install openfortivpn";
    let mut swift = Swift::start(Setup {
        client: Err(not_found),
        ..Setup::fixture("success")
    });
    assert!(swift.connect_app(1, 501));
    assert_eq!(
        swift.status(1),
        json!({"childAlive": false, "binaryProblem": message})
    );
    assert_eq!(
        swift.connect(1, ATTEMPT),
        error("binaryUnavailable", message)
    );
}

#[test]
fn a_client_with_another_problem_keeps_its_path_in_the_status() {
    let path = "/opt/homebrew/Cellar/openfortivpn/1.24.1/bin/openfortivpn";
    let mut swift = Swift::start(Setup {
        client: Err(ClientProblem {
            kind: ClientProblemKind::NotExecutable,
            path: PathBuf::from(path),
        }),
        ..Setup::fixture("success")
    });
    assert!(swift.connect_app(1, 501));
    assert_eq!(
        swift.status(1),
        json!({
            "childAlive": false,
            "openfortivpnPath": path,
            "binaryProblem": format!("openfortivpn is not executable: {path}"),
        })
    );
}

#[test]
fn the_owner_going_away_stops_its_run_and_its_queued_requests_are_answered() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    assert_eq!(swift.connect(1, ATTEMPT), None);
    swift.wait_for_milestone(1, OTP_PROMPTED);
    let status = swift.request(1, json!({"status": {}}));
    swift.disconnect_app(1);

    assert_eq!(swift.reply_json(status)["childAlive"], true);
    let facts = swift.wait_for_exit(1);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "clientInvalidated"}})
    );
    assert!(swift.has_log(&format!(
        "notice helper: run {ATTEMPT}: owning connection invalidated"
    )));
    swift.wait_until("the exit log", |swift| {
        swift.logs().iter().any(|line| line.contains(" exited: "))
    });
    assert!(swift.scratch.run_directory_names().is_empty());
}

#[test]
fn every_request_of_a_gone_app_gets_exactly_one_reply_and_starts_no_run() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    swift.disconnect_app(1);
    let gone = error("notAuthorized", "the app connection is closed");
    let connect = swift.request(1, connect_request(ATTEMPT));
    let otp = swift.request(1, submit_otp(ATTEMPT, OTP));
    let stop = swift.request(1, disconnect("", "userDisconnect"));
    let status = swift.request(1, json!({"status": {}}));
    assert_eq!(swift.reply(connect), gone);
    assert_eq!(swift.reply(otp), gone);
    assert_eq!(swift.reply(stop), gone);
    assert_eq!(swift.reply_json(status)["childAlive"], false);

    let late = swift.received_within(Duration::from_millis(300));
    assert!(late.is_empty(), "{late:#?}");
    for id in [connect, otp, stop, status] {
        assert_eq!(swift.replies(id).len(), 1, "request {id}");
    }
    assert!(swift.events(1).is_empty());
    assert_eq!(swift.scratch.record_value("password_len"), None);
    assert!(swift.scratch.run_directory_names().is_empty());
}

#[test]
fn replies_carry_the_request_id_they_answer() {
    let mut swift = Swift::start(Setup::fixture("success"));
    assert!(swift.connect_app(1, 501));
    let ids: Vec<u64> = (0..5)
        .map(|_| swift.request(1, json!({"status": {}})))
        .collect();
    swift.wait_until("all replies", |swift| {
        ids.iter().all(|&id| !swift.replies(id).is_empty())
    });
    let answered: Vec<Value> = swift
        .received
        .iter()
        .filter_map(|message| message["reply"].get("id").cloned())
        .collect();
    assert_eq!(
        answered,
        ids.iter().map(|&id| json!(id)).collect::<Vec<_>>()
    );
}
