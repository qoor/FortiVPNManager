//! The routing table: read whole per family with `netstat -rn -f inet` or `-f inet6`, changed one
//! route at a time with `/sbin/route -n` (macOS has no command that changes several at once). route
//! exits 0 even when the kernel refuses a change, so only reading the table again tells what
//! happened. Every address that reaches an argv was checked as a dotted quad or an IPv6 address.

use std::net::{Ipv4Addr, Ipv6Addr};

use fvm_contract::app_helper::BypassRoute;
use fvm_contract::helper_internal::LogCategory;
use fvm_os_api_macos::log::Level;

use crate::{command, log};

const ROUTE: &str = "/sbin/route";
const NETSTAT: &str = "/usr/sbin/netstat";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Family {
    Inet,
    Inet6,
}

impl Family {
    fn name(self) -> &'static str {
        match self {
            Self::Inet => "inet",
            Self::Inet6 => "inet6",
        }
    }
}

/// A route through a gateway, in dotted quads or IPv6 addresses.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) struct Route {
    pub family: Family,
    pub destination: String,
    pub gateway: String,
    pub prefix_length: u8,
}

impl Route {
    /// `None` unless all three are dotted quads and the mask is contiguous.
    pub fn with_mask(destination: &str, subnet_mask: &str, gateway: &str) -> Option<Self> {
        let mask = u32::from(subnet_mask.parse::<Ipv4Addr>().ok()?);
        destination.parse::<Ipv4Addr>().ok()?;
        if mask.count_ones() != 32 - mask.trailing_zeros() {
            return None;
        }
        Self::new(
            destination,
            u8::try_from(mask.count_ones()).expect("a u32 mask has at most 32 ones"),
            gateway,
        )
    }

    pub fn new(destination: &str, prefix_length: u8, gateway: &str) -> Option<Self> {
        if destination.parse::<Ipv4Addr>().is_ok() {
            if prefix_length > 32 || gateway.parse::<Ipv4Addr>().is_err() {
                return None;
            }
            return Some(Self {
                family: Family::Inet,
                destination: destination.to_owned(),
                gateway: gateway.to_owned(),
                prefix_length,
            });
        }
        if prefix_length > 128 {
            return None;
        }
        Some(Self {
            family: Family::Inet6,
            destination: canonical_ipv6(destination)?,
            gateway: ipv6_gateway(gateway)?,
            prefix_length,
        })
    }

    pub fn from_bypass(route: &BypassRoute) -> Option<Self> {
        Self::with_mask(&route.destination, &route.subnet_mask, &route.gateway)
    }

    fn destination_arguments(&self) -> Vec<String> {
        match self.family {
            Family::Inet => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix_length))
                    .unwrap_or(0);
                if self.prefix_length == 32 {
                    vec!["-host".into(), self.destination.clone()]
                } else {
                    vec![
                        "-net".into(),
                        self.destination.clone(),
                        "-netmask".into(),
                        Ipv4Addr::from(mask).to_string(),
                    ]
                }
            }
            Family::Inet6 => {
                if self.prefix_length == 128 {
                    vec!["-inet6".into(), "-host".into(), self.destination.clone()]
                } else {
                    vec![
                        "-inet6".into(),
                        "-net".into(),
                        format!("{}/{}", self.destination, self.prefix_length),
                    ]
                }
            }
        }
    }

    fn arguments(&self, verb: &str) -> Vec<String> {
        let mut arguments = vec![verb.to_owned()];
        arguments.extend(self.destination_arguments());
        arguments.push(self.gateway.clone());
        arguments
    }

    /// After `route -n`.
    pub fn add_arguments(&self) -> Vec<String> {
        self.arguments("add")
    }

    pub fn delete_arguments(&self) -> Vec<String> {
        self.arguments("delete")
    }

    pub fn change_arguments(&self) -> Vec<String> {
        self.arguments("change")
    }

    /// Whether `row` is this route as `add` puts it in the table: static, for every interface,
    /// with this destination, prefix and gateway. Not a copy the system made of another route for
    /// one host (W), nor a route bound to one interface (I).
    pub fn matches(&self, row: &Row) -> bool {
        row.flags.contains('S')
            && !row.flags.contains('W')
            && !row.flags.contains('I')
            && row.destination == self.destination
            && row.prefix_length == self.prefix_length
            && row.gateway == self.gateway
    }
}

/// One route of `netstat -rn`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Row {
    /// The network address in a dotted quad or an IPv6 address, `0.0.0.0` or `::` for the default
    /// route.
    pub destination: String,
    pub prefix_length: u8,
    /// As printed: an address, `link#N` or a hardware address.
    pub gateway: String,
    /// `U`p, `G`ateway, `H`ost, `S`tatic, `W`as cloned, `I`nterface scoped, ...
    pub flags: String,
    pub interface: String,
}

/// The table, or `None` when it cannot be read.
pub(crate) fn read(family: Family) -> Option<Vec<Row>> {
    let arguments = ["-rn".to_owned(), "-f".to_owned(), family.name().to_owned()];
    let result = command::run(NETSTAT, &arguments);
    if !result.succeeded() {
        log(
            LogCategory::Routes,
            Level::Error,
            &format!(
                "netstat -rn -f {} failed ({}): {}",
                family.name(),
                result.status,
                result.error_output
            ),
        );
        return None;
    }
    Some(parse(&result.output, family))
}

