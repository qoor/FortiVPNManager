//! The exception routes through the Helper-internal protocol: what the entry asks the OS side to
//! look up, add and remove for each app, what it records before adding, and what it removes when
//! an app quits or goes away, or an earlier Helper left.

mod common;

use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::time::Duration;

use common::*;
use serde_json::{Value, json};

const SERVICE: &str = "8A3F1C2E-5B6D-4E7F-9A0B-1C2D3E4F5A6B";

fn network() -> Value {
    network_of(SERVICE)
}

fn network_of(service: &str) -> Value {
    json!({
        "serviceID": service,
        "serviceName": "Ethernet",
        "router": "10.20.161.1",
        "address": "10.20.161.20",
        "subnetMask": "255.255.255.0",
    })
}

fn host(last: u8) -> Value {
    json!({"destination": format!("10.30.161.{last}"), "subnetMask": "255.255.255.255", "gateway": "10.20.161.1"})
}

/// `host(last)` as the Helper added it through `SERVICE`.
fn added(last: u8) -> Value {
    added_through(last, SERVICE)
}

fn added_through(last: u8, service: &str) -> Value {
    json!({"route": host(last), "serviceID": service})
}

fn write(routes: &[Value]) -> Value {
    write_on(network(), routes)
}

fn write_on(network: Value, routes: &[Value]) -> Value {
    let request = json!({"write": {"expected": network, "routes": routes}});
    json!({"writeBypassRoutes": {"json": request.to_string()}})
}

fn remove() -> Value {
    json!({"writeBypassRoutes": {"json": json!({"remove": {}}).to_string()}})
}

fn read() -> Value {
    json!({"readBypassRoutes": {}})
}

fn look_up(routes: &[Value]) -> Value {
    json!({"network": {"expect": {"network": network()}}, "remove": [], "add": [], "lookUp": routes})
}

fn removal(routes: &[Value]) -> Value {
    json!({"network": {"skip": {}}, "remove": routes, "add": [], "lookUp": []})
}

impl OsSide {
    /// The changes asked of the OS side so far.
    fn changes(&self) -> Vec<Value> {
        self.received
            .iter()
            .filter_map(|message| Some(message.get("changeBypassRoutes")?["change"].clone()))
            .collect()
    }

    /// Waits for the `count`-th change and returns it.
    fn change(&mut self, count: usize) -> Value {
        self.wait_until("the change", |os_side| os_side.changes().len() >= count);
        self.changes()[count - 1].clone()
    }

    /// Answers the change under way; `network` when it read one.
    fn changed(&self, network: Option<Value>, added: &[Value], found: &[Value], left: &[Value]) {
        let mut changed = json!({"added": added, "found": found, "left": left});
        if let Some(network) = network {
            changed["network"] = network;
        }
        self.send(json!({"bypassRoutesChanged": {"outcome": {"changed": changed}}}));
    }

    fn refused(&self, error: &Value) {
        self.send(json!({"bypassRoutesChanged": {"outcome": {"refused": {"error": error}}}}));
    }

    /// The `BypassRoutesReply` to request `id`.
    fn route_reply(&mut self, id: u64) -> Value {
        self.reply(id)
            .expect("a route request is answered with JSON")
    }

    fn recorded(&self) -> Option<Value> {
        let path = self.scratch.run_directory().join("exception-routes.json");
        let bytes = fs::read(path).ok()?;
        Some(serde_json::from_slice::<Value>(&bytes).unwrap()["routes"].clone())
    }

    /// Has app `app` write `routes`, none of them in the table yet, so that the Helper adds them
    /// all; its changes are the `count`-th and the next.
    fn write_added(&mut self, app: u64, routes: &[Value], count: usize) {
        let id = self.request(app, write(routes));
        self.change(count);
        self.changed(Some(network()), &[], &[], &[]);
        self.change(count + 1);
        self.changed(Some(network()), routes, routes, &[]);
        assert_eq!(
            self.route_reply(id),
            json!({"success": {"snapshot": network(), "routes": routes}})
        );
    }
}

