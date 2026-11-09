//! Route planning for the exception rules.

use std::collections::HashSet;

use fvm_contract::app_helper::{AdditionalRoute, BypassRoutesSnapshot};
use fvm_core::bypass_rules::{Applicability, Ipv4Network, RoutePlan, plan_routes};

const ROUTER: &str = "10.20.161.254";

fn net(text: &str) -> Ipv4Network {
    text.parse().unwrap()
}

fn office() -> Ipv4Network {
    net("10.20.161.0/24")
}
fn servers() -> Ipv4Network {
    net("10.30.161.0/24")
}
fn host() -> Ipv4Network {
    net("10.30.161.16")
}
fn network() -> Ipv4Network {
    net("10.30.170.0/24")
}
fn other() -> Ipv4Network {
    net("10.50.100.10")
}

fn via(network: Ipv4Network) -> AdditionalRoute {
    network.route_through(ROUTER)
}

fn via_gw(network: Ipv4Network, gateway: &str) -> AdditionalRoute {
    network.route_through(gateway)
}

fn route(destination: &str, mask: &str, gateway: &str) -> AdditionalRoute {
    AdditionalRoute {
        destination: destination.into(),
        subnet_mask: mask.into(),
        gateway: gateway.into(),
    }
}

/// The Mac at the office desk (10.20.161.20/24) unless told otherwise.
fn snapshot(routes: Vec<AdditionalRoute>) -> BypassRoutesSnapshot {
    snapshot_at(routes, ROUTER, "10.20.161.20", "255.255.255.0")
}

fn snapshot_at(
    routes: Vec<AdditionalRoute>,
    router: &str,
    address: &str,
    mask: &str,
) -> BypassRoutesSnapshot {
    BypassRoutesSnapshot {
        service_id: "SERVICE-1".into(),
        service_name: "Ethernet".into(),
        router: router.into(),
        address: address.into(),
        subnet_mask: mask.into(),
        routes,
    }
}

fn plan(
    targets: &[Ipv4Network],
    applied: &[AdditionalRoute],
    snapshot: &BypassRoutesSnapshot,
) -> RoutePlan {
    plan_routes(&[office()], targets, applied, snapshot)
}

fn set(routes: &[AdditionalRoute]) -> HashSet<AdditionalRoute> {
    routes.iter().cloned().collect()
}

fn matched(plan: &RoutePlan) -> Option<Ipv4Network> {
    match plan.applicability {
        Applicability::Applies { network } => Some(network),
        _ => None,
    }
}

#[test]
fn already_applied_needs_no_write() {
    let outcome = plan(
        &[host(), network()],
        &[via(host()), via(network())],
        &snapshot(vec![via(network()), via(host())]),
    );
    assert_eq!(matched(&outcome), Some(office()));
    assert!(!outcome.needs_write);
    assert!(outcome.imported.is_empty());
    assert_eq!(outcome.applied, vec![via(host()), via(network())]);
}

#[test]
fn a_reformatted_mac_gets_the_list() {
    let outcome = plan(&[host(), network()], &[], &snapshot(vec![]));
    assert!(outcome.needs_write);
    assert_eq!(outcome.routes, vec![via(host()), via(network())]);
}

#[test]
fn outside_the_networks_the_apps_routes_go_and_the_rest_stays() {
    let home_router = "192.168.0.1";
    let home_route = via_gw(other(), home_router);
    let listed_through_home_router = via_gw(network(), home_router);
    let outcome = plan(
        &[host(), network()],
        &[via(host())],
        &snapshot_at(
            vec![
                via(host()),
                home_route.clone(),
                listed_through_home_router.clone(),
            ],
            home_router,
            "192.168.0.12",
            "255.255.255.0",
        ),
    );
    assert_eq!(outcome.applicability, Applicability::OutsideNetworks);
    assert!(outcome.imported.is_empty());
    assert!(outcome.applied.is_empty());
    assert_eq!(outcome.routes, vec![home_route, listed_through_home_router]);
    assert!(outcome.needs_write);
}

#[test]
fn without_networks_the_rules_apply_nowhere() {
    let outcome = plan_routes(&[], &[host()], &[via(host())], &snapshot(vec![via(host())]));
    assert_eq!(outcome.applicability, Applicability::NoNetworksListed);
    assert!(outcome.routes.is_empty());
    assert!(outcome.needs_write);
}

