//! Changes the exception routes in the routing table as the Helper entry asks, through
//! `route_table`, and reads the primary network service they go through. The routes are static
//! routes of the running system, which a restart forgets; only routes such as the Helper adds count
//! as its own.

use std::net::Ipv4Addr;

use core_foundation::base::CFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use fvm_contract::app_helper::{BypassRoute, BypassRoutesSnapshot, HelperError, HelperErrorCode};
use fvm_contract::helper_internal::{
    BypassRoutesChange, BypassRoutesOutcome, LogCategory, NetworkCheck,
};
use fvm_helper_core::entry;
use fvm_os_api_macos::log::Level;
use fvm_os_api_macos::network;
use system_configuration::dynamic_store::SCDynamicStore;

use crate::route_table::{self, Family, Route, Row};
use crate::{HELPER_ID, log};

/// Carries out `change`, as `BypassRoutesChange` describes it: one read of the table before the
/// changes and, if there were any, one after them.
pub(crate) fn change(change: &BypassRoutesChange) -> BypassRoutesOutcome {
    let network = match checked_network(&change.network) {
        Ok(network) => network,
        Err(error) => return BypassRoutesOutcome::Refused { error },
    };
    let gateway = network.as_ref().map(|network| network.router.as_str());
    if let Err(error) = entry::check_writable(&change.add, gateway) {
        return BypassRoutesOutcome::Refused { error };
    }
    let Some(before) = route_table::read(Family::Inet) else {
        return BypassRoutesOutcome::Refused {
            error: error(
                HelperErrorCode::InternalError,
                "the routing table could not be read",
            ),
        };
    };
    let mut changed = false;
    let mut removed = 0;
    for added in &change.remove {
        let Some(route) = Route::from_bypass(&added.route) else {
            continue;
        };
        if !has(&before, &route) {
            continue;
        }
        changed = true;
        if route_table::delete(&route) {
            removed += 1;
        }
    }
    let mut tried: Vec<&BypassRoute> = Vec::new();
    for wanted in &change.add {
        let Some(route) = Route::from_bypass(wanted) else {
            continue;
        };
        if has(&before, &route) {
            continue;
        }
        changed = true;
        route_table::add_replacing_copies(&route, &before);
        tried.push(wanted);
    }
    let after = if changed {
        route_table::read(Family::Inet)
    } else {
        Some(before)
    };
    let in_table = |route: &BypassRoute| -> Option<bool> {
        let Some(route) = Route::from_bypass(route) else {
            return Some(false);
        };
        after.as_ref().map(|after| has(after, &route))
    };
    let added: Vec<BypassRoute> = tried
        .into_iter()
        .filter(|route| in_table(route).unwrap_or(true))
        .cloned()
        .collect();
    if removed > 0 || !added.is_empty() {
        log(
            LogCategory::Routes,
            Level::Notice,
            &format!(
                "removed {removed} and added {} exception route(s)",
                added.len()
            ),
        );
    }
    let found = if network.is_none() {
        vec![]
    } else {
        change
            .look_up
            .iter()
            .filter(|route| in_table(route).unwrap_or(false))
            .cloned()
            .collect()
    };
    let left = change
        .remove
        .iter()
        .filter(|added| in_table(&added.route).unwrap_or(true))
        .cloned()
        .collect();
    BypassRoutesOutcome::Changed {
        network,
        added,
        found,
        left,
    }
}

fn checked_network(check: &NetworkCheck) -> Result<Option<BypassRoutesSnapshot>, HelperError> {
    match check {
        NetworkCheck::Skip {} => Ok(None),
        NetworkCheck::Read {} => Ok(Some(primary_network()?)),
        NetworkCheck::Expect { network: expected } => {
            let read = primary_network()?;
            if read != *expected {
                return Err(error(
                    HelperErrorCode::WrongPhase,
                    "the network changed since it was read; read it again",
                ));
            }
            Ok(Some(read))
        }
    }
}

