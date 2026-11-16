//! The VPN client the app runs: the settings file's choice on macOS, openconnect on Windows; Linux
//! comes later. The runtime uses its adapter for the account checks, and the certificate list it
//! trusts follows the client, not the OS. Importing settings reads openfortivpn's settings file on
//! macOS and the official FortiClient's connection on Windows: openconnect has no settings file of
//! its own.

use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_backend_adapter::openfortivpn::OpenFortiVpn;
use fvm_contract::app_helper::VpnClient;
use fvm_contract::app_internal::problem::AccountValueProblem;
use fvm_contract::app_internal::section::AccountSaveError;
use fvm_core::config::{CertificateList, ClientRules, Platform, Settings};

#[cfg(not(windows))]
const PLATFORM: Platform = Platform::MacOs;
#[cfg(windows)]
const PLATFORM: Platform = Platform::Windows;

#[cfg(not(windows))]
const CERTIFICATE_LISTS: &[CertificateList] = &[CertificateList::Digests, CertificateList::Pins];
#[cfg(windows)]
const CERTIFICATE_LISTS: &[CertificateList] = &[CertificateList::Pins];

#[cfg(not(windows))]
const IMPORTED_CLIENT: &str = "openfortivpn";
#[cfg(windows)]
const IMPORTED_CLIENT: &str = "openconnect";

/// Whether a client killed by someone else shows it as a signal in its exit status.
pub(crate) const KILLS_SHOW_AS_SIGNALS: bool = cfg!(unix);

pub(crate) fn chosen(settings: &Settings) -> VpnClient {
    if cfg!(windows) {
        VpnClient::Openconnect
    } else {
        settings.vpn.client
    }
}

pub(crate) fn check_account(
    client: VpnClient,
    host: &str,
    username: &str,
    realm: &str,
) -> Result<(), AccountValueProblem> {
    match client {
        VpnClient::Openfortivpn => OpenFortiVpn.check_account(host, username, realm),
        VpnClient::Openconnect => OpenConnect.check_account(host, username, realm),
    }
}

pub(crate) fn check_vpn_password(
    client: VpnClient,
    password: &str,
) -> Result<(), AccountSaveError> {
    match client {
        VpnClient::Openfortivpn => OpenFortiVpn.check_vpn_password(password),
        VpnClient::Openconnect => OpenConnect.check_vpn_password(password),
    }
}

/// How config.yaml is read and written on this OS.
pub(crate) fn rules() -> ClientRules<'static> {
    ClientRules {
        name: IMPORTED_CLIENT,
        platform: PLATFORM,
        certificate_lists: CERTIFICATE_LISTS,
        digest_value: &|text| OpenFortiVpn.certificate_value(text),
        pin_value: &|text| OpenConnect.certificate_value(text),
    }
}
