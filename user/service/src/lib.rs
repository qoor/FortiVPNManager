pub mod config;
pub mod update;

use fvm_user_protocol::service::ServiceMessage;

/// The app's main service: it starts the features and passes what one reports to the others.
pub struct VpnAppService;

impl Default for VpnAppService {
    fn default() -> Self {
        Self::new()
    }
}

impl VpnAppService {
    /// Creates the main service.
    #[must_use]
    pub fn new() -> Self {
        todo!("create the main service with its message queue");
    }

    /// Puts one message into the main service's queue.
    pub fn push(&self, _message: ServiceMessage) {
        todo!("put the message into the message queue");
    }

    /// Makes what the app needs before it can serve.
    pub fn init(&self) {
        todo!("get the app ready to serve");
        // 1. Get the log manager ready.
        // 2. Create the features and have each make its own preparations.
        // 3. Run each feature on a thread of its own.
        // 4. Have the Helper link manager start the Helper and connect to it.
    }

    /// Passes messages on until the app is asked to stop.
    pub fn run(&self) {
        loop {
            match self.next_message() {
                ServiceMessage::StopRequested => break,
                message => self.pass(&message),
            }
        }
        self.stop();
    }

    fn next_message(&self) -> ServiceMessage {
        todo!("wait for the next message in the message queue");
    }

    fn pass(&self, _message: &ServiceMessage) {
        todo!("turn the message into work for the features it concerns");
        // The main service only looks at the kind of the message; it decides nothing.
        //
        // From the UI:
        // - What the user did goes to the feature that acts on it: the connection to the VPN
        //   connection controller, the settings to the config manager, the accounts to the
        //   auth manager, updates to the update manager.
        //
        // To the UI:
        // - What the UI shows about the connection, the settings, the accounts and updates, a
        //   problem in the settings file, the result of a test or of a load, a change of the
        //   exception routes' state, a notice, and that the app can quit.
        //
        // Between the features:
        // - A request for the Helper goes to the Helper link manager.
        // - The Helper's answer, or a request coming back unanswered, goes to the feature that
        //   makes that kind of request: connecting, replacing, disconnecting, submitting an OTP,
        //   listing the sessions and quitting to the VPN connection controller; preparing,
        //   installing, reading and discarding an update to the update manager; the exception
        //   rules to the config manager.
        // - A change of the VPN connection state the Helper told of, and whether the Helper is
        //   connected, go to the VPN connection controller. That the Helper is connected also
        //   goes to the update manager.
        // - That the Helper could not be started goes to the VPN connection controller. That it
        //   waits for the user's approval goes to the VPN connection controller and to the UI.
        // - The closing of the connection to the Helper also has the Helper link manager start
        //   the Helper and connect again.
        // - A need for the VPN account or for an OTP, and that the OTP is no longer needed, go to
        //   the auth manager. The account, the OTP and whether the accounts are ready go to the
        //   VPN connection controller.
        // - The settings go to the VPN connection controller, the connection policy manager
        //   and the auth manager.
        // - The schedule, idleness and sleep go to the VPN connection controller.
        // - A value of the app state to keep goes to the config manager, and the app state
        //   that was kept goes to the VPN connection controller and the update manager.
        // - The end of the connection goes to the update manager. Its asking for the connection
        //   to be ended, and its giving an installation up, go to the VPN connection controller.
    }

    fn stop(&self) {
        todo!("tell every feature to stop and wait for its thread to end");
    }
}