fn started() -> OsSide {
    let mut os_side = OsSide::start(Setup::default());
    assert!(os_side.connect_app(1, 501));
    os_side
}

fn wrong_phase() -> Value {
    json!({"code": "wrongPhase", "message": "the network changed since it was read; read it again"})
}

#[test]
fn the_exception_routes_are_kept_outside_in_the_apply_and_the_routing_check() {
    let mut os_side = OsSide::start(Setup::with([up_without_otp(settings(
        "utun9",
        "10.10.0.5",
    ))]));
    assert!(os_side.connect_app(1, 501));
    os_side.write_added(1, &[host(16)], 1);
    assert_eq!(os_side.connect(1, ATTEMPT), None);
    os_side.answer_network("lookUp", looked_up(outside_routes()));
    let apply = os_side.answer_network("apply", applied(outside_routes()));
    let outside = json!([
        {"address": GATEWAY, "prefixLength": 32},
        {"address": "10.20.161.16", "prefixLength": 32},
        {"address": "10.30.161.16", "prefixLength": 32},
    ]);
    assert_eq!(apply["outside"], outside);
    os_side.wait_for_milestone(1, TUNNEL_UP);
    let status = os_side.request(1, json!({"status": {}}));
    let check = os_side.answer_network("check", json!({"checked": {"complete": true}}));
    assert_eq!(check["outside"], outside);
    assert_eq!(os_side.reply_json(status)["routingComplete"], true);
}

#[test]
fn a_write_looks_up_first_and_records_only_what_it_adds() {
    let mut os_side = started();
    let id = os_side.request(1, write(&[host(16), host(17)]));
    assert_eq!(os_side.change(1), look_up(&[host(16), host(17)]));
    assert_eq!(os_side.recorded(), None);
    // The first one is in the table already: someone else's.
    os_side.changed(Some(network()), &[], &[host(16)], &[]);
    assert_eq!(
        os_side.change(2),
        json!({
            "network": {"expect": {"network": network()}},
            "remove": [],
            "add": [host(17)],
            "lookUp": [host(16), host(17)],
        })
    );
    assert_eq!(os_side.recorded(), Some(json!([added(17)])));
    os_side.changed(Some(network()), &[host(17)], &[host(16), host(17)], &[]);
    assert_eq!(
        os_side.route_reply(id),
        json!({"success": {"snapshot": network(), "routes": [host(16), host(17)]}})
    );
    assert_eq!(os_side.recorded(), Some(json!([added(17)])));
}

#[test]
fn routes_all_in_the_table_already_are_left_where_they_are() {
    let mut os_side = started();
    let id = os_side.request(1, write(&[host(16)]));
    os_side.change(1);
    os_side.changed(Some(network()), &[], &[host(16)], &[]);
    assert_eq!(
        os_side.route_reply(id),
        json!({"success": {"snapshot": network(), "routes": [host(16)]}})
    );
    assert_eq!(os_side.recorded(), None);
    // Someone else's: quitting removes nothing.
    let id = os_side.request(1, remove());
    assert_eq!(os_side.route_reply(id), json!({"removed": {}}));
    assert_eq!(os_side.changes().len(), 1);
}

#[test]
fn a_new_list_removes_the_routes_no_longer_wanted() {
    let mut os_side = started();
    os_side.write_added(1, &[host(16), host(17)], 1);
    let id = os_side.request(1, write(&[host(17), host(18)]));
    assert_eq!(os_side.change(3), look_up(&[host(17), host(18)]));
    os_side.changed(Some(network()), &[], &[host(17)], &[]);
    assert_eq!(
        os_side.change(4),
        json!({
            "network": {"expect": {"network": network()}},
            "remove": [added(16)],
            "add": [host(18)],
            "lookUp": [host(17), host(18)],
        })
    );
    os_side.changed(Some(network()), &[host(18)], &[host(17), host(18)], &[]);
    assert_eq!(
        os_side.route_reply(id),
        json!({"success": {"snapshot": network(), "routes": [host(17), host(18)]}})
    );
    assert_eq!(os_side.recorded(), Some(json!([added(17), added(18)])));
}

