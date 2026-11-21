use fvm_contract::helper_internal::{FamilySettings, TunnelDevice, TunnelRoutes};

use super::*;

const CONFIGURED: &str = "utun5: flags=8051<UP,POINTOPOINT,RUNNING,MULTICAST> mtu 1412
\tinet 10.212.133.105 --> 10.212.133.105 netmask 0xffffffff
\tinet6 fe80::8683:2d63:15a1:fa95%utun5 prefixlen 64 scopeid 0x1f
\tinet6 fd00:10::5 prefixlen 64

";

const BARE: &str = "utun5: flags=8050<POINTOPOINT,RUNNING,MULTICAST> mtu 1500

";

fn network(address: &str, prefix_length: u8) -> IpNetwork {
    IpNetwork {
        address: address.parse().unwrap(),
        prefix_length,
    }
}

fn settings(address: &str, mtu: Option<u32>, ipv6: Option<IpNetwork>) -> NetworkSettings {
    NetworkSettings {
        tunnel: TunnelDevice {
            name: "utun5".into(),
            index: None,
        },
        gateway: "203.0.113.10".parse().unwrap(),
        mtu,
        ipv4: FamilySettings {
            address: network(address, 32),
            routes: TunnelRoutes::Split {
                include: vec![network("10.30.0.0", 16)],
            },
        },
        ipv6: ipv6.map(|address| FamilySettings {
            address,
            routes: TunnelRoutes::Full {},
        }),
        split_exclude: vec![],
        dns_servers: vec![],
        default_domains: vec![],
        split_dns_domains: vec![],
        wins_servers: vec![],
    }
}

fn plain() -> NetworkSettings {
    settings("10.212.133.105", None, None)
}

fn with_ipv6(address: &str, prefix_length: u8) -> NetworkSettings {
    settings(
        "10.212.133.105",
        None,
        Some(network(address, prefix_length)),
    )
}

#[test]
fn ifconfig_is_read_with_its_flags_addresses_and_mtu() {
    let interface = parse(CONFIGURED).unwrap();
    assert!(interface.up);
    assert_eq!(interface.mtu, Some(1412));
    assert_eq!(
        interface.ipv4,
        [Ipv4Address {
            address: "10.212.133.105".into(),
            destination: Some("10.212.133.105".into()),
        }]
    );
    assert_eq!(
        interface.ipv6,
        [
            network("fe80::8683:2d63:15a1:fa95", 64),
            network("fd00:10::5", 64)
        ]
    );
    let down = parse(BARE).unwrap();
    assert!(!down.up && down.mtu == Some(1500) && down.ipv4.is_empty() && down.ipv6.is_empty());
}

#[test]
fn an_address_without_peer_has_no_destination() {
    let output = "en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500
\toptions=567<RXCSUM,TXCSUM,VLAN_MTU,TSO4,TSO6,AV,CHANNEL_IO>
\tinet 10.71.161.20 netmask 0xffffff00 broadcast 10.71.161.255

";
    let interface = parse(output).unwrap();
    assert_eq!(
        interface.ipv4,
        [Ipv4Address {
            address: "10.71.161.20".into(),
            destination: None,
        }]
    );
}

#[test]
fn output_without_flags_is_no_interface() {
    assert_eq!(parse(""), None);
    assert_eq!(parse("ifconfig: interface utun9 does not exist\n"), None);
}

#[test]
fn a_bare_tunnel_gets_its_address_up_and_the_default_mtu() {
    let changes = changes(&plain(), &parse(BARE).unwrap());
    let steps: Vec<NetworkStep> = changes.iter().map(|(step, _)| *step).collect();
    assert_eq!(
        steps,
        [
            NetworkStep::Addresses,
            NetworkStep::Addresses,
            NetworkStep::Mtu
        ]
    );
    let arguments: Vec<Vec<String>> = changes
        .into_iter()
        .map(|(_, arguments)| arguments)
        .collect();
    assert_eq!(
        arguments,
        [
            strings(&[
                "inet",
                "10.212.133.105",
                "10.212.133.105",
                "netmask",
                "255.255.255.255"
            ]),
            strings(&["up"]),
            strings(&["mtu", "1412"]),
        ]
    );
}

#[test]
fn the_given_mtu_and_ipv6_address_are_set() {
    let wanted = settings(
        "10.212.133.105",
        Some(1354),
        Some(network("fd00:10::5", 64)),
    );
    let changes = changes(&wanted, &parse(BARE).unwrap());
    let steps: Vec<NetworkStep> = changes.iter().map(|(step, _)| *step).collect();
    assert_eq!(
        steps,
        [
            NetworkStep::Addresses,
            NetworkStep::Addresses,
            NetworkStep::Addresses,
            NetworkStep::Mtu
        ]
    );
    let arguments: Vec<Vec<String>> = changes
        .into_iter()
        .map(|(_, arguments)| arguments)
        .collect();
    assert_eq!(
        arguments,
        [
            strings(&[
                "inet",
                "10.212.133.105",
                "10.212.133.105",
                "netmask",
                "255.255.255.255"
            ]),
            strings(&["inet6", "fd00:10::5/64"]),
            strings(&["up"]),
            strings(&["mtu", "1354"]),
        ]
    );
}

#[test]
fn a_tunnel_that_has_the_settings_is_left_alone() {
    let interface = parse(CONFIGURED).unwrap();
    let wanted = with_ipv6("FD00:10:0::5", 64);
    assert!(changes(&wanted, &interface).is_empty());
    assert_eq!(interface.mismatch(&wanted, None), None);
}

#[test]
fn only_addresses_the_settings_no_longer_have_are_removed() {
    let dual = with_ipv6("fd00:10::5", 64);
    assert_eq!(
        removals(&dual, &plain()),
        [strings(&["inet6", "fd00:10::5", "delete"])]
    );
    assert_eq!(
        removals(&dual, &with_ipv6("fd00:10::5", 80)),
        [strings(&["inet6", "fd00:10::5", "delete"])]
    );
    assert_eq!(
        removals(&plain(), &settings("10.212.133.106", None, None)),
        [strings(&["inet", "10.212.133.105", "delete"])]
    );
    assert!(removals(&dual, &dual).is_empty());
}

#[test]
fn a_missing_address_a_leftover_or_another_mtu_is_a_mismatch() {
    let interface = parse(CONFIGURED).unwrap();
    assert_eq!(interface.mismatch(&plain(), None), None);
    assert_eq!(
        interface.mismatch(&plain(), Some(&with_ipv6("fd00:10::5", 64))),
        Some(NetworkStep::Addresses)
    );
    assert_eq!(
        interface.mismatch(&settings("10.212.133.106", None, None), None),
        Some(NetworkStep::Addresses)
    );
    assert_eq!(
        interface.mismatch(&with_ipv6("fd00:10::6", 64), None),
        Some(NetworkStep::Addresses)
    );
    assert_eq!(
        interface.mismatch(&settings("10.212.133.105", Some(1354), None), None),
        Some(NetworkStep::Mtu)
    );
    assert_eq!(
        parse(BARE).unwrap().mismatch(&plain(), None),
        Some(NetworkStep::Addresses)
    );
}
