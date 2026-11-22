pub(crate) mod interface;

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use fvm_contract::helper_internal::{
    IpNetwork, LogCategory, NetworkChange, NetworkOutcome, NetworkSettings, NetworkStep,
    OutsideChanges, OutsideRoute, TunnelRoutes,
};
use fvm_os_api_macos::log::Level;
use fvm_os_api_macos::network::{self, Entity};
use system_configuration::dynamic_store::{SCDynamicStore, SCDynamicStoreBuilder};

use crate::route_table::{self, Family, Route, Row};
use crate::{HELPER_ID, log, routes};

pub(crate) const SERVICE_ID: &str = "com.qoor.FortiVPNManager.Helper.VPN";

type Failure = (NetworkStep, String);

pub(crate) fn change(change: &NetworkChange) -> NetworkOutcome {
    match change {
        NetworkChange::LookUp { outside, .. } => look_up(outside),
        NetworkChange::Apply {
            settings,
            previous,
            remove,
            add,
            outside,
        } => apply(settings, previous.as_deref(), remove, add, outside, true),
        NetworkChange::Prepare { settings, add } => apply(settings, None, &[], add, &[], false),
        NetworkChange::Switch {
            settings,
            replaced,
            outside,
        } => switch_traffic(settings, replaced, outside),
        NetworkChange::Revert { changes } => revert(changes),
        NetworkChange::Check { settings, outside } => check(settings, outside),
    }
}

pub(crate) fn store() -> Option<SCDynamicStore> {
    SCDynamicStoreBuilder::new(HELPER_ID).build()
}

fn look_up(outside: &[IpNetwork]) -> NetworkOutcome {
    let Some((store, routers)) = store().and_then(|store| {
        let routers = primary_routers(&store)?;
        Some((store, routers))
    }) else {
        return failed(NetworkStep::Network, "no active network service", vec![]);
    };
    let outside_routes: Vec<OutsideRoute> = outside
        .iter()
        .filter_map(|destination| {
            Some(OutsideRoute {
                destination: *destination,
                router: routers.router(destination)?,
                service_id: routers.service_id.clone(),
            })
        })
        .collect();
    let families = outside_routes
        .iter()
        .filter_map(|route| Some(system_route(route, &store)?.family))
        .collect();
    let Some(table) = read_table(&families) else {
        return failed(
            NetworkStep::Network,
            "the routing table could not be read",
            vec![],
        );
    };
    let missing = outside_routes
        .into_iter()
        .filter(|route| is_in(Some(&table), route, &store) != Some(true))
        .collect();
    NetworkOutcome::LookedUp {
        dns_service_id: SERVICE_ID.to_owned(),
        missing,
    }
}

