use std::env;
use std::fs::File;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;
use std::time::Duration;

use fvm_contract::helper_internal::{
    FamilySettings, IpNetwork, LogCategory, LogLevel, NetworkChange, NetworkOutcome,
    NetworkSettings, NetworkStep, OutsideChanges, OutsideRoute, TunnelDevice, TunnelRoutes,
};
use fvm_os_api_windows::network::{self, DadState, Family, Luid, Route};
use fvm_os_api_windows::process;

use crate::Log;
use crate::routes::{self, BYPASS_ROUTE_METRIC};

const NETSH: &str = r"C:\Windows\System32\netsh.exe";
const NETSH_LIMIT: Duration = Duration::from_secs(10);
const FULL_TUNNEL_INTERFACE_METRIC: u32 = 1;
const TUNNEL_ROUTE_METRIC: u32 = 0;
const IPV6_FULL_TUNNEL: (Ipv6Addr, u8) = (Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3);

struct Failure {
    step: NetworkStep,
    error: String,
}

fn failure(step: NetworkStep, error: &(impl ToString + ?Sized)) -> Failure {
    Failure {
        step,
        error: error.to_string(),
    }
}

pub(crate) fn change(change: &NetworkChange, log: &Log) -> NetworkOutcome {
    let outcome = match change {
        NetworkChange::LookUp { tunnel, outside } => {
            look_up(tunnel, outside, log).unwrap_or_else(|failure| NetworkOutcome::Failed {
                step: failure.step,
                error: failure.error,
                added: Vec::new(),
                left: Vec::new(),
            })
        }
        NetworkChange::Apply {
            settings,
            previous,
            remove,
            add,
            outside,
        } => apply(
            settings,
            previous.as_deref(),
            remove,
            add,
            true,
            Some(outside),
            log,
        ),
        NetworkChange::Prepare { settings, add } => {
            apply(settings, None, &[], add, false, None, log)
        }
        NetworkChange::Switch {
            settings,
            replaced,
            outside,
        } => switch(settings, replaced, outside, log),
        NetworkChange::Revert { changes } => revert(changes, log),
        NetworkChange::Check { settings, outside } => return check(settings, outside, log),
    };
    if let NetworkOutcome::Failed { step, error, .. } = &outcome {
        let message = format!("changing the VPN network settings failed ({step:?}): {error}");
        log(LogLevel::Error, LogCategory::Routes, &message);
    }
    outcome
}

fn tunnel_interface(tunnel: &TunnelDevice) -> Result<Luid, Failure> {
    let index = tunnel.index.ok_or_else(|| {
        failure(
            NetworkStep::Network,
            &format!("the tunnel {} has no interface index", tunnel.name),
        )
    })?;
    Luid::from_index(index).map_err(|error| failure(NetworkStep::Network, &error))
}

struct Physical {
    luid: Luid,
    service_id: String,
    ipv4_router: IpAddr,
    ipv6_router: Option<IpAddr>,
}

impl Physical {
    fn read(log: &Log) -> Result<Self, Failure> {
        let table =
            network::routes(Family::Ipv4).map_err(|error| failure(NetworkStep::Network, &error))?;
        let primary = routes::primary(&table, log)
            .map_err(|error| failure(NetworkStep::Network, &error.message))?;
        let ipv4_router = primary
            .network
            .router
            .parse()
            .expect("the primary network's router is an IPv4 address");
        let ipv6_router = network::routes(Family::Ipv6)
            .map_err(|error| failure(NetworkStep::Network, &error))?
            .into_iter()
            .filter(|route| {
                route.interface == primary.luid
                    && route.prefix_length == 0
                    && route.destination.is_unspecified()
                    && !route.next_hop.is_unspecified()
            })
            .min_by_key(|route| route.metric)
            .map(|route| route.next_hop);
        Ok(Self {
            luid: primary.luid,
            service_id: primary.network.service_id,
            ipv4_router,
            ipv6_router,
        })
    }

