//! The `/sbin/route` commands the Helper builds and how it reads `netstat -rn -f inet`. The table is
//! as macOS 27 prints it, with the Helper's routes, the VPN's and copies the system made.

use super::*;

const TABLE: &str = "Routing tables

Internet:
Destination        Gateway            Flags               Netif Expire
default            10.20.161.254      UGScg                 en0
10.20.161/24       link#22            UCS                   en0      !
10.20.161.20/32    link#22            UCS                   en0      !
10.30.161.16       10.20.161.254      UGHS                  en0
10.30.161.17       10.20.161.254      UGHWIi                en0
10.30.161.18       10.20.161.254      UGHW                  en0
10.30.161.19       10.20.161.254      UGHSI                 en0
10.30.170/24       10.20.161.254      UGSc                  en0
10.30.171.5/32     10.212.133.100     UGSc                utun6
10.30/16           10.212.133.100     UGSc                utun6
127                127.0.0.1          UCS                   lo0
127.0.0.1          127.0.0.1          UH                    lo0
169.254            link#22            UCS                   en0      !
192.168.50         10.20.161.254      UGSc                  en0

";

const IPV6_TABLE: &str = "Routing tables

Internet6:
Destination                             Gateway                                 Flags               Netif Expire
default                                 fe80::1%en0                             UGcg                  en0
default                                 fe80::%utun0                            UGcIg               utun0
::/1                                    fd00:10::5                              UGSc                utun5
::1                                     ::1                                     UHL                   lo0
2001:db8::10                            fe80::1%en0                             UGHS                  en0
2001:db8::12                            fe80::1%en0                             UGHW                  en0
2001:db8:1::/48                         2001:db8::1                             UGSc                  en0
8000::/1                                fd00:10::5                              UGSc                utun5
fe80::%lo0/64                           fe80::1%lo0                             UcI                   lo0
fe80::1%lo0                             link#1                                  UHLI                  lo0
fe80::877:2144:ddd4:6e55%en0            2:0:0:0:0:0                             UHLI                  lo0
ff00::/8                                ::1                                     UmCI                  lo0

";

fn route(destination: &str, mask: &str) -> Route {
    route_through(destination, mask, "10.20.161.254")
}

fn route_through(destination: &str, mask: &str, gateway: &str) -> Route {
    Route::with_mask(destination, mask, gateway).unwrap()
}

fn host() -> Route {
    route("10.30.161.16", "255.255.255.255")
}

fn network() -> Route {
    route("10.30.170.0", "255.255.255.0")
}

fn has(route: &Route) -> bool {
    parse(TABLE, Family::Inet)
        .iter()
        .any(|row| route.matches(row))
}

fn has_ipv6(route: &Route) -> bool {
    parse(IPV6_TABLE, Family::Inet6)
        .iter()
        .any(|row| route.matches(row))
}

fn ipv6(destination: &str, prefix_length: u8, gateway: &str) -> Route {
    Route::new(destination, prefix_length, gateway).unwrap()
}

#[test]
fn only_dotted_quads_and_contiguous_masks_are_routes() {
    let gateway = "10.20.161.254";
    assert_eq!(Route::with_mask("-net", "255.255.255.255", gateway), None);
    assert_eq!(
        Route::with_mask("10.30.161.16", "255.0.255.0", gateway),
        None
    );
    assert_eq!(
        Route::with_mask("10.30.161.16", "255.255.255.255", "10.20.161.254 "),
        None
    );
    assert_eq!(
        Route::with_mask("010.30.161.16", "255.255.255.255", gateway),
        None
    );
    assert_eq!(
        Route::with_mask("0.0.0.0", "0.0.0.0", gateway).map(|route| route.prefix_length),
        Some(0)
    );
    let bypass = BypassRoute {
        destination: "10.30.170.0".into(),
        subnet_mask: "255.255.255.0".into(),
        gateway: gateway.into(),
    };
    assert_eq!(Route::from_bypass(&bypass), Some(network()));
    assert_eq!(network().prefix_length, 24);
}

#[test]
fn a_host_is_named_as_one_and_a_network_with_its_mask() {
    assert_eq!(
        host().add_arguments(),
        ["add", "-host", "10.30.161.16", "10.20.161.254"]
    );
    assert_eq!(
        host().delete_arguments(),
        ["delete", "-host", "10.30.161.16", "10.20.161.254"]
    );
    assert_eq!(
        network().add_arguments(),
        [
            "add",
            "-net",
            "10.30.170.0",
            "-netmask",
            "255.255.255.0",
            "10.20.161.254"
        ]
    );
    assert_eq!(
        network().delete_arguments(),
        [
            "delete",
            "-net",
            "10.30.170.0",
            "-netmask",
            "255.255.255.0",
            "10.20.161.254"
        ]
    );
}

#[test]
fn the_table_is_read_with_netstats_short_forms() {
    let rows = parse(TABLE, Family::Inet);
    assert_eq!(rows.len(), 14);
    let found = |destination: &str, length: u8| {
        rows.iter()
            .find(|row| row.destination == destination && row.prefix_length == length)
    };
    assert_eq!(found("0.0.0.0", 0).unwrap().gateway, "10.20.161.254");
    assert_eq!(found("10.20.161.0", 24).unwrap().flags, "UCS");
    assert!(found("10.20.161.20", 32).is_some());
    assert_eq!(found("10.30.0.0", 16).unwrap().interface, "utun6");
    assert!(found("127.0.0.0", 8).is_some());
    assert!(found("127.0.0.1", 32).is_some());
    assert!(found("169.254.0.0", 16).is_some());
    assert!(found("192.168.50.0", 24).is_some());
}