/// The primary service with its IPv4 router, this Mac's first IPv4 address and mask on it, and its
/// name in the network settings.
fn primary_network() -> Result<BypassRoutesSnapshot, HelperError> {
    let Some((store, service_id)) = crate::network::store().and_then(|store| {
        let service_id = primary_service_id_in(&store)?;
        Some((store, service_id))
    }) else {
        return Err(error(
            HelperErrorCode::WrongPhase,
            "no active network service",
        ));
    };
    let dotted_quad = |text: &String| text.parse::<Ipv4Addr>().is_ok();
    let state = network::dictionary(&store, &format!("State:/Network/Service/{service_id}/IPv4"));
    let first = |key: &str| {
        state
            .as_ref()
            .and_then(|state| network::strings(&*state.find(CFString::new(key))?))
            .and_then(|values| values.into_iter().next())
    };
    let router = state
        .as_ref()
        .and_then(|state| text(state, network::ROUTER))
        .filter(dotted_quad);
    let address = first(network::ADDRESSES);
    let subnet_mask = first(network::SUBNET_MASKS);
    let (Some(router), Some(address), Some(subnet_mask)) = (router, address, subnet_mask) else {
        return Err(no_ipv4());
    };
    if !dotted_quad(&address) || !dotted_quad(&subnet_mask) {
        return Err(no_ipv4());
    }
    let Some(service_name) =
        network::service_name(HELPER_ID, &service_id).filter(|name| !name.is_empty())
    else {
        return Err(error(
            HelperErrorCode::InternalError,
            &format!("the active network service {service_id} has no name"),
        ));
    };
    Ok(BypassRoutesSnapshot {
        service_id,
        service_name,
        router,
        address,
        subnet_mask,
    })
}

fn no_ipv4() -> HelperError {
    error(
        HelperErrorCode::WrongPhase,
        "the active network service has no IPv4 router or address",
    )
}

pub(crate) fn primary_service_id_in(store: &SCDynamicStore) -> Option<String> {
    let value = |key: &str| network::dictionary(store, key);
    let global = value("State:/Network/Global/IPv4").and_then(|global| {
        Some((
            text(&global, network::PRIMARY_SERVICE)?,
            text(&global, network::PRIMARY_INTERFACE)?,
        ))
    });
    let service_order = value("Setup:/Network/Global/IPv4")
        .and_then(|setup| network::strings(&*setup.find(CFString::new(network::SERVICE_ORDER))?))
        .unwrap_or_default();
    primary_service_id(global, &service_order, |service| {
        let state = value(&format!("State:/Network/Service/{service}/IPv4"))?;
        Some(Ipv4State {
            interface: text(&state, network::INTERFACE_NAME),
            has_router: state.find(CFString::new(network::ROUTER)).is_some(),
        })
    })
}

pub(crate) struct Ipv4State {
    pub interface: Option<String>,
    pub has_router: bool,
}

pub(crate) fn primary_service_id(
    global: Option<(String, String)>,
    service_order: &[String],
    ipv4: impl Fn(&str) -> Option<Ipv4State>,
) -> Option<String> {
    let is_tunnel = |interface: &str| interface.starts_with("ppp") || interface.starts_with("utun");
    let (service_id, interface) = global?;
    if service_id != crate::network::SERVICE_ID {
        return (!is_tunnel(&interface)).then_some(service_id);
    }
    service_order
        .iter()
        .find(|candidate| {
            let Some(state) = ipv4(candidate) else {
                return false;
            };
            let Some(interface) = state.interface else {
                return false;
            };
            state.has_router && !is_tunnel(&interface)
        })
        .cloned()
}

pub(crate) fn text(dictionary: &CFDictionary<CFString, CFType>, key: &str) -> Option<String> {
    dictionary
        .find(CFString::new(key))?
        .downcast::<CFString>()
        .map(|text| text.to_string())
}

fn has(table: &[Row], route: &Route) -> bool {
    table.iter().any(|row| route.matches(row))
}

fn error(code: HelperErrorCode, message: &str) -> HelperError {
    HelperError {
        code,
        message: message.to_owned(),
    }
}