fn apply(
    settings: &NetworkSettings,
    previous: Option<&NetworkSettings>,
    remove: &[OutsideRoute],
    add: &[OutsideRoute],
    outside: &[IpNetwork],
    routes_and_dns: bool,
) -> NetworkOutcome {
    let Some(store) = store() else {
        return failed(
            NetworkStep::Network,
            "the network configuration could not be read",
            remove.to_vec(),
        );
    };
    if !add.is_empty() {
        let Some(routers) = primary_routers(&store) else {
            return failed(
                NetworkStep::Network,
                "no active network service",
                remove.to_vec(),
            );
        };
        if !add.iter().all(|route| {
            route.service_id == routers.service_id
                && routers.router(&route.destination) == Some(route.router)
        }) {
            return failed(
                NetworkStep::Network,
                "the network changed since it was looked up; look it up again",
                remove.to_vec(),
            );
        }
    }
    let wanted = if routes_and_dns {
        tunnel_routes(settings)
    } else {
        vec![]
    };
    let stale: Vec<Route> = previous
        .map(|previous| {
            tunnel_routes(previous)
                .into_iter()
                .filter(|route| !wanted.contains(route))
                .collect()
        })
        .unwrap_or_default();
    let families: HashSet<Family> = wanted
        .iter()
        .chain(&stale)
        .map(|route| route.family)
        .chain(
            remove
                .iter()
                .chain(add)
                .filter_map(|route| Some(system_route(route, &store)?.family)),
        )
        .collect();
    let Some(before) = read_table(&families) else {
        return failed(
            NetworkStep::Network,
            "the routing table could not be read",
            remove.to_vec(),
        );
    };
    let name = &settings.tunnel.name;
    let mut tried: Vec<OutsideRoute> = Vec::new();
    let finish = |failure: Option<Failure>, table: Option<&[Row]>, tried: &[OutsideRoute]| {
        let added = tried
            .iter()
            .filter(|route| is_in(table, route, &store).unwrap_or(true))
            .cloned()
            .collect();
        let left = remove
            .iter()
            .filter(|route| is_in(table, route, &store).unwrap_or(true))
            .cloned()
            .collect();
        let Some((step, error)) = failure else {
            log(
                LogCategory::Routes,
                Level::Notice,
                &format!("applied the VPN network settings to {name}"),
            );
            return NetworkOutcome::Applied { added, left };
        };
        log(
            LogCategory::Routes,
            Level::Error,
            &format!(
                "applying the VPN network settings to {name} failed at {}: {error}",
                step_name(step)
            ),
        );
        NetworkOutcome::Failed {
            step,
            error,
            added,
            left,
        }
    };
    let failure = |step: NetworkStep, error: String, tried: &[OutsideRoute]| {
        finish(Some((step, error)), read_table(&families).as_deref(), tried)
    };

    for route in stale.iter().filter(|route| has(&before, route)) {
        route_table::delete(route);
    }
    if let Some(previous) = previous {
        for arguments in interface::removals(previous, settings) {
            interface::change(name, &arguments);
        }
    }
    for route in remove {
        if let Some(system) = system_route(route, &store)
            && has(&before, &system)
        {
            route_table::delete(&system);
        }
    }

    let Some(current) = interface::read(name) else {
        return failure(
            NetworkStep::Addresses,
            format!("the tunnel {name} could not be read"),
            &tried,
        );
    };
    for (step, arguments) in interface::changes(settings, &current) {
        if !interface::change(name, &arguments) {
            return failure(
                step,
                format!("ifconfig {name} {} failed", arguments.join(" ")),
                &tried,
            );
        }
    }
    for route in add {
        let Some(system) = system_route(route, &store) else {
            return failure(
                NetworkStep::OutsideRoutes,
                format!(
                    "the network service {} has no IPv6 interface",
                    route.service_id
                ),
                &tried,
            );
        };
        if has(&before, &system) || tried.contains(route) {
            continue;
        }
        tried.push(route.clone());
        if !route_table::add_replacing_copies(&system, &before) {
            return failure(
                NetworkStep::OutsideRoutes,
                format!("route -n {} failed", system.add_arguments().join(" ")),
                &tried,
            );
        }
    }
    for route in wanted.iter().filter(|route| !has(&before, route)) {
        if !route_table::add_replacing_copies(route, &before) {
            return failure(
                NetworkStep::Routes,
                format!("route -n {} failed", route.add_arguments().join(" ")),
                &tried,
            );
        }
    }
    if routes_and_dns && !write_dns(settings, &store) {
        return failure(
            NetworkStep::Dns,
            "the DNS settings could not be written".to_owned(),
            &tried,
        );
    }

    let Some(current) = interface::read(name) else {
        return failure(
            NetworkStep::Addresses,
            format!("the tunnel {name} could not be read"),
            &tried,
        );
    };
    if let Some(step) = current.mismatch(settings, previous) {
        let what = if step == NetworkStep::Mtu {
            "MTU"
        } else {
            "addresses"
        };
        return failure(
            step,
            format!("the tunnel {name} does not have its {what} after ifconfig"),
            &tried,
        );
    }
    let Some(after) = read_table(&families) else {
        return finish(
            Some((
                NetworkStep::OutsideRoutes,
                "the routing table could not be read".to_owned(),
            )),
            None,
            &tried,
        );
    };
    let missing: Vec<String> = add
        .iter()
        .filter(|route| is_in(Some(&after), route, &store) != Some(true))
        .map(|route| {
            format!(
                "{}/{}",
                route.destination.address, route.destination.prefix_length
            )
        })
        .collect();
    if !missing.is_empty() {
        return finish(
            Some((
                NetworkStep::OutsideRoutes,
                format!("the routing table lacks {}", missing.join(", ")),
            )),
            Some(&after),
            &tried,
        );
    }
    let wrong: Vec<&Route> = wanted
        .iter()
        .filter(|route| !has(&after, route))
        .chain(stale.iter().filter(|route| has(&after, route)))
        .collect();
    if !wrong.is_empty() {
        return finish(
            Some((
                NetworkStep::Routes,
                format!("the routing table lacks or still has {}", describe(&wrong)),
            )),
            Some(&after),
            &tried,
        );
    }
    if routes_and_dns && !has_dns(settings, &store) {
        return finish(
            Some((
                NetworkStep::Dns,
                "the DNS settings were not kept".to_owned(),
            )),
            Some(&after),
            &tried,
        );
    }
    if !routes_and_dns {
        return finish(None, Some(&after), &tried);
    }
    let Some(owners) = interface::owners(interface::ipv4_address(settings)) else {
        return finish(
            Some((
                NetworkStep::Addresses,
                "the interface addresses could not be read".to_owned(),
            )),
            Some(&after),
            &tried,
        );
    };
    finish(
        routing_problem(settings, outside, &after, &owners),
        Some(&after),
        &tried,
    )
}

