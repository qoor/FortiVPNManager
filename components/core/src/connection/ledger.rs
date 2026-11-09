//! The authentication failure record of one VPN account, kept across launches.

use std::time::SystemTime;

use fvm_contract::app_helper::HelperErrorCode;

use super::policy;

/// Why the connection stopped, or the counted authentication failure it waits after.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Failure {
    /// Rejected after reaching the gateway and before the OTP prompt.
    AuthenticationRejected,
    /// No OTP email matched in time; `in_spam` once it was found in the spam folder.
    OtpNotReceived {
        in_spam: bool,
    },
    /// The OTP was submitted but the login did not complete.
    OtpRejected,
    /// Logged in, but the tunnel never came up.
    TunnelSetupFailed,
    /// The gateway refused tunnel mode after the login.
    TunnelModeRejected,
    /// The tunnel came up but dropped before it was stable.
    SessionDroppedEarly,
    /// The gateway certificate was rejected after the OTP was used.
    CertificateRejectedAfterLogin,
    /// Gmail refused the credential of the OTP session.
    GmailRejected,
    /// Too many automatic sign-in attempts within the window.
    TooManyReconnects,
    Network,
    /// The Helper refused to start the VPN client.
    Helper(HelperErrorCode),
}

/// What state.json keeps for one account, so the retry rules survive a relaunch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FailureLedger {
    /// [`super::VpnAccount::ledger_key`] of the account.
    pub account_key: String,
    /// Counted authentication failures since the last stable session or user action.
    pub consecutive_auth_failures: u32,
    pub last_auth_failure_at: Option<SystemTime>,
    pub last_auth_failure: Option<Failure>,
    /// When attempts reached the gateway, so the credentials went out; oldest first, within the
    /// window.
    pub recent_auth_attempts: Vec<SystemTime>,
}

impl FailureLedger {
    pub fn new(account_key: String) -> Self {
        Self {
            account_key,
            consecutive_auth_failures: 0,
            last_auth_failure_at: None,
            last_auth_failure: None,
            recent_auth_attempts: Vec::new(),
        }
    }

    /// No attempt may start before it.
    pub(super) fn cooldown_end(&self) -> Option<SystemTime> {
        self.last_auth_failure_at
            .map(|at| at + policy::AUTH_FAILURE_RETRY_DELAY)
    }

    /// Drops the attempts that fell out of the window.
    pub(super) fn prune(&mut self, now: SystemTime) {
        if let Some(start) = now.checked_sub(policy::AUTH_ATTEMPT_WINDOW) {
            self.recent_auth_attempts.retain(|at| *at > start);
        }
    }

    pub(super) fn record_auth_attempt(&mut self, at: SystemTime) {
        self.prune(at);
        self.recent_auth_attempts.push(at);
    }

    pub(super) fn record_auth_failure(&mut self, failure: Failure, at: SystemTime) {
        self.consecutive_auth_failures += 1;
        self.last_auth_failure_at = Some(at);
        self.last_auth_failure = Some(failure);
    }

    /// A stable session proves the credentials work.
    pub(super) fn reset_consecutive_failures(&mut self) {
        self.consecutive_auth_failures = 0;
    }

    /// The user's Connect or Reconnect Now starts the count and the window over. The time of the
    /// last failure stays, so its cooldown still holds.
    pub(super) fn reset_for_user_action(&mut self) {
        self.consecutive_auth_failures = 0;
        self.recent_auth_attempts.clear();
    }
}
