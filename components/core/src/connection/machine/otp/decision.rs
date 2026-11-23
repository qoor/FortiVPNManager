use std::time::SystemTime;

use fvm_contract::app_helper::Milestone;

use crate::connection::machine::{Attempt, OtpSession, StaleOtp};
use crate::connection::{Effect, policy};

/// How an attempt's open OTP session closes.
pub(in crate::connection::machine) struct OtpClose {
    effect: Effect,
    /// The next login waits for the unused code email the session goes on looking for.
    stale: Option<StaleOtp>,
}

impl Attempt {
    /// How the session closes if it is open. Once the credentials went out and no code was used,
    /// the gateway may have sent a code email that arrives late, within the next prompt's arrival
    /// tolerance. With `search`, the session goes on looking for that email to trash it, from the
    /// prompt or else from when the credentials went out (user decision 2026-11-16). The callers
    /// turn it off on the app's quit and the Mac's sleep, at the OTP time limit and after a
    /// rejected login.
    pub(in crate::connection::machine) fn otp_close(
        &self,
        check_spam: bool,
        search: bool,
        now: SystemTime,
    ) -> Option<OtpClose> {
        if self.otp_session != OtpSession::Open {
            return None;
        }
        Some(match self.credentials_sent_at {
            Some(sent_at) if search && self.milestone <= Some(Milestone::OtpPrompted) => OtpClose {
                effect: Effect::TrashStaleOtp {
                    attempt: self.id.clone(),
                    since: self.otp_prompted_at.unwrap_or(sent_at),
                },
                stale: Some(StaleOtp {
                    attempt: self.id.clone(),
                    until: now + policy::STALE_OTP_WAIT,
                }),
            },
            _ => OtpClose {
                effect: Effect::CloseOtpSession {
                    attempt: self.id.clone(),
                    check_spam,
                },
                stale: None,
            },
        })
    }
}

impl OtpClose {
    pub(super) fn into_parts(self) -> (Effect, Option<StaleOtp>) {
        (self.effect, self.stale)
    }
}
