mod common;

use common::*;
use serde_json::{Value, json};

const NEW: &str = OTHER_ATTEMPT;
const THIRD: &str = "3C5E7A90-1B2D-4F6A-8C0E-2D4F6A8C0E1B";

fn start(new_address: &str) -> OsSide {
    OsSide::start(Setup::with([
        up_without_otp(settings("utun9", "10.10.0.5")),
        up_without_otp(settings("utun10", new_address)),
    ]))
}

fn connect(attempt: &str, replacing: Option<&str>) -> Value {
    let session = match attempt {
        ATTEMPT => 277_070_470,
        NEW => 277_070_471,
        _ => 277_070_472,
    };
    let mut request = json!({
        "attemptID": attempt,
        "sessionID": session,
        "host": GATEWAY,
        "port": 443,
        "username": "alice",
        "trustedCerts": [PIN],
        "dtls": false,
        "password": "secret",
    });
    if let Some(replacing) = replacing {
        request["replacingAttemptID"] = json!(replacing);
    }
    connect_request_with(request)
}

fn events_of(os_side: &OsSide, attempt: &str) -> Vec<(usize, Value)> {
    os_side
        .received
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            let json = message.get("sendEvent")?["json"].as_str()?;
            let event: Value = serde_json::from_str(json).unwrap();
            (event["attemptID"] == attempt).then(|| (index, event["kind"].clone()))
        })
        .collect()
}

fn milestones_of(os_side: &OsSide, attempt: &str) -> Vec<u64> {
    events_of(os_side, attempt)
        .iter()
        .filter_map(|(_, kind)| kind["milestone"]["_0"].as_u64())
        .collect()
}

fn wait_for_exit_of(os_side: &mut OsSide, attempt: &str) -> Value {
    let exit = |os_side: &OsSide| {
        events_of(os_side, attempt)
            .into_iter()
            .find_map(|(_, kind)| Some(kind.get("exited")?["_0"].clone()))
    };
    os_side.wait_until("the exit", |os_side| exit(os_side).is_some());
    exit(os_side).unwrap()
}

fn first_run_up(os_side: &mut OsSide) {
    assert!(os_side.connect_app(1, 501));
    let id = os_side.request(1, connect(ATTEMPT, None));
    assert_eq!(os_side.reply(id), None);
    os_side.answer_network("lookUp", looked_up(outside_routes()));
    os_side.answer_network("apply", applied(outside_routes()));
    os_side.wait_until("the first tunnel", |os_side| {
        milestones_of(os_side, ATTEMPT).contains(&TUNNEL_UP)
    });
}

#[test]
fn the_new_run_takes_over_the_traffic_and_the_old_one_ends_as_replaced() {
    let mut os_side = start("10.10.0.6");
    first_run_up(&mut os_side);

    let id = os_side.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(os_side.reply(id), None);
    let look_up = os_side.answer_network("lookUp", looked_up(json!([])));
    assert_eq!(look_up["tunnel"]["name"], "utun10");
    let prepare = os_side.next_network_change();
    assert_eq!(prepare["prepare"]["settings"]["tunnel"]["name"], "utun10");
    assert_eq!(
        prepare["prepare"]["settings"]["ipv4"]["address"]["address"],
        "10.10.0.6"
    );
    assert_eq!(prepare["prepare"]["add"], json!([]));
    assert_eq!(
        os_side.scratch.recorded("run.json.2"),
        json!({"dnsServiceID": "", "routes": []})
    );
    os_side.send(json!({"networkChanged": {"outcome": applied(json!([]))}}));

    let switch = os_side.next_network_change();
    assert_eq!(switch["switch"]["settings"]["tunnel"]["name"], "utun10");
    assert_eq!(switch["switch"]["replaced"]["tunnel"]["name"], "utun9");
    let owned = json!({"dnsServiceID": "svc", "routes": outside_routes()});
    assert_eq!(os_side.scratch.recorded("run.json.2"), owned);
    assert_eq!(os_side.scratch.recorded("run.json"), owned);
    assert_eq!(
        milestones_of(&os_side, NEW),
        [STARTED, CONNECTED_TO_GATEWAY, AUTHENTICATED]
    );
    os_side.send(json!({"networkChanged": {"outcome": applied(json!([]))}}));

    let revert = os_side.answer_network("revert", reverted());
    assert_eq!(
        revert,
        json!({"changes": {"dnsServiceID": "", "routes": []}})
    );
    let facts = wait_for_exit_of(&mut os_side, ATTEMPT);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "replaced"}})
    );
    assert_eq!(facts["furthestMilestone"], TUNNEL_UP);
    assert_eq!(
        milestones_of(&os_side, NEW),
        [STARTED, CONNECTED_TO_GATEWAY, AUTHENTICATED, TUNNEL_UP]
    );
    let new_up = events_of(&os_side, NEW).last().unwrap().0;
    let old_exit = events_of(&os_side, ATTEMPT).last().unwrap().0;
    assert!(new_up < old_exit);
    assert!(os_side.has_log(&format!(
        "notice helper: 0277070470 run {ATTEMPT}: replaced by 0277070471 run {NEW}"
    )));
    os_side.wait_until("the first run's files removed", |os_side| {
        os_side
            .logs()
            .iter()
            .any(|line| line.contains(&format!("run {ATTEMPT} exited")))
    });

    let id = os_side.request(1, disconnect(NEW, "userDisconnect"));
    assert_eq!(os_side.reply(id), None);
    let revert = os_side.next_network_change();
    assert_eq!(revert["revert"], json!({"changes": owned}));
    assert_eq!(os_side.scratch.run_directory_names(), ["run.json"]);
    assert_eq!(os_side.scratch.recorded("run.json"), owned);
    os_side.send(json!({"networkChanged": {"outcome": reverted()}}));
    wait_for_exit_of(&mut os_side, NEW);
    os_side.wait_until("the new run's files removed", |os_side| {
        os_side
            .logs()
            .iter()
            .any(|line| line.contains(&format!("run {NEW} exited")))
    });
    assert!(os_side.scratch.run_directory_names().is_empty());
}

