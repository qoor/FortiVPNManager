//! The account values openfortivpn takes and its gateway certificate values, as the app checks them
//! before saving on any OS and the Helper checks them again before a run.

use fvm_contract::app_internal::problem::{AccountField, AccountValueProblem};
use fvm_contract::app_internal::section::AccountSaveError;

use crate::values;

// openfortivpn's buffer sizes (src/config.h): load_config() silently cuts longer values, so the
// client would use a different value than the one checked.
pub(super) const MAX_HOST_BYTES: usize = 253; // GATEWAY_HOST_SIZE
pub(super) const MAX_USERNAME_BYTES: usize = 64; // USERNAME_SIZE
pub(super) const MAX_REALM_BYTES: usize = 63; // REALM_SIZE

/// read_password() reads at most PASSWORD_SIZE (256) bytes and stops early only at '\n'. A longer
/// password would leave its '\n' in stdin, and the OTP prompt would read that as an empty answer.
pub(super) const MAX_PASSWORD_BYTES: usize = 255;

/// The app's check of the account before it is saved or used, with the Helper's rules: the first
/// value openfortivpn would refuse, in the order server address, username, realm. An empty server
/// address or username is left to the caller, as an account not set up yet; an empty realm means
/// none.
pub(super) fn check_account(
    host: &str,
    username: &str,
    realm: &str,
) -> Result<(), AccountValueProblem> {
    values::account_value(AccountField::Host, host, MAX_HOST_BYTES)?;
    if !host.is_empty() && !values::is_host(host) {
        return Err(AccountValueProblem::InvalidHost {});
    }
    values::account_value(AccountField::Username, username, MAX_USERNAME_BYTES)?;
    values::account_value(AccountField::Realm, realm, MAX_REALM_BYTES)
}

/// The app's check of a typed VPN password before it is saved. An empty one is left to the
/// caller, for which it keeps the stored password.
pub(super) fn check_vpn_password(password: &str) -> Result<(), AccountSaveError> {
    if values::forbidden_in_password(password).is_some() || password.len() > MAX_PASSWORD_BYTES {
        return Err(AccountSaveError::InvalidVpnPassword {
            max_bytes: u32::try_from(MAX_PASSWORD_BYTES).expect("the buffer size fits in u32"),
        });
    }
    Ok(())
}

/// `text` as openfortivpn compares it, if it is a SHA-256 digest in hex of either case.
pub(super) fn certificate_value(text: &str) -> Option<String> {
    let value = text.to_ascii_lowercase();
    is_certificate_value(&value).then_some(value)
}

/// openfortivpn compares the gateway's SHA-256 digest as lowercase hex.
pub(super) fn is_certificate_value(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