#[test]
fn the_routes_the_helper_adds_are_found() {
    assert!(has(&host()));
    assert!(has(&network()));
    assert!(has(&route("192.168.50.0", "255.255.255.0")));
}

/// A copy of the default route the system made for one host (W), or a route bound to one
/// interface (I), is not a route the Helper adds.
#[test]
fn copies_and_scoped_routes_are_not_the_helpers() {
    assert!(!has(&route("10.30.161.17", "255.255.255.255")));
    assert!(!has(&route("10.30.161.18", "255.255.255.255")));
    assert!(!has(&route("10.30.161.19", "255.255.255.255")));
}

#[test]
fn another_gateway_or_prefix_is_another_route() {
    assert!(!has(&route_through(
        "10.30.161.16",
        "255.255.255.255",
        "10.20.161.1"
    )));
    assert!(!has(&route("10.30.170.0", "255.255.254.0")));
    // The VPN's single host is a /32 network through the tunnel.
    assert!(!has(&route("10.30.171.5", "255.255.255.255")));
}

#[test]
fn nothing_before_the_header_is_a_route() {
    assert!(parse("", Family::Inet).is_empty());
    assert!(parse("Routing tables\n\nInternet:\n", Family::Inet).is_empty());
}

#[test]
fn a_prefix_length_makes_the_same_ipv4_route_as_a_mask() {
    assert_eq!(
        Route::new("10.30.170.0", 24, "10.20.161.254"),
        Some(network())
    );
    assert_eq!(
        Route::new("0.0.0.0", 1, "10.212.133.105").map(|route| route.add_arguments()),
        Some(
            [
                "add",
                "-net",
                "0.0.0.0",
                "-netmask",
                "128.0.0.0",
                "10.212.133.105"
            ]
            .map(String::from)
            .to_vec()
        )
    );
    assert_eq!(Route::new("10.30.170.0", 33, "10.20.161.254"), None);
    assert_eq!(Route::new("10.30.170.0", 24, "fe80::1"), None);
}

#[test]
fn ipv6_routes_are_canonical_and_named_with_their_prefix() {
    let host = ipv6("2001:DB8:0:0::10", 128, "FE80:0::1%en0");
    assert_eq!(host.family, Family::Inet6);
    assert_eq!(host.destination, "2001:db8::10");
    assert_eq!(host.gateway, "fe80::1%en0");
    assert_eq!(
        host.add_arguments(),
        ["add", "-inet6", "-host", "2001:db8::10", "fe80::1%en0"]
    );
    let network = ipv6("2001:db8:1::", 48, "2001:db8::1");
    assert_eq!(
        network.delete_arguments(),
        ["delete", "-inet6", "-net", "2001:db8:1::/48", "2001:db8::1"]
    );
}

#[test]
fn only_ipv6_addresses_and_interface_names_reach_an_ipv6_route() {
    assert_eq!(Route::new("-net", 64, "fe80::1%en0"), None);
    assert_eq!(Route::new("2001:db8::", 129, "fe80::1%en0"), None);
    assert_eq!(Route::new("2001:db8::", 64, "fe80::1%"), None);
    assert_eq!(Route::new("2001:db8::", 64, "fe80::1%-ifscope"), None);
    assert_eq!(Route::new("2001:db8::", 64, "10.20.161.254"), None);
    assert_eq!(Route::new("2001:db8::", 64, "fe80::1 "), None);
}

#[test]
fn the_ipv6_table_is_read_with_scopes_and_hosts_without_prefix() {
    let rows = parse(IPV6_TABLE, Family::Inet6);
    assert_eq!(rows.len(), 12);
    assert_eq!(
        rows[0],
        Row {
            destination: "::".into(),
            prefix_length: 0,
            gateway: "fe80::1%en0".into(),
            flags: "UGcg".into(),
            interface: "en0".into(),
        }
    );
    let found = |destination: &str, length: u8| {
        rows.iter()
            .find(|row| row.destination == destination && row.prefix_length == length)
    };
    assert!(found("::1", 128).is_some());
    assert!(found("2001:db8::10", 128).is_some());
    assert!(found("8000::", 1).is_some());
    assert_eq!(found("fe80::", 64).unwrap().gateway, "fe80::1%lo0");
    assert_eq!(found("fe80::1", 128).unwrap().gateway, "link#1");
    assert_eq!(
        found("fe80::877:2144:ddd4:6e55", 128).unwrap().gateway,
        "2:0:0:0:0:0"
    );
}

#[test]
fn the_ipv6_routes_the_helper_adds_are_found() {
    assert!(has_ipv6(&ipv6("2001:db8::10", 128, "fe80::1%en0")));
    assert!(has_ipv6(&ipv6("2001:db8:1::", 48, "2001:db8::1")));
    assert!(has_ipv6(&ipv6("::", 1, "fd00:10::5")));
    assert!(!has_ipv6(&ipv6("2001:db8::12", 128, "fe80::1%en0")));
    assert!(!has_ipv6(&ipv6("2001:db8::10", 128, "fe80::1%en1")));
    assert!(!has_ipv6(&ipv6("::", 0, "fe80::1%en0")));
}
