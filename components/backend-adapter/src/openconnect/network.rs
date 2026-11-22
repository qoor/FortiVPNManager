use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use fvm_contract::helper_internal::{
    FamilySettings, IpNetwork, NetworkSettings, TunnelDevice, TunnelRoutes,
};

const MIN_MTU: u32 = 68;
const MAX_MTU: u32 = 65_535;
const UNSET_NBNS: &str = "0.0.0.0";

pub(super) struct IpInfo {
    pub(super) addr: Option<String>,
    pub(super) netmask: Option<String>,
    pub(super) addr6: Option<String>,
    pub(super) netmask6: Option<String>,
    pub(super) dns: Vec<String>,
    pub(super) nbns: Vec<String>,
    pub(super) domain: Option<String>,
    pub(super) mtu: i32,
    pub(super) split_dns: Vec<String>,
    pub(super) split_includes: Vec<String>,
    pub(super) split_excludes: Vec<String>,
    pub(super) gateway_addr: Option<String>,
}

pub(super) fn settings(
    ip: &IpInfo,
    name: String,
    index: Option<u32>,
) -> Result<NetworkSettings, String> {
    let mtu = u32::try_from(ip.mtu)
        .ok()
        .filter(|mtu| (MIN_MTU..=MAX_MTU).contains(mtu))
        .ok_or_else(|| format!("the MTU {} is outside {MIN_MTU} to {MAX_MTU}", ip.mtu))?;
    let ipv4_address = required::<Ipv4Addr>("IPv4 address", ip.addr.as_deref())?;
    let ipv4_prefix = text(ip.netmask.as_deref())
        .and_then(mask_length)
        .unwrap_or(32);
    let ipv4_include = ipv4_routes(&ip.split_includes);
    let ipv4_tunnel_routes = if ipv4_include.is_empty()
        || ipv4_include
            .iter()
            .any(|network| network.address.is_unspecified())
    {
        TunnelRoutes::Full {}
    } else {
        TunnelRoutes::Split {
            include: ipv4_include,
        }
    };
    let mut split_exclude: Vec<IpNetwork> = ipv4_routes(&ip.split_excludes)
        .into_iter()
        .filter(|network| forwardable(network.address))
        .collect();
    split_exclude.extend(ipv6_routes(&ip.split_excludes)?);
    let ipv6 = match ipv6_address(ip)? {
        None => None,
        Some(address) => {
            let include = ipv6_routes(&ip.split_includes)?;
            let routes =
                if include.is_empty() || include.iter().any(|network| network.prefix_length == 0) {
                    TunnelRoutes::Full {}
                } else {
                    TunnelRoutes::Split { include }
                };
            Some(FamilySettings { address, routes })
        }
    };
    let dns_servers = ip
        .dns
        .iter()
        .flat_map(|servers| servers.split_whitespace())
        .map(|server| parse::<IpAddr>("DNS server", server))
        .collect::<Result<Vec<_>, _>>()?;
    let wins_servers = if ip.nbns.iter().any(|server| server == UNSET_NBNS) {
        vec![Ipv4Addr::UNSPECIFIED]
    } else {
        Vec::new()
    };
    Ok(NetworkSettings {
        tunnel: TunnelDevice { name, index },
        gateway: required("gateway address", ip.gateway_addr.as_deref())?,
        mtu: Some(mtu),
        ipv4: FamilySettings {
            address: IpNetwork {
                address: ipv4_address.into(),
                prefix_length: ipv4_prefix,
            },
            routes: ipv4_tunnel_routes,
        },
        ipv6,
        split_exclude,
        dns_servers: unique(dns_servers),
        default_domains: domains("default domain", text(ip.domain.as_deref()).into_iter())?,
        split_dns_domains: domains("split DNS domain", ip.split_dns.iter().map(String::as_str))?,
        wins_servers,
    })
}

fn text(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.is_empty())
}

fn required<T: FromStr>(what: &str, value: Option<&str>) -> Result<T, String> {
    parse(
        what,
        text(value).ok_or_else(|| format!("the {what} is missing"))?,
    )
}

fn parse<T: FromStr>(what: &str, text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("the {what} is unreadable: {text:?}"))
}

fn mask_length(mask: &str) -> Option<u8> {
    let bits = u32::from(mask.parse::<Ipv4Addr>().ok()?);
    let length = bits.leading_ones();
    (bits.checked_shl(length).unwrap_or(0) == 0).then(|| u8::try_from(length).expect("at most 32"))
}

fn ipv6_mask_length(mask: &str) -> Option<u8> {
    let bits = u128::from(mask.parse::<Ipv6Addr>().ok()?);
    let length = bits.leading_ones();
    (bits.checked_shl(length).unwrap_or(0) == 0 && length % 8 == 0)
        .then(|| u8::try_from(length).expect("at most 128"))
}

