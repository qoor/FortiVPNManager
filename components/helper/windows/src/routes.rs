//! The routing table, through the IP Helper API in `fvm_os_api_windows::network`. The exception
//! routes go through the gateway of the primary interface: the hardware Ethernet or Wi-Fi adapter
//! whose default route has the lowest metric. Virtual adapters, such as FortiClient's or Hyper-V's,
//! report Ethernet too, so the hardware flag tells them apart. The app sees its GUID as the service
//! ID and its name as the service name, as a network service on macOS. The routes live in the
//! active routing table only, so Windows forgets them at a restart.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};

use fvm_contract::app_helper::{
    BypassRoute, BypassRoutesSnapshot, HelperError, HelperErrorCode, Ipv4Network, MAX_ENTRIES,
};
use fvm_contract::helper_internal::{
    AddedRoute, BypassRoutesChange, BypassRoutesOutcome, LogCategory, LogLevel, NetworkCheck,
};
use fvm_os_api_windows::network::{self, DadState, Family, Luid, Medium, Route};

use crate::Log;

/// The route metric New-NetRoute gives a route, which the exception routes had while the Helper
/// wrote them with PowerShell.
pub(crate) const BYPASS_ROUTE_METRIC: u32 = 256;

/// Carries out `change`, as `BypassRoutesChange` describes it: one read of the routing table before
/// the changes, and one after them.
pub(crate) fn change_bypass_routes(change: &BypassRoutesChange, log: &Log) -> BypassRoutesOutcome {
    match apply(change, log) {
        Ok(outcome) => outcome,
        Err(error) => BypassRoutesOutcome::Refused { error },
    }
}

fn apply(change: &BypassRoutesChange, log: &Log) -> Result<BypassRoutesOutcome, HelperError> {
    let before = read_table(log)?;
    let primary = match &change.network {
        NetworkCheck::Skip {} => None,
        NetworkCheck::Read {} => Some(primary(&before, log)?),
        NetworkCheck::Expect { network } => {
            let primary = primary(&before, log)?;
            if primary.network != *network {
                return Err(error(
                    HelperErrorCode::WrongPhase,
                    "the network changed since it was read; read it again",
                ));
            }
            Some(primary)
        }
    };
    let to_add = match &primary {
        _ if change.add.is_empty() => Vec::new(),
        Some(primary) if writable(&change.add, &primary.network.router) => change
            .add
            .iter()
            .filter_map(|route| Some((as_route(route, primary.luid)?, route)))
            .collect(),
        _ => {
            return Err(error(
                HelperErrorCode::InvalidRequest,
                "the route list has repeats, too many routes, or routes this helper does not write",
            ));
        }
    };
    let mut changed = false;
    let mut removed = 0;
    for added in &change.remove {
        for route in before.iter().filter(|route| was_added(route, added)) {
            changed = true;
            match network::delete_route(route) {
                Ok(()) => removed += 1,
                Err(failure) => log(
                    LogLevel::Error,
                    LogCategory::Routes,
                    &format!(
                        "removing the exception route to {} failed: {failure}",
                        added.route.destination
                    ),
                ),
            }
        }
    }
    let mut tried = Vec::new();
    for (route, asked) in to_add {
        if has(&before, &route) {
            continue;
        }
        changed = true;
        match network::add_route(&route) {
            Ok(()) => tried.push(asked),
            // Added since the table was read, by someone else.
            Err(failure) if failure.already_exists() => {}
            Err(failure) => log(
                LogLevel::Error,
                LogCategory::Routes,
                &format!(
                    "adding the exception route to {} failed: {failure}",
                    asked.destination
                ),
            ),
        }
    }
    // Unread, the routes tried count as added and those to remove as left, so that the Helper
    // removes them later rather than leaving them behind.
    let after = if changed {
        read_table(log).ok()
    } else {
        Some(before)
    };
    let on_primary = |route: &BypassRoute| {
        let primary = primary.as_ref()?;
        let table = after.as_ref()?;
        Some(has(table, &as_route(route, primary.luid)?))
    };
    let added: Vec<BypassRoute> = tried
        .into_iter()
        .filter(|route| on_primary(route).unwrap_or(after.is_none()))
        .cloned()
        .collect();
    if removed > 0 || !added.is_empty() {
        let line = format!(
            "removed {removed} and added {} exception route(s)",
            added.len()
        );
        log(LogLevel::Notice, LogCategory::Routes, &line);
    }
    let found = change
        .look_up
        .iter()
        .filter(|route| on_primary(route).unwrap_or(false))
        .cloned()
        .collect();
    let left = change
        .remove
        .iter()
        .filter(|added| {
            after
                .as_ref()
                .is_none_or(|table| table.iter().any(|route| was_added(route, added)))
        })
        .cloned()
        .collect();
    Ok(BypassRoutesOutcome::Changed {
        network: primary.map(|primary| primary.network),
        added,
        found,
        left,
    })
}

