use fvm_common_base::protocol::{AppResponse, ChannelMessage, StateChange};

use crate::service::{AppId, Requester, ServiceClient};

/// One piece of work for the app link manager.
pub enum AppLinkWork {
    /// Send the answer to an app's request.
    Respond(Requester, AppResponse),
    /// Send a state change to the connected apps.
    Propagate(StateChange),
    /// Something the channel reader saw.
    Channel(ChannelEvent),
    Stop,
}

/// What the channel reader saw on the OS connection.
pub enum ChannelEvent {
    AppConnected(AppId),
    MessageArrived(AppId, ChannelMessage),
    AppDisconnected(AppId),
}

/// The Helper's end of the channels between the apps and the Helper.
pub struct AppLinkManager;

impl AppLinkManager {
    /// Creates the manager with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the manager");
        // 1. Create the queue that holds the manager's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Makes what the manager needs before it can run.
    pub fn init(&self) {
        todo!("get ready for the apps to connect");
        // 1. Open the place on the OS that apps connect to. An app that connects before the
        //    manager runs waits there until it does.
        // 2. Ask the log manager for the lines it records from now on.
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: AppLinkWork) {
        todo!("put the work into the work queue");
    }

    /// Serves the apps and takes the manager's work one at a time until it is told to stop.
    pub fn run(&self) {
        self.accept();
        loop {
            match self.next_work() {
                AppLinkWork::Respond(requester, response) => self.respond(&requester, &response),
                AppLinkWork::Propagate(change) => self.propagate(&change),
                AppLinkWork::Channel(event) => self.handle(&event),
                AppLinkWork::Stop => break,
            }
        }
        self.stop();
    }

    fn accept(&self) {
        todo!("start accepting the apps that connect");
        // 1. Start the thread that accepts connections and reads what the apps send.
        // 2. Start sending the lines the log manager records to the connected apps, on the log
        //    channel.
    }

    fn next_work(&self) -> AppLinkWork {
        todo!("wait for the next work in the work queue");
    }

    fn respond(&self, _requester: &Requester, _response: &AppResponse) {
        todo!("send the answer to the app that asked");
        // The answer goes out on the request channel with the number the app gave its request,
        // in the order answers become ready.
        // An answer for an app that is no longer connected is dropped.
    }

    fn propagate(&self, _change: &StateChange) {
        todo!("send the state change to the connected apps");
        // The change goes out on the propagation channel, marked as a VPN connection state or an
        // exception route state.
        // With no app connected the change is dropped.
    }

    fn handle(&self, _event: &ChannelEvent) {
        todo!("handle what the channel reader saw");
        // An app connected:
        // - The two sides negotiate, and a peer that is not this product's app is refused.
        //
        // A message arrived:
        // - The message is decoded and sorted by its channel.
        // - A request whose form or values are wrong is answered with an error and goes no further.
        // - The version check is answered here.
        // - Any other request is reported to the main service, with the app and the user it came
        //   from and the number the app gave it.
        //
        // An app's connection closed:
        // - The app is forgotten and the closing is reported to the main service.
    }

    fn stop(&self) {
        todo!("close every app connection and end the reader thread");
    }
}
