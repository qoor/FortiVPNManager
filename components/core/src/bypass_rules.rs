//! Exception rules: whether they apply on the current network, the routes it should have, routes to
//! import, and the edit drafts of both lists.
//!
//! Both lists, the networks where the rules apply (`routes.networks`) and the targets that skip the
//! VPN there (`routes.bypass_vpn`), hold IPv4 hosts and networks written `10.30.161.16` or
//! `10.30.170.0/24`.

use std::collections::HashSet;
use std::fmt;
use std::net::Ipv4Addr;
use std::str::FromStr;

use fvm_contract::app_helper::{AdditionalRoute, BypassRoutesSnapshot};
use fvm_contract::app_internal::ButtonState;
use fvm_contract::app_internal::command::BypassList;
use fvm_contract::app_internal::problem::{AddressProblem, ConfigFileError};
use fvm_contract::app_internal::section::{
    BypassNotice, BypassRulesSection, BypassStatus, CurrentNetwork, RowProblem, RuleList,
};

/// Most entries one list may hold; the settings file refuses more.
pub const MAX_ENTRIES: usize = 100;

pub const MIN_PREFIX_LENGTH: u32 = 8;
pub const MAX_PREFIX_LENGTH: u32 = 32;

/// An IPv4 host (a /32) or network as the exception rules use it: no bits set after the prefix,
/// a prefix of 8 to 32, and an address that can be routed. Shown as `10.30.161.16` for a host and
/// `10.30.170.0/24` for a network.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Ipv4Network {
    address: u32,
    prefix_length: u32,
}

impl Ipv4Network {
    /// A route destination as the network settings report it: an address and a subnet mask.
    pub fn from_route(route: &AdditionalRoute) -> Result<Self, AddressProblem> {
        let (address, prefix_length) = address_and_mask(&route.destination, &route.subnet_mask)?;
        Self::new(address, prefix_length)
    }

    /// The network an interface address belongs to, such as `10.20.161.0/24` for `10.20.161.20`
    /// with `255.255.255.0`: the host bits are cleared instead of refused.
    pub fn enclosing(address: &str, subnet_mask: &str) -> Result<Self, AddressProblem> {
        let (address, prefix_length) = address_and_mask(address, subnet_mask)?;
        Self::new(address & mask(prefix_length), prefix_length)
    }

    fn new(address: u32, prefix_length: u32) -> Result<Self, AddressProblem> {
        if !(MIN_PREFIX_LENGTH..=MAX_PREFIX_LENGTH).contains(&prefix_length) {
            return Err(AddressProblem::PrefixOutOfRange {
                length: prefix_length,
                minimum: MIN_PREFIX_LENGTH,
                maximum: MAX_PREFIX_LENGTH,
            });
        }
        // 0.0.0.0/8, loopback, and multicast, reserved and broadcast from 224.0.0.0 up.
        let first_octet = address >> 24;
        if first_octet == 0 || first_octet == 127 || first_octet >= 224 {
            return Err(AddressProblem::Unroutable {});
        }
        let network = address & mask(prefix_length);
        if network != address {
            let network = Self {
                address: network,
                prefix_length,
            };
            return Err(AddressProblem::HostBitsSet {
                network: network.to_string(),
            });
        }
        Ok(Self {
            address,
            prefix_length,
        })
    }

    pub fn address(&self) -> Ipv4Addr {
        Ipv4Addr::from(self.address)
    }

    pub fn prefix_length(&self) -> u8 {
        u8::try_from(self.prefix_length).expect("prefix lengths are at most 32")
    }

    pub fn contains_address(&self, address: Ipv4Addr) -> bool {
        u32::from(address) & mask(self.prefix_length) == self.address
    }

    /// Whether all of `other` is inside this network.
    pub fn contains(&self, other: &Self) -> bool {
        other.prefix_length >= self.prefix_length
            && other.address & mask(self.prefix_length) == self.address
    }

    /// Whether the two share an address, which means one contains the other.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.contains(other) || other.contains(self)
    }

    /// The additional route that sends this network through `gateway`.
    pub fn route_through(&self, gateway: &str) -> AdditionalRoute {
        AdditionalRoute {
            destination: self.address().to_string(),
            subnet_mask: Ipv4Addr::from(mask(self.prefix_length)).to_string(),
            gateway: gateway.to_owned(),
        }
    }
}