fn switch_traffic(
    settings: &NetworkSettings,
    replaced: &NetworkSettings,
    outside: &[IpNetwork],
) -> NetworkOutcome {
    let Some(store) = store() else {
        return failed(
            NetworkStep::Network,
            "the network configuration could not be read",
            vec![],
        );
    };
    let wanted = tunnel_routes(settings);
    let old = tunnel_routes(replaced);
    let families = wanted
        .iter()
        .chain(&old)
        .map(|route| route.family)
        .collect();
    let Some(before) = read_table(&families) else {
        return failed(
            NetworkStep::Routes,
            "the routing table could not be read",
            vec![],
        );
    };
    for step in switch_steps(&wanted, &old, &before) {
        let done = match &step {
            SwitchStep::Move(route) => route_table::change(&route.change_arguments()),
            SwitchStep::Add(route) => route_table::add_replacing_copies(route, &before),
            SwitchStep::Delete(route) => route_table::delete(route),
        };
        if !done {
            return failed(
                NetworkStep::Routes,
                &format!("route -n {} failed", step.arguments().join(" ")),
                vec![],
            );
        }
    }
    if !write_dns(settings, &store) {
        return failed(
            NetworkStep::Dns,
            "the DNS settings could not be written",
            vec![],
        );
    }
    let Some(after) = read_table(&families) else {
        return failed(
            NetworkStep::Routes,
            "the routing table could not be read",
            vec![],
        );
    };
    let wrong: Vec<&Route> = wanted
        .iter()
        .filter(|route| !has(&after, route))
        .chain(
            old.iter()
                .filter(|route| !wanted.contains(route) && has(&after, route)),
        )
        .collect();
    if !wrong.is_empty() {
        return failed(
            NetworkStep::Routes,
            &format!("the routing table lacks or still has {}", describe(&wrong)),
            vec![],
        );
    }
    if !has_dns(settings, &store) {
        return failed(NetworkStep::Dns, "the DNS settings were not kept", vec![]);
    }
    let Some(owners) = interface::owners(interface::ipv4_address(settings)) else {
        return failed(
            NetworkStep::Addresses,
            "the interface addresses could not be read",
            vec![],
        );
    };
    if let Some((step, error)) = routing_problem(settings, outside, &after, &owners) {
        return failed(step, &error, vec![]);
    }
    log(
        LogCategory::Routes,
        Level::Notice,
        &format!(
            "moved the VPN traffic from {} to {}",
            replaced.tunnel.name, settings.tunnel.name
        ),
    );
    NetworkOutcome::Applied {
        added: vec![],
        left: vec![],
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum SwitchStep {
    Move(Route),
    Add(Route),
    Delete(Route),
}

impl SwitchStep {
    fn arguments(&self) -> Vec<String> {
        match self {
            Self::Move(route) => route.change_arguments(),
            Self::Add(route) => route.add_arguments(),
            Self::Delete(route) => route.delete_arguments(),
        }
    }
}

pub(crate) fn switch_steps(wanted: &[Route], replaced: &[Route], table: &[Row]) -> Vec<SwitchStep> {
    let same_destination = |one: &Route, other: &Route| {
        one.family == other.family
            && one.destination == other.destination
            && one.prefix_length == other.prefix_length
    };
    let present: Vec<&Route> = replaced.iter().filter(|route| has(table, route)).collect();
    let moves = wanted
        .iter()
        .filter(|route| !has(table, route))
        .map(|route| {
            if present.iter().any(|old| same_destination(old, route)) {
                SwitchStep::Move(route.clone())
            } else {
                SwitchStep::Add(route.clone())
            }
        });
    let deletes = present
        .iter()
        .filter(|old| !wanted.iter().any(|route| same_destination(old, route)))
        .map(|old| SwitchStep::Delete((*old).clone()));
    moves.chain(deletes).collect()
}

fn check(settings: &NetworkSettings, outside: &[IpNetwork]) -> NetworkOutcome {
    let name = &settings.tunnel.name;
    let families = tunnel_routes(settings)
        .iter()
        .map(|route| route.family)
        .collect();
    let Some((table, owners)) = read_table(&families).and_then(|table| {
        let owners = interface::owners(interface::ipv4_address(settings))?;
        Some((table, owners))
    }) else {
        let error = "the routing table or the interface addresses could not be read";
        log(
            LogCategory::Routes,
            Level::Error,
            &format!("checking the VPN routing of {name} failed: {error}"),
        );
        return NetworkOutcome::Failed {
            step: NetworkStep::Routes,
            error: error.to_owned(),
            added: vec![],
            left: vec![],
        };
    };
    let Some((_, problem)) = routing_problem(settings, outside, &table, &owners) else {
        return NetworkOutcome::Checked { complete: true };
    };
    log(
        LogCategory::Routes,
        Level::Error,
        &format!("the VPN routing of {name} is not complete: {problem}"),
    );
    NetworkOutcome::Checked { complete: false }
}

pub(crate) fn routing_problem(
    settings: &NetworkSettings,
    outside: &[IpNetwork],
    table: &[Row],
    owners: &[String],
) -> Option<Failure> {
    let name = &settings.tunnel.name;
    if owners != [name.as_str()] {
        let holders = if owners.is_empty() {
            "no interface".to_owned()
        } else {
            owners.join(", ")
        };
        return Some((
            NetworkStep::Addresses,
            format!(
                "the tunnel address {} is on {holders}, not only on {name}",
                settings.ipv4.address.address
            ),
        ));
    }
    let unscoped: Vec<&Row> = table
        .iter()
        .filter(|row| !row.flags.contains('I'))
        .collect();
    for route in tunnel_routes(settings) {
        let first = text_bits(&route.destination, route.prefix_length)
            .unwrap_or_else(|| panic!("the tunnel route to {} has no address", route.destination));
        let chosen = unscoped
            .iter()
            .filter(|row| {
                text_bits(&row.destination, row.prefix_length)
                    == Some(address_bits(&first, row.prefix_length))
            })
            .fold(None::<&&Row>, |chosen, row| match chosen {
                Some(chosen) if chosen.prefix_length >= row.prefix_length => Some(chosen),
                _ => Some(row),
            });
        let Some(chosen) = chosen else {
            return Some((
                NetworkStep::Routes,
                format!(
                    "no route covers {}/{}",
                    route.destination, route.prefix_length
                ),
            ));
        };
        let kept = outside.iter().any(|network| {
            network.prefix_length == chosen.prefix_length
                && Some(address_bits(&bytes(network.address), chosen.prefix_length))
                    == text_bits(&chosen.destination, chosen.prefix_length)
        });
        if chosen.interface == *name || kept {
            continue;
        }
        return Some((
            NetworkStep::Routes,
            format!(
                "the route for {}/{} goes through {} ({}/{}), not {name}",
                route.destination,
                route.prefix_length,
                chosen.interface,
                chosen.destination,
                chosen.prefix_length
            ),
        ));
    }
    None
}

fn bytes(address: IpAddr) -> Vec<u8> {
    match address {
        IpAddr::V4(address) => address.octets().to_vec(),
        IpAddr::V6(address) => address.octets().to_vec(),
    }
}

fn text_bits(address: &str, prefix_length: u8) -> Option<Vec<u8>> {
    let address = address
        .parse::<Ipv4Addr>()
        .map(IpAddr::V4)
        .or_else(|_| address.parse::<Ipv6Addr>().map(IpAddr::V6))
        .ok()?;
    Some(address_bits(&bytes(address), prefix_length))
}

fn address_bits(bytes: &[u8], prefix_length: u8) -> Vec<u8> {
    bytes
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            let kept = (i32::from(prefix_length)
                - i32::try_from(index).expect("an address has at most 16 bytes") * 8)
                .clamp(0, 8);
            if kept == 8 {
                *byte
            } else {
                byte & !(u8::MAX >> kept)
            }
        })
        .collect()
}

