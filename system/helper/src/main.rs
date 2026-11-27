fn main() {
    todo!("run the Helper service until the OS asks it to stop");
    // 1. Have an internal error that stops the process recorded in the OS log.
    // 2. Create the main service and have it make its preparations.
    // 3. When the OS tells the service to stop, put the stop request into the main service's
    //    queue.
    // 4. Run the main service. The process ends when it returns.
}