/// Reads `a.b.c.d` or `a.b.c.d/n` without surrounding spaces; `a.b.c.d/32` is `a.b.c.d`. The
/// address follows the contract's IPv4 rule (`Ipv4Addr`'s: four decimal octets without leading
/// zeros); the prefix is one or two digits without a leading zero.
impl FromStr for Ipv4Network {
    type Err = AddressProblem;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (address, prefix) = match text.split_once('/') {
            Some((address, prefix)) => (address, Some(prefix)),
            None => (text, None),
        };
        let address = address
            .parse::<Ipv4Addr>()
            .map_err(|_| AddressProblem::Malformed {})?;
        let prefix_length = match prefix {
            None => MAX_PREFIX_LENGTH,
            Some(prefix) => {
                let plain = (1..=2).contains(&prefix.len())
                    && prefix.bytes().all(|byte| byte.is_ascii_digit())
                    && (prefix.len() == 1 || !prefix.starts_with('0'));
                if !plain {
                    return Err(AddressProblem::Malformed {});
                }
                prefix.parse().expect("one or two ASCII digits")
            }
        };
        Self::new(u32::from(address), prefix_length)
    }
}

impl fmt::Display for Ipv4Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.prefix_length == MAX_PREFIX_LENGTH {
            write!(f, "{}", self.address())
        } else {
            write!(f, "{}/{}", self.address(), self.prefix_length)
        }
    }
}

fn mask(prefix_length: u32) -> u32 {
    u32::MAX.checked_shl(32 - prefix_length).unwrap_or(0)
}

fn address_and_mask(address: &str, subnet_mask: &str) -> Result<(u32, u32), AddressProblem> {
    let (Ok(address), Ok(subnet_mask)) =
        (address.parse::<Ipv4Addr>(), subnet_mask.parse::<Ipv4Addr>())
    else {
        return Err(AddressProblem::Malformed {});
    };
    let subnet_mask = u32::from(subnet_mask);
    let prefix_length = subnet_mask.leading_ones();
    if mask(prefix_length) != subnet_mask {
        return Err(AddressProblem::NonContiguousMask {});
    }
    Ok((u32::from(address), prefix_length))
}

/// The network this Mac is on, such as `10.20.161.0/24`; `None` when its address or mask is not
/// usable.
pub fn local_network(snapshot: &BypassRoutesSnapshot) -> Option<Ipv4Network> {
    Ipv4Network::enclosing(&snapshot.address, &snapshot.subnet_mask).ok()
}

/// The primary network as the exception rules section shows it, against the saved `networks`.
pub fn current_network(
    snapshot: &BypassRoutesSnapshot,
    networks: &[Ipv4Network],
) -> CurrentNetwork {
    let within = snapshot
        .address
        .parse::<Ipv4Addr>()
        .ok()
        .and_then(|address| {
            networks
                .iter()
                .find(|network| network.contains_address(address))
        });
    CurrentNetwork {
        service_name: snapshot.service_name.clone(),
        address: snapshot.address.clone(),
        prefix_length: local_network(snapshot).map(|network| network.prefix_length()),
        within: within.map(ToString::to_string),
    }
}

// MARK: Routes

/// Whether the rules apply on the primary network.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Applicability {
    /// This Mac's address is inside `network`, the first listed network that holds it.
    Applies {
        network: Ipv4Network,
    },
    OutsideNetworks,
    NoNetworksListed,
}