fn ipv4_routes(routes: &[String]) -> Vec<IpNetwork> {
    routes
        .iter()
        .filter(|route| !route.contains(':'))
        .filter_map(|route| ipv4_route(route))
        .collect()
}

fn ipv4_route(route: &str) -> Option<IpNetwork> {
    let (address, mask) = match route.split_once('/') {
        Some((address, mask)) => (address, Some(mask)),
        None => (route, None),
    };
    let address = address.parse::<Ipv4Addr>().ok()?;
    let prefix_length = match mask {
        None => 32,
        Some(mask) => match leading_number(mask) {
            (length @ 0..=32, rest) if !rest.starts_with('.') => {
                u8::try_from(length).expect("at most 32")
            }
            _ => mask_length(mask)?,
        },
    };
    Some(network(address.into(), prefix_length))
}

fn ipv6_routes(routes: &[String]) -> Result<Vec<IpNetwork>, String> {
    routes
        .iter()
        .filter(|route| route.contains(':'))
        .filter_map(|route| ipv6_route(route))
        .collect()
}

fn ipv6_route(route: &str) -> Option<Result<IpNetwork, String>> {
    let unreadable = || format!("the IPv6 route is unreadable: {route:?}");
    let (address, prefix_length) = match route.split_once('/') {
        None => (route, 128),
        Some((address, mask)) => {
            let prefix_length = match leading_number(mask) {
                (length, rest) if length <= 128 && rest.chars().next().is_none_or(is_c_space) => {
                    match u8::try_from(length) {
                        Ok(length) => length,
                        Err(_) => return Some(Err(unreadable())),
                    }
                }
                _ => ipv6_mask_length(mask)?,
            };
            (
                if address.is_empty() { route } else { address },
                prefix_length,
            )
        }
    };
    Some(
        address
            .parse::<Ipv6Addr>()
            .map(|address| network(address.into(), prefix_length))
            .map_err(|_| unreadable()),
    )
}

fn leading_number(text: &str) -> (i64, &str) {
    let trimmed = text.trim_start_matches(is_c_space);
    let (negative, unsigned) = match trimmed.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let digits = unsigned.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return (0, text);
    }
    let value = unsigned[..digits].parse::<i64>().unwrap_or(i64::MAX);
    (if negative { -value } else { value }, &unsigned[digits..])
}

fn is_c_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r')
}

fn network(address: IpAddr, prefix_length: u8) -> IpNetwork {
    let address = match address {
        IpAddr::V4(address) => {
            let kept = u32::MAX
                .checked_shl(32 - u32::from(prefix_length))
                .unwrap_or(0);
            Ipv4Addr::from(u32::from(address) & kept).into()
        }
        IpAddr::V6(address) => {
            let kept = u128::MAX
                .checked_shl(128 - u32::from(prefix_length))
                .unwrap_or(0);
            Ipv6Addr::from(u128::from(address) & kept).into()
        }
    };
    IpNetwork {
        address,
        prefix_length,
    }
}

fn ipv6_address(ip: &IpInfo) -> Result<Option<IpNetwork>, String> {
    let address = text(ip.addr6.as_deref())
        .map(|address| parse::<Ipv6Addr>("IPv6 address", address))
        .transpose()?;
    let with_length = text(ip.netmask6.as_deref())
        .map(|text| {
            let unreadable =
                || format!("the IPv6 address and prefix length are unreadable: {text:?}");
            let (network_address, length) = text.split_once('/').ok_or_else(unreadable)?;
            let network_address = network_address
                .parse::<Ipv6Addr>()
                .map_err(|_| unreadable())?;
            let length = length
                .parse::<u8>()
                .ok()
                .filter(|length| *length <= 128)
                .ok_or_else(unreadable)?;
            Ok::<_, String>((network_address, length))
        })
        .transpose()?;
    Ok(match (address, with_length) {
        (None, None) => None,
        (Some(address), None) => Some(IpNetwork {
            address: address.into(),
            prefix_length: 128,
        }),
        (address, Some((network_address, length))) => Some(IpNetwork {
            address: address.unwrap_or(network_address).into(),
            prefix_length: length,
        }),
    })
}

fn forwardable(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [first, second, ..] = address.octets();
            first != 0 && first != 127 && (first, second) != (169, 254)
        }
        IpAddr::V6(_) => true,
    }
}

