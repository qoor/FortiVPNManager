use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use fvm_contract::helper_internal::{IpNetwork, LogCategory, NetworkSettings, NetworkStep};
use fvm_os_api_macos::log::Level;
use nix::ifaddrs::getifaddrs;

use crate::{command, log};

const IFCONFIG: &str = "/sbin/ifconfig";
const DEFAULT_MTU: u32 = 1412;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Ipv4Address {
    pub address: String,
    pub destination: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Interface {
    pub up: bool,
    pub mtu: Option<u32>,
    pub ipv4: Vec<Ipv4Address>,
    pub ipv6: Vec<IpNetwork>,
}

pub(crate) fn read(name: &str) -> Option<Interface> {
    let result = command::run(IFCONFIG, &[name.to_owned()]);
    if !result.succeeded() {
        log(
            LogCategory::Routes,
            Level::Error,
            &format!(
                "ifconfig {name} failed ({}): {}",
                result.status, result.error_output
            ),
        );
        return None;
    }
    parse(&result.output)
}

pub(crate) fn owners(address: Ipv4Addr) -> Option<Vec<String>> {
    let list = match getifaddrs() {
        Ok(list) => list,
        Err(error) => {
            log(
                LogCategory::Routes,
                Level::Error,
                &format!("getifaddrs failed: {}", error.desc()),
            );
            return None;
        }
    };
    let mut owners: Vec<String> = Vec::new();
    for entry in list {
        let Some(found) = entry
            .address
            .as_ref()
            .and_then(|socket| socket.as_sockaddr_in())
            .map(nix::sys::socket::SockaddrIn::ip)
        else {
            continue;
        };
        if found == address && !owners.contains(&entry.interface_name) {
            owners.push(entry.interface_name);
        }
    }
    Some(owners)
}

pub(crate) fn parse(output: &str) -> Option<Interface> {
    let lines: Vec<Vec<&str>> = output
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.split_whitespace().collect())
        .collect();
    let first = lines.first()?;
    if first.len() < 2 || !first[1].starts_with("flags=") {
        return None;
    }
    let flags: Vec<&str> = first[1]
        .split_once('<')
        .map(|(_, rest)| rest.split('>').next().unwrap_or_default())
        .unwrap_or_default()
        .split(',')
        .collect();
    let mut interface = Interface {
        up: flags.contains(&"UP"),
        mtu: value(first, "mtu").and_then(|mtu| mtu.parse().ok()),
        ipv4: Vec::new(),
        ipv6: Vec::new(),
    };
    for fields in lines.iter().skip(1).filter(|fields| fields.len() >= 2) {
        match fields[0] {
            "inet" => interface.ipv4.push(Ipv4Address {
                address: fields[1].to_owned(),
                destination: value(fields, "-->").map(str::to_owned),
            }),
            "inet6" => {
                let address = fields[1]
                    .split('%')
                    .next()
                    .and_then(|address| address.parse::<Ipv6Addr>().ok());
                let prefix_length =
                    value(fields, "prefixlen").and_then(|length| length.parse().ok());
                if let (Some(address), Some(prefix_length)) = (address, prefix_length) {
                    interface.ipv6.push(IpNetwork {
                        address: IpAddr::V6(address),
                        prefix_length,
                    });
                }
            }
            _ => {}
        }
    }
    Some(interface)
}

fn value<'a>(fields: &[&'a str], key: &str) -> Option<&'a str> {
    let index = fields.iter().position(|field| *field == key)?;
    fields.get(index + 1).copied()
}

pub(crate) fn change(name: &str, arguments: &[String]) -> bool {
    let mut argv = vec![name.to_owned()];
    argv.extend_from_slice(arguments);
    let result = command::run(IFCONFIG, &argv);
    if !result.succeeded() {
        let message = format!("{}{}", result.error_output, result.output);
        log(
            LogCategory::Routes,
            Level::Error,
            &format!(
                "ifconfig {} failed ({}): {}",
                argv.join(" "),
                result.status,
                message.trim()
            ),
        );
    }
    result.succeeded()
}

pub(crate) fn changes(
    settings: &NetworkSettings,
    current: &Interface,
) -> Vec<(NetworkStep, Vec<String>)> {
    let ipv4 = ipv4_address(settings).to_string();
    let mut changes = Vec::new();
    let wanted = Ipv4Address {
        address: ipv4.clone(),
        destination: Some(ipv4.clone()),
    };
    if !current.ipv4.contains(&wanted) {
        changes.push((
            NetworkStep::Addresses,
            strings(&["inet", &ipv4, &ipv4, "netmask", "255.255.255.255"]),
        ));
    }
    if let Some(ipv6) = ipv6_network(settings)
        && !current.ipv6.contains(&ipv6)
    {
        changes.push((
            NetworkStep::Addresses,
            vec![
                "inet6".to_owned(),
                format!("{}/{}", ipv6.address, ipv6.prefix_length),
            ],
        ));
    }
    if !current.up {
        changes.push((NetworkStep::Addresses, strings(&["up"])));
    }
    if current.mtu != Some(mtu(settings)) {
        changes.push((
            NetworkStep::Mtu,
            vec!["mtu".to_owned(), mtu(settings).to_string()],
        ));
    }
    changes
}

pub(crate) fn removals(previous: &NetworkSettings, settings: &NetworkSettings) -> Vec<Vec<String>> {
    let mut removals = Vec::new();
    let ipv4 = ipv4_address(previous);
    if ipv4 != ipv4_address(settings) {
        removals.push(vec![
            "inet".to_owned(),
            ipv4.to_string(),
            "delete".to_owned(),
        ]);
    }
    if let Some(ipv6) = ipv6_network(previous)
        && Some(ipv6) != ipv6_network(settings)
    {
        removals.push(vec![
            "inet6".to_owned(),
            ipv6.address.to_string(),
            "delete".to_owned(),
        ]);
    }
    removals
}

impl Interface {
    pub fn mismatch(
        &self,
        settings: &NetworkSettings,
        previous: Option<&NetworkSettings>,
    ) -> Option<NetworkStep> {
        if let Some(previous) = previous {
            let old_address = ipv4_address(previous);
            if old_address != ipv4_address(settings)
                && self
                    .ipv4
                    .iter()
                    .any(|address| address.address == old_address.to_string())
            {
                return Some(NetworkStep::Addresses);
            }
            if let Some(old_network) = ipv6_network(previous)
                && Some(old_network) != ipv6_network(settings)
                && self.ipv6.contains(&old_network)
            {
                return Some(NetworkStep::Addresses);
            }
        }
        changes(settings, self).first().map(|(step, _)| *step)
    }
}

fn mtu(settings: &NetworkSettings) -> u32 {
    settings.mtu.unwrap_or(DEFAULT_MTU)
}

pub(crate) fn ipv4_address(settings: &NetworkSettings) -> Ipv4Addr {
    match settings.ipv4.address.address {
        IpAddr::V4(address) => address,
        IpAddr::V6(address) => {
            panic!("the Helper entry sent {address} as the tunnel's IPv4 address")
        }
    }
}

fn ipv6_network(settings: &NetworkSettings) -> Option<IpNetwork> {
    settings.ipv6.as_ref().map(|family| {
        assert!(
            !family.address.address.is_ipv4(),
            "the Helper entry sent {} as the tunnel's IPv6 address",
            family.address.address
        );
        family.address
    })
}

fn strings(arguments: &[&str]) -> Vec<String> {
    arguments
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect()
}

#[cfg(test)]
mod tests;
