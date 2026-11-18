mod common;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::*;
use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_contract::app_helper::RunRequest;
use fvm_contract::backend::{
    Backend, ClientCheckWait, ClientExecutable, ClientProblem, LaunchSpec, NetworkSettings,
    OutputInterpreter, UntestedVersion,
};
use serde_json::{Value, json};

const NEW: &str = OTHER_ATTEMPT;
const THIRD: &str = "3C5E7A90-1B2D-4F6A-8C0E-2D4F6A8C0E1B";
const PIN: &str = "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=";
const AUTHENTICATED: u64 = 4;

struct Scripted {
    clients: HashMap<String, String>,
}

impl Backend for Scripted {
    fn client_name(&self) -> &'static str {
        OpenConnect.client_name()
    }

    fn locate_client(&self, _: ClientCheckWait) -> Result<ClientExecutable, ClientProblem> {
        Ok(ClientExecutable {
            path: PathBuf::from("/bin/sh"),
            version: "9.21".to_owned(),
        })
    }

    fn describe_problem(&self, _: &ClientProblem) -> String {
        unreachable!("the client is always found")
    }

    fn untested_version(&self, _: &str) -> Option<UntestedVersion> {
        unreachable!("the tests do not ask for the client's version")
    }

    fn launch_spec(
        &self,
        client: &ClientExecutable,
        request: &RunRequest,
        config_path: &Path,
        values_path: &Path,
    ) -> Result<LaunchSpec, String> {
        let tunnel = OpenConnect
            .launch_spec(client, request, config_path, values_path)?
            .session
            .expect("openconnect opens the tunnel in a run of its own")
            .tunnel;
        let script = tunnel
            .args
            .iter()
            .find_map(|arg| arg.strip_prefix("--script="))
            .expect("the tunnel run gets the value script")
            .to_owned();
        Ok(LaunchSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec![
                "-c".to_owned(),
                self.clients[&request.attempt_id.to_string()].clone(),
                "openconnect".to_owned(),
                script,
            ],
            ..tunnel
        })
    }

    fn network_settings(&self, values: &[u8]) -> Result<NetworkSettings, String> {
        OpenConnect.network_settings(values)
    }

    fn check_otp(&self, code: &str) -> Result<(), String> {
        OpenConnect.check_otp(code)
    }

    fn interpreter(&self) -> Box<dyn OutputInterpreter + Send> {
        OpenConnect.interpreter()
    }

    fn stop_grace_period(&self) -> Duration {
        Duration::from_secs(2)
    }

    fn is_own_executable(&self, _: &Path) -> bool {
        false
    }
}

fn client(device: &str, address: &str) -> String {
    format!(
        "echo 'Got Legacy IP address {address}'\n\
         echo 'Configured as {address}, with SSL connected and DTLS in progress'\n\
         reason=connect VPNGATEWAY={GATEWAY} TUNDEV={device} INTERNAL_IP4_ADDRESS={address} \
         INTERNAL_IP4_DNS=10.20.0.53 CISCO_SPLIT_INC=1 CISCO_SPLIT_INC_0_ADDR=10.20.0.0 \
         CISCO_SPLIT_INC_0_MASK=255.255.0.0 CISCO_SPLIT_EXC=1 \
         CISCO_SPLIT_EXC_0_ADDR=10.20.161.16 CISCO_SPLIT_EXC_0_MASK=255.255.255.255 \
         /bin/sh -c \"$1\"\n\
         exec /bin/sleep 30"
    )
}