/// The routes of `netstat -rn -f inet`'s or `-f inet6`'s output, after its header line. netstat
/// writes an IPv4 network without its trailing zero octets, and without its prefix length when
/// that is the address class's (`127` is 127.0.0.0/8, `192.168.1` is 192.168.1.0/24); a host
/// route, flag `H`, as a whole address.
pub(crate) fn parse(output: &str, family: Family) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut header = false;
    for line in output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if !header {
            header = fields.first() == Some(&"Destination");
            continue;
        }
        if fields.len() < 4 {
            continue;
        }
        let flags = fields[2];
        let parsed = match family {
            Family::Inet => destination(fields[0], flags.contains('H')),
            Family::Inet6 => ipv6_destination(fields[0]),
        };
        let Some((destination, prefix_length)) = parsed else {
            continue;
        };
        let gateway = match family {
            Family::Inet => fields[1].to_owned(),
            Family::Inet6 => ipv6_gateway(fields[1]).unwrap_or_else(|| fields[1].to_owned()),
        };
        rows.push(Row {
            destination,
            prefix_length,
            gateway,
            flags: flags.to_owned(),
            interface: fields[3].to_owned(),
        });
    }
    rows
}

fn destination(text: &str, host: bool) -> Option<(String, u8)> {
    if text == "default" {
        return Some(("0.0.0.0".to_owned(), 0));
    }
    let mut parts = text.splitn(2, '/');
    let octets: Vec<&str> = parts.next().unwrap_or_default().split('.').collect();
    if !(1..=4).contains(&octets.len())
        || !octets
            .iter()
            .all(|octet| !octet.is_empty() && octet.is_ascii() && octet.parse::<u8>().is_ok())
    {
        return None;
    }
    let mut padded: Vec<&str> = octets.clone();
    padded.resize(4, "0");
    let address = padded.join(".");
    if let Some(length) = parts.next() {
        let length = length.parse::<u8>().ok().filter(|length| *length <= 32)?;
        return Some((address, length));
    }
    if host {
        return (octets.len() == 4).then_some((address, 32));
    }
    let first = octets[0].parse::<u8>().unwrap_or(0);
    Some((
        address,
        if first < 128 {
            8
        } else if first < 192 {
            16
        } else {
            24
        },
    ))
}

fn ipv6_destination(text: &str) -> Option<(String, u8)> {
    if text == "default" {
        return Some(("::".to_owned(), 0));
    }
    let mut parts = text.splitn(2, '/');
    let address = parts.next().unwrap_or_default();
    let address = canonical_ipv6(address.split('%').next().unwrap_or_default())?;
    let Some(length) = parts.next() else {
        return Some((address, 128));
    };
    let length = length.parse::<u8>().ok().filter(|length| *length <= 128)?;
    Some((address, length))
}

pub(crate) fn ipv6_gateway(text: &str) -> Option<String> {
    let Some((address, scope)) = text.split_once('%') else {
        return canonical_ipv6(text);
    };
    let canonical = canonical_ipv6(address)?;
    if scope.is_empty()
        || !scope
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(format!("{canonical}%{scope}"))
}

pub(crate) fn canonical_ipv6(text: &str) -> Option<String> {
    text.parse::<Ipv6Addr>()
        .ok()
        .map(|address| address.to_string())
}

pub(crate) fn add(route: &Route) -> bool {
    change(&route.add_arguments())
}

/// A copy of the default route the system made for this host, for every interface, makes the add
/// fail; the system makes another when it needs one.
pub(crate) fn add_replacing_copies(route: &Route, table: &[Row]) -> bool {
    for copy in table.iter().filter(|copy| {
        copy.flags.contains('W')
            && !copy.flags.contains('I')
            && copy.destination == route.destination
            && copy.prefix_length == route.prefix_length
    }) {
        if let Some(copy) = Route::new(&copy.destination, route.prefix_length, &copy.gateway) {
            delete(&copy);
        }
    }
    add(route)
}

pub(crate) fn delete(route: &Route) -> bool {
    change(&route.delete_arguments())
}

/// Runs `route -n <arguments>`. The kernel's refusal shows only as "route: writing to routing
/// socket: …" on stderr, so stderr output counts as failure. Callers that need certainty read the
/// table again.
pub(crate) fn change(arguments: &[String]) -> bool {
    let mut argv = vec!["-n".to_owned()];
    argv.extend_from_slice(arguments);
    let result = command::run(ROUTE, &argv);
    let succeeded = result.succeeded() && result.error_output.trim().is_empty();
    if !succeeded {
        let message = format!("{}{}", result.error_output, result.output);
        log(
            LogCategory::Routes,
            Level::Error,
            &format!(
                "route -n {} failed ({}): {}",
                arguments.join(" "),
                result.status,
                message.trim()
            ),
        );
    }
    succeeded
}

#[cfg(test)]
mod tests;