fn revert(changes: &OutsideChanges) -> NetworkOutcome {
    let Some(store) = store() else {
        return failed(
            NetworkStep::Dns,
            "the network configuration could not be read",
            changes.routes.clone(),
        );
    };
    let keys = [Entity::Dns, Entity::Ipv4]
        .map(|entity| network::service_state_key(&changes.dns_service_id, entity));
    for key in &keys {
        store.remove(key.as_str());
    }
    let families = changes
        .routes
        .iter()
        .filter_map(|route| Some(system_route(route, &store)?.family))
        .collect();
    let before = read_table(&families);
    let mut reverted = false;
    for route in &changes.routes {
        if let Some(system) = system_route(route, &store)
            && before.as_ref().is_some_and(|before| has(before, &system))
        {
            route_table::delete(&system);
            reverted = true;
        }
    }
    let after = if reverted {
        read_table(&families)
    } else {
        before
    };
    let left: Vec<OutsideRoute> = changes
        .routes
        .iter()
        .filter(|route| is_in(after.as_deref(), route, &store).unwrap_or(true))
        .cloned()
        .collect();
    if !keys.iter().all(|key| store.get(key.as_str()).is_none()) {
        return failed(
            NetworkStep::Dns,
            &format!(
                "the DNS settings under {} could not be removed",
                changes.dns_service_id
            ),
            left,
        );
    }
    if after.is_none() {
        return failed(
            NetworkStep::OutsideRoutes,
            "the routing table could not be read",
            left,
        );
    }
    log(
        LogCategory::Routes,
        Level::Notice,
        &format!(
            "reverted the VPN network settings; {} route(s) left",
            left.len()
        ),
    );
    NetworkOutcome::Reverted { left }
}