#[test]
fn when_the_old_run_ends_during_the_preparation_the_new_one_applies_everything_itself() {
    let mut os_side = start("10.10.0.6");
    first_run_up(&mut os_side);

    let id = os_side.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(os_side.reply(id), None);
    os_side.answer_network("lookUp", looked_up(json!([])));
    let prepare = os_side.next_network_change();
    assert!(prepare.get("prepare").is_some(), "{prepare:#}");
    let id = os_side.request(1, disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(os_side.reply(id), None);
    os_side.send(json!({"networkChanged": {"outcome": applied(json!([]))}}));

    let revert = os_side.answer_network("revert", reverted());
    assert_eq!(
        revert,
        json!({"changes": {"dnsServiceID": "svc", "routes": outside_routes()}})
    );
    let facts = wait_for_exit_of(&mut os_side, ATTEMPT);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "userDisconnect"}})
    );
    os_side.answer_network("lookUp", looked_up(outside_routes()));
    let apply = os_side.next_network_change();
    assert_eq!(apply["apply"]["settings"]["tunnel"]["name"], "utun10");
    assert!(apply["apply"].get("previous").is_none(), "{apply:#}");
    assert_eq!(apply["apply"]["add"], outside_routes());
    let owned = json!({"dnsServiceID": "svc", "routes": outside_routes()});
    assert_eq!(os_side.scratch.run_directory_names(), ["run.json"]);
    assert_eq!(os_side.scratch.recorded("run.json"), owned);
    os_side.send(json!({"networkChanged": {"outcome": applied(outside_routes())}}));
    os_side.wait_until("the new tunnel", |os_side| {
        milestones_of(os_side, NEW).contains(&TUNNEL_UP)
    });
    assert!(
        !os_side
            .network_changes()
            .iter()
            .any(|change| change.get("switch").is_some())
    );

    let id = os_side.request(1, disconnect(NEW, "userDisconnect"));
    assert_eq!(os_side.reply(id), None);
    let revert = os_side.answer_network("revert", reverted());
    assert_eq!(revert, json!({"changes": owned}));
    wait_for_exit_of(&mut os_side, NEW);
}

#[test]
fn a_switch_that_keeps_failing_ends_both_runs_as_network_setup_failures() {
    let mut os_side = start("10.10.0.6");
    first_run_up(&mut os_side);

    let id = os_side.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(os_side.reply(id), None);
    os_side.answer_network("lookUp", looked_up(json!([])));
    os_side.answer_network("prepare", applied(json!([])));
    let failed = json!({"failed": {
        "step": "routes",
        "error": "route -n change failed",
        "added": [],
        "left": [],
    }});
    for _ in 0..4 {
        os_side.answer_network("switch", failed.clone());
    }
    let owned = json!({"changes": {"dnsServiceID": "svc", "routes": outside_routes()}});
    for _ in 0..2 {
        let revert = os_side.answer_network("revert", reverted());
        assert_eq!(revert, owned);
    }
    for (attempt, furthest) in [(NEW, AUTHENTICATED), (ATTEMPT, TUNNEL_UP)] {
        let facts = wait_for_exit_of(&mut os_side, attempt);
        assert_eq!(
            facts["stopInitiator"],
            json!({"helper": {"_0": "networkSetupFailed"}}),
            "{attempt}"
        );
        assert_eq!(facts["furthestMilestone"], furthest, "{attempt}");
    }
    assert!(!milestones_of(&os_side, NEW).contains(&TUNNEL_UP));
}