/// The same router through another network service, as when the Mac moves from Wi-Fi to its cable
/// on the same network: a route belongs to no service on macOS, so the table's route through the
/// router is the one the Helper added. It stays the Helper's, as recorded.
#[test]
fn a_new_network_service_with_the_same_router_keeps_the_routes() {
    let mut os_side = started();
    let mut wifi = network_of("WIFI");
    wifi["serviceName"] = json!("Wi-Fi");
    let id = os_side.request(1, write_on(wifi.clone(), &[host(16)]));
    os_side.change(1);
    os_side.changed(Some(wifi.clone()), &[], &[], &[]);
    os_side.change(2);
    os_side.changed(Some(wifi), &[host(16)], &[host(16)], &[]);
    os_side.route_reply(id);
    assert_eq!(os_side.recorded(), Some(json!([added_through(16, "WIFI")])));

    let id = os_side.request(1, write(&[host(16)]));
    assert_eq!(os_side.change(3), look_up(&[host(16)]));
    os_side.changed(Some(network()), &[], &[host(16)], &[]);
    assert_eq!(
        os_side.route_reply(id),
        json!({"success": {"snapshot": network(), "routes": [host(16)]}})
    );
    assert_eq!(os_side.changes().len(), 3);
    assert_eq!(os_side.recorded(), Some(json!([added_through(16, "WIFI")])));
    // Still the Helper's: quitting removes it.
    let id = os_side.request(1, remove());
    assert_eq!(os_side.change(4), removal(&[added_through(16, "WIFI")]));
    os_side.changed(None, &[], &[], &[]);
    assert_eq!(os_side.route_reply(id), json!({"removed": {}}));
}

/// A route an earlier Helper recorded through two network services is one route on macOS: both
/// records stay while an app wants it, and go together once none does.
#[test]
fn a_route_recorded_through_two_services_is_removed_once_no_app_wants_it() {
    let scratch = Scratch::new();
    let directory = scratch.run_directory();
    DirBuilder::new().mode(0o700).create(&directory).unwrap();
    let both = [added(16), added_through(16, "WIFI")];
    let record = json!({"routes": both});
    fs::write(directory.join("exception-routes.json"), record.to_string()).unwrap();
    let mut os_side = OsSide::launch(scratch, Setup::default());
    assert_eq!(os_side.change(1), removal(&both));
    // It could not be removed then.
    os_side.changed(None, &[], &[], &both);
    os_side.wait_until("ready", |os_side| os_side.position(is("ready")).is_some());
    assert!(os_side.connect_app(1, 501));

    let id = os_side.request(1, write(&[host(16)]));
    os_side.change(2);
    os_side.changed(Some(network()), &[], &[host(16)], &[]);
    os_side.route_reply(id);
    assert_eq!(os_side.changes().len(), 2);
    assert_eq!(os_side.recorded(), Some(json!(both)));

    let id = os_side.request(1, remove());
    assert_eq!(os_side.change(3), removal(&both));
    os_side.changed(None, &[], &[], &[]);
    assert_eq!(os_side.route_reply(id), json!({"removed": {}}));
    assert_eq!(os_side.recorded(), None);
}

#[test]
fn a_refused_look_up_answers_the_error() {
    let mut os_side = started();
    let id = os_side.request(1, write(&[host(16)]));
    os_side.change(1);
    os_side.refused(&wrong_phase());
    assert_eq!(
        os_side.route_reply(id),
        json!({"failure": {"_0": wrong_phase()}})
    );
    assert_eq!(os_side.recorded(), None);
    assert_eq!(os_side.changes().len(), 1);
}