fn failed(step: NetworkStep, error: &str, left: Vec<OutsideRoute>) -> NetworkOutcome {
    log(
        LogCategory::Routes,
        Level::Error,
        &format!(
            "the VPN network change failed at {}: {error}",
            step_name(step)
        ),
    );
    NetworkOutcome::Failed {
        step,
        error: error.to_owned(),
        added: vec![],
        left,
    }
}

fn step_name(step: NetworkStep) -> &'static str {
    match step {
        NetworkStep::Network => "network",
        NetworkStep::Addresses => "addresses",
        NetworkStep::Mtu => "mtu",
        NetworkStep::Routes => "routes",
        NetworkStep::OutsideRoutes => "outsideRoutes",
        NetworkStep::Dns => "dns",
        NetworkStep::Wins => "wins",
    }
}

struct Routers {
    service_id: String,
    ipv4: Ipv4Addr,
    ipv6: Option<Ipv6Addr>,
}

impl Routers {
    fn router(&self, destination: &IpNetwork) -> Option<IpAddr> {
        if destination.address.is_ipv4() {
            Some(IpAddr::V4(self.ipv4))
        } else {
            self.ipv6.map(IpAddr::V6)
        }
    }
}

fn primary_routers(store: &SCDynamicStore) -> Option<Routers> {
    let service_id = routes::primary_service_id_in(store)?;
    let ipv4 = service_text(store, &service_id, Entity::Ipv4, network::ROUTER)?
        .parse()
        .ok()?;
    let ipv6 = service_text(store, &service_id, Entity::Ipv6, network::ROUTER)
        .and_then(|router| router.parse().ok());
    Some(Routers {
        service_id,
        ipv4,
        ipv6,
    })
}