#[test]
fn another_listed_network_uses_its_own_router_and_skips_targets_on_the_same_lan() {
    let lan_router = "10.30.161.1";
    let outcome = plan_routes(
        &[office(), servers()],
        &[host(), network()],
        &[],
        &snapshot_at(vec![], lan_router, "10.30.161.50", "255.255.255.0"),
    );
    assert_eq!(matched(&outcome), Some(servers()));
    assert_eq!(outcome.on_link, vec![host()]);
    assert_eq!(outcome.routes, vec![via_gw(network(), lan_router)]);
}

#[test]
fn a_target_that_became_on_link_is_removed() {
    let lan_router = "10.30.161.1";
    let outcome = plan_routes(
        &[servers()],
        &[host()],
        &[via_gw(host(), lan_router)],
        &snapshot_at(
            vec![via_gw(host(), lan_router)],
            lan_router,
            "10.30.161.50",
            "255.255.255.0",
        ),
    );
    assert_eq!(outcome.on_link, vec![host()]);
    assert!(outcome.routes.is_empty());
    assert!(outcome.needs_write);
}

#[test]
fn a_route_on_this_macs_network_the_app_never_wrote_is_kept() {
    // An admin's VLAN behind the router, more specific than the Mac's /16.
    let vlan = route("10.20.200.0", "255.255.255.0", ROUTER);
    let outcome = plan(
        &[host()],
        &[],
        &snapshot_at(vec![vlan.clone()], ROUTER, "10.20.161.20", "255.255.0.0"),
    );
    assert!(outcome.imported.is_empty());
    assert_eq!(outcome.applied, vec![via(host())]);
    assert_eq!(outcome.routes, vec![via(host()), vlan]);
    assert_eq!(outcome.kept_through_other_gateways, 0);
}

#[test]
fn a_listed_target_on_this_macs_network_keeps_a_route_the_app_never_wrote() {
    let desk_neighbour = net("10.20.161.16");
    let outcome = plan(&[desk_neighbour], &[], &snapshot(vec![via(desk_neighbour)]));
    assert_eq!(outcome.on_link, vec![desk_neighbour]);
    assert!(outcome.applied.is_empty());
    assert_eq!(outcome.routes, vec![via(desk_neighbour)]);
    assert!(!outcome.needs_write);
}

#[test]
fn removed_from_the_list_is_deleted() {
    let outcome = plan(
        &[host()],
        &[via(host()), via(network())],
        &snapshot(vec![via(host()), via(network())]),
    );
    assert!(outcome.needs_write);
    assert!(outcome.imported.is_empty());
    assert_eq!(outcome.routes, vec![via(host())]);
    assert_eq!(outcome.applied, vec![via(host())]);
}

#[test]
fn added_outside_the_app_through_the_router_joins_the_list() {
    let outcome = plan(
        &[host()],
        &[via(host())],
        &snapshot(vec![via(host()), via(other())]),
    );
    assert_eq!(outcome.imported, vec![other()]);
    assert!(!outcome.needs_write);
    assert_eq!(outcome.applied, vec![via(host()), via(other())]);
}

#[test]
fn first_adoption_imports_what_is_there_once() {
    let outcome = plan(&[], &[], &snapshot(vec![via(host()), via(host())]));
    assert_eq!(outcome.imported, vec![host()]);
    assert!(outcome.needs_write); // the repeat goes away
    assert_eq!(outcome.routes, vec![via(host())]);
}

#[test]
fn other_gateways_are_kept_whatever_the_order() {
    let foreign = via_gw(other(), "192.168.0.1");
    let wide_route = route("16.0.0.0", "240.0.0.0", ROUTER);
    for routes in [
        vec![via(other()), foreign.clone(), wide_route.clone()],
        vec![foreign.clone(), wide_route.clone(), via(other())],
    ] {
        let outcome = plan(&[host()], &[], &snapshot(routes));
        assert_eq!(outcome.imported, vec![other()]);
        assert_eq!(set(&outcome.applied), set(&[via(host()), via(other())]));
        assert_eq!(
            set(&outcome.routes),
            set(&[
                via(host()),
                via(other()),
                foreign.clone(),
                wide_route.clone()
            ])
        );
    }
}

