//! The VPN client of the OS the app runs on: openconnect on Windows, openfortivpn on macOS (Linux
//! and openconnect on macOS come later). The runtime uses its adapter for the account checks, and
//! the certificate list it trusts follows the client, not the OS. Importing settings reads
//! openfortivpn's settings file on macOS and the official FortiClient's connection on Windows:
//! openconnect has no settings file of its own.

use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_backend_adapter::openfortivpn::OpenFortiVpn;
use fvm_contract::backend::Backend;
use fvm_core::config::{ClientRules, Platform};

#[cfg(windows)]
mod chosen {
    use fvm_core::config::CertificateList;

    pub(crate) use fvm_backend_adapter::openconnect::OpenConnect as Client;
    /// openconnect compares `pin-sha256:` values.
    pub(crate) const CERTIFICATES: CertificateList = CertificateList::Pins;
}

#[cfg(not(windows))]
mod chosen {
    use fvm_core::config::CertificateList;

    pub(crate) use fvm_backend_adapter::openfortivpn::OpenFortiVpn as Client;
    /// openfortivpn compares SHA-256 digests.
    pub(crate) const CERTIFICATES: CertificateList = CertificateList::Digests;
}

pub(crate) use chosen::{CERTIFICATES, Client};

#[cfg(not(windows))]
const PLATFORM: Platform = Platform::MacOs;
#[cfg(windows)]
const PLATFORM: Platform = Platform::Windows;

/// Whether a client killed by someone else shows it as a signal in its exit status.
pub(crate) const KILLS_SHOW_AS_SIGNALS: bool = cfg!(unix);

/// How config.yaml is read and written on this OS.
pub(crate) fn rules() -> ClientRules<'static> {
    ClientRules {
        name: Client.client_name(),
        platform: PLATFORM,
        certificate_list: CERTIFICATES,
        digest_value: &|text| OpenFortiVpn.certificate_value(text),
        pin_value: &|text| OpenConnect.certificate_value(text),
    }
}