fn start(new_address: &str) -> Swift {
    let clients = [
        (ATTEMPT, client("utun9", "10.10.0.5")),
        (NEW, client("utun10", new_address)),
        (THIRD, client("utun10", new_address)),
    ]
    .into_iter()
    .map(|(attempt, script)| (attempt.to_owned(), script))
    .collect();
    Swift::start_with(Arc::new(Scripted { clients }))
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

fn route(destination: &str) -> Value {
    json!({
        "destination": {"address": destination, "prefixLength": 32},
        "router": "192.0.2.1",
        "serviceID": "ethernet",
    })
}

fn outside_routes() -> Value {
    json!([route(GATEWAY), route("10.20.161.16")])
}

struct Network {
    answered: usize,
}

impl Network {
    fn changes(swift: &Swift) -> Vec<Value> {
        swift
            .received
            .iter()
            .filter_map(|message| Some(message.get("changeNetwork")?["change"].clone()))
            .collect()
    }

    fn next(&mut self, swift: &mut Swift) -> Value {
        let answered = self.answered;
        swift.wait_until("a network change", |swift| {
            Self::changes(swift).len() > answered
        });
        self.answered += 1;
        Self::changes(swift)[answered].clone()
    }

    fn answer(&mut self, swift: &mut Swift, kind: &str, outcome: Value) -> Value {
        let change = self.next(swift);
        assert!(change.get(kind).is_some(), "not {kind}: {change:#}");
        swift.send(json!({"networkChanged": {"outcome": outcome}}));
        change[kind].clone()
    }
}

fn looked_up(missing: Value) -> Value {
    json!({"lookedUp": {"dnsServiceID": "svc", "missing": missing}})
}

fn applied(added: Value) -> Value {
    json!({"applied": {"added": added, "left": []}})
}

fn reverted() -> Value {
    json!({"reverted": {"left": []}})
}

fn events_of(swift: &Swift, attempt: &str) -> Vec<(usize, Value)> {
    swift
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

fn milestones_of(swift: &Swift, attempt: &str) -> Vec<u64> {
    events_of(swift, attempt)
        .iter()
        .filter_map(|(_, kind)| kind["milestone"]["_0"].as_u64())
        .collect()
}

fn wait_for_exit_of(swift: &mut Swift, attempt: &str) -> Value {
    let exit = |swift: &Swift| {
        events_of(swift, attempt)
            .into_iter()
            .find_map(|(_, kind)| Some(kind.get("exited")?["_0"].clone()))
    };
    swift.wait_until("the exit", |swift| exit(swift).is_some());
    exit(swift).unwrap()
}

fn recorded(swift: &Swift, name: &str) -> Value {
    let record = fs::read(swift.scratch.run_directory().join(name)).unwrap();
    serde_json::from_slice::<Value>(&record).unwrap()["outsideChanges"].clone()
}

fn first_run_up(swift: &mut Swift, network: &mut Network) {
    assert!(swift.connect_app(1, 501));
    let id = swift.request(1, connect(ATTEMPT, None));
    assert_eq!(swift.reply(id), None);
    network.answer(swift, "lookUp", looked_up(outside_routes()));
    network.answer(swift, "apply", applied(outside_routes()));
    swift.wait_until("the first tunnel", |swift| {
        milestones_of(swift, ATTEMPT).contains(&TUNNEL_UP)
    });
}

#[test]
fn the_new_run_takes_over_the_traffic_and_the_old_one_ends_as_replaced() {
    let mut swift = start("10.10.0.6");
    let mut network = Network { answered: 0 };
    first_run_up(&mut swift, &mut network);

    let id = swift.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(swift.reply(id), None);
    let look_up = network.answer(&mut swift, "lookUp", looked_up(json!([])));
    assert_eq!(look_up["tunnel"]["name"], "utun10");
    let prepare = network.next(&mut swift);
    assert_eq!(prepare["prepare"]["settings"]["tunnel"]["name"], "utun10");
    assert_eq!(
        prepare["prepare"]["settings"]["ipv4"]["address"]["address"],
        "10.10.0.6"
    );
    assert_eq!(prepare["prepare"]["add"], json!([]));
    assert_eq!(
        recorded(&swift, "run.json.2"),
        json!({"dnsServiceID": "", "routes": []})
    );
    swift.send(json!({"networkChanged": {"outcome": applied(json!([]))}}));

    let switch = network.next(&mut swift);
    assert_eq!(switch["switch"]["settings"]["tunnel"]["name"], "utun10");
    assert_eq!(switch["switch"]["replaced"]["tunnel"]["name"], "utun9");
    let owned = json!({"dnsServiceID": "svc", "routes": outside_routes()});
    assert_eq!(recorded(&swift, "run.json.2"), owned);
    assert_eq!(recorded(&swift, "run.json"), owned);
    assert_eq!(milestones_of(&swift, NEW), [SPAWNED, AUTHENTICATED]);
    swift.send(json!({"networkChanged": {"outcome": applied(json!([]))}}));

    let revert = network.answer(&mut swift, "revert", reverted());
    assert_eq!(
        revert,
        json!({"changes": {"dnsServiceID": "", "routes": []}})
    );
    let facts = wait_for_exit_of(&mut swift, ATTEMPT);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "replaced"}})
    );
    assert_eq!(facts["furthestMilestone"], TUNNEL_UP);
    assert_eq!(
        milestones_of(&swift, NEW),
        [SPAWNED, AUTHENTICATED, TUNNEL_UP]
    );
    let new_up = events_of(&swift, NEW).last().unwrap().0;
    let old_exit = events_of(&swift, ATTEMPT).last().unwrap().0;
    assert!(new_up < old_exit);
    assert!(swift.has_log(&format!(
        "notice helper: 0277070470 run {ATTEMPT}: replaced by 0277070471 run {NEW}"
    )));
    swift.wait_until("the first run's files removed", |swift| {
        swift
            .logs()
            .iter()
            .any(|line| line.contains(&format!("run {ATTEMPT} exited")))
    });

    let id = swift.request(1, disconnect(NEW, "userDisconnect"));
    assert_eq!(swift.reply(id), None);
    let revert = network.next(&mut swift);
    assert_eq!(revert["revert"], json!({"changes": owned}));
    assert_eq!(swift.scratch.run_directory_names(), ["run.json", "values"]);
    assert_eq!(recorded(&swift, "run.json"), owned);
    swift.send(json!({"networkChanged": {"outcome": reverted()}}));
    wait_for_exit_of(&mut swift, NEW);
    swift.wait_until("the new run's files removed", |swift| {
        swift
            .logs()
            .iter()
            .any(|line| line.contains(&format!("run {NEW} exited")))
    });
    assert_eq!(swift.scratch.run_directory_names(), ["values"]);
}