/// What the primary network service's additional routes become.
///
/// A route on the service is the app's own when the app wrote it last time (`previously_applied`,
/// compared whole) or, while the rules apply, when it is a listed target through the current
/// router outside this Mac's own network. The app's own routes are rewritten from the list, or
/// removed where the rules do not apply. While they apply, other routes through the router were
/// added elsewhere and join the list, unless they overlap a listed target or a wider route joining
/// with them, lie inside this Mac's own network, or the list is full. Every other route is kept
/// exactly as it is: other gateways, routes on this Mac's network the app never wrote, and anything
/// on a network where the rules do not apply.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RoutePlan {
    pub applicability: Applicability,
    /// Targets to append to the list; save them before writing the routes, so that a failed save
    /// leaves them looking added elsewhere rather than removed from the list.
    pub imported: Vec<Ipv4Network>,
    /// Routes added elsewhere that did not fit in the full list; they are kept.
    pub not_imported: usize,
    /// Listed targets on this Mac's own network: reached directly, so no route is written.
    pub on_link: Vec<Ipv4Network>,
    /// How many kept routes go through another gateway than the router.
    pub kept_through_other_gateways: usize,
    /// The app's own routes after the write, to record as applied.
    pub applied: Vec<AdditionalRoute>,
    /// The complete list the service should have: `applied`, then the kept routes.
    pub routes: Vec<AdditionalRoute>,
    /// `routes` differs from what the service has.
    pub needs_write: bool,
}

/// Plans the service's routes from the saved `networks` and `targets`, the routes the app last
/// wrote on this service, and the service as read now.
pub fn plan_routes(
    networks: &[Ipv4Network],
    targets: &[Ipv4Network],
    previously_applied: &[AdditionalRoute],
    snapshot: &BypassRoutesSnapshot,
) -> RoutePlan {
    let router = snapshot.router.as_str();
    let matched = snapshot
        .address
        .parse::<Ipv4Addr>()
        .ok()
        .and_then(|address| {
            networks
                .iter()
                .find(|network| network.contains_address(address))
        });
    let applicability = match matched {
        Some(network) => Applicability::Applies { network: *network },
        None if networks.is_empty() => Applicability::NoNetworksListed,
        None => Applicability::OutsideNetworks,
    };
    let active = matched.is_some();
    let local = local_network(snapshot);
    let on_local = |network: &Ipv4Network| local.is_some_and(|local| local.contains(network));

    // A route through the router the rules may own: a valid target outside this Mac's network.
    let candidate = |route: &AdditionalRoute| -> Option<Ipv4Network> {
        if !active || route.gateway != router || previously_applied.contains(route) {
            return None;
        }
        let network = Ipv4Network::from_route(route).ok()?;
        (!on_local(&network)).then_some(network)
    };

    let mut imported: Vec<Ipv4Network> = Vec::new();
    let mut not_imported = 0;
    let room = MAX_ENTRIES.saturating_sub(targets.len());
    let mut candidates: Vec<Ipv4Network> = snapshot.routes.iter().filter_map(candidate).collect();
    // Widest first (the sort is stable), so of two overlapping routes the wider one joins the list
    // whatever their order on the service.
    candidates.sort_by_key(|network| network.prefix_length);
    for network in candidates {
        if targets
            .iter()
            .chain(&imported)
            .any(|listed| listed.overlaps(&network))
        {
            continue;
        }
        if imported.len() < room {
            imported.push(network);
        } else {
            not_imported += 1;
        }
    }

    let mut kept: Vec<AdditionalRoute> = Vec::new();
    for route in &snapshot.routes {
        if previously_applied.contains(route) {
            continue;
        }
        if candidate(route)
            .is_some_and(|network| targets.contains(&network) || imported.contains(&network))
        {
            continue;
        }
        if !kept.contains(route) {
            kept.push(route.clone());
        }
    }

    let mut on_link = Vec::new();
    let mut applied = Vec::new();
    if active {
        for target in targets.iter().chain(&imported) {
            if on_local(target) {
                on_link.push(*target);
            } else {
                applied.push(target.route_through(router));
            }
        }
    }
    let kept_through_other_gateways = kept.iter().filter(|route| route.gateway != router).count();
    let routes: Vec<AdditionalRoute> = applied.iter().cloned().chain(kept).collect();
    let needs_write = routes.len() != snapshot.routes.len()
        || routes.iter().collect::<HashSet<_>>() != snapshot.routes.iter().collect::<HashSet<_>>();
    RoutePlan {
        applicability,
        imported,
        not_imported,
        on_link,
        kept_through_other_gateways,
        applied,
        routes,
        needs_write,
    }
}