#[test]
fn a_refused_change_puts_the_record_back() {
    let mut os_side = started();
    let id = os_side.request(1, write(&[host(16)]));
    os_side.change(1);
    os_side.changed(Some(network()), &[], &[], &[]);
    os_side.change(2);
    assert_eq!(os_side.recorded(), Some(json!([added(16)])));
    os_side.refused(&wrong_phase());
    assert_eq!(
        os_side.route_reply(id),
        json!({"failure": {"_0": wrong_phase()}})
    );
    assert_eq!(os_side.recorded(), None);
}

#[test]
fn reading_looks_up_the_routes_the_app_last_wrote() {
    let mut os_side = started();
    os_side.write_added(1, &[host(16)], 1);
    let id = os_side.request(1, read());
    assert_eq!(
        os_side.change(3),
        json!({"network": {"read": {}}, "remove": [], "add": [], "lookUp": [host(16)]})
    );
    // The system removed it meanwhile, as when the network went down.
    os_side.changed(Some(network()), &[], &[], &[]);
    assert_eq!(
        os_side.route_reply(id),
        json!({"success": {"snapshot": network(), "routes": []}})
    );
}

#[test]
fn quitting_removes_the_apps_routes_wherever_the_network_is() {
    let mut os_side = started();
    os_side.write_added(1, &[host(16), host(17)], 1);
    let id = os_side.request(1, remove());
    assert_eq!(os_side.change(3), removal(&[added(16), added(17)]));
    os_side.changed(None, &[], &[], &[]);
    assert_eq!(os_side.route_reply(id), json!({"removed": {}}));
    assert_eq!(os_side.recorded(), None);
}

#[test]
fn the_routes_of_a_connection_that_goes_away_are_removed() {
    let mut os_side = started();
    os_side.write_added(1, &[host(16)], 1);
    os_side.disconnect_app(1);
    assert_eq!(os_side.change(3), removal(&[added(16)]));
    // It stays in the table: it remains the Helper's to remove later.
    os_side.changed(None, &[], &[], &[added(16)]);
    os_side.wait_until("the log line", |os_side| {
        os_side.has_log(
            "error routes: 1 exception route(s) are still in the routing table after removing them",
        )
    });
    assert_eq!(os_side.recorded(), Some(json!([added(16)])));
}

#[test]
fn a_removal_the_os_side_could_not_carry_out_keeps_the_routes_recorded() {
    let mut os_side = started();
    os_side.write_added(1, &[host(16)], 1);
    os_side.disconnect_app(1);
    os_side.change(3);
    let error = json!({"code": "internalError", "message": "reading the routing table failed"});
    os_side.refused(&error);
    os_side.wait_until("the log line", |os_side| {
        os_side.has_log(
            "error routes: removing exception routes failed: reading the routing table failed",
        )
    });
    assert_eq!(os_side.recorded(), Some(json!([added(16)])));
}

#[test]
fn a_route_another_app_still_wants_stays() {
    let mut os_side = started();
    assert!(os_side.connect_app(2, 502));
    os_side.write_added(1, &[host(16), host(17)], 1);
    // The second app wants one of them, which the Helper added already.
    let id = os_side.request(2, write(&[host(16)]));
    os_side.change(3);
    os_side.changed(Some(network()), &[], &[host(16)], &[]);
    os_side.route_reply(id);
    os_side.disconnect_app(1);
    assert_eq!(os_side.change(4), removal(&[added(17)]));
    os_side.changed(None, &[], &[], &[]);
    let id = os_side.request(2, remove());
    assert_eq!(os_side.change(5), removal(&[added(16)]));
    os_side.changed(None, &[], &[], &[]);
    assert_eq!(os_side.route_reply(id), json!({"removed": {}}));
    assert_eq!(os_side.recorded(), None);
}

