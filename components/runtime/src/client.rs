//! The VPN client the app runs: openconnect; Linux comes later. Importing settings reads
//! openfortivpn's settings file on macOS and the official FortiClient's connection on Windows:
//! openconnect has no settings file of its own.

use fvm_core::config::{ClientRules, Platform};

#[cfg(not(windows))]
const PLATFORM: Platform = Platform::MacOs;
#[cfg(windows)]
const PLATFORM: Platform = Platform::Windows;

#[cfg(not(windows))]
const IMPORTED_CLIENT: &str = "openfortivpn";
#[cfg(windows)]
const IMPORTED_CLIENT: &str = "openconnect";

/// Whether a client killed by someone else shows it as a signal in its exit status.
pub(crate) const KILLS_SHOW_AS_SIGNALS: bool = cfg!(unix);

/// How config.yaml is read and written on this OS.
pub(crate) fn rules() -> ClientRules<'static> {
    ClientRules {
        name: IMPORTED_CLIENT,
        platform: PLATFORM,
    }
}