    fn route_to(&self, destination: IpNetwork) -> Option<OutsideRoute> {
        let router = match destination.address {
            IpAddr::V4(_) => Some(self.ipv4_router),
            IpAddr::V6(_) => self.ipv6_router,
        }?;
        Some(OutsideRoute {
            destination,
            router,
            service_id: self.service_id.clone(),
        })
    }

    fn os_route(&self, route: &OutsideRoute) -> Route {
        Route {
            interface: self.luid,
            destination: route.destination.address,
            prefix_length: route.destination.prefix_length,
            next_hop: route.router,
            metric: BYPASS_ROUTE_METRIC,
        }
    }
}

fn table(
    destinations: impl IntoIterator<Item = IpAddr>,
) -> Result<Vec<Route>, fvm_os_api_windows::Error> {
    let families: Vec<Family> = destinations.into_iter().map(Family::of).collect();
    let mut table = Vec::new();
    for family in [Family::Ipv4, Family::Ipv6] {
        if families.contains(&family) {
            table.extend(network::routes(family)?);
        }
    }
    Ok(table)
}

fn found(table: &[Route], route: &OutsideRoute) -> Option<Route> {
    table.iter().copied().find(|row| {
        row.destination == route.destination.address
            && row.prefix_length == route.destination.prefix_length
            && row.next_hop == route.router
            && network::interface(row.interface)
                .is_ok_and(|interface| interface.guid == route.service_id)
    })
}

fn look_up(
    tunnel: &TunnelDevice,
    outside: &[IpNetwork],
    log: &Log,
) -> Result<NetworkOutcome, Failure> {
    let tunnel = tunnel_interface(tunnel)?;
    let dns_service_id = network::interface(tunnel)
        .map_err(|error| failure(NetworkStep::Network, &error))?
        .guid;
    let physical = Physical::read(log)?;
    let routes: Vec<OutsideRoute> = outside
        .iter()
        .filter_map(|destination| physical.route_to(*destination))
        .collect();
    let table = table(routes.iter().map(|route| route.destination.address))
        .map_err(|error| failure(NetworkStep::Network, &error))?;
    let missing = routes
        .into_iter()
        .filter(|route| found(&table, route).is_none())
        .collect();
    Ok(NetworkOutcome::LookedUp {
        dns_service_id,
        missing,
    })
}