#[test]
fn a_new_run_given_the_old_runs_address_ends_without_touching_the_old_one() {
    let mut os_side = start("10.10.0.5");
    first_run_up(&mut os_side);

    let id = os_side.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(os_side.reply(id), None);
    os_side.answer_network("lookUp", looked_up(json!([])));
    let facts = wait_for_exit_of(&mut os_side, NEW);
    assert_eq!(facts["tunnelAddressInUse"], true);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "networkSetupFailed"}})
    );
    assert_eq!(os_side.network_changes().len(), 3);
    assert!(
        events_of(&os_side, ATTEMPT)
            .iter()
            .all(|(_, kind)| kind.get("exited").is_none())
    );

    let id = os_side.request(1, disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(os_side.reply(id), None);
    let revert = os_side.answer_network("revert", reverted());
    assert_eq!(
        revert,
        json!({"changes": {"dnsServiceID": "svc", "routes": outside_routes()}})
    );
    wait_for_exit_of(&mut os_side, ATTEMPT);
}

#[test]
fn a_replacement_is_accepted_only_for_the_owner_of_an_up_run_once() {
    let mut os_side = start("10.10.0.6");
    assert!(os_side.connect_app(1, 501));
    assert!(os_side.connect_app(2, 502));
    let id = os_side.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(
        os_side.reply(id),
        error("unknownAttempt", "the connection to replace is not running")
    );
    let id = os_side.request(1, connect(ATTEMPT, None));
    assert_eq!(os_side.reply(id), None);
    let look_up = os_side.next_network_change();
    assert!(look_up.get("lookUp").is_some(), "{look_up:#}");
    let id = os_side.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(
        os_side.reply(id),
        error("wrongPhase", "the connection to replace is not up")
    );
    os_side.send(json!({"networkChanged": {"outcome": looked_up(outside_routes())}}));
    os_side.answer_network("apply", applied(outside_routes()));
    os_side.wait_until("the first tunnel", |os_side| {
        milestones_of(os_side, ATTEMPT).contains(&TUNNEL_UP)
    });

    let id = os_side.request(2, connect(NEW, Some(ATTEMPT)));
    assert_eq!(
        os_side.reply(id),
        error("notAuthorized", "the VPN run belongs to another user")
    );
    let id = os_side.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(os_side.reply(id), None);
    let id = os_side.request(1, connect(THIRD, Some(ATTEMPT)));
    assert_eq!(
        os_side.reply(id),
        error("busy", "a replacement connection is already running")
    );
    let id = os_side.request(1, connect(THIRD, None));
    assert_eq!(
        os_side.reply(id),
        error("busy", "openconnect is already running")
    );
    assert!(os_side.has_log(&format!(
        "notice helper: 0277070471 run {NEW} started for uid 501 with openconnect 9.21, replacing \
         0277070470 run {ATTEMPT}"
    )));
    os_side.disconnect_app(1);
    for run in [
        format!("0277070470 run {ATTEMPT}"),
        format!("0277070471 run {NEW}"),
    ] {
        os_side.wait_until("the stop", |os_side| {
            os_side.has_log(&format!(
                "notice helper: {run}: owning connection invalidated"
            ))
        });
    }
}

#[test]
fn a_connect_that_reuses_a_running_attempt_id_is_invalid() {
    let mut os_side = start("10.10.0.6");
    first_run_up(&mut os_side);
    let lowercase = ATTEMPT.to_lowercase();
    for (attempt, replacing) in [
        (ATTEMPT, Some(ATTEMPT)),
        (ATTEMPT, None),
        (lowercase.as_str(), Some(ATTEMPT)),
    ] {
        let id = os_side.request(1, connect(attempt, replacing));
        assert_eq!(
            os_side.reply(id),
            error(
                "invalidRequest",
                "the attempt ID belongs to a running connection"
            ),
            "{attempt} replacing {replacing:?}"
        );
    }
    let id = os_side.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(os_side.reply(id), None);
    let id = os_side.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(
        os_side.reply(id),
        error(
            "invalidRequest",
            "the attempt ID belongs to a running connection"
        )
    );
}
