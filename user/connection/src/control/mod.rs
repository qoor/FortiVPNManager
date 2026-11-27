use fvm_common_base::protocol::{AppRequest, AppResponse, VpnStateChange};
use fvm_user_protocol::service::{
    AccountState, AppState, ConnectionAction, HelperStartFailure, IdleState, Otp, OtpFailure,
    ScheduleState, ServiceClient, Settings, SleepState, VpnAccount,
};

/// One piece of work for the VPN connection controller.
pub enum ControlWork {
    /// What the user did about the connection.
    User(ConnectionAction),
    /// End the connection so that an update can be installed.
    DisconnectForInstall,
    /// The installation the connection was ended for was given up.
    InstallAbandoned,
    HelperConnected,
    HelperDisconnected,
    HelperStartFailed(HelperStartFailure),
    /// The Helper waits for the user's approval in the system settings.
    HelperApprovalNeeded,
    /// The Helper's answer to a request this controller made.
    HelperReplied(AppRequest, AppResponse),
    /// A request this controller made that the Helper did not answer.
    HelperUnanswered(AppRequest),
    /// A change of the VPN connection state the Helper told of.
    HelperVpnStateChanged(VpnStateChange),
    VpnAccountReady(VpnAccount),
    OtpReady(Otp),
    OtpUnavailable(OtpFailure),
    AccountStateChanged(AccountState),
    ScheduleStateChanged(ScheduleState),
    IdleStateChanged(IdleState),
    SleepStateChanged(SleepState),
    SettingsChanged(Settings),
    AppStateLoaded(AppState),
    Stop,
}

/// The one place that judges whether the VPN is to be connected, and has the Helper do it.
pub struct VpnConnectionController;

impl VpnConnectionController {
    /// Creates the controller with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the controller");
        // 1. Create the queue that holds the controller's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Puts one piece of work into the controller's queue.
    pub fn push(&self, _work: ControlWork) {
        todo!("put the work into the work queue");
    }