/// Whether the Helper writes `routes`: at most a list's worth, no repeats, and each to an exception
/// target through `router`.
fn writable(routes: &[BypassRoute], router: &str) -> bool {
    routes.len() <= MAX_ENTRIES
        && routes.iter().collect::<HashSet<_>>().len() == routes.len()
        && routes
            .iter()
            .all(|route| route.gateway == router && Ipv4Network::from_route(route).is_ok())
}

/// Whether the table's `route` is `added` as the Helper added it: same destination, prefix and
/// gateway, on the interface of the network service it went through.
fn was_added(route: &Route, added: &AddedRoute) -> bool {
    parts(&added.route).is_some_and(|(destination, prefix_length, gateway)| {
        route.destination == IpAddr::V4(destination)
            && route.prefix_length == prefix_length
            && route.next_hop == IpAddr::V4(gateway)
            // An interface that went away took its routes with it.
            && network::interface(route.interface)
                .is_ok_and(|interface| interface.guid == added.service_id)
    })
}

/// Whether `table` has `wanted`: the same destination, prefix, next hop and interface.
pub(crate) fn has(table: &[Route], wanted: &Route) -> bool {
    table.iter().any(|route| {
        route.interface == wanted.interface
            && route.destination == wanted.destination
            && route.prefix_length == wanted.prefix_length
            && route.next_hop == wanted.next_hop
    })
}

/// The primary interface and the network the app sees on it.
pub(crate) struct Primary {
    pub(crate) luid: Luid,
    pub(crate) network: BypassRoutesSnapshot,
}

/// The primary interface among the default routes of `table`.
pub(crate) fn primary(table: &[Route], log: &Log) -> Result<Primary, HelperError> {
    let unreadable = |failure: fvm_os_api_windows::Error| {
        let message = format!("reading the primary interface failed: {failure}");
        log(LogLevel::Error, LogCategory::Routes, &message);
        error(HelperErrorCode::InternalError, message)
    };
    let mut best: Option<(u32, &Route, network::Interface)> = None;
    for route in table {
        if route.prefix_length != 0 || !route.destination.is_unspecified() {
            continue;
        }
        // An interface that went away meanwhile is not the primary one.
        let Ok(interface) = network::interface(route.interface) else {
            continue;
        };
        if !interface.up || !interface.hardware || interface.medium == Medium::Other {
            continue;
        }
        let Ok(settings) = network::interface_settings(route.interface, Family::Ipv4) else {
            continue;
        };
        let metric = route.metric.saturating_add(settings.metric);
        if best.as_ref().is_none_or(|(lowest, ..)| metric < *lowest) {
            best = Some((metric, route, interface));
        }
    }
    let Some((_, route, interface)) = best else {
        return Err(error(
            HelperErrorCode::WrongPhase,
            "no active network service",
        ));
    };
    let no_router_or_address = || {
        error(
            HelperErrorCode::WrongPhase,
            "the active network service has no IPv4 router or address",
        )
    };
    let router = match route.next_hop {
        IpAddr::V4(router) if !router.is_unspecified() => router,
        _ => return Err(no_router_or_address()),
    };
    let address = network::addresses(Family::Ipv4)
        .map_err(unreadable)?
        .into_iter()
        .find(|address| {
            address.interface == route.interface && address.dad_state == DadState::Preferred
        })
        .ok_or_else(no_router_or_address)?;
    let subnet_mask = mask(address.prefix_length).ok_or_else(no_router_or_address)?;
    Ok(Primary {
        luid: route.interface,
        network: BypassRoutesSnapshot {
            service_id: interface.guid,
            service_name: interface.alias,
            router: router.to_string(),
            address: address.address.to_string(),
            subnet_mask,
        },
    })
}

fn read_table(log: &Log) -> Result<Vec<Route>, HelperError> {
    network::routes(Family::Ipv4).map_err(|failure| {
        let message = format!("reading the routing table failed: {failure}");
        log(LogLevel::Error, LogCategory::Routes, &message);
        error(HelperErrorCode::InternalError, message)
    })
}

/// `route` through the interface `luid`, if its values are an IPv4 route.
fn as_route(route: &BypassRoute, luid: Luid) -> Option<Route> {
    let (destination, prefix_length, gateway) = parts(route)?;
    Some(Route {
        interface: luid,
        destination: destination.into(),
        prefix_length,
        next_hop: gateway.into(),
        metric: BYPASS_ROUTE_METRIC,
    })
}