impl RoutePlan {
    /// The status line once the service has `routes`, as read back in `snapshot`.
    pub fn status(&self, snapshot: &BypassRoutesSnapshot) -> BypassStatus {
        let kept_through_other_gateways = count(self.kept_through_other_gateways);
        match self.applicability {
            Applicability::Applies { .. } => BypassStatus::Applied {
                service_name: snapshot.service_name.clone(),
                router: snapshot.router.clone(),
                on_link_targets: count(self.on_link.len()),
                not_imported: count(self.not_imported),
                kept_through_other_gateways,
            },
            Applicability::OutsideNetworks => BypassStatus::OutsideNetworks {
                kept_through_other_gateways,
            },
            Applicability::NoNetworksListed => BypassStatus::NoNetworksListed {
                kept_through_other_gateways,
            },
        }
    }
}

fn count(value: usize) -> u32 {
    u32::try_from(value).expect("route counts fit in u32")
}

// MARK: Edit drafts

/// One list as edited in Settings: rows as typed against the list as saved. Nothing is saved until
/// Apply writes `entries`; Discard Changes goes back to the saved list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListDraft {
    /// Trimmed, never blank.
    rows: Vec<String>,
    saved: Vec<Ipv4Network>,
}

impl ListDraft {
    pub fn new(saved: Vec<Ipv4Network>) -> Self {
        Self {
            rows: saved.iter().map(ToString::to_string).collect(),
            saved,
        }
    }

    pub fn rows(&self) -> &[String] {
        &self.rows
    }

    pub fn saved(&self) -> &[Ipv4Network] {
        &self.saved
    }

    /// Replaces the rows with the list editor's; blank rows are dropped.
    pub fn set_rows(&mut self, rows: Vec<String>) {
        self.rows = rows
            .iter()
            .map(|row| row.trim())
            .filter(|row| !row.is_empty())
            .map(ToOwned::to_owned)
            .collect();
    }

    /// Appends `entry` unless a row already names it.
    pub fn add(&mut self, entry: Ipv4Network) {
        if !self.rows.iter().any(|row| row.parse() == Ok(entry)) {
            self.rows.push(entry.to_string());
        }
    }

    /// Whether a row names or overlaps `entry`, so adding it would make a problem row.
    pub fn has_row_overlapping(&self, entry: &Ipv4Network) -> bool {
        self.rows.iter().any(|row| {
            row.parse::<Ipv4Network>()
                .is_ok_and(|row| row.overlaps(entry))
        })
    }

    pub fn discard_changes(&mut self) {
        self.rows = self.saved.iter().map(ToString::to_string).collect();
    }

    /// The rows with a problem, in row order. A row is compared with the earlier rows without a
    /// problem.
    pub fn problems(&self) -> Vec<(usize, RowProblem)> {
        let mut problems = Vec::new();
        let mut seen: Vec<Ipv4Network> = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            match row.parse::<Ipv4Network>() {
                Err(problem) => problems.push((index, RowProblem::Invalid { problem })),
                Ok(entry) if seen.contains(&entry) => problems.push((
                    index,
                    RowProblem::Duplicate {
                        entry: entry.to_string(),
                    },
                )),
                Ok(entry) => match seen.iter().find(|other| other.overlaps(&entry)) {
                    Some(other) => problems.push((
                        index,
                        RowProblem::Overlap {
                            other: other.to_string(),
                        },
                    )),
                    None => seen.push(entry),
                },
            }
        }
        problems
    }

    /// The rows as entries; `None` while a row has a problem.
    pub fn entries(&self) -> Option<Vec<Ipv4Network>> {
        let mut entries: Vec<Ipv4Network> = Vec::new();
        for row in &self.rows {
            let entry = row.parse::<Ipv4Network>().ok()?;
            if entries.iter().any(|earlier| earlier.overlaps(&entry)) {
                return None;
            }
            entries.push(entry);
        }
        Some(entries)
    }

    /// Whether Apply would change the list. Rows with a problem count as a change; another
    /// spelling of the same entry, such as `/32` on a host, does not.
    pub fn has_changes(&self) -> bool {
        self.entries().as_deref() != Some(self.saved.as_slice())
    }

    /// The saved list is now `saved`, after a settings file edit or imported routes. Without edits
    /// the rows follow it; with edits, rows of entries removed there go away, entries added there
    /// are appended, and every other row stays as typed.
    pub fn merge_saved(&mut self, saved: Vec<Ipv4Network>) {
        if saved == self.saved {
            return;
        }
        if self.has_changes() {
            let removed: Vec<Ipv4Network> = self
                .saved
                .iter()
                .filter(|entry| !saved.contains(entry))
                .copied()
                .collect();
            self.rows.retain(|row| {
                !row.parse::<Ipv4Network>()
                    .is_ok_and(|entry| removed.contains(&entry))
            });
            let added: Vec<Ipv4Network> = saved
                .iter()
                .filter(|entry| !self.saved.contains(entry))
                .copied()
                .collect();
            for entry in added {
                self.add(entry);
            }
        } else {
            self.rows = saved.iter().map(ToString::to_string).collect();
        }
        self.saved = saved;
    }

    /// The list as the section shows it.
    pub fn rule_list(&self) -> RuleList {
        RuleList {
            rows: self.rows.clone(),
            problem_rows: self
                .problems()
                .iter()
                .map(|(index, _)| u32::try_from(*index).expect("row indexes fit in u32"))
                .collect(),
            add_row: enabled(self.rows.len() < MAX_ENTRIES),
        }
    }

    /// The first problem row and its problem, to explain under the lists.
    fn first_problem(&self) -> Option<BypassNotice> {
        self.problems()
            .into_iter()
            .next()
            .map(|(index, problem)| BypassNotice::RowProblem {
                row: self.rows[index].clone(),
                problem,
            })
    }
}