#[test]
fn the_os_side_gets_one_change_at_a_time() {
    let mut os_side = started();
    assert!(os_side.connect_app(2, 502));
    let first = os_side.request(1, write(&[host(16)]));
    let second = os_side.request(2, write(&[host(17)]));
    os_side.change(1);
    let meanwhile = os_side.received_within(Duration::from_millis(300));
    assert!(meanwhile.is_empty(), "{meanwhile:#?}");
    os_side.changed(Some(network()), &[], &[], &[]);
    assert_eq!(os_side.change(2)["add"], json!([host(16)]));
    os_side.changed(Some(network()), &[host(16)], &[host(16)], &[]);
    os_side.route_reply(first);
    assert_eq!(os_side.change(3), look_up(&[host(17)]));
    os_side.changed(Some(network()), &[], &[], &[]);
    os_side.change(4);
    os_side.changed(Some(network()), &[host(17)], &[host(17)], &[]);
    os_side.route_reply(second);
    assert_eq!(os_side.recorded(), Some(json!([added(16), added(17)])));
}

#[test]
fn a_closed_connection_changes_no_route() {
    let mut os_side = started();
    let id = os_side.request(9, write(&[host(16)]));
    assert_eq!(
        os_side.route_reply(id),
        json!({"failure": {"_0": {"code": "notAuthorized", "message": "the app connection is closed"}}})
    );
    assert!(os_side.changes().is_empty());
}

#[test]
fn an_unreadable_request_is_refused() {
    let mut os_side = started();
    let id = os_side.request(1, json!({"writeBypassRoutes": {"json": "{}"}}));
    assert_eq!(
        os_side.route_reply(id),
        json!({"failure": {"_0": {"code": "invalidRequest", "message": "could not decode the route request"}}})
    );
}

#[test]
fn the_routes_an_earlier_helper_added_are_removed_before_ready() {
    let scratch = Scratch::new();
    let directory = scratch.run_directory();
    DirBuilder::new().mode(0o700).create(&directory).unwrap();
    let record = json!({"routes": [added(16), added(17)]});
    fs::write(directory.join("exception-routes.json"), record.to_string()).unwrap();
    let mut os_side = OsSide::launch(scratch, Setup::default());

    let first = os_side.receive_within(Duration::from_secs(10));
    assert_eq!(
        first,
        Some(json!({"changeBypassRoutes": {"change": removal(&[added(16), added(17)])}}))
    );
    let before_answer = os_side.received_within(Duration::from_millis(300));
    assert!(before_answer.is_empty(), "{before_answer:#?}");
    // One of them could not be removed.
    os_side.changed(None, &[], &[], &[added(17)]);
    os_side.wait_until("ready", |os_side| os_side.position(is("ready")).is_some());
    assert!(
        os_side.has_log(
            "notice routes: removed 1 of the 2 exception route(s) an earlier helper added"
        )
    );
    assert_eq!(os_side.recorded(), Some(json!([added(17)])));

    // It stays the Helper's: the first change of an app removes it.
    assert!(os_side.connect_app(1, 501));
    let id = os_side.request(1, write(&[host(18)]));
    os_side.change(2);
    os_side.changed(Some(network()), &[], &[], &[]);
    assert_eq!(os_side.change(3)["remove"], json!([added(17)]));
    os_side.changed(Some(network()), &[host(18)], &[host(18)], &[]);
    os_side.route_reply(id);
    assert_eq!(os_side.recorded(), Some(json!([added(18)])));
}

#[test]
fn an_unreadable_record_is_dropped() {
    let scratch = Scratch::new();
    let directory = scratch.run_directory();
    DirBuilder::new().mode(0o700).create(&directory).unwrap();
    fs::write(directory.join("exception-routes.json"), "not JSON").unwrap();
    let mut os_side = OsSide::launch(scratch, Setup::default());
    os_side.wait_until("ready", |os_side| os_side.position(is("ready")).is_some());
    assert!(os_side.changes().is_empty());
    assert_eq!(os_side.recorded(), None);
}
