//! Route planning for the exception rules.

use fvm_contract::app_helper::{BypassRoute, BypassRoutesSnapshot, Ipv4Network};
use fvm_contract::app_internal::section::BypassStatus;
use fvm_core::bypass_rules::{Applicability, RoutePlan, plan_routes, vpn_server_target};

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

fn via(network: Ipv4Network) -> BypassRoute {
    network.route_through(ROUTER)
}

fn via_gw(network: Ipv4Network, gateway: &str) -> BypassRoute {
    network.route_through(gateway)
}

/// The Mac at the office desk (10.20.161.20/24).
fn at_the_desk() -> BypassRoutesSnapshot {
    snapshot_at(ROUTER, "10.20.161.20", "255.255.255.0")
}

fn snapshot_at(router: &str, address: &str, mask: &str) -> BypassRoutesSnapshot {
    BypassRoutesSnapshot {
        service_id: "SERVICE-1".into(),
        service_name: "Ethernet".into(),
        router: router.into(),
        address: address.into(),
        subnet_mask: mask.into(),
    }
}

fn plan(targets: &[Ipv4Network], snapshot: &BypassRoutesSnapshot) -> RoutePlan {
    plan_routes(&[office()], targets, snapshot, true)
}

fn matched(plan: &RoutePlan) -> Option<Ipv4Network> {
    match plan.applicability {
        Applicability::Applies { network } => Some(network),
        _ => None,
    }
}

#[test]
fn on_a_listed_network_every_target_goes_through_the_router_in_list_order() {
    let outcome = plan(&[network(), host()], &at_the_desk());
    assert_eq!(matched(&outcome), Some(office()));
    assert_eq!(outcome.routes, vec![via(network()), via(host())]);
    assert!(outcome.on_link.is_empty());
    assert_eq!(
        outcome.status(&at_the_desk()),
        BypassStatus::Applied {
            service_name: "Ethernet".into(),
            router: ROUTER.into(),
            on_link_targets: 0,
        }
    );
}

#[test]
fn outside_the_networks_no_route_is_planned() {
    let home = snapshot_at("192.168.0.1", "192.168.0.12", "255.255.255.0");
    let outcome = plan(&[host(), network()], &home);
    assert_eq!(outcome.applicability, Applicability::OutsideNetworks);
    assert!(outcome.routes.is_empty());
    assert_eq!(outcome.status(&home), BypassStatus::OutsideNetworks {});
}

#[test]
fn without_networks_the_rules_apply_nowhere() {
    let outcome = plan_routes(&[], &[host()], &at_the_desk(), true);
    assert_eq!(outcome.applicability, Applicability::NoNetworksListed);
    assert!(outcome.routes.is_empty());
    assert_eq!(
        outcome.status(&at_the_desk()),
        BypassStatus::NoNetworksListed {}
    );
}

#[test]
fn another_listed_network_uses_its_own_router_and_skips_targets_on_the_same_lan() {
    let lan_router = "10.30.161.1";
    let on_the_lan = snapshot_at(lan_router, "10.30.161.50", "255.255.255.0");
    let outcome = plan_routes(
        &[office(), servers()],
        &[host(), network()],
        &on_the_lan,
        true,
    );
    assert_eq!(matched(&outcome), Some(servers()));
    assert_eq!(outcome.on_link, vec![host()]);
    assert_eq!(outcome.routes, vec![via_gw(network(), lan_router)]);
    assert_eq!(
        outcome.status(&on_the_lan),
        BypassStatus::Applied {
            service_name: "Ethernet".into(),
            router: lan_router.into(),
            on_link_targets: 1,
        }
    );
}

#[test]
fn a_target_on_this_macs_network_needs_no_route() {
    let desk_neighbour = net("10.20.161.16");
    let outcome = plan(&[desk_neighbour, host()], &at_the_desk());
    assert_eq!(outcome.on_link, vec![desk_neighbour]);
    assert_eq!(outcome.routes, vec![via(host())]);
}

#[test]
fn a_new_router_takes_every_route() {
    let new_router = "10.20.161.1";
    let outcome = plan(
        &[host(), network()],
        &snapshot_at(new_router, "10.20.161.20", "255.255.255.0"),
    );
    assert_eq!(
        outcome.routes,
        vec![via_gw(host(), new_router), via_gw(network(), new_router)]
    );
}

/// Without a VPN the routes mean nothing: none while the app does not want it up.
#[test]
fn without_the_vpn_wanted_no_route_is_planned() {
    let outcome = plan_routes(&[office()], &[host(), network()], &at_the_desk(), false);
    assert_eq!(outcome.applicability, Applicability::VpnOff);
    assert!(outcome.routes.is_empty());
    assert_eq!(outcome.status(&at_the_desk()), BypassStatus::VpnOff {});
    // Outside the networks the networks still tell.
    let home = snapshot_at("192.168.0.1", "192.168.0.12", "255.255.255.0");
    let outcome = plan_routes(&[office()], &[host()], &home, false);
    assert_eq!(outcome.applicability, Applicability::OutsideNetworks);
}

#[test]
fn only_a_host_that_is_the_vpn_servers_address_is_found() {
    let targets = [network(), host()];
    assert_eq!(vpn_server_target(&targets, "10.30.161.16"), Some(1));
    assert_eq!(vpn_server_target(&targets, "10.30.170.1"), None);
    assert_eq!(vpn_server_target(&targets, "vpn.example.com"), None);
    assert_eq!(vpn_server_target(&targets, ""), None);
}

#[test]
fn overlaps() {
    let wide = net("10.30.0.0/16");
    assert!(wide.overlaps(&network()) && network().overlaps(&wide));
    assert!(network().overlaps(&network()));
    assert!(!network().overlaps(&host()));
}

#[test]
fn the_local_network_comes_from_the_address_and_mask() {
    assert_eq!(
        fvm_core::bypass_rules::local_network(&at_the_desk()),
        Some(office())
    );
}