    /// Takes the controller's work one at a time, and replaces the connection when its time
    /// comes, until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                Some(ControlWork::User(ConnectionAction::Connect)) => self.connect(),
                Some(ControlWork::User(ConnectionAction::Disconnect)) => self.disconnect(),
                Some(ControlWork::User(ConnectionAction::ReconnectNow)) | None => self.refresh(),
                Some(ControlWork::User(ConnectionAction::Quit)) => self.quit(),
                Some(ControlWork::DisconnectForInstall) => self.disconnect_for_install(),
                Some(ControlWork::HelperConnected) => self.take_over(),
                Some(ControlWork::HelperStartFailed(failure)) => self.give_up(&failure),
                Some(ControlWork::Stop) => break,
                Some(other) => self.handle(&other),
            }
        }
    }

    fn next_work(&self) -> Option<ControlWork> {
        todo!("wait for the next work, or for the time to replace the connection");
        // The time to replace the connection is counted from when the connection came up.
        // When that time comes before any work does, there is no work to give.
    }

    fn connect(&self) {
        todo!("have the Helper open a session and bring its tunnel up");
        // 1. Ask for the VPN account and wait for it.
        // 2. Ask the Helper to connect, with the settings and the account.
        // 3. As the Helper tells of each stage, report the connection state for the UI.
        // 4. When the gateway asks for an OTP, ask for one, wait for it and have the Helper
        //    submit it.
        // 5. When the Helper tells that the tunnel is up, report the connection as connected.
        //
        // While waiting in any step:
        // - The user disconnecting has the Helper end the session, and the OTP asked for is
        //   given up.
        // - The user quitting, or an update asking for the connection to be ended, cancels the
        //   connection the same way and goes on with that procedure.
        // - The user asking to connect or to reconnect changes nothing; a connection is already
        //   being made.
        // - The accounts being reported as not ready ends the attempt.
        // - A failure, the Helper refusing a request, a request coming back unanswered, or the
        //   session ending, is judged: whether to try again, wait or stop.
        // - A stop ends the wait and the run. The Helper is asked nothing; it cancels the
        //   connection when the app's connection closes.
        // - Any other work is handled right away, the same way as while no connection is being
        //   made.
    }

    fn refresh(&self) {
        todo!("replace the connection with a new one while the old one keeps carrying traffic");
        // 1. Ask the Helper to open a new session in place of the one that is up.
        // 2. Go through the same stages as a connection, the OTP included.
        // 3. When the Helper tells that the replacement is done, report the new connection.
        //
        // When the replacement fails, the old connection stays and the failure is reported.
        // With no connection up, this is a connection.
        //
        // While waiting in any step:
        // - The user disconnecting, the user quitting, or an update asking for the connection
        //   to be ended, cancels the replacement and goes on with that procedure.
        // - The user asking to connect or to reconnect changes nothing; a replacement is
        //   already being made.
        // - A stop ends the wait and the run; the old connection stays.
        // - Any other work is handled right away, the same way as while no connection is being
        //   made.
    }

    fn disconnect(&self) {
        todo!("have the Helper end the session, and stop connecting by itself");
    }

    fn disconnect_for_install(&self) {
        todo!("end the connection so that an update can be installed");
        // 1. Have whether the VPN was connected kept for the next run.
        // 2. Have the Helper end the session and wait for its end.
        // 3. Report that the connection ended.
        //
        // With no session up, or with the Helper not connected, there is nothing to wait for
        // and the end is reported at once.
        //
        // While waiting:
        // - The user quitting goes on with quitting.
        // - The connection to the Helper closing counts as the end of the connection.
        // - A stop ends the wait and the run.
    }

    fn quit(&self) {
        todo!("tell the Helper that the app is quitting and let the app quit");
        // 1. Tell the Helper that the app is quitting, which ends the user's sessions and
        //    removes the user's exception routes.
        // 2. Wait for the Helper's answer that it took the request.
        // 3. Report that the app can quit now.
        //
        // The end of the sessions is not waited for.
        // With the Helper not connected there is nobody to tell, and the app can quit at once.
        //
        // While waiting:
        // - The request coming back unanswered lets the app quit.
        // - A command of the user is not taken.
        // - A stop ends the wait and the run.
    }

    fn give_up(&self, _failure: &HelperStartFailure) {
        todo!("tell the user why the Helper cannot be used and let the app quit");
        // 1. Raise a notice with the reason and what the user can do about it.
        // 2. Report that the app can quit now.
    }

    fn take_over(&self) {
        todo!("take over the sessions the Helper holds for the user");
        // 1. Ask the Helper which sessions the user has, and wait for the answer.
        // 2. A session that is up becomes this controller's connection, and its state is
        //    reported for the UI.
        // 3. With no session, judge whether to connect.
        //
        // While waiting for the answer:
        // - The user quitting goes on with quitting.
        // - The user asking to connect, to disconnect or to reconnect, and an update asking for
        //   the connection to be ended, are kept and taken up after the answer.
        // - The request coming back unanswered ends the taking over.
        // - A stop ends the wait and the run.
    }

    fn handle(&self, _work: &ControlWork) {
        todo!("take in what changed and judge again whether the VPN is to be connected");
        // What goes into the judgement: what the user asked for last, whether the time is in
        // the schedule, whether the user is away, whether the device sleeps, whether the
        // accounts are ready, whether the Helper is connected, why the last connection ended
        // and how often connecting has failed.
        //
        // A Helper that waits for the user's approval counts as not connected, and the wait
        // says so.
        // Whether the VPN was connected before an installation, kept from the last run, goes
        // into the judgement once and is then cleared.
        // An installation that was given up has that kept value cleared and the judgement made
        // again.
        //
        // The judgement ends in one of: connect now, wait and say why, or stay as it is.
        // The count of failures is kept for the next run, so that quitting does not reset it.
    }
}
