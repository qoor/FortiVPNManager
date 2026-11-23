//! Whether everything a connection needs is in place, and following the Helper's registration
//! and approval.

use std::time::SystemTime;

use fvm_contract::app_helper::HelperStatus;
use fvm_contract::app_internal::port::{HelperChannelFailure, RegistrationError};

/// What the runtime gathered for a readiness check. Secrets are only asked about, never read,
/// so the OS asks the user for nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReadinessFacts {
    /// The settings file is in use without an error.
    pub config_valid: bool,
    /// The server address, username and Gmail address are set, and the VPN client would accept
    /// the account values.
    pub account_complete: bool,
    pub secrets: StoredSecrets,
    pub helper: HelperFacts,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StoredSecrets {
    /// The secret storage holds a VPN password for the account.
    pub vpn_password: bool,
    /// The secret storage holds a Gmail app password, or with the Gmail API method the Google
    /// account is connected.
    pub gmail_credential: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HelperFacts {
    /// Not registered as a system service, or the system cannot find it.
    NotRegistered,
    /// Registered, waiting for the user's approval.
    RequiresApproval,
    /// Registered, but it did not answer the build check.
    Unreachable(HelperChannelFailure),
    /// It answered with `build`. With the app's build, `status` is its status if that answered.
    Answered {
        build: String,
        status: Option<HelperStatus>,
    },
}

/// Something to set up before connecting, in the order readiness checks it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SetupNeed {
    ConfigFile,
    AccountDetails,
    VpnPassword,
    GmailCredential,
    HelperApproval,
    /// The running Helper is another build, or it did not answer.
    HelperVersion,
}

/// Readiness checks every two seconds for a while after the Helper's registration or its
/// approval settings opened, so the approval shows without the user doing anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct FollowApproval {
    pub(super) until: SystemTime,
    pub(super) next: SystemTime,
}

#[derive(Default)]
pub(super) struct Readiness {
    /// The setup the last readiness check found missing; `None` before the first check.
    pub(super) setup_needed: Option<Vec<SetupNeed>>,
    pub(super) helper: Option<HelperFacts>,
    pub(super) register_error: Option<RegistrationError>,
    pub(super) follow_approval: Option<FollowApproval>,
    pub(super) settings_open_decided: bool,
}
