use std::process::Command;
use std::sync::{Arc, PoisonError};

use super::*;
use crate::routes::{ROUTE_WRITES, Undo};

fn loopback() -> Luid {
    network::best_route(Ipv4Addr::LOCALHOST.into())
        .unwrap()
        .0
        .interface
}

fn dns_key(luid: Luid, family: Family) -> String {
    let service = match family {
        Family::Ipv4 => "Tcpip",
        Family::Ipv6 => "Tcpip6",
    };
    let guid = network::interface(luid).unwrap().guid;
    format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{service}\Parameters\Interfaces\{guid}")
}

fn reg(args: &[&str]) -> Option<String> {
    let output = Command::new(r"C:\Windows\System32\reg.exe")
        .args(args)
        .output()
        .ok()?;
    let succeeded = output.status.success();
    succeeded.then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn registered_servers(key: &str, value: &str, kind: &str) -> Registered {
    if reg(&["query", key]).is_none() {
        return Registered::NoKey;
    }
    match reg(&["query", key, "/v", value]) {
        None => Registered::NoValue,
        Some(output) => {
            let line = output.lines().find(|line| line.contains(kind));
            let (_, list) = line.and_then(|line| line.split_once(kind)).unwrap();
            Registered::Value(list.trim().to_owned())
        }
    }
}

/// A registry value as reg.exe finds it.
#[derive(Clone, Debug, PartialEq)]
enum Registered {
    NoKey,
    NoValue,
    Value(String),
}

fn quiet() -> Log {
    Arc::new(|_, _, _| {})
}

fn network(address: &str, prefix_length: u8) -> IpNetwork {
    IpNetwork {
        address: address.parse().unwrap(),
        prefix_length,
    }
}

fn loopback_tunnel() -> TunnelDevice {
    TunnelDevice {
        name: "loopback".to_owned(),
        index: Some(loopback().index().unwrap()),
    }
}

fn settings(include: IpNetwork) -> NetworkSettings {
    NetworkSettings {
        tunnel: loopback_tunnel(),
        gateway: "192.0.2.97".parse().unwrap(),
        mtu: None,
        ipv4: FamilySettings {
            address: network("198.51.100.97", 32),
            routes: TunnelRoutes::Split {
                include: vec![include],
            },
        },
        ipv6: None,
        split_exclude: Vec::new(),
        dns_servers: vec!["192.0.2.53".parse().unwrap()],
        default_domains: Vec::new(),
        split_dns_domains: Vec::new(),
        wins_servers: Vec::new(),
    }
}

fn put_back_loopback_dns() -> Undo<impl FnMut()> {
    let key = dns_key(loopback(), Family::Ipv4);
    let before = registered_servers(&key, "NameServer", "REG_SZ");
    Undo(move || {
        let key = key.as_str();
        match &before {
            Registered::NoKey => reg(&["delete", key, "/f"]),
            Registered::NoValue => reg(&["delete", key, "/v", "NameServer", "/f"]),
            Registered::Value(list) => reg(&[
                "add",
                key,
                "/v",
                "NameServer",
                "/t",
                "REG_SZ",
                "/d",
                list,
                "/f",
            ]),
        };
    })
}

fn look_up_route(outside: IpNetwork) -> (String, OutsideRoute) {
    let change = NetworkChange::LookUp {
        tunnel: loopback_tunnel(),
        outside: vec![outside],
    };
    match super::change(&change, &quiet()) {
        NetworkOutcome::LookedUp {
            dns_service_id,
            mut missing,
        } => {
            assert_eq!(missing.len(), 1, "{missing:?}");
            (dns_service_id, missing.remove(0))
        }
        other => panic!("not looked up: {other:?}"),
    }
}

fn on_loopback(destination: IpNetwork) -> Route {
    Route {
        interface: loopback(),
        destination: destination.address,
        prefix_length: destination.prefix_length,
        next_hop: Ipv4Addr::UNSPECIFIED.into(),
        metric: TUNNEL_ROUTE_METRIC,
    }
}

#[test]
fn split_settings_are_applied_changed_and_reverted_on_the_loopback_interface() {
    let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
    let _dns = put_back_loopback_dns();
    let first_include = network("198.51.100.128", 25);
    let second_include = network("198.51.100.0", 26);
    let (dns_service_id, gateway_route) = look_up_route(network("192.0.2.97", 32));
    assert_eq!(dns_service_id, network::interface(loopback()).unwrap().guid);
    let physical = Physical::read(&quiet()).ok().unwrap();
    let gateway_row = physical.os_route(&gateway_route);
    let _undo = Undo(move || {
        let _ = network::delete_route(&gateway_row);
        let _ = network::delete_route(&on_loopback(first_include));
        let _ = network::delete_route(&on_loopback(second_include));
        let _ = network::delete_address(loopback(), "198.51.100.97".parse().unwrap());
    });

    let first = settings(first_include);
    let applied = super::change(
        &NetworkChange::Apply {
            settings: Box::new(first.clone()),
            previous: None,
            remove: Vec::new(),
            add: vec![gateway_route.clone()],
            outside: vec![gateway_route.destination],
        },
        &quiet(),
    );
    assert_eq!(
        applied,
        NetworkOutcome::Applied {
            added: vec![gateway_route.clone()],
            left: Vec::new(),
        }
    );
    let table = network::routes(Family::Ipv4).unwrap();
    assert!(routes::has(&table, &on_loopback(first_include)));
    assert!(routes::has(&table, &gateway_row));
    assert_eq!(
        network::dns_servers(loopback(), Family::Ipv4),
        Ok(first.dns_servers.clone())
    );

    let reapplied = super::change(
        &NetworkChange::Apply {
            settings: Box::new(settings(second_include)),
            previous: Some(Box::new(first)),
            remove: Vec::new(),
            add: Vec::new(),
            outside: vec![gateway_route.destination],
        },
        &quiet(),
    );
    assert_eq!(
        reapplied,
        NetworkOutcome::Applied {
            added: Vec::new(),
            left: Vec::new(),
        }
    );
    let table = network::routes(Family::Ipv4).unwrap();
    assert!(!routes::has(&table, &on_loopback(first_include)));
    assert!(routes::has(&table, &on_loopback(second_include)));

    let reverted = super::change(
        &NetworkChange::Revert {
            changes: OutsideChanges {
                dns_service_id,
                routes: vec![gateway_route],
            },
        },
        &quiet(),
    );
    assert_eq!(reverted, NetworkOutcome::Reverted { left: Vec::new() });
    assert!(!routes::has(
        &network::routes(Family::Ipv4).unwrap(),
        &gateway_row
    ));
    assert_eq!(
        network::dns_servers(loopback(), Family::Ipv4),
        Ok(Vec::new())
    );
}

#[test]
fn a_revert_without_a_dns_service_id_reverts_only_the_routes() {
    let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
    let (_, gateway_route) = look_up_route(network("192.0.2.96", 32));
    let physical = Physical::read(&quiet()).ok().unwrap();
    let gateway_row = physical.os_route(&gateway_route);
    let _undo = Undo(move || {
        let _ = network::delete_route(&gateway_row);
    });
    network::add_route(&gateway_row).unwrap();

    let reverted = super::change(
        &NetworkChange::Revert {
            changes: OutsideChanges {
                dns_service_id: String::new(),
                routes: vec![gateway_route],
            },
        },
        &quiet(),
    );
    assert_eq!(reverted, NetworkOutcome::Reverted { left: Vec::new() });
    assert!(!routes::has(
        &network::routes(Family::Ipv4).unwrap(),
        &gateway_row
    ));
}

#[test]
fn a_prepared_tunnel_takes_over_the_routes_and_dns_of_the_one_it_replaces() {
    let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
    let _dns = put_back_loopback_dns();
    let replaced_include = network("198.51.100.128", 25);
    let include = network("198.51.100.0", 26);
    let (_, gateway_route) = look_up_route(network("192.0.2.99", 32));
    let physical = Physical::read(&quiet()).ok().unwrap();
    let gateway_row = physical.os_route(&gateway_route);
    let _undo = Undo(move || {
        let _ = network::delete_route(&gateway_row);
        let _ = network::delete_route(&on_loopback(replaced_include));
        let _ = network::delete_route(&on_loopback(include));
        for address in ["198.51.100.97", "198.51.100.98"] {
            let _ = network::delete_address(loopback(), address.parse().unwrap());
        }
    });
    let replaced = settings(replaced_include);
    let applied = super::change(
        &NetworkChange::Apply {
            settings: Box::new(replaced.clone()),
            previous: None,
            remove: Vec::new(),
            add: Vec::new(),
            outside: Vec::new(),
        },
        &quiet(),
    );
    assert!(
        matches!(applied, NetworkOutcome::Applied { .. }),
        "{applied:?}"
    );
    let mut next = settings(include);
    next.ipv4.address = network("198.51.100.98", 32);
    next.dns_servers = vec!["192.0.2.54".parse().unwrap()];

    let prepared = super::change(
        &NetworkChange::Prepare {
            settings: Box::new(next.clone()),
            add: vec![gateway_route.clone()],
        },
        &quiet(),
    );
    assert_eq!(
        prepared,
        NetworkOutcome::Applied {
            added: vec![gateway_route.clone()],
            left: Vec::new(),
        }
    );
    let table = network::routes(Family::Ipv4).unwrap();
    assert!(routes::has(&table, &gateway_row));
    assert!(routes::has(&table, &on_loopback(replaced_include)));
    assert!(!routes::has(&table, &on_loopback(include)));
    let addresses = network::addresses(Family::Ipv4).unwrap();
    assert!(addresses.iter().any(|found| found.interface == loopback()
        && found.address == "198.51.100.98".parse::<IpAddr>().unwrap()));
    assert_eq!(
        network::dns_servers(loopback(), Family::Ipv4),
        Ok(replaced.dns_servers.clone())
    );

    network::delete_address(loopback(), "198.51.100.97".parse().unwrap()).unwrap();
    let switched = super::change(
        &NetworkChange::Switch {
            settings: Box::new(next.clone()),
            replaced: Box::new(replaced),
            outside: vec![gateway_route.destination],
        },
        &quiet(),
    );
    assert_eq!(
        switched,
        NetworkOutcome::Applied {
            added: Vec::new(),
            left: Vec::new(),
        }
    );
    let table = network::routes(Family::Ipv4).unwrap();
    assert!(routes::has(&table, &on_loopback(include)));
    assert!(!routes::has(&table, &on_loopback(replaced_include)));
    assert!(routes::has(&table, &gateway_row));
    assert_eq!(
        network::dns_servers(loopback(), Family::Ipv4),
        Ok(next.dns_servers)
    );
}

#[test]
fn wins_servers_set_on_the_loopback_interface_are_cleared_by_its_guid() {
    let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
    let loopback = loopback();
    let guid = network::interface(loopback).unwrap().guid;
    let key =
        format!(r"HKLM\SYSTEM\CurrentControlSet\Services\NetBT\Parameters\Interfaces\Tcpip_{guid}");
    let before = registered_servers(&key, "NameServerList", "REG_MULTI_SZ");
    let (undo_key, undo_before) = (key.clone(), before.clone());
    let _undo = Undo(move || {
        let index = loopback.index().unwrap().to_string();
        let _ = netsh(&["interface", "ipv4", "delete", "winsservers", &index, "all"]);
        let key = undo_key.as_str();
        match &undo_before {
            Registered::NoKey => reg(&["delete", key, "/f"]),
            Registered::NoValue => reg(&["delete", key, "/v", "NameServerList", "/f"]),
            Registered::Value(list) => reg(&[
                "add",
                key,
                "/v",
                "NameServerList",
                "/t",
                "REG_MULTI_SZ",
                "/d",
                list,
                "/f",
            ]),
        };
    });
    let servers = [Ipv4Addr::new(192, 0, 2, 137), Ipv4Addr::new(192, 0, 2, 138)];
    set_wins_servers(loopback, &[], &servers).ok().unwrap();
    assert_eq!(
        registered_servers(&key, "NameServerList", "REG_MULTI_SZ"),
        Registered::Value(r"192.0.2.137\0192.0.2.138".to_owned())
    );
    network::clear_wins_servers(&guid).unwrap();
    assert_eq!(
        registered_servers(&key, "NameServerList", "REG_MULTI_SZ"),
        Registered::Value(String::new())
    );
    reg(&["delete", &key, "/v", "NameServerList", "/f"]).unwrap();
    let error = network::clear_wins_servers(&guid).unwrap_err();
    assert!(error.not_found(), "{error}");
    assert_eq!(
        registered_servers(&key, "NameServerList", "REG_MULTI_SZ"),
        Registered::NoValue
    );
}

#[test]
fn an_apply_through_a_router_that_is_no_longer_current_changes_nothing() {
    let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
    let (_, mut gateway_route) = look_up_route(network("192.0.2.98", 32));
    gateway_route.router = "192.0.2.254".parse().unwrap();
    let outcome = super::change(
        &NetworkChange::Apply {
            settings: Box::new(settings(network("198.51.100.128", 25))),
            previous: None,
            remove: Vec::new(),
            add: vec![gateway_route],
            outside: Vec::new(),
        },
        &quiet(),
    );
    assert_eq!(
        outcome,
        NetworkOutcome::Failed {
            step: NetworkStep::Network,
            error: "the network changed since it was looked up".to_owned(),
            added: Vec::new(),
            left: Vec::new(),
        }
    );
    let addresses = network::addresses(Family::Ipv4).unwrap();
    assert!(!addresses.iter().any(|found| found.interface == loopback()
        && found.address == "198.51.100.97".parse::<IpAddr>().unwrap()));
}

fn on_loopback_from(destination: IpNetwork, source: &str) -> Chosen {
    let route = on_loopback(destination);
    (route, route, source.parse().unwrap())
}

#[test]
fn the_tunnel_address_on_a_second_interface_fails_the_routing() {
    let primary = Physical::read(&quiet()).ok().unwrap().luid;
    let settings = settings(network("198.51.100.128", 25));
    let chosen = [on_loopback_from(
        network("198.51.100.128", 25),
        "198.51.100.97",
    )];
    assert!(routing_problem(&settings, loopback(), &[], &[loopback()], &chosen).is_none());
    let problem =
        routing_problem(&settings, loopback(), &[], &[loopback(), primary], &chosen).unwrap();
    assert_eq!(problem.step, NetworkStep::Addresses);
    assert!(
        problem
            .error
            .starts_with("the tunnel address 198.51.100.97 is on "),
        "{}",
        problem.error
    );
    assert!(
        problem.error.ends_with(", not only on loopback"),
        "{}",
        problem.error
    );
}

#[test]
fn a_route_through_another_interface_or_from_another_address_fails_the_routing() {
    let primary = Physical::read(&quiet()).ok().unwrap().luid;
    let include = network("198.51.100.128", 25);
    let settings = settings(include);
    let tunnel_route = on_loopback(include);
    let elsewhere = Route {
        interface: primary,
        ..tunnel_route
    };
    for (best, source) in [(elsewhere, "198.51.100.97"), (tunnel_route, "127.0.0.1")] {
        let chosen = [(tunnel_route, best, source.parse().unwrap())];
        let problem = routing_problem(&settings, loopback(), &[], &[loopback()], &chosen).unwrap();
        assert_eq!(problem.step, NetworkStep::Routes);
        assert!(
            problem
                .error
                .starts_with("the route for 198.51.100.128/25 goes through "),
            "{}",
            problem.error
        );
        assert!(
            problem.error.ends_with(&format!(
                "(198.51.100.128/25) from {source}, not loopback from 198.51.100.97"
            )),
            "{}",
            problem.error
        );
    }
}

#[test]
fn a_route_kept_outside_the_tunnel_passes_the_routing() {
    let primary = Physical::read(&quiet()).ok().unwrap().luid;
    let include = network("198.51.100.128", 25);
    let excluded = network("198.51.100.128", 26);
    let best = Route {
        interface: primary,
        ..on_loopback(excluded)
    };
    let chosen = [(on_loopback(include), best, "192.0.2.10".parse().unwrap())];
    let settings = settings(include);
    assert!(routing_problem(&settings, loopback(), &[excluded], &[loopback()], &chosen).is_none());
    assert!(routing_problem(&settings, loopback(), &[], &[loopback()], &chosen).is_some());
}

#[test]
fn the_first_address_of_a_route_clears_its_host_bits() {
    let route = |destination, prefix_length| on_loopback(network(destination, prefix_length));
    assert_eq!(
        first_address(&route("198.51.100.200", 25)),
        "198.51.100.128".parse::<IpAddr>().unwrap()
    );
    assert_eq!(
        first_address(&route("0.0.0.0", 0)),
        "0.0.0.0".parse::<IpAddr>().unwrap()
    );
    assert_eq!(
        first_address(&route("2001:db8::5", 3)),
        "2000::".parse::<IpAddr>().unwrap()
    );
}

#[test]
fn a_more_specific_route_through_another_interface_fails_the_check_unless_kept_outside() {
    let _turn = ROUTE_WRITES.lock().unwrap_or_else(PoisonError::into_inner);
    let _dns = put_back_loopback_dns();
    let include = network("198.51.100.128", 25);
    let excluded = network("198.51.100.128", 26);
    let physical = Physical::read(&quiet()).ok().unwrap();
    let through_primary = Route {
        interface: physical.luid,
        destination: excluded.address,
        prefix_length: excluded.prefix_length,
        next_hop: physical.ipv4_router,
        metric: BYPASS_ROUTE_METRIC,
    };
    let _undo = Undo(move || {
        let _ = network::delete_route(&through_primary);
        let _ = network::delete_route(&on_loopback(include));
        let _ = network::delete_address(loopback(), "198.51.100.97".parse().unwrap());
    });
    let settings = settings(include);
    let apply = |outside: Vec<IpNetwork>| {
        super::change(
            &NetworkChange::Apply {
                settings: Box::new(settings.clone()),
                previous: None,
                remove: Vec::new(),
                add: Vec::new(),
                outside,
            },
            &quiet(),
        )
    };
    let check = |outside: Vec<IpNetwork>| {
        super::change(
            &NetworkChange::Check {
                settings: Box::new(settings.clone()),
                outside,
            },
            &quiet(),
        )
    };
    assert!(
        matches!(apply(Vec::new()), NetworkOutcome::Applied { .. }),
        "the tunnel's own route"
    );
    assert_eq!(
        check(Vec::new()),
        NetworkOutcome::Checked { complete: true }
    );

    network::add_route(&through_primary).unwrap();
    assert_eq!(
        check(Vec::new()),
        NetworkOutcome::Checked { complete: false }
    );
    assert_eq!(
        check(vec![excluded]),
        NetworkOutcome::Checked { complete: true }
    );
    match apply(Vec::new()) {
        NetworkOutcome::Failed { step, error, .. } => {
            assert_eq!(step, NetworkStep::Routes);
            assert!(
                error.starts_with("the route for 198.51.100.128/25 goes through "),
                "{error}"
            );
            assert!(error.contains("(198.51.100.128/26)"), "{error}");
        }
        other => panic!("not failed: {other:?}"),
    }
    assert!(
        matches!(apply(vec![excluded]), NetworkOutcome::Applied { .. }),
        "kept outside"
    );
}

#[test]
fn a_tunnel_without_an_index_cannot_be_looked_up() {
    let outcome = super::change(
        &NetworkChange::LookUp {
            tunnel: TunnelDevice {
                name: "utun9".to_owned(),
                index: None,
            },
            outside: Vec::new(),
        },
        &quiet(),
    );
    assert_eq!(
        outcome,
        NetworkOutcome::Failed {
            step: NetworkStep::Network,
            error: "the tunnel utun9 has no interface index".to_owned(),
            added: Vec::new(),
            left: Vec::new(),
        }
    );
}
