use crate::connection::machine::{Attempt, ConnectionManager, OtpSession};

use super::decision::OtpClose;

impl Attempt {
    /// Marks the session closed as decided and hands the close on.
    pub(in crate::connection::machine) fn otp_closed(
        &mut self,
        close: Option<OtpClose>,
    ) -> Option<OtpClose> {
        if close.is_some() {
            self.otp_session = OtpSession::Closed;
        }
        close
    }
}

impl ConnectionManager {
    /// Hands over the close of an ended attempt's OTP session. A search for its unused code holds
    /// the next login, which the OTP preparation decides, until it ends or its time limit.
    pub(in crate::connection::machine) fn push_otp_close(&mut self, close: Option<OtpClose>) {
        let Some(close) = close else {
            return;
        };
        let (effect, stale) = close.into_parts();
        if stale.is_some() {
            self.stale_otp = stale;
        }
        self.effects.push(effect);
    }
}
