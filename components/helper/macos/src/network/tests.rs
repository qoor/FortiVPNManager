use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use fvm_contract::helper_internal::{FamilySettings, TunnelDevice};

use super::*;
use crate::routes::{Ipv4State, primary_service_id};

fn network(address: &str, prefix_length: u8) -> IpNetwork {
    IpNetwork {
        address: address.parse().unwrap(),
        prefix_length,
    }
}

fn addresses(addresses: &[&str]) -> Vec<IpAddr> {
    addresses
        .iter()
        .map(|address| address.parse().unwrap())
        .collect()
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn split() -> NetworkSettings {
    NetworkSettings {
        tunnel: TunnelDevice {
            name: "utun5".into(),
            index: None,
        },
        gateway: "203.0.113.10".parse().unwrap(),
        mtu: Some(1354),
        ipv4: FamilySettings {
            address: network("10.212.133.105", 32),
            routes: TunnelRoutes::Split {
                include: vec![network("10.30.0.0", 16), network("192.168.50.0", 24)],
            },
        },
        ipv6: None,
        split_exclude: vec![network("10.30.161.0", 24)],
        dns_servers: addresses(&["10.30.0.53", "10.30.0.54"]),
        default_domains: strings(&["corp.example.com"]),
        split_dns_domains: strings(&["corp.example.com", "example.net"]),
        wins_servers: vec![],
    }
}

fn full() -> NetworkSettings {
    NetworkSettings {
        tunnel: TunnelDevice {
            name: "utun5".into(),
            index: None,
        },
        gateway: "203.0.113.10".parse().unwrap(),
        mtu: None,
        ipv4: FamilySettings {
            address: network("10.212.133.105", 24),
            routes: TunnelRoutes::Full {},
        },
        ipv6: Some(FamilySettings {
            address: network("fd00:10::5", 64),
            routes: TunnelRoutes::Full {},
        }),
        split_exclude: vec![],
        dns_servers: addresses(&["10.30.0.53", "fd00:10::53"]),
        default_domains: vec![],
        split_dns_domains: vec![],
        wins_servers: vec!["10.30.0.60".parse().unwrap()],
    }
}

fn add_arguments(settings: &NetworkSettings) -> Vec<Vec<String>> {
    tunnel_routes(settings)
        .iter()
        .map(Route::add_arguments)
        .collect()
}

fn argv(arguments: &[&[&str]]) -> Vec<Vec<String>> {
    arguments
        .iter()
        .map(|arguments| strings(arguments))
        .collect()
}

#[test]
fn a_split_tunnel_routes_its_includes_through_its_own_address() {
    assert_eq!(
        add_arguments(&split()),
        argv(&[
            &[
                "add",
                "-net",
                "10.30.0.0",
                "-netmask",
                "255.255.0.0",
                "10.212.133.105"
            ],
            &[
                "add",
                "-net",
                "192.168.50.0",
                "-netmask",
                "255.255.255.0",
                "10.212.133.105"
            ],
        ])
    );
}

#[test]
fn a_full_tunnel_covers_everything_beside_the_default_route() {
    assert_eq!(
        add_arguments(&full()),
        argv(&[
            &[
                "add",
                "-net",
                "10.212.133.0",
                "-netmask",
                "255.255.255.0",
                "10.212.133.105"
            ],
            &[
                "add",
                "-net",
                "0.0.0.0",
                "-netmask",
                "128.0.0.0",
                "10.212.133.105"
            ],
            &[
                "add",
                "-net",
                "128.0.0.0",
                "-netmask",
                "128.0.0.0",
                "10.212.133.105"
            ],
            &["add", "-inet6", "-net", "::/1", "fd00:10::5"],
            &["add", "-inet6", "-net", "8000::/1", "fd00:10::5"],
        ])
    );
}

#[test]
fn an_include_of_the_tunnels_own_network_is_routed_once() {
    let mut settings = split();
    settings.ipv4 = FamilySettings {
        address: network("10.212.133.105", 24),
        routes: TunnelRoutes::Split {
            include: vec![network("10.212.133.0", 24), network("10.30.0.0", 16)],
        },
    };
    assert_eq!(
        add_arguments(&settings),
        argv(&[
            &[
                "add",
                "-net",
                "10.212.133.0",
                "-netmask",
                "255.255.255.0",
                "10.212.133.105"
            ],
            &[
                "add",
                "-net",
                "10.30.0.0",
                "-netmask",
                "255.255.0.0",
                "10.212.133.105"
            ],
        ])
    );
}

#[test]
fn the_tunnel_routes_are_found_as_netstat_prints_them() {
    let mut table = route_table::parse(
        "Routing tables

Internet:
Destination        Gateway            Flags               Netif Expire
0/1                10.212.133.105     UGSc                utun5
default            10.71.161.254      UGScg                 en0
10.212.133/24      10.212.133.105     UGSc                utun5
10.212.133.105     10.212.133.105     UH                  utun5
128.0/1            10.212.133.105     UGSc                utun5

",
        Family::Inet,
    );
    table.extend(route_table::parse(
        "Routing tables

Internet6:
Destination                             Gateway                                 Flags               Netif Expire
::/1                                    fd00:10::5                              UGSc                utun5
8000::/1                                fd00:10::5                              UGSc                utun5

",
        Family::Inet6,
    ));
    assert!(
        tunnel_routes(&full())
            .iter()
            .all(|route| has(&table, route))
    );
    assert!(
        !tunnel_routes(&split())
            .iter()
            .any(|route| has(&table, route))
    );
}

fn next() -> NetworkSettings {
    let mut settings = split();
    settings.tunnel = TunnelDevice {
        name: "utun6".into(),
        index: None,
    };
    settings.ipv4 = FamilySettings {
        address: network("10.212.133.106", 32),
        routes: TunnelRoutes::Split {
            include: vec![network("10.30.0.0", 16), network("172.16.0.0", 12)],
        },
    };
    settings
}

fn switch(netstat: &str) -> Vec<Vec<String>> {
    let table = route_table::parse(
        &format!(
            "Routing tables

Internet:
Destination        Gateway            Flags               Netif Expire
{netstat}

"
        ),
        Family::Inet,
    );
    switch_steps(&tunnel_routes(&next()), &tunnel_routes(&split()), &table)
        .iter()
        .map(SwitchStep::arguments)
        .collect()
}

#[test]
fn the_switch_moves_the_shared_routes_adds_the_new_ones_then_deletes_the_rest() {
    assert_eq!(
        switch(
            "10.30/16           10.212.133.105     UGSc                utun5
192.168.50         10.212.133.105     UGSc                utun5"
        ),
        argv(&[
            &[
                "change",
                "-net",
                "10.30.0.0",
                "-netmask",
                "255.255.0.0",
                "10.212.133.106"
            ],
            &[
                "add",
                "-net",
                "172.16.0.0",
                "-netmask",
                "255.240.0.0",
                "10.212.133.106"
            ],
            &[
                "delete",
                "-net",
                "192.168.50.0",
                "-netmask",
                "255.255.255.0",
                "10.212.133.105"
            ],
        ])
    );
}

#[test]
fn a_switch_tried_again_does_only_what_is_left() {
    assert_eq!(
        switch(
            "10.30/16           10.212.133.106     UGSc                utun6
192.168.50         10.212.133.105     UGSc                utun5"
        ),
        argv(&[
            &[
                "add",
                "-net",
                "172.16.0.0",
                "-netmask",
                "255.240.0.0",
                "10.212.133.106"
            ],
            &[
                "delete",
                "-net",
                "192.168.50.0",
                "-netmask",
                "255.255.255.0",
                "10.212.133.105"
            ],
        ])
    );
}

#[test]
fn with_the_replaced_tunnels_routes_gone_the_switch_adds_every_route() {
    assert_eq!(
        switch("default            10.71.161.254      UGScg                 en0"),
        argv(&[
            &[
                "add",
                "-net",
                "10.30.0.0",
                "-netmask",
                "255.255.0.0",
                "10.212.133.106"
            ],
            &[
                "add",
                "-net",
                "172.16.0.0",
                "-netmask",
                "255.240.0.0",
                "10.212.133.106"
            ],
        ])
    );
}

fn string_array(values: &[&str]) -> CFType {
    let values: Vec<CFString> = values.iter().map(|value| CFString::new(value)).collect();
    CFArray::from_CFTypes(&values).as_CFType()
}

fn dictionary(pairs: &[(&str, CFType)]) -> CFDictionary<CFString, CFType> {
    let pairs: Vec<(CFString, CFType)> = pairs
        .iter()
        .map(|(key, value)| (CFString::new(key), value.clone()))
        .collect();
    CFDictionary::from_CFType_pairs(&pairs)
}

#[test]
fn split_dns_answers_its_domains_without_marking_the_tunnel_primary() {
    let entries = dns_entries(&split());
    let expected = dictionary(&[
        (
            "ServerAddresses",
            string_array(&["10.30.0.53", "10.30.0.54"]),
        ),
        ("SearchDomains", string_array(&["corp.example.com"])),
        (
            "SupplementalMatchDomains",
            string_array(&["corp.example.com", "example.net"]),
        ),
    ]);
    assert!(entries.dns.unwrap().as_CFType() == expected.as_CFType());
    assert!(entries.ipv4.is_none());
}

#[test]
fn full_dns_marks_the_tunnel_primary_only_in_the_running_state() {
    let entries = dns_entries(&full());
    let dns = dictionary(&[(
        "ServerAddresses",
        string_array(&["10.30.0.53", "fd00:10::53"]),
    )]);
    assert!(entries.dns.unwrap().as_CFType() == dns.as_CFType());
    let ipv4 = dictionary(&[
        ("Addresses", string_array(&["10.212.133.105"])),
        ("SubnetMasks", string_array(&["255.255.255.255"])),
        ("Router", CFString::new("10.212.133.105").as_CFType()),
        ("InterfaceName", CFString::new("utun5").as_CFType()),
        ("OverridePrimary", CFNumber::from(1).as_CFType()),
    ]);
    assert!(entries.ipv4.unwrap().as_CFType() == ipv4.as_CFType());
}

#[test]
fn no_dns_servers_write_no_state() {
    let mut settings = full();
    settings.dns_servers = vec![];
    let entries = dns_entries(&settings);
    assert!(entries.dns.is_none() && entries.ipv4.is_none());
}

fn split_table(rows: &str) -> Vec<Row> {
    route_table::parse(
        &format!(
            "Routing tables

Internet:
Destination        Gateway            Flags               Netif Expire
default            10.71.161.254      UGScg                 en0
10.212.133.105     10.212.133.105     UH                  utun5
192.168.50         10.212.133.105     UGSc                utun5
203.0.113.10       10.71.161.254      UGHS                  en0
{rows}

"
        ),
        Family::Inet,
    )
}

const TUNNEL_ROW: &str = "10.30/16           10.212.133.105     UGSc                utun5";

fn owners(names: &[&str]) -> Vec<String> {
    strings(names)
}

#[test]
fn routes_through_the_tunnel_and_its_address_on_it_alone_complete_the_routing() {
    let problem = routing_problem(&split(), &[], &split_table(TUNNEL_ROW), &owners(&["utun5"]));
    assert_eq!(problem, None);
}

#[test]
fn a_route_bound_to_another_interface_fails_the_routing() {
    let table = split_table("10.30/16           10.212.133.105     UGSc                utun3");
    let problem = routing_problem(&split(), &[], &table, &owners(&["utun5"]));
    assert_eq!(
        problem,
        Some((
            NetworkStep::Routes,
            "the route for 10.30.0.0/16 goes through utun3 (10.30.0.0/16), not utun5".into()
        ))
    );
}

#[test]
fn the_tunnel_address_on_two_interfaces_fails_the_routing() {
    let table = split_table(TUNNEL_ROW);
    let problem = routing_problem(&split(), &[], &table, &owners(&["utun3", "utun5"]));
    assert_eq!(
        problem,
        Some((
            NetworkStep::Addresses,
            "the tunnel address 10.212.133.105 is on utun3, utun5, not only on utun5".into()
        ))
    );
    let missing = routing_problem(&split(), &[], &table, &[]);
    assert_eq!(
        missing.map(|(_, error)| error),
        Some("the tunnel address 10.212.133.105 is on no interface, not only on utun5".into())
    );
}

#[test]
fn a_more_specific_route_through_another_interface_fails_the_routing() {
    let table = split_table(&format!(
        "{TUNNEL_ROW}\n10.30.0/24          10.71.161.254      UGSc                  en0"
    ));
    let problem = routing_problem(&split(), &[], &table, &owners(&["utun5"]));
    assert_eq!(
        problem,
        Some((
            NetworkStep::Routes,
            "the route for 10.30.0.0/16 goes through en0 (10.30.0.0/24), not utun5".into()
        ))
    );
}

#[test]
fn a_route_kept_outside_the_tunnel_passes_the_routing() {
    let table = split_table(&format!(
        "{TUNNEL_ROW}\n10.30.0/24          10.71.161.254      UGSc                  en0"
    ));
    let outside = [network("203.0.113.10", 32), network("10.30.0.0", 24)];
    assert_eq!(
        routing_problem(&split(), &outside, &table, &owners(&["utun5"])),
        None
    );
}

#[test]
fn a_route_scoped_to_another_interface_does_not_take_the_traffic() {
    let table = split_table(&format!(
        "{TUNNEL_ROW}\n10.30.0/24          10.71.161.254      UGScI                 en0"
    ));
    assert_eq!(
        routing_problem(&split(), &[], &table, &owners(&["utun5"])),
        None
    );
}

#[test]
fn a_tunnel_route_missing_from_the_table_fails_the_routing() {
    let problem = routing_problem(&split(), &[], &split_table(""), &owners(&["utun5"]));
    assert_eq!(
        problem.map(|(_, error)| error),
        Some("the route for 10.30.0.0/16 goes through en0 (0.0.0.0/0), not utun5".into())
    );
}

#[test]
fn only_fe80_slash_10_routers_are_link_local() {
    let link_local = |address: &str| is_link_local(address.parse().unwrap());
    assert!(link_local("fe80::1"));
    assert!(link_local("febf::1"));
    assert!(!link_local("fec0::1"));
    assert!(!link_local("2001:db8::1"));
    assert!(!link_local("10.20.161.1"));
}

fn state(interface: Option<&str>, has_router: bool) -> Ipv4State {
    Ipv4State {
        interface: interface.map(str::to_owned),
        has_router,
    }
}

#[test]
fn the_primary_service_is_the_global_one_unless_it_is_a_tunnel() {
    let states = |service: &str| (service == "ETH").then(|| state(Some("en0"), true));
    let primary = |service: &str, interface: &str| {
        primary_service_id(
            Some((service.to_owned(), interface.to_owned())),
            &strings(&["ETH"]),
            states,
        )
    };
    assert_eq!(primary("ETH", "en0"), Some("ETH".into()));
    assert_eq!(primary("PPP", "ppp0"), None);
    assert_eq!(primary("OTHER", "utun3"), None);
    assert_eq!(primary_service_id(None, &strings(&["ETH"]), states), None);
}

#[test]
fn while_the_tunnel_is_primary_the_first_physical_service_in_order_is() {
    let states = |service: &str| match service {
        "BRIDGE" => Some(state(Some("bridge0"), false)),
        "VPN" => Some(state(Some("utun3"), true)),
        "WIFI" => Some(state(Some("en1"), true)),
        "ETH" => Some(state(Some("en0"), true)),
        _ => None,
    };
    let global = || Some((SERVICE_ID.to_owned(), "utun5".to_owned()));
    assert_eq!(
        primary_service_id(
            global(),
            &strings(&["GONE", "BRIDGE", "VPN", "WIFI", "ETH"]),
            states
        ),
        Some("WIFI".into())
    );
    assert_eq!(
        primary_service_id(global(), &strings(&["BRIDGE", "VPN"]), states),
        None
    );
}