fn service_text(
    store: &SCDynamicStore,
    service_id: &str,
    entity: Entity,
    key: &str,
) -> Option<String> {
    let state = network::dictionary(store, &network::service_state_key(service_id, entity))?;
    routes::text(&state, key)
}

fn read_table(families: &HashSet<Family>) -> Option<Vec<Row>> {
    let mut rows = Vec::new();
    for family in [Family::Inet, Family::Inet6] {
        if families.contains(&family) {
            rows.extend(route_table::read(family)?);
        }
    }
    Some(rows)
}

fn has(table: &[Row], route: &Route) -> bool {
    table.iter().any(|row| route.matches(row))
}

fn is_in(table: Option<&[Row]>, route: &OutsideRoute, store: &SCDynamicStore) -> Option<bool> {
    let table = table?;
    let system = system_route(route, store)?;
    Some(has(table, &system))
}

fn system_route(route: &OutsideRoute, store: &SCDynamicStore) -> Option<Route> {
    if !is_link_local(route.router) {
        return Some(table_route(&route.destination, &route.router.to_string()));
    }
    let interface = service_text(
        store,
        &route.service_id,
        Entity::Ipv6,
        network::INTERFACE_NAME,
    )?;
    Some(table_route(
        &route.destination,
        &format!("{}%{interface}", route.router),
    ))
}

pub(crate) fn is_link_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(_) => false,
        IpAddr::V6(address) => address.segments()[0] & 0xffc0 == 0xfe80,
    }
}

pub(crate) fn tunnel_routes(settings: &NetworkSettings) -> Vec<Route> {
    let ipv4 = &settings.ipv4.address;
    let IpAddr::V4(address) = ipv4.address else {
        panic!(
            "the Helper entry sent {} as the tunnel's IPv4 address",
            ipv4.address
        );
    };
    let gateway = address.to_string();
    let mut routes = Vec::new();
    if ipv4.prefix_length < 32 {
        let mask = u32::MAX
            .checked_shl(32 - u32::from(ipv4.prefix_length))
            .unwrap_or(0);
        let network = IpNetwork {
            address: IpAddr::V4(Ipv4Addr::from(u32::from(address) & mask)),
            prefix_length: ipv4.prefix_length,
        };
        routes.push(table_route(&network, &gateway));
    }
    routes.extend(
        destinations(
            &settings.ipv4.routes,
            [
                Ipv4Addr::UNSPECIFIED.into(),
                Ipv4Addr::new(128, 0, 0, 0).into(),
            ],
        )
        .iter()
        .map(|destination| table_route(destination, &gateway)),
    );
    if let Some(ipv6) = &settings.ipv6 {
        let gateway = ipv6.address.address.to_string();
        routes.extend(
            destinations(
                &ipv6.routes,
                [
                    Ipv6Addr::UNSPECIFIED.into(),
                    Ipv6Addr::new(0x8000, 0, 0, 0, 0, 0, 0, 0).into(),
                ],
            )
            .iter()
            .map(|destination| table_route(destination, &gateway)),
        );
    }
    let mut unique: Vec<Route> = Vec::new();
    for route in routes {
        if !unique.contains(&route) {
            unique.push(route);
        }
    }
    unique
}

fn destinations(routes: &TunnelRoutes, full: [IpAddr; 2]) -> Vec<IpNetwork> {
    match routes {
        TunnelRoutes::Full {} => full
            .map(|address| IpNetwork {
                address,
                prefix_length: 1,
            })
            .to_vec(),
        TunnelRoutes::Split { include } => include.clone(),
    }
}

fn table_route(destination: &IpNetwork, gateway: &str) -> Route {
    Route::new(
        &destination.address.to_string(),
        destination.prefix_length,
        gateway,
    )
    .unwrap_or_else(|| {
        panic!(
            "the Helper entry sent {}/{} through {gateway}, which is not a route",
            destination.address, destination.prefix_length
        )
    })
}