fn parts(route: &BypassRoute) -> Option<(Ipv4Addr, u8, Ipv4Addr)> {
    Some((
        route.destination.parse().ok()?,
        prefix_length(&route.subnet_mask)?,
        route.gateway.parse().ok()?,
    ))
}

/// The prefix length of a subnet mask, such as 24 for `255.255.255.0`, if it is one.
fn prefix_length(mask: &str) -> Option<u8> {
    let bits = u32::from(mask.parse::<Ipv4Addr>().ok()?);
    let length = bits.leading_ones();
    (bits.checked_shl(length).unwrap_or(0) == 0).then(|| u8::try_from(length).expect("at most 32"))
}

/// The subnet mask of a prefix length, such as `255.255.255.0` for 24.
fn mask(prefix_length: u8) -> Option<String> {
    let length = u32::from(prefix_length);
    (length <= 32).then(|| {
        let bits = u32::MAX.checked_shl(32 - length).unwrap_or(0);
        Ipv4Addr::from(bits).to_string()
    })
}

fn error(code: HelperErrorCode, message: impl Into<String>) -> HelperError {
    HelperError {
        code,
        message: message.into(),
    }
}

/// The library's tests that add or remove routes, directly or as the on-link routes of an address,
/// take turns, and so do the tests that need the primary interface's routes to stay as they read
/// them.
#[cfg(test)]
pub(crate) static ROUTE_WRITES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs its closure when dropped, so a failing test still puts back what it changed.
#[cfg(test)]
pub(crate) struct Undo<F: FnMut()>(pub(crate) F);

#[cfg(test)]
impl<F: FnMut()> Drop for Undo<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

#[cfg(test)]
mod tests {
    //! These change the test PC's routing table, only for TEST-NET-1 addresses (192.0.2.0/24),
    //! which nothing uses, through the primary interface, and put back what they changed.

    use std::sync::{Arc, PoisonError};

    use super::*;

    fn quiet() -> Log {
        Arc::new(|_, _, _| {})
    }

    fn host(destination: &str, gateway: &str) -> BypassRoute {
        BypassRoute {
            destination: destination.to_owned(),
            subnet_mask: "255.255.255.255".to_owned(),
            gateway: gateway.to_owned(),
        }
    }

    fn change(
        network: NetworkCheck,
        remove: &[AddedRoute],
        add: &[BypassRoute],
        look_up: &[BypassRoute],
    ) -> BypassRoutesChange {
        BypassRoutesChange {
            network,
            remove: remove.to_vec(),
            add: add.to_vec(),
            look_up: look_up.to_vec(),
        }
    }

    struct Changed {
        added: Vec<BypassRoute>,
        found: Vec<BypassRoute>,
        left: Vec<AddedRoute>,
    }

    fn changed(outcome: BypassRoutesOutcome) -> Changed {
        match outcome {
            BypassRoutesOutcome::Changed {
                added, found, left, ..
            } => Changed { added, found, left },
            BypassRoutesOutcome::Refused { error } => panic!("{error:?}"),
        }
    }

    fn refused(outcome: BypassRoutesOutcome) -> HelperError {
        match outcome {
            BypassRoutesOutcome::Refused { error } => error,
            BypassRoutesOutcome::Changed { added, .. } => panic!("added {added:?}"),
        }
    }

    fn primary_now() -> Primary {
        primary(&network::routes(Family::Ipv4).unwrap(), &quiet()).unwrap()
    }

    /// Whether the table has `route` through the primary interface.
    fn on_primary(route: &BypassRoute) -> bool {
        let wanted = as_route(route, primary_now().luid).unwrap();
        has(&network::routes(Family::Ipv4).unwrap(), &wanted)
    }