#[test]
fn when_the_old_run_ends_during_the_preparation_the_new_one_applies_everything_itself() {
    let mut swift = start("10.10.0.6");
    let mut network = Network { answered: 0 };
    first_run_up(&mut swift, &mut network);

    let id = swift.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(swift.reply(id), None);
    network.answer(&mut swift, "lookUp", looked_up(json!([])));
    let prepare = network.next(&mut swift);
    assert!(prepare.get("prepare").is_some(), "{prepare:#}");
    let id = swift.request(1, disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(swift.reply(id), None);
    swift.send(json!({"networkChanged": {"outcome": applied(json!([]))}}));

    let revert = network.answer(&mut swift, "revert", reverted());
    assert_eq!(
        revert,
        json!({"changes": {"dnsServiceID": "svc", "routes": outside_routes()}})
    );
    let facts = wait_for_exit_of(&mut swift, ATTEMPT);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "userDisconnect"}})
    );
    network.answer(&mut swift, "lookUp", looked_up(outside_routes()));
    let apply = network.next(&mut swift);
    assert_eq!(apply["apply"]["settings"]["tunnel"]["name"], "utun10");
    assert!(apply["apply"].get("previous").is_none(), "{apply:#}");
    assert_eq!(apply["apply"]["add"], outside_routes());
    let owned = json!({"dnsServiceID": "svc", "routes": outside_routes()});
    assert_eq!(swift.scratch.run_directory_names(), ["run.json", "values"]);
    assert_eq!(recorded(&swift, "run.json"), owned);
    swift.send(json!({"networkChanged": {"outcome": applied(outside_routes())}}));
    swift.wait_until("the new tunnel", |swift| {
        milestones_of(swift, NEW).contains(&TUNNEL_UP)
    });
    assert!(
        !Network::changes(&swift)
            .iter()
            .any(|change| change.get("switch").is_some())
    );

    let id = swift.request(1, disconnect(NEW, "userDisconnect"));
    assert_eq!(swift.reply(id), None);
    let revert = network.answer(&mut swift, "revert", reverted());
    assert_eq!(revert, json!({"changes": owned}));
    wait_for_exit_of(&mut swift, NEW);
}

