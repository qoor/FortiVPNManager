use fvm_user_protocol::service::UserAction;

/// Starts the app's main service. The app's entry calls it once, before the UI runs.
pub fn start() {
    todo!("start the app's main service");
    // 1. Create the main service and have it make its preparations.
    // 2. Run the main service on a thread of its own, which leaves the calling thread to the UI.
    // 3. Start handing what the main service has for the UI, and the log lines, to the UI.
}

/// Hands what the user did in the UI to the main service.
pub fn push(_action: UserAction) {
    todo!("put what the user did into the main service's queue");
}

/// Stops the app's main service. The app's entry calls it once, when the app quits.
pub fn stop() {
    todo!("ask the main service to stop and wait until it has");
}