    fn read_network() -> BypassRoutesSnapshot {
        let read = change(NetworkCheck::Read {}, &[], &[], &[]);
        match change_bypass_routes(&read, &quiet()) {
            BypassRoutesOutcome::Changed {
                network: Some(network),
                ..
            } => network,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_primary_network_is_read() {
        // The test PC has a wired network.
        let network = read_network();
        assert!(network.service_id.starts_with('{'), "{network:?}");
        assert!(!network.service_name.is_empty(), "{network:?}");
        assert!(network.router.parse::<Ipv4Addr>().is_ok(), "{network:?}");
        assert!(network.address.parse::<Ipv4Addr>().is_ok(), "{network:?}");
        assert!(prefix_length(&network.subnet_mask).is_some(), "{network:?}");
    }

    #[test]
    fn exception_routes_are_added_found_and_removed() {
        let log = quiet();
        let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let network = read_network();
        let route = host("192.0.2.88", &network.router);
        let undo_route = as_route(&route, primary_now().luid).unwrap();
        let _undo = Undo(move || {
            let _ = network::delete_route(&undo_route);
        });
        let expect = || NetworkCheck::Expect {
            network: network.clone(),
        };
        let routes = std::slice::from_ref(&route);

        let first = changed(change_bypass_routes(
            &change(expect(), &[], routes, routes),
            &log,
        ));
        assert_eq!(first.added, routes);
        assert_eq!(first.found, routes);
        assert!(on_primary(&route));
        // There already: found, but not added again.
        let again = changed(change_bypass_routes(
            &change(expect(), &[], routes, routes),
            &log,
        ));
        assert!(again.added.is_empty());
        assert_eq!(again.found, routes);

        // As if added through another network service: not the route this one has, so it stays.
        let elsewhere = AddedRoute {
            route: route.clone(),
            service_id: "{00000000-0000-0000-0000-000000000000}".to_owned(),
        };
        let kept = changed(change_bypass_routes(
            &change(
                NetworkCheck::Skip {},
                std::slice::from_ref(&elsewhere),
                &[],
                &[],
            ),
            &log,
        ));
        assert!(kept.left.is_empty());
        assert!(on_primary(&route));

        let added = AddedRoute {
            route: route.clone(),
            service_id: network.service_id.clone(),
        };
        let removed = changed(change_bypass_routes(
            &change(
                NetworkCheck::Skip {},
                std::slice::from_ref(&added),
                &[],
                &[],
            ),
            &log,
        ));
        assert!(removed.left.is_empty());
        assert!(!on_primary(&route));
    }

    #[test]
    fn a_change_for_a_network_that_changed_since_it_was_read_changes_nothing() {
        let log = quiet();
        let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        // As if the app had read another router, which the network no longer has.
        let mut read = read_network();
        read.router = "192.0.2.1".to_owned();
        let route = host("192.0.2.93", "192.0.2.1");
        let routes = std::slice::from_ref(&route);
        let error = refused(change_bypass_routes(
            &change(NetworkCheck::Expect { network: read }, &[], routes, routes),
            &log,
        ));
        assert_eq!(error.code, HelperErrorCode::WrongPhase, "{error:?}");
        let destination = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 93));
        let table = network::routes(Family::Ipv4).unwrap();
        assert!(
            !table
                .iter()
                .any(|present| present.destination == destination)
        );
    }

    #[test]
    fn routes_this_helper_does_not_write_are_refused_before_any_change() {
        let network = read_network();
        let router = network.router.clone();
        let new = |third: usize| host(&format!("10.0.{third}.1"), &router);
        let network_route = |destination: &str, mask: &str| BypassRoute {
            destination: destination.to_owned(),
            subnet_mask: mask.to_owned(),
            gateway: router.clone(),
        };
        let too_many: Vec<_> = (0..=MAX_ENTRIES).map(new).collect();
        let refusals = [
            ("repeat", vec![new(0), new(0)]),
            ("too many", too_many),
            ("another router", vec![host("10.0.0.1", "192.0.2.2")]),
            (
                "prefix too short",
                vec![network_route("10.0.0.0", "254.0.0.0")],
            ),
            (
                "host bits",
                vec![network_route("10.0.0.1", "255.255.255.0")],
            ),
            ("loopback", vec![host("127.0.0.2", &router)]),
            ("multicast", vec![host("224.0.0.9", &router)]),
            ("not an address", vec![host("10.0.0.1 ", &router)]),
        ];
        for (what, routes) in refusals {
            let expect = NetworkCheck::Expect {
                network: network.clone(),
            };
            let error = refused(change_bypass_routes(
                &change(expect, &[], &routes, &[]),
                &quiet(),
            ));
            assert_eq!(
                error.code,
                HelperErrorCode::InvalidRequest,
                "{what}: {error:?}"
            );
        }
        // Without a network there is no router to add through.
        let error = refused(change_bypass_routes(
            &change(NetworkCheck::Skip {}, &[], &[new(0)], &[]),
            &quiet(),
        ));
        assert_eq!(error.code, HelperErrorCode::InvalidRequest, "{error:?}");
    }

    #[test]
    fn masks_and_prefixes_follow_each_other() {
        assert_eq!(mask(0).as_deref(), Some("0.0.0.0"));
        assert_eq!(mask(24).as_deref(), Some("255.255.255.0"));
        assert_eq!(mask(32).as_deref(), Some("255.255.255.255"));
        assert_eq!(mask(33), None);
        assert_eq!(prefix_length("0.0.0.0"), Some(0));
        assert_eq!(prefix_length("255.255.255.0"), Some(24));
        assert_eq!(prefix_length("255.255.255.255"), Some(32));
        assert_eq!(prefix_length("255.0.255.0"), None);
    }
}
