use fvm_common_base::protocol::{AppRequest, ChannelMessage};

use crate::service::ServiceClient;

/// One piece of work for the Helper link manager.
pub enum HelperLinkWork {
    /// Start the Helper service when it is not running, and connect to it.
    StartAndConnect,
    /// Send a request to the Helper.
    Send(AppRequest),
    /// Something the channel reader saw.
    Channel(ChannelEvent),
    Stop,
}

/// What the channel reader saw on the OS connection.
pub enum ChannelEvent {
    Connected,
    MessageArrived(ChannelMessage),
    Disconnected,
}

/// The app's end of the channels between the app and the Helper.
pub struct HelperLinkManager;

impl HelperLinkManager {
    /// Creates the manager with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the manager");
        // 1. Create the queue that holds the manager's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: HelperLinkWork) {
        todo!("put the work into the work queue");
    }

    /// Takes the manager's work one at a time, and tries starting the Helper again when its time
    /// comes, until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                Some(HelperLinkWork::StartAndConnect) | None => self.start_and_connect(),
                Some(HelperLinkWork::Send(request)) => self.send(&request),
                Some(HelperLinkWork::Channel(event)) => self.handle(&event),
                Some(HelperLinkWork::Stop) => break,
            }
        }
        self.stop();
    }

    fn next_work(&self) -> Option<HelperLinkWork> {
        todo!("wait for the next work, or for the next time to try starting the Helper again");
        // There is such a time only while the Helper waits for the user's approval.
        // When that time comes before any work does, there is no work to give.
    }

    fn start_and_connect(&self) {
        todo!("start the Helper service when it is not running, and connect to it");
        // 1. Ask the OS whether the Helper service runs, and start it when it does not.
        // 2. Open the connection to the Helper and start the thread that reads what it sends.
        // 3. Negotiate with the Helper: each side checks the other, and the versions are
        //    compared.
        //
        // A step that fails is reported with its name and the reason.
        // A Helper that waits for the user's approval is reported as waiting for it, and
        // starting is tried again when its time comes.
    }

    fn send(&self, _request: &AppRequest) {
        todo!("send the request to the Helper");
        // The request goes out on the request channel with a number, and is kept under that
        // number until its answer arrives.
        // A request made while the Helper is not connected is reported back as unanswered.
    }

    fn handle(&self, _event: &ChannelEvent) {
        todo!("handle what the channel reader saw");
        // The connection is up:
        // - It is reported to the main service.
        //
        // A message arrived, sorted by its channel:
        // - An answer is matched to its request by the number and reported with that request.
        // - A state change is reported to the main service, as a change of the VPN connection
        //   state or of the exception routes' state.
        // - A log line is recorded through the log manager; it does not go to the main service.
        //
        // The connection closed:
        // - The requests still waiting for an answer are reported back as unanswered, and the
        //   closing is reported to the main service.
    }

    fn stop(&self) {
        todo!("close the connection to the Helper and end the reader thread");
    }
}
