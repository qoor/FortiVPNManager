use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::{self, FromStr};

use fvm_contract::backend::{
    FamilySettings, IpNetwork, NetworkSettings, TunnelDevice, TunnelRoutes,
};

const MIN_MTU: u32 = 68;
const MAX_MTU: u32 = 65_535;

pub(super) fn settings(bytes: &[u8]) -> Result<NetworkSettings, String> {
    let values = Values::read(bytes);
    let reason = values.required("reason")?;
    if reason != "connect" {
        return Err(format!("reason is {reason:?}, not connect"));
    }
    let name = values.required("TUNDEV")?;
    if name.starts_with('-') || name.chars().any(char::is_control) {
        return Err(format!("TUNDEV is not a device name: {name:?}"));
    }
    let mtu = values.parsed::<u32>("INTERNAL_IP4_MTU")?;
    if let Some(mtu) = mtu.filter(|mtu| !(MIN_MTU..=MAX_MTU).contains(mtu)) {
        return Err(format!(
            "INTERNAL_IP4_MTU {mtu} is outside {MIN_MTU} to {MAX_MTU}"
        ));
    }
    let ipv4_address = values.required_parsed::<Ipv4Addr>("INTERNAL_IP4_ADDRESS")?;
    let ipv4_prefix = match values.text("INTERNAL_IP4_NETMASK")? {
        Some(mask) => mask_length(mask)
            .ok_or_else(|| format!("INTERNAL_IP4_NETMASK is not a subnet mask: {mask:?}"))?,
        None => 32,
    };
    let ipv4_include = ipv4_networks(&values, "CISCO_SPLIT_INC")?;
    let ipv4_routes = if ipv4_include.is_empty()
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
    let mut split_exclude: Vec<IpNetwork> = ipv4_networks(&values, "CISCO_SPLIT_EXC")?
        .into_iter()
        .filter(|network| forwardable(network.address))
        .collect();
    split_exclude.extend(ipv6_networks(&values, "CISCO_IPV6_SPLIT_EXC")?);
    let ipv6 = match ipv6_address(&values)? {
        None => None,
        Some(address) => {
            let include = ipv6_networks(&values, "CISCO_IPV6_SPLIT_INC")?;
            let routes =
                if include.is_empty() || include.iter().any(|network| network.prefix_length == 0) {
                    TunnelRoutes::Full {}
                } else {
                    TunnelRoutes::Split { include }
                };
            Some(FamilySettings { address, routes })
        }
    };
    let mut dns_servers = addresses::<IpAddr>(&values, "INTERNAL_IP4_DNS")?;
    dns_servers.extend(
        addresses::<Ipv6Addr>(&values, "INTERNAL_IP6_DNS")?
            .into_iter()
            .map(IpAddr::V6),
    );
    Ok(NetworkSettings {
        tunnel: TunnelDevice {
            name: name.to_owned(),
            index: values.parsed("TUNIDX")?,
        },
        gateway: values.required_parsed("VPNGATEWAY")?,
        mtu,
        ipv4: FamilySettings {
            address: IpNetwork {
                address: ipv4_address.into(),
                prefix_length: ipv4_prefix,
            },
            routes: ipv4_routes,
        },
        ipv6,
        split_exclude,
        dns_servers: unique(dns_servers),
        default_domains: domains(&values, "CISCO_DEF_DOMAIN")?,
        split_dns_domains: domains(&values, "CISCO_SPLIT_DNS")?,
        wins_servers: unique(addresses(&values, "INTERNAL_IP4_NBNS")?),
    })
}

struct Values<'a>(HashMap<&'a [u8], &'a [u8]>);

impl<'a> Values<'a> {
    fn read(bytes: &'a [u8]) -> Self {
        Self(
            bytes
                .split(|&byte| byte == 0)
                .filter_map(|entry| {
                    let equals = entry.iter().skip(1).position(|&byte| byte == b'=')? + 1;
                    Some((&entry[..equals], &entry[equals + 1..]))
                })
                .collect(),
        )
    }