fn describe(routes: &[&Route]) -> String {
    routes
        .iter()
        .map(|route| {
            format!(
                "{}/{} through {}",
                route.destination, route.prefix_length, route.gateway
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) struct DnsEntries {
    pub dns: Option<CFDictionary<CFString, CFType>>,
    pub ipv4: Option<CFDictionary<CFString, CFType>>,
}

pub(crate) fn dns_entries(settings: &NetworkSettings) -> DnsEntries {
    if settings.dns_servers.is_empty() {
        return DnsEntries {
            dns: None,
            ipv4: None,
        };
    }
    let strings = |values: &[String]| {
        let values: Vec<CFString> = values.iter().map(|value| CFString::new(value)).collect();
        CFArray::from_CFTypes(&values).as_CFType()
    };
    let servers: Vec<String> = settings
        .dns_servers
        .iter()
        .map(ToString::to_string)
        .collect();
    let mut dns = vec![(
        CFString::new(network::DNS_SERVER_ADDRESSES),
        strings(&servers),
    )];
    if !settings.default_domains.is_empty() {
        dns.push((
            CFString::new(network::DNS_SEARCH_DOMAINS),
            strings(&settings.default_domains),
        ));
    }
    let mut match_domains: Vec<String> = Vec::new();
    for domain in settings
        .default_domains
        .iter()
        .chain(&settings.split_dns_domains)
    {
        if !match_domains.contains(domain) {
            match_domains.push(domain.clone());
        }
    }
    if !match_domains.is_empty() {
        dns.push((
            CFString::new(network::DNS_SUPPLEMENTAL_MATCH_DOMAINS),
            strings(&match_domains),
        ));
    }
    let dns = Some(CFDictionary::from_CFType_pairs(&dns));
    let TunnelRoutes::Full {} = settings.ipv4.routes else {
        return DnsEntries { dns, ipv4: None };
    };
    let address = settings.ipv4.address.address.to_string();
    let ipv4 = CFDictionary::from_CFType_pairs(&[
        (
            CFString::new(network::ADDRESSES),
            strings(std::slice::from_ref(&address)),
        ),
        (
            CFString::new(network::SUBNET_MASKS),
            strings(&["255.255.255.255".to_owned()]),
        ),
        (
            CFString::new(network::ROUTER),
            CFString::new(&address).as_CFType(),
        ),
        (
            CFString::new(network::INTERFACE_NAME),
            CFString::new(&settings.tunnel.name).as_CFType(),
        ),
        (
            CFString::new(network::OVERRIDE_PRIMARY),
            CFNumber::from(1).as_CFType(),
        ),
    ]);
    DnsEntries {
        dns,
        ipv4: Some(ipv4),
    }
}

fn dns_keys(settings: &NetworkSettings) -> [(String, Option<CFDictionary<CFString, CFType>>); 2] {
    let entries = dns_entries(settings);
    [
        (
            network::service_state_key(SERVICE_ID, Entity::Dns),
            entries.dns,
        ),
        (
            network::service_state_key(SERVICE_ID, Entity::Ipv4),
            entries.ipv4,
        ),
    ]
}

fn write_dns(settings: &NetworkSettings, store: &SCDynamicStore) -> bool {
    for (key, value) in dns_keys(settings) {
        match value {
            Some(value) => {
                if !store.set(key.as_str(), value.to_untyped()) {
                    return false;
                }
            }
            None => {
                store.remove(key.as_str());
            }
        }
    }
    true
}

fn has_dns(settings: &NetworkSettings, store: &SCDynamicStore) -> bool {
    dns_keys(settings).into_iter().all(|(key, value)| {
        let stored = store
            .get(key.as_str())
            .and_then(core_foundation::propertylist::CFPropertyList::downcast_into::<CFDictionary>);
        match (stored, value) {
            (None, None) => true,
            (Some(stored), Some(value)) => stored.as_CFType() == value.as_CFType(),
            _ => false,
        }
    })
}

#[cfg(test)]
mod tests;
