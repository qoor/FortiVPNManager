//! Rules for the gateway certificate values the app trusts (`vpn.trusted_certs`). Their format is
//! the backend adapter's: values reach here only as it reported or accepted them.

use super::VpnAccount;

pub const TRUSTED_CERT_LIMIT: usize = 4;

/// `learned` in front of `existing`, without repeats, the newest [`TRUSTED_CERT_LIMIT`].
pub fn merge_trusted_certs(learned: &[String], existing: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = Vec::with_capacity(TRUSTED_CERT_LIMIT);
    for value in learned.iter().chain(existing) {
        if merged.len() == TRUSTED_CERT_LIMIT {
            break;
        }
        if !merged.contains(value) {
            merged.push(value.clone());
        }
    }
    merged
}

/// Whether `after` replacing `before` drops the trusted values: they belong to one gateway, so a
/// new server address or port makes them the old gateway's, unless they were changed with it.
/// Values the user supplied for the new gateway are kept. Both the settings window's save and a
/// change of the settings file follow this rule.
pub fn gateway_change_drops_certs(before: &VpnAccount, after: &VpnAccount) -> bool {
    !before.same_gateway(after) && after.trusted_certs == before.trusted_certs
}
