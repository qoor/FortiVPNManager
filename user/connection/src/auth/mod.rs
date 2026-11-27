use fvm_user_protocol::service::{AuthAction, OtpNeed, Secret, ServiceClient, Settings};

/// One piece of work for the auth manager.
pub enum AuthWork {
    /// What the user did about the accounts.
    User(AuthAction),
    /// Give the VPN account to connect with.
    GiveVpnAccount,
    /// Get the OTP the gateway asked for.
    FetchOtp(OtpNeed),
    /// Give up the OTP that was asked for.
    CancelOtp,
    SettingsChanged(Settings),
    Stop,
}

/// The accounts a connection needs: the VPN account, and the mail account the OTP arrives at.
pub struct AuthManager;

impl AuthManager {
    /// Creates the manager with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the manager");
        // 1. Create the queue that holds the manager's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Makes what the manager needs before it can run.
    pub fn init(&self) {
        todo!("look in the secret store of the OS for what is kept there");
        // Whether the accounts are ready is reported, with what is missing.
        // A secret that cannot be read counts as missing, and the others are still used.
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: AuthWork) {
        todo!("put the work into the work queue");
    }

    /// Takes the manager's work one at a time until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                AuthWork::User(AuthAction::SaveVpnPassword(password)) => {
                    self.save_vpn_password(&password);
                }
                AuthWork::User(AuthAction::SaveAppPassword(password)) => {
                    self.save_app_password(&password);
                }
                AuthWork::User(AuthAction::LinkGoogleAccount) => self.link_google_account(),
                AuthWork::User(AuthAction::UnlinkGoogleAccount) => self.unlink_google_account(),
                AuthWork::User(AuthAction::TestGmail) => self.test_gmail(),
                AuthWork::GiveVpnAccount => self.give_vpn_account(),
                AuthWork::FetchOtp(need) => self.fetch_otp(&need),
                AuthWork::CancelOtp => self.cancel_otp(),
                AuthWork::SettingsChanged(settings) => self.apply_settings(&settings),
                AuthWork::Stop => break,
            }
        }
    }

    fn next_work(&self) -> AuthWork {
        todo!("wait for the next work in the work queue");
    }

    fn save_vpn_password(&self, _password: &Secret) {
        todo!("keep the VPN password in the secret store of the OS");
        // An empty password leaves the one that is kept as it is.
        // Whether the accounts are ready is reported when it changed.
    }

    fn save_app_password(&self, _password: &Secret) {
        todo!("keep the mail app password in the secret store of the OS");
        // Whether the accounts are ready is reported when it changed.
    }

    fn link_google_account(&self) {
        todo!("link the Google account the OTP mail arrives at");
        // 1. Open the Google sign-in in the browser and wait for its result.
        // 2. Refuse an account other than the Gmail address in the settings, or one that did
        //    not grant what reading the OTP mail needs, and report why.
        // 3. Keep the token in the secret store of the OS.
        // 4. Report whether the accounts are ready.
        //
        // While waiting for the sign-in:
        // - A request for the VPN account or for an OTP, and changed settings, are handled right
        //   away.
        // - The user unlinking the account gives the sign-in up and goes on with unlinking.
        // - A stop gives the sign-in up and ends the run.
    }

    fn unlink_google_account(&self) {
        todo!("remove the Google account's token from the secret store of the OS");
        // Whether the accounts are ready is reported when it changed.
    }

    fn test_gmail(&self) {
        todo!("try to reach the mailbox and report how it went");
        // The test changes neither whether the accounts count as ready nor the record of
        // failures.
    }

    fn give_vpn_account(&self) {
        todo!("report the user name and the password to connect with");
        // A password that is not kept is reported as the account not being ready.
    }

    fn fetch_otp(&self, _need: &OtpNeed) {
        todo!("get the OTP the gateway asked for from the OTP mail");
        // 1. Wait for an OTP mail that arrived after the gateway asked, through the Gmail API
        //    or IMAP, whichever the accounts allow.
        //    The wait lasts only as long as the OTP stays valid, counted from when the gateway
        //    asked.
        // 2. Read the OTP from the latest such mail and report it.
        // 3. Move the mail that was used to the trash.
        //
        // While waiting:
        // - Being told to give the OTP up ends the wait.
        // - A new request for an OTP gives this one up and goes on with the new one.
        // - A stop ends the wait and the run.
        // - Any other work is handled right away.
        //
        // An OTP that did not arrive while it was valid is reported as such.
        // An OTP that cannot be got is reported with the reason: the mail account refused the
        // sign-in, or the OTP mail is in the spam folder.
    }

    fn cancel_otp(&self) {
        todo!("give up the OTP that was asked for");
        // With no wait for an OTP mail going on there is nothing to give up.
    }

    fn apply_settings(&self, _settings: &Settings) {
        todo!("take in the account settings that changed");
        // The user name, the Gmail address and the way the OTP mail is read come from the
        // settings. Whether the accounts are ready is reported when it changed.
    }
}