#[test]
fn a_switch_that_keeps_failing_ends_both_runs_as_network_setup_failures() {
    let mut swift = start("10.10.0.6");
    let mut network = Network { answered: 0 };
    first_run_up(&mut swift, &mut network);

    let id = swift.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(swift.reply(id), None);
    network.answer(&mut swift, "lookUp", looked_up(json!([])));
    network.answer(&mut swift, "prepare", applied(json!([])));
    let failed = json!({"failed": {
        "step": "routes",
        "error": "route -n change failed",
        "added": [],
        "left": [],
    }});
    for _ in 0..4 {
        network.answer(&mut swift, "switch", failed.clone());
    }
    let owned = json!({"changes": {"dnsServiceID": "svc", "routes": outside_routes()}});
    for _ in 0..2 {
        let revert = network.answer(&mut swift, "revert", reverted());
        assert_eq!(revert, owned);
    }
    for (attempt, furthest) in [(NEW, AUTHENTICATED), (ATTEMPT, TUNNEL_UP)] {
        let facts = wait_for_exit_of(&mut swift, attempt);
        assert_eq!(
            facts["stopInitiator"],
            json!({"helper": {"_0": "networkSetupFailed"}}),
            "{attempt}"
        );
        assert_eq!(facts["furthestMilestone"], furthest, "{attempt}");
    }
    assert!(!milestones_of(&swift, NEW).contains(&TUNNEL_UP));
}

#[test]
fn a_new_run_given_the_old_runs_address_ends_without_touching_the_old_one() {
    let mut swift = start("10.10.0.5");
    let mut network = Network { answered: 0 };
    first_run_up(&mut swift, &mut network);

    let id = swift.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(swift.reply(id), None);
    network.answer(&mut swift, "lookUp", looked_up(json!([])));
    let facts = wait_for_exit_of(&mut swift, NEW);
    assert_eq!(facts["tunnelAddressInUse"], true);
    assert_eq!(
        facts["stopInitiator"],
        json!({"helper": {"_0": "networkSetupFailed"}})
    );
    assert_eq!(Network::changes(&swift).len(), network.answered);
    assert!(
        events_of(&swift, ATTEMPT)
            .iter()
            .all(|(_, kind)| kind.get("exited").is_none())
    );

    let id = swift.request(1, disconnect(ATTEMPT, "userDisconnect"));
    assert_eq!(swift.reply(id), None);
    let revert = network.answer(&mut swift, "revert", reverted());
    assert_eq!(
        revert,
        json!({"changes": {"dnsServiceID": "svc", "routes": outside_routes()}})
    );
    wait_for_exit_of(&mut swift, ATTEMPT);
}

#[test]
fn a_replacement_is_accepted_only_for_the_owner_of_an_up_run_once() {
    let mut swift = start("10.10.0.6");
    let mut network = Network { answered: 0 };
    assert!(swift.connect_app(1, 501));
    assert!(swift.connect_app(2, 502));
    let id = swift.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(
        swift.reply(id),
        error("unknownAttempt", "the connection to replace is not running")
    );
    let id = swift.request(1, connect(ATTEMPT, None));
    assert_eq!(swift.reply(id), None);
    let look_up = network.next(&mut swift);
    assert!(look_up.get("lookUp").is_some(), "{look_up:#}");
    let id = swift.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(
        swift.reply(id),
        error("wrongPhase", "the connection to replace is not up")
    );
    swift.send(json!({"networkChanged": {"outcome": looked_up(outside_routes())}}));
    network.answer(&mut swift, "apply", applied(outside_routes()));
    swift.wait_until("the first tunnel", |swift| {
        milestones_of(swift, ATTEMPT).contains(&TUNNEL_UP)
    });

    let id = swift.request(2, connect(NEW, Some(ATTEMPT)));
    assert_eq!(
        swift.reply(id),
        error("notAuthorized", "the VPN run belongs to another user")
    );
    let id = swift.request(1, connect(NEW, Some(ATTEMPT)));
    assert_eq!(swift.reply(id), None);
    let id = swift.request(1, connect(THIRD, Some(ATTEMPT)));
    assert_eq!(
        swift.reply(id),
        error("busy", "a replacement connection is already running")
    );
    let id = swift.request(1, connect(THIRD, None));
    assert_eq!(
        swift.reply(id),
        error("busy", "openconnect is already running")
    );
    assert!(swift.has_log(&format!(
        "notice helper: 0277070471 run {NEW} started for uid 501 with openconnect 9.21, replacing \
         0277070470 run {ATTEMPT}"
    )));
    swift.disconnect_app(1);
    for run in [
        format!("0277070470 run {ATTEMPT}"),
        format!("0277070471 run {NEW}"),
    ] {
        swift.wait_until("the stop", |swift| {
            swift.has_log(&format!(
                "notice helper: {run}: owning connection invalidated"
            ))
        });
    }
}