    fn text(&self, name: &str) -> Result<Option<&'a str>, String> {
        let Some(&value) = self.0.get(name.as_bytes()) else {
            return Ok(None);
        };
        let text = str::from_utf8(value).map_err(|_| format!("{name} is not UTF-8"))?;
        Ok(Some(text).filter(|text| !text.is_empty()))
    }

    fn required(&self, name: &str) -> Result<&'a str, String> {
        self.text(name)?.ok_or_else(|| format!("{name} is missing"))
    }

    fn parsed<T: FromStr>(&self, name: &str) -> Result<Option<T>, String> {
        self.text(name)?.map(|text| parse(name, text)).transpose()
    }

    fn required_parsed<T: FromStr>(&self, name: &str) -> Result<T, String> {
        parse(name, self.required(name)?)
    }
}

fn parse<T: FromStr>(name: &str, text: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("{name} has an unreadable value: {text:?}"))
}

fn mask_length(mask: &str) -> Option<u8> {
    let bits = u32::from(mask.parse::<Ipv4Addr>().ok()?);
    let length = bits.leading_ones();
    (bits.checked_shl(length).unwrap_or(0) == 0).then(|| u8::try_from(length).expect("at most 32"))
}

fn ipv4_networks(values: &Values, list: &str) -> Result<Vec<IpNetwork>, String> {
    (0..values.parsed::<usize>(list)?.unwrap_or(0))
        .map(|index| {
            let address = values.required_parsed::<Ipv4Addr>(&format!("{list}_{index}_ADDR"))?;
            let mask_name = format!("{list}_{index}_MASK");
            let mask = values.required(&mask_name)?;
            let prefix_length = mask_length(mask)
                .ok_or_else(|| format!("{mask_name} is not a subnet mask: {mask:?}"))?;
            let kept = u32::MAX
                .checked_shl(32 - u32::from(prefix_length))
                .unwrap_or(0);
            Ok(IpNetwork {
                address: Ipv4Addr::from(u32::from(address) & kept).into(),
                prefix_length,
            })
        })
        .collect()
}

fn ipv6_networks(values: &Values, list: &str) -> Result<Vec<IpNetwork>, String> {
    (0..values.parsed::<usize>(list)?.unwrap_or(0))
        .map(|index| {
            let address = values.required_parsed::<Ipv6Addr>(&format!("{list}_{index}_ADDR"))?;
            let length_name = format!("{list}_{index}_MASKLEN");
            let prefix_length = values.required_parsed::<u8>(&length_name)?;
            if prefix_length > 128 {
                return Err(format!("{length_name} {prefix_length} is over 128"));
            }
            let kept = u128::MAX
                .checked_shl(128 - u32::from(prefix_length))
                .unwrap_or(0);
            Ok(IpNetwork {
                address: Ipv6Addr::from(u128::from(address) & kept).into(),
                prefix_length,
            })
        })
        .collect()
}

fn ipv6_address(values: &Values) -> Result<Option<IpNetwork>, String> {
    let address = values.parsed::<Ipv6Addr>("INTERNAL_IP6_ADDRESS")?;
    let with_length = match values.text("INTERNAL_IP6_NETMASK")? {
        None => None,
        Some(text) => {
            let unreadable = || format!("INTERNAL_IP6_NETMASK is not an address/length: {text:?}");
            let (network_address, length) = text.split_once('/').ok_or_else(unreadable)?;
            let network_address = network_address
                .parse::<Ipv6Addr>()
                .map_err(|_| unreadable())?;
            let length = length
                .parse::<u8>()
                .ok()
                .filter(|length| *length <= 128)
                .ok_or_else(unreadable)?;
            Some((network_address, length))
        }
    };
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

fn addresses<T: FromStr>(values: &Values, name: &str) -> Result<Vec<T>, String> {
    values
        .text(name)?
        .unwrap_or_default()
        .split_whitespace()
        .map(|text| parse(name, text))
        .collect()
}

fn domains(values: &Values, name: &str) -> Result<Vec<String>, String> {
    let listed: Vec<String> = values
        .text(name)?
        .unwrap_or_default()
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|domain| !domain.is_empty())
        .map(|domain| {
            if is_domain(domain) {
                Ok(domain.to_owned())
            } else {
                Err(format!(
                    "{name} has a value that is not a domain: {domain:?}"
                ))
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
