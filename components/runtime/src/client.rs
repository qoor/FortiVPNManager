//! The VPN client the app runs: openconnect; Linux comes later.

/// Whether a client killed by someone else shows it as a signal in its exit status.
pub(crate) const KILLS_SHOW_AS_SIGNALS: bool = cfg!(unix);