fn domains<'a>(what: &str, lists: impl Iterator<Item = &'a str>) -> Result<Vec<String>, String> {
    let listed = lists
        .flat_map(|list| list.split(|c: char| c.is_whitespace() || c == ','))
        .filter(|domain| !domain.is_empty())
        .map(|domain| {
            if is_domain(domain) {
                Ok(domain.to_owned())
            } else {
                Err(format!("the {what} is not a domain: {domain:?}"))
            }
        })
        .collect::<Result<_, String>>()?;
    Ok(unique(listed))
}

fn is_domain(domain: &str) -> bool {
    domain.len() <= 253
        && domain.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        })
}

fn unique<T: PartialEq>(items: Vec<T>) -> Vec<T> {
    let mut kept = Vec::with_capacity(items.len());
    for item in items {
        if !kept.contains(&item) {
            kept.push(item);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip_info() -> IpInfo {
        IpInfo {
            addr: Some("10.10.0.5".into()),
            netmask: None,
            addr6: None,
            netmask6: None,
            dns: Vec::new(),
            nbns: Vec::new(),
            domain: None,
            mtu: 1351,
            split_dns: Vec::new(),
            split_includes: Vec::new(),
            split_excludes: Vec::new(),
            gateway_addr: Some("203.0.113.10".into()),
        }
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|&item| item.to_owned()).collect()
    }

    fn net(address: &str, prefix_length: u8) -> IpNetwork {
        IpNetwork {
            address: address.parse().unwrap(),
            prefix_length,
        }
    }

    fn address(address: &str) -> IpAddr {
        address.parse().unwrap()
    }

    #[test]
    fn a_split_tunnel_is_read_without_the_unroutable_excludes() {
        let ip = IpInfo {
            dns: strings(&["10.20.0.53", "10.20.0.54", "10.20.0.53"]),
            nbns: strings(&["10.20.0.60"]),
            domain: Some("corp.example.com".into()),
            split_dns: strings(&["corp.example.com", "example.net"]),
            split_includes: strings(&["10.20.161.0/255.255.255.0", "192.168.50.7/255.255.255.0"]),
            split_excludes: strings(&[
                "10.20.161.16/255.255.255.255",
                "0.0.0.0/255.0.0.0",
                "127.0.0.0/255.0.0.0",
                "169.254.0.0/255.255.0.0",
            ]),
            ..ip_info()
        };
        assert_eq!(
            settings(&ip, "utun5".into(), None),
            Ok(NetworkSettings {
                tunnel: TunnelDevice {
                    name: "utun5".into(),
                    index: None,
                },
                gateway: address("203.0.113.10"),
                mtu: Some(1351),
                ipv4: FamilySettings {
                    address: net("10.10.0.5", 32),
                    routes: TunnelRoutes::Split {
                        include: vec![net("10.20.161.0", 24), net("192.168.50.0", 24)],
                    },
                },
                ipv6: None,
                split_exclude: vec![net("10.20.161.16", 32)],
                dns_servers: vec![address("10.20.0.53"), address("10.20.0.54")],
                default_domains: vec!["corp.example.com".into()],
                split_dns_domains: vec!["corp.example.com".into(), "example.net".into()],
                wins_servers: Vec::new(),
            })
        );
    }

    #[test]
    fn a_full_tunnel_with_ipv6_is_read_with_the_windows_interface_index() {
        let ip = IpInfo {
            netmask: Some("255.255.255.0".into()),
            addr6: Some("fd00:10::5".into()),
            netmask6: Some("fd00:10::/64".into()),
            dns: strings(&["10.20.0.53", "fd00:10::53"]),
            split_includes: strings(&[
                "10.20.161.0/255.255.255.0",
                "0.0.0.0/0.0.0.0",
                "fd00:20::1/48",
            ]),
            split_excludes: strings(&["2001:db8:5::/64"]),
            gateway_addr: Some("2001:db8::10".into()),
            ..ip_info()
        };
        assert_eq!(
            settings(&ip, "203.0.113.10".into(), Some(54)),
            Ok(NetworkSettings {
                tunnel: TunnelDevice {
                    name: "203.0.113.10".into(),
                    index: Some(54),
                },
                gateway: address("2001:db8::10"),
                mtu: Some(1351),
                ipv4: FamilySettings {
                    address: net("10.10.0.5", 24),
                    routes: TunnelRoutes::Full {},
                },
                ipv6: Some(FamilySettings {
                    address: net("fd00:10::5", 64),
                    routes: TunnelRoutes::Split {
                        include: vec![net("fd00:20::", 48)],
                    },
                }),
                split_exclude: vec![net("2001:db8:5::", 64)],
                dns_servers: vec![address("10.20.0.53"), address("fd00:10::53")],
                default_domains: Vec::new(),
                split_dns_domains: Vec::new(),
                wins_servers: Vec::new(),
            })
        );
    }

    #[test]
    fn no_split_include_is_a_full_tunnel_for_each_family() {
        let ip = IpInfo {
            netmask: Some("0.0.0.0".into()),
            addr6: Some("fd00:10::5".into()),
            split_includes: strings(&["::/0"]),
            domain: Some("corp.example.com example.net".into()),
            ..ip_info()
        };
        let settings = settings(&ip, "utun5".into(), None).unwrap();
        assert_eq!(
            settings.ipv4,
            FamilySettings {
                address: net("10.10.0.5", 0),
                routes: TunnelRoutes::Full {},
            }
        );
        assert_eq!(
            settings.ipv6,
            Some(FamilySettings {
                address: net("fd00:10::5", 128),
                routes: TunnelRoutes::Full {},
            })
        );
        assert_eq!(
            settings.default_domains,
            vec!["corp.example.com".to_owned(), "example.net".to_owned()]
        );
    }

    #[test]
    fn split_routes_and_masks_are_read_as_openconnect_reads_them() {
        let ip = IpInfo {
            netmask: Some("255.0.255.0".into()),
            addr6: Some("fd00:10::5".into()),
            split_includes: strings(&[
                "192.168.50.7/255.255.255.0",
                "10.30.0.0/16",
                "10.40.0.0/255.0.255.0",
                "10.50.0.9",
                "bad/24",
                "fd00:20::1/48",
                "fd00:30::/ffff:ffff::",
                "fd00:40::/ffff:fff0::",
                "fd00:50::/129",
            ]),
            ..ip_info()
        };
        let settings = settings(&ip, "utun5".into(), None).unwrap();
        assert_eq!(
            settings.ipv4,
            FamilySettings {
                address: net("10.10.0.5", 32),
                routes: TunnelRoutes::Split {
                    include: vec![
                        net("192.168.50.0", 24),
                        net("10.30.0.0", 16),
                        net("10.50.0.9", 32),
                    ],
                },
            }
        );
        assert_eq!(
            settings.ipv6.unwrap().routes,
            TunnelRoutes::Split {
                include: vec![net("fd00:20::", 48), net("fd00:30::", 32)],
            }
        );
    }

    #[test]
    fn the_mtu_must_be_between_68_and_65535() {
        for (mtu, expected) in [
            (68, Some(68)),
            (65_535, Some(65_535)),
            (67, None),
            (65_536, None),
            (0, None),
            (-1, None),
        ] {
            let read = settings(&IpInfo { mtu, ..ip_info() }, "utun5".into(), None);
            assert_eq!(
                read.ok().and_then(|settings| settings.mtu),
                expected,
                "{mtu}"
            );
        }
    }

    #[test]
    fn a_wins_server_is_set_only_when_the_gateway_sends_0_0_0_0() {
        for (nbns, wins) in [
            (Vec::new(), Vec::new()),
            (strings(&["10.20.0.60"]), Vec::new()),
            (
                strings(&["10.20.0.60", "0.0.0.0"]),
                vec![Ipv4Addr::UNSPECIFIED],
            ),
        ] {
            let read = settings(&IpInfo { nbns, ..ip_info() }, "utun5".into(), None);
            assert_eq!(read.unwrap().wins_servers, wins);
        }
    }

    #[test]
    fn values_that_cannot_be_applied_are_refused() {
        let refused = [
            (
                IpInfo {
                    addr: None,
                    ..ip_info()
                },
                "the IPv4 address is missing",
            ),
            (
                IpInfo {
                    gateway_addr: Some("vpn.example.com".into()),
                    ..ip_info()
                },
                "the gateway address is unreadable: \"vpn.example.com\"",
            ),
            (
                IpInfo {
                    dns: strings(&["10.20.0.53 dns"]),
                    ..ip_info()
                },
                "the DNS server is unreadable: \"dns\"",
            ),
            (
                IpInfo {
                    split_dns: strings(&["example.net", "bad domain!"]),
                    ..ip_info()
                },
                "the split DNS domain is not a domain: \"domain!\"",
            ),
            (
                IpInfo {
                    addr6: Some("fd00:10::5".into()),
                    netmask6: Some("fd00::/129".into()),
                    ..ip_info()
                },
                "the IPv6 address and prefix length are unreadable: \"fd00::/129\"",
            ),
            (
                IpInfo {
                    split_excludes: strings(&["zz::1/64"]),
                    ..ip_info()
                },
                "the IPv6 route is unreadable: \"zz::1/64\"",
            ),
        ];
        for (ip, error) in refused {
            assert_eq!(settings(&ip, "utun5".into(), None), Err(error.to_owned()));
        }
    }
}
