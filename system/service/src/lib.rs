pub mod operation_state;
pub mod update;

use fvm_system_protocol::service::ServiceMessage;

/// The Helper's main service: it starts the features and passes what one reports to the others.
pub struct VpnHelperService;

impl Default for VpnHelperService {
    fn default() -> Self {
        Self::new()
    }
}

impl VpnHelperService {
    /// Creates the main service.
    #[must_use]
    pub fn new() -> Self {
        todo!("create the main service with its message queue");
    }

    /// Puts one message into the main service's queue.
    pub fn push(&self, _message: ServiceMessage) {
        todo!("put the message into the message queue");
    }

    /// Makes what the Helper needs before it can serve.
    pub fn init(&self) {
        todo!("get the Helper ready to serve");
        // 1. Get the log manager ready.
        // 2. Create the features and have each make its own preparations.
        // 3. Run each feature on a thread of its own, except the app link manager.
        // 4. Ask the operation state manager what an earlier Helper left.
    }

    /// Passes messages on until the Helper is asked to stop.
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
        // From the apps:
        // - A request goes to the feature that carries it out: VPN connection requests to the
        //   VPN connection manager, exception rule requests to the network manager, update
        //   requests to the update manager.
        // - An app that is quitting has its user's sessions ended and its user's exception
        //   routes removed.
        // - An app whose connection closed has the connection it was making cancelled; sessions
        //   that are up stay.
        //
        // From the features:
        // - An answer to an app's request goes to the app link manager.
        // - The progress and the end of a connection or a replacement go to the app link
        //   manager for the apps.
        // - A session's tunnel settings go to the network manager, and what came of applying
        //   them goes back to the VPN connection manager.
        // - What a feature created goes to the operation state manager to be written down, and
        //   what it cleaned up goes there to be removed. The exception routes in place are
        //   written down as a whole, in place of the record before.
        // - A session that ended goes to the app link manager for the apps, to the network
        //   manager to undo its settings and to the operation state manager to remove its record.
        // - A change of the tunnel's route state or of the exception routes' state goes to the
        //   app link manager for the apps.
        // - What an earlier Helper left goes to the feature that made it.
        // - When everything an earlier Helper left has been reported, the app link manager is
        //   run on a thread of its own, so that apps are accepted from then on.
    }

    fn stop(&self) {
        todo!("tell every feature to stop and wait for its thread to end");
    }
}