#[test]
fn routes_overlapping_a_target_are_not_imported() {
    let inside = route("10.30.170.5", "255.255.255.255", ROUTER);
    let wider = route("10.30.0.0", "255.255.0.0", ROUTER);
    let other_gateway = via_gw(other(), "10.50.0.0");
    let outcome = plan(
        &[network()],
        &[],
        &snapshot(vec![
            inside.clone(),
            wider.clone(),
            via(other()),
            other_gateway.clone(),
        ]),
    );
    assert_eq!(outcome.imported, vec![other()]);
    let mut expected = outcome.applied.clone();
    expected.extend([inside, wider, other_gateway]);
    assert_eq!(outcome.routes, expected);
    assert_eq!(outcome.kept_through_other_gateways, 1);
}

#[test]
fn of_overlapping_routes_added_outside_the_wider_joins_whatever_the_order() {
    let wide = net("10.30.0.0/16");
    for routes in [
        vec![via(host()), via(wide), via(other())],
        vec![via(wide), via(other()), via(host())],
    ] {
        let outcome = plan(&[], &[], &snapshot(routes));
        assert_eq!(outcome.imported, vec![wide, other()]);
        assert_eq!(set(&outcome.applied), set(&[via(wide), via(other())]));
        assert_eq!(
            set(&outcome.routes),
            set(&[via(wide), via(other()), via(host())])
        );
        assert!(!outcome.needs_write);
    }
}

#[test]
fn only_other_gateways_count_as_such() {
    let home_router = "192.168.0.1";
    let home_route = via_gw(other(), home_router);
    let foreign = via_gw(network(), "192.168.0.254");
    let outcome = plan(
        &[host()],
        &[],
        &snapshot_at(
            vec![home_route.clone(), foreign.clone()],
            home_router,
            "192.168.0.12",
            "255.255.255.0",
        ),
    );
    assert!(outcome.applied.is_empty());
    assert_eq!(outcome.routes, vec![home_route, foreign]);
    assert_eq!(outcome.kept_through_other_gateways, 1);
}

#[test]
fn a_listed_target_through_another_gateway_is_left_alone() {
    let foreign = via_gw(host(), "192.168.0.1");
    let outcome = plan(&[host()], &[], &snapshot(vec![foreign.clone()]));
    assert_eq!(outcome.applied, vec![via(host())]);
    assert_eq!(outcome.routes, vec![via(host()), foreign]);
}

#[test]
fn a_new_router_rewrites_the_apps_routes_only() {
    let old_router = "10.20.161.1";
    let foreign = via_gw(other(), "192.168.0.1");
    let outcome = plan(
        &[host()],
        &[via_gw(host(), old_router), via_gw(network(), old_router)],
        &snapshot(vec![
            via_gw(host(), old_router),
            via_gw(network(), old_router),
            foreign.clone(),
        ]),
    );
    assert!(outcome.imported.is_empty());
    assert_eq!(outcome.routes, vec![via(host()), foreign]);
    assert!(outcome.needs_write);
}

#[test]
fn a_full_list_leaves_further_outside_routes_untouched() {
    let targets: Vec<_> = (1..=99).map(|n| net(&format!("10.90.0.{n}"))).collect();
    let outside: Vec<_> = (1..=3).map(|n| net(&format!("10.91.0.{n}"))).collect();
    let applied: Vec<_> = targets.iter().copied().map(via).collect();
    let present: Vec<_> = targets.iter().chain(&outside).copied().map(via).collect();
    let outcome = plan(&targets, &applied, &snapshot(present));
    assert_eq!(outcome.imported, vec![outside[0]]);
    assert_eq!(outcome.not_imported, 2);
    let mut expected = outcome.applied.clone();
    expected.extend([via(outside[1]), via(outside[2])]);
    assert_eq!(outcome.routes, expected);
    assert_eq!(outcome.kept_through_other_gateways, 0);
    assert!(!outcome.needs_write);
}

#[test]
fn overlaps() {
    let wide = net("10.30.0.0/16");
    assert!(wide.overlaps(&network()) && network().overlaps(&wide));
    assert!(network().overlaps(&network()));
    assert!(!network().overlaps(&other()));
}

#[test]
fn the_local_network_comes_from_the_address_and_mask() {
    assert_eq!(
        fvm_core::bypass_rules::local_network(&snapshot(vec![])),
        Some(office())
    );
}