/// Both lists as edited in Settings. The drafts stay while the settings window is closed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RuleDrafts {
    pub networks: ListDraft,
    pub targets: ListDraft,
}

impl RuleDrafts {
    /// Drafts without changes, on load and after Apply saved `networks` and `targets`.
    pub fn new(networks: Vec<Ipv4Network>, targets: Vec<Ipv4Network>) -> Self {
        Self {
            networks: ListDraft::new(networks),
            targets: ListDraft::new(targets),
        }
    }

    pub fn list_mut(&mut self, list: BypassList) -> &mut ListDraft {
        match list {
            BypassList::Networks => &mut self.networks,
            BypassList::Targets => &mut self.targets,
        }
    }

    /// Adds the network this Mac is on to the networks list, unless it is unknown or a row names
    /// or overlaps it.
    pub fn add_current_network(&mut self, snapshot: &BypassRoutesSnapshot) {
        if let Some(network) = self.addable_network(Some(snapshot)) {
            self.networks.add(network);
        }
    }

    fn addable_network(&self, snapshot: Option<&BypassRoutesSnapshot>) -> Option<Ipv4Network> {
        local_network(snapshot?).filter(|network| !self.networks.has_row_overlapping(network))
    }

    pub fn discard_changes(&mut self) {
        self.networks.discard_changes();
        self.targets.discard_changes();
    }

    pub fn has_changes(&self) -> bool {
        self.networks.has_changes() || self.targets.has_changes()
    }

    /// The networks and targets Apply saves; `None` while a row of either list has a problem.
    pub fn entries(&self) -> Option<(Vec<Ipv4Network>, Vec<Ipv4Network>)> {
        Some((self.networks.entries()?, self.targets.entries()?))
    }

    /// The section, with the primary network as last read, the status of the routes and why the
    /// last Apply failed, if it did.
    pub fn section(
        &self,
        current: Option<&BypassRoutesSnapshot>,
        status: BypassStatus,
        apply_error: Option<ConfigFileError>,
    ) -> BypassRulesSection {
        let has_changes = self.has_changes();
        let notice = self
            .networks
            .first_problem()
            .or_else(|| self.targets.first_problem())
            .or_else(|| apply_error.map(|error| BypassNotice::ApplyFailed { error }));
        BypassRulesSection {
            current_network: current
                .map(|snapshot| current_network(snapshot, self.networks.saved())),
            networks: self.networks.rule_list(),
            targets: self.targets.rule_list(),
            notice,
            status,
            has_changes,
            add_current_network: enabled(self.addable_network(current).is_some()),
            discard: enabled(has_changes),
            apply: enabled(has_changes && self.entries().is_some()),
        }
    }
}

fn enabled(condition: bool) -> ButtonState {
    if condition {
        ButtonState::Enabled
    } else {
        ButtonState::Disabled
    }
}
