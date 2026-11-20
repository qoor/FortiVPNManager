//! Exception rules: whether they apply on the current network, the routes it should have, and the
//! edit drafts of both lists.
//!
//! Both lists, the networks where the rules apply (`routes.networks`) and the targets that skip the
//! VPN there (`routes.bypass_vpn`), hold IPv4 hosts and networks written `10.30.161.16` or
//! `10.30.170.0/24`.

use std::net::Ipv4Addr;

use fvm_contract::app_helper::{BypassRoute, BypassRoutesSnapshot, Ipv4Network, MAX_ENTRIES};
use fvm_contract::app_internal::ButtonState;
use fvm_contract::app_internal::command::BypassList;
use fvm_contract::app_internal::problem::ConfigFileError;
use fvm_contract::app_internal::section::{
    BypassNotice, BypassRulesSection, BypassStatus, CurrentNetwork, RowProblem, RuleList,
};

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

/// The position of the target that is the VPN server's address, when `host` is an address: the
/// server always goes out directly, so it can't be an exception target (user decision,
/// 2026-11-16). A network that holds it is fine.
pub fn vpn_server_target(targets: &[Ipv4Network], host: &str) -> Option<usize> {
    let server = Ipv4Network::host(host.parse().ok()?)?;
    targets.iter().position(|target| *target == server)
}

// MARK: Routes

/// Whether the rules apply on the primary network.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Applicability {
    /// This Mac's address is inside `network`, the first listed network that holds it.
    Applies {
        network: Ipv4Network,
    },
    /// On a listed network, but the app does not want the VPN up.
    VpnOff,
    OutsideNetworks,
    NoNetworksListed,
}

/// The exception routes the primary network should have.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RoutePlan {
    pub applicability: Applicability,
    /// Listed targets on this Mac's own network: reached directly, so no route is written.
    pub on_link: Vec<Ipv4Network>,
    /// Where the rules apply, a route through the router for every other listed target, in list
    /// order; none elsewhere.
    pub routes: Vec<BypassRoute>,
}

pub fn plan_routes(
    networks: &[Ipv4Network],
    targets: &[Ipv4Network],
    snapshot: &BypassRoutesSnapshot,
    vpn_wanted: bool,
) -> RoutePlan {
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
        Some(_) if !vpn_wanted => Applicability::VpnOff,
        Some(network) => Applicability::Applies { network: *network },
        None if networks.is_empty() => Applicability::NoNetworksListed,
        None => Applicability::OutsideNetworks,
    };
    let local = local_network(snapshot);
    let mut on_link = Vec::new();
    let mut routes = Vec::new();
    if matches!(applicability, Applicability::Applies { .. }) {
        for target in targets {
            if local.is_some_and(|local| local.contains(target)) {
                on_link.push(*target);
            } else {
                routes.push(target.route_through(&snapshot.router));
            }
        }
    }
    RoutePlan {
        applicability,
        on_link,
        routes,
    }
}

impl RoutePlan {
    /// The status line once the routes are in place.
    pub fn status(&self, snapshot: &BypassRoutesSnapshot) -> BypassStatus {
        match self.applicability {
            Applicability::Applies { .. } => BypassStatus::Applied {
                service_name: snapshot.service_name.clone(),
                router: snapshot.router.clone(),
                on_link_targets: u32::try_from(self.on_link.len())
                    .expect("a list holds at most MAX_ENTRIES"),
            },
            Applicability::VpnOff => BypassStatus::VpnOff {},
            Applicability::OutsideNetworks => BypassStatus::OutsideNetworks {},
            Applicability::NoNetworksListed => BypassStatus::NoNetworksListed {},
        }
    }
}

// MARK: Edit drafts

/// One list as edited in Settings: rows as typed against the list as saved. Nothing is saved until
/// Apply writes `entries`; Discard Changes goes back to the saved list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListDraft {
    /// Trimmed, never blank.
    rows: Vec<String>,
    saved: Vec<Ipv4Network>,
    /// The VPN server's address, which no row of the targets may name.
    vpn_server: Option<Ipv4Network>,
}

impl ListDraft {
    pub fn new(saved: Vec<Ipv4Network>) -> Self {
        Self {
            rows: saved.iter().map(ToString::to_string).collect(),
            saved,
            vpn_server: None,
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
                Ok(entry) if Some(entry) == self.vpn_server => {
                    problems.push((index, RowProblem::VpnServer {}));
                }
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
            if Some(entry) == self.vpn_server
                || entries.iter().any(|earlier| earlier.overlaps(&entry))
            {
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

    /// The saved list is now `saved`, after a settings file edit. Without edits
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
    /// `vpn_server` is the VPN server's address, when `vpn.host` is one.
    pub fn new(
        networks: Vec<Ipv4Network>,
        targets: Vec<Ipv4Network>,
        vpn_server: Option<Ipv4Addr>,
    ) -> Self {
        let mut drafts = Self {
            networks: ListDraft::new(networks),
            targets: ListDraft::new(targets),
        };
        drafts.set_vpn_server(vpn_server);
        drafts
    }

    /// The VPN server's address changed in the settings.
    pub fn set_vpn_server(&mut self, vpn_server: Option<Ipv4Addr>) {
        self.targets.vpn_server = vpn_server.and_then(Ipv4Network::host);
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
