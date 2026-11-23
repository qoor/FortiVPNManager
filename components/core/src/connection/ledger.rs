//! The authentication failure record of one VPN account, kept across launches.

use std::time::SystemTime;

use fvm_contract::app_helper::HelperErrorCode;

/// Why the connection stopped, or the counted authentication failure it waits after.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Failure {
    /// The gateway asked for the password again.
    AuthenticationRejected,
    /// No OTP email matched in time; `in_spam` once it was found in the spam folder.
    OtpNotReceived {
        in_spam: bool,
    },
    /// The OTP was submitted but the gateway asked again.
    OtpRejected,
    /// Logged in, but the tunnel device could not be set up.
    TunnelSetupFailed,
    /// The gateway certificate was rejected after the OTP was used.
    CertificateRejectedAfterLogin,
    /// Gmail refused the credential of the OTP session.
    GmailRejected,
    /// Too many automatic sign-in attempts within the window.
    TooManyReconnects,
    Network,
    /// The Helper refused to start the VPN client.
    Helper {
        code: HelperErrorCode,
    },
}

/// What state.json keeps for one account, so the retry rules survive a relaunch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FailureLedger {
    /// [`super::VpnAccount::ledger_key`] of the account.
    pub account_key: String,
    /// Counted authentication failures since the last tunnel up or user action.
    pub consecutive_auth_failures: u32,
    pub last_auth_failure_at: Option<SystemTime>,
    pub last_auth_failure: Option<Failure>,
    /// When attempts reached the gateway, so the credentials went out; oldest first, within the
    /// window.
    pub recent_auth_attempts: Vec<SystemTime>,
}

impl FailureLedger {
    #[must_use]
    pub fn new(account_key: String) -> Self {
        Self {
            account_key,
            consecutive_auth_failures: 0,
            last_auth_failure_at: None,
            last_auth_failure: None,
            recent_auth_attempts: Vec::new(),
        }
    }
}