fn apply(
    settings: &NetworkSettings,
    previous: Option<&NetworkSettings>,
    remove: &[OutsideRoute],
    add: &[OutsideRoute],
    routes_and_dns: bool,
    outside: Option<&[IpNetwork]>,
    log: &Log,
) -> NetworkOutcome {
    let mut added = Vec::new();
    let mut unchanged = None;
    match apply_steps(
        settings,
        previous,
        remove,
        add,
        routes_and_dns,
        outside,
        &mut added,
        &mut unchanged,
        log,
    ) {
        Ok(left) => {
            let message = format!(
                "applied the VPN network settings to {} ({} route(s) outside the tunnel added)",
                settings.tunnel.name,
                added.len()
            );
            log(LogLevel::Notice, LogCategory::Routes, &message);
            NetworkOutcome::Applied { added, left }
        }
        Err(failure) => {
            let table = unchanged
                .or_else(|| table(remove.iter().map(|route| route.destination.address)).ok());
            let left = remove
                .iter()
                .filter(|route| {
                    table
                        .as_ref()
                        .is_none_or(|table| found(table, route).is_some())
                })
                .cloned()
                .collect();
            NetworkOutcome::Failed {
                step: failure.step,
                error: failure.error,
                added,
                left,
            }
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn apply_steps(
    settings: &NetworkSettings,
    previous: Option<&NetworkSettings>,
    remove: &[OutsideRoute],
    add: &[OutsideRoute],
    routes_and_dns: bool,
    outside: Option<&[IpNetwork]>,
    added: &mut Vec<OutsideRoute>,
    unchanged: &mut Option<Vec<Route>>,
    log: &Log,
) -> Result<Vec<OutsideRoute>, Failure> {
    let tunnel = tunnel_interface(&settings.tunnel)?;
    let physical = Physical::read(log)?;
    if add
        .iter()
        .any(|route| physical.route_to(route.destination).as_ref() != Some(route))
    {
        return Err(failure(
            NetworkStep::Network,
            "the network changed since it was looked up",
        ));
    }
    if let Some(previous) = previous {
        remove_previous(tunnel, previous, settings)?;
    }
    let before = table(remove.iter().map(|route| route.destination.address))
        .map_err(|error| failure(NetworkStep::OutsideRoutes, &error))?;
    let rows: Vec<Route> = remove
        .iter()
        .filter_map(|route| found(&before, route))
        .collect();
    *unchanged = Some(before);
    for row in rows {
        network::delete_route(&row).map_err(|error| failure(NetworkStep::OutsideRoutes, &error))?;
        *unchanged = None;
    }
    if let Some(mtu) = settings.mtu {
        let families: &[Family] = if settings.ipv6.is_some() {
            &[Family::Ipv4, Family::Ipv6]
        } else {
            &[Family::Ipv4]
        };
        for &family in families {
            network::set_mtu(tunnel, family, mtu)
                .map_err(|error| failure(NetworkStep::Mtu, &error))?;
        }
    }
    for address in addresses(settings) {
        match network::add_address(tunnel, address.address, address.prefix_length) {
            Ok(()) => {}
            Err(error) if error.already_exists() => {}
            Err(error) => return Err(failure(NetworkStep::Addresses, &error)),
        }
    }
    for route in add {
        match network::add_route(&physical.os_route(route)) {
            Ok(()) => added.push(route.clone()),
            Err(error) if error.already_exists() => {}
            Err(error) => return Err(failure(NetworkStep::OutsideRoutes, &error)),
        }
    }
    if !routes_and_dns {
        confirm(tunnel, settings, &physical, add, false)?;
        return Ok(Vec::new());
    }
    if matches!(settings.ipv4.routes, TunnelRoutes::Full {}) {
        network::set_metric(tunnel, Family::Ipv4, Some(FULL_TUNNEL_INTERFACE_METRIC))
            .map_err(|error| failure(NetworkStep::Routes, &error))?;
    }
    for route in tunnel_routes(tunnel, settings) {
        match network::add_route(&route) {
            Ok(()) => {}
            Err(error) if error.already_exists() => {}
            Err(error) => return Err(failure(NetworkStep::Routes, &error)),
        }
    }
    let guid = network::interface(tunnel)
        .map_err(|error| failure(NetworkStep::Dns, &error))?
        .guid;
    for family in [Family::Ipv4, Family::Ipv6] {
        let servers = dns_servers(settings, family);
        let had = previous.is_some_and(|previous| !dns_servers(previous, family).is_empty());
        if !servers.is_empty() || had {
            network::set_dns_servers(&guid, family, &servers)
                .map_err(|error| failure(NetworkStep::Dns, &error))?;
        }
    }
    let previous_wins = previous.map_or(&[][..], |previous| &previous.wins_servers[..]);
    if settings.wins_servers != previous_wins {
        set_wins_servers(tunnel, previous_wins, &settings.wins_servers)?;
    }
    confirm(tunnel, settings, &physical, add, true)?;
    if let Some(outside) = outside {
        routing(settings, tunnel, outside)?;
    }
    let after = table(remove.iter().map(|route| route.destination.address))
        .map_err(|error| failure(NetworkStep::OutsideRoutes, &error))?;
    Ok(remove
        .iter()
        .filter(|route| found(&after, route).is_some())
        .cloned()
        .collect())
}

fn addresses(settings: &NetworkSettings) -> Vec<IpNetwork> {
    [Some(&settings.ipv4), settings.ipv6.as_ref()]
        .into_iter()
        .flatten()
        .map(|family| family.address)
        .collect()
}

fn tunnel_routes(tunnel: Luid, settings: &NetworkSettings) -> Vec<Route> {
    let on_link = |destination: IpAddr, prefix_length: u8| Route {
        interface: tunnel,
        destination,
        prefix_length,
        next_hop: match destination {
            IpAddr::V4(_) => Ipv4Addr::UNSPECIFIED.into(),
            IpAddr::V6(_) => Ipv6Addr::UNSPECIFIED.into(),
        },
        metric: TUNNEL_ROUTE_METRIC,
    };
    let family_routes = |family: &FamilySettings, full: (IpAddr, u8)| match &family.routes {
        TunnelRoutes::Full {} => vec![on_link(full.0, full.1)],
        TunnelRoutes::Split { include } => include
            .iter()
            .map(|network| on_link(network.address, network.prefix_length))
            .collect(),
    };
    let mut routes = family_routes(&settings.ipv4, (Ipv4Addr::UNSPECIFIED.into(), 0));
    if let Some(ipv6) = &settings.ipv6 {
        routes.extend(family_routes(
            ipv6,
            (IPV6_FULL_TUNNEL.0.into(), IPV6_FULL_TUNNEL.1),
        ));
    }
    routes
}

fn dns_servers(settings: &NetworkSettings, family: Family) -> Vec<IpAddr> {
    settings
        .dns_servers
        .iter()
        .copied()
        .filter(|server| Family::of(*server) == family)
        .collect()
}

fn remove_previous(
    tunnel: Luid,
    previous: &NetworkSettings,
    settings: &NetworkSettings,
) -> Result<(), Failure> {
    let kept = addresses(settings);
    for address in addresses(previous) {
        if kept.contains(&address) {
            continue;
        }
        match network::delete_address(tunnel, address.address) {
            Ok(()) => {}
            Err(error) if error.not_found() => {}
            Err(error) => return Err(failure(NetworkStep::Addresses, &error)),
        }
    }
    let wanted = tunnel_routes(tunnel, settings);
    for route in tunnel_routes(tunnel, previous) {
        if wanted.contains(&route) {
            continue;
        }
        match network::delete_route(&route) {
            Ok(()) => {}
            Err(error) if error.not_found() => {}
            Err(error) => return Err(failure(NetworkStep::Routes, &error)),
        }
    }
    let was_full = matches!(previous.ipv4.routes, TunnelRoutes::Full {});
    if was_full && !matches!(settings.ipv4.routes, TunnelRoutes::Full {}) {
        network::set_metric(tunnel, Family::Ipv4, None)
            .map_err(|error| failure(NetworkStep::Routes, &error))?;
    }
    Ok(())
}

fn set_wins_servers(
    tunnel: Luid,
    previous: &[Ipv4Addr],
    servers: &[Ipv4Addr],
) -> Result<(), Failure> {
    let index = tunnel
        .index()
        .map_err(|error| failure(NetworkStep::Wins, &error))?
        .to_string();
    if !previous.is_empty() {
        netsh(&["interface", "ipv4", "delete", "winsservers", &index, "all"])?;
    }
    for (position, server) in servers.iter().enumerate() {
        netsh(&[
            "interface",
            "ipv4",
            "add",
            "winsservers",
            &index,
            &server.to_string(),
            &format!("index={}", position + 1),
        ])?;
    }
    Ok(())
}

fn netsh(args: &[&str]) -> Result<(), Failure> {
    let environment: Vec<_> = ["SystemRoot", "windir"]
        .iter()
        .filter_map(|&name| Some((name, env::var_os(name)?)))
        .collect();
    let process = File::options()
        .read(true)
        .write(true)
        .open("NUL")
        .and_then(|null| {
            Ok([
                null.try_clone()?.into(),
                null.try_clone()?.into(),
                null.into(),
            ])
        })
        .map_err(|error| error.to_string())
        .and_then(|stdio| {
            process::spawn(NETSH, args, &environment, stdio).map_err(|error| error.to_string())
        })
        .map_err(|error| failure(NetworkStep::Wins, &format!("netsh: {error}")))?;
    let ended = process
        .ended_within(NETSH_LIMIT)
        .expect("netsh is this process's child and its handle open");
    if !ended {
        let _ = process.kill();
        process
            .wait()
            .expect("netsh is this process's child and its handle open");
        return Err(failure(
            NetworkStep::Wins,
            &format!(
                "netsh {} did not finish within {}s",
                args.join(" "),
                NETSH_LIMIT.as_secs()
            ),
        ));
    }
    let status = ExitStatus::from_raw(
        process
            .wait()
            .expect("netsh is this process's child and its handle open"),
    );
    if status.success() {
        Ok(())
    } else {
        Err(failure(
            NetworkStep::Wins,
            &format!("netsh {} ended with {status}", args.join(" ")),
        ))
    }
}

fn confirm(
    tunnel: Luid,
    settings: &NetworkSettings,
    physical: &Physical,
    add: &[OutsideRoute],
    routes_and_dns: bool,
) -> Result<(), Failure> {
    for address in addresses(settings) {
        let present = network::addresses(Family::of(address.address))
            .map_err(|error| failure(NetworkStep::Addresses, &error))?
            .iter()
            .any(|found| {
                found.interface == tunnel
                    && found.address == address.address
                    && found.prefix_length == address.prefix_length
                    && !matches!(found.dad_state, DadState::Duplicate | DadState::Invalid)
            });
        if !present {
            return Err(failure(
                NetworkStep::Addresses,
                &format!(
                    "{}/{} is not on the tunnel",
                    address.address, address.prefix_length
                ),
            ));
        }
    }
    if let Some(mtu) = settings.mtu {
        let read = network::interface_settings(tunnel, Family::Ipv4)
            .map_err(|error| failure(NetworkStep::Mtu, &error))?;
        if read.mtu != mtu {
            return Err(failure(
                NetworkStep::Mtu,
                &format!("the tunnel's MTU is {}, not {mtu}", read.mtu),
            ));
        }
    }
    let wanted = if routes_and_dns {
        tunnel_routes(tunnel, settings)
    } else {
        Vec::new()
    };
    let tunnel_table = table(wanted.iter().map(|route| route.destination))
        .map_err(|error| failure(NetworkStep::Routes, &error))?;
    for route in wanted {
        if !routes::has(&tunnel_table, &route) {
            return Err(failure(
                NetworkStep::Routes,
                &format!(
                    "the route to {}/{} through the tunnel is missing",
                    route.destination, route.prefix_length
                ),
            ));
        }
    }
    let outside_table = table(add.iter().map(|route| route.destination.address))
        .map_err(|error| failure(NetworkStep::OutsideRoutes, &error))?;
    for route in add {
        if !routes::has(&outside_table, &physical.os_route(route)) {
            return Err(failure(
                NetworkStep::OutsideRoutes,
                &format!(
                    "the route to {}/{} through {} is missing",
                    route.destination.address, route.destination.prefix_length, route.router
                ),
            ));
        }
    }
    for family in [Family::Ipv4, Family::Ipv6]
        .into_iter()
        .filter(|_| routes_and_dns)
    {
        let servers = dns_servers(settings, family);
        let read = network::dns_servers(tunnel, family)
            .map_err(|error| failure(NetworkStep::Dns, &error))?;
        if read != servers {
            return Err(failure(
                NetworkStep::Dns,
                &format!("the tunnel's DNS servers are {read:?}, not {servers:?}"),
            ));
        }
    }
    Ok(())
}

fn switch(
    settings: &NetworkSettings,
    replaced: &NetworkSettings,
    outside: &[IpNetwork],
    log: &Log,
) -> NetworkOutcome {
    let applied = apply(settings, None, &[], &[], true, None, log);
    if !matches!(applied, NetworkOutcome::Applied { .. }) {
        return applied;
    }
    let routed = remove_replaced(replaced).and_then(|()| {
        let tunnel = tunnel_interface(&settings.tunnel)?;
        routing(settings, tunnel, outside)
    });
    match routed {
        Ok(()) => {
            let message = format!(
                "moved the VPN traffic from {} to {}",
                replaced.tunnel.name, settings.tunnel.name
            );
            log(LogLevel::Notice, LogCategory::Routes, &message);
            applied
        }
        Err(failure) => NetworkOutcome::Failed {
            step: failure.step,
            error: failure.error,
            added: Vec::new(),
            left: Vec::new(),
        },
    }
}

fn remove_replaced(replaced: &NetworkSettings) -> Result<(), Failure> {
    let Ok(tunnel) = tunnel_interface(&replaced.tunnel) else {
        return Ok(());
    };
    let routes = tunnel_routes(tunnel, replaced);
    for route in &routes {
        match network::delete_route(route) {
            Ok(()) => {}
            Err(error) if error.not_found() => {}
            Err(error) => return Err(failure(NetworkStep::Routes, &error)),
        }
    }
    let table = table(routes.iter().map(|route| route.destination))
        .map_err(|error| failure(NetworkStep::Routes, &error))?;
    match routes.iter().find(|route| routes::has(&table, route)) {
        Some(route) => Err(failure(
            NetworkStep::Routes,
            &format!(
                "the route to {}/{} through {} is still in place",
                route.destination, route.prefix_length, replaced.tunnel.name
            ),
        )),
        None => Ok(()),
    }
}

fn check(settings: &NetworkSettings, outside: &[IpNetwork], log: &Log) -> NetworkOutcome {
    let facts = tunnel_interface(&settings.tunnel)
        .and_then(|tunnel| Ok((tunnel, routing_facts(settings, tunnel)?)));
    let (tunnel, (owners, chosen)) = match facts {
        Ok(facts) => facts,
        Err(failure) => {
            let message = format!(
                "checking the VPN routing of {} failed ({:?}): {}",
                settings.tunnel.name, failure.step, failure.error
            );
            log(LogLevel::Error, LogCategory::Routes, &message);
            return NetworkOutcome::Failed {
                step: failure.step,
                error: failure.error,
                added: Vec::new(),
                left: Vec::new(),
            };
        }
    };
    let Some(problem) = routing_problem(settings, tunnel, outside, &owners, &chosen) else {
        return NetworkOutcome::Checked { complete: true };
    };
    let message = format!(
        "the VPN routing of {} is not complete: {}",
        settings.tunnel.name, problem.error
    );
    log(LogLevel::Error, LogCategory::Routes, &message);
    NetworkOutcome::Checked { complete: false }
}

fn routing(settings: &NetworkSettings, tunnel: Luid, outside: &[IpNetwork]) -> Result<(), Failure> {
    let (owners, chosen) = routing_facts(settings, tunnel)?;
    routing_problem(settings, tunnel, outside, &owners, &chosen).map_or(Ok(()), Err)
}

type Chosen = (Route, Route, IpAddr);

fn routing_facts(
    settings: &NetworkSettings,
    tunnel: Luid,
) -> Result<(Vec<Luid>, Vec<Chosen>), Failure> {
    let address = settings.ipv4.address.address;
    let owners = network::addresses(Family::Ipv4)
        .map_err(|error| failure(NetworkStep::Addresses, &error))?
        .into_iter()
        .filter(|found| found.address == address)
        .map(|found| found.interface)
        .collect();
    let chosen = tunnel_routes(tunnel, settings)
        .into_iter()
        .map(|route| {
            let (best, source) = network::best_route(first_address(&route))
                .map_err(|error| failure(NetworkStep::Routes, &error))?;
            Ok((route, best, source))
        })
        .collect::<Result<_, Failure>>()?;
    Ok((owners, chosen))
}

fn first_address(route: &Route) -> IpAddr {
    let length = u32::from(route.prefix_length);
    match route.destination {
        IpAddr::V4(address) => {
            let mask = u32::MAX.checked_shl(32 - length).unwrap_or(0);
            Ipv4Addr::from(u32::from(address) & mask).into()
        }
        IpAddr::V6(address) => {
            let mask = u128::MAX.checked_shl(128 - length).unwrap_or(0);
            Ipv6Addr::from(u128::from(address) & mask).into()
        }
    }
}

fn routing_problem(
    settings: &NetworkSettings,
    tunnel: Luid,
    outside: &[IpNetwork],
    owners: &[Luid],
    chosen: &[Chosen],
) -> Option<Failure> {
    let name = &settings.tunnel.name;
    let address = settings.ipv4.address.address;
    if owners != [tunnel] {
        let holders = if owners.is_empty() {
            "no interface".to_owned()
        } else {
            owners
                .iter()
                .map(|&luid| alias(luid))
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Some(failure(
            NetworkStep::Addresses,
            &format!("the tunnel address {address} is on {holders}, not only on {name}"),
        ));
    }
    for (route, best, source) in chosen {
        let own = match route.destination {
            IpAddr::V4(_) => address,
            IpAddr::V6(_) => {
                let ipv6 = settings
                    .ipv6
                    .as_ref()
                    .expect("IPv6 tunnel routes come with an IPv6 address");
                ipv6.address.address
            }
        };
        let kept = outside.contains(&IpNetwork {
            address: best.destination,
            prefix_length: best.prefix_length,
        });
        if (best.interface == tunnel && *source == own) || kept {
            continue;
        }
        return Some(failure(
            NetworkStep::Routes,
            &format!(
                "the route for {}/{} goes through {} ({}/{}) from {source}, not {name} from {own}",
                route.destination,
                route.prefix_length,
                alias(best.interface),
                best.destination,
                best.prefix_length
            ),
        ));
    }
    None
}

fn alias(luid: Luid) -> String {
    network::interface(luid).map_or_else(
        |error| format!("an unreadable interface ({error})"),
        |interface| interface.alias,
    )
}

fn revert(changes: &OutsideChanges, log: &Log) -> NetworkOutcome {
    let mut failed = None;
    if !changes.dns_service_id.is_empty() {
        for family in [Family::Ipv4, Family::Ipv6] {
            if let Err(error) = network::set_dns_servers(&changes.dns_service_id, family, &[]) {
                failed.get_or_insert(failure(NetworkStep::Dns, &error));
            }
        }
        match network::clear_wins_servers(&changes.dns_service_id) {
            Ok(()) => {}
            Err(error) if error.not_found() => {}
            Err(error) => {
                failed.get_or_insert(failure(NetworkStep::Wins, &error));
            }
        }
    }
    let destinations = || changes.routes.iter().map(|route| route.destination.address);
    let before = table(destinations());
    let mut deleted = false;
    match &before {
        Ok(before) => {
            for route in &changes.routes {
                if let Some(row) = found(before, route) {
                    match network::delete_route(&row) {
                        Ok(()) => deleted = true,
                        Err(error) => {
                            failed.get_or_insert(failure(NetworkStep::OutsideRoutes, &error));
                        }
                    }
                }
            }
        }
        Err(error) => {
            failed.get_or_insert(failure(NetworkStep::OutsideRoutes, error));
        }
    }
    let after = if deleted {
        table(destinations()).ok()
    } else {
        before.ok()
    };
    let left: Vec<OutsideRoute> = changes
        .routes
        .iter()
        .filter(|route| {
            after
                .as_ref()
                .is_none_or(|table| found(table, route).is_some())
        })
        .cloned()
        .collect();
    let message = format!(
        "reverted the VPN network settings ({} of {} route(s) outside the tunnel still in place)",
        left.len(),
        changes.routes.len()
    );
    log(LogLevel::Notice, LogCategory::Routes, &message);
    match failed {
        None => NetworkOutcome::Reverted { left },
        Some(failure) => NetworkOutcome::Failed {
            step: failure.step,
            error: failure.error,
            added: Vec::new(),
            left,
        },
    }
}

#[cfg(test)]
mod tests;
