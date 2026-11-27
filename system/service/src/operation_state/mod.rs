use fvm_system_protocol::service::{
    ExceptionRoutesRecord, ServiceClient, SessionRecord, TunnelSettingsRecord,
};

/// One piece of work for the operation state manager.
pub enum OperationStateWork {
    /// Report what an earlier Helper left behind.
    CheckLeftovers,
    /// Write the record of something a feature created.
    Save(OperationRecord),
    /// Remove the record of something that was cleaned up.
    Forget(OperationRecord),
    Stop,
}

/// What the Helper writes down so that a later Helper can clean up after it.
pub enum OperationRecord {
    Session(SessionRecord),
    TunnelSettings(TunnelSettingsRecord),
    ExceptionRoutes(ExceptionRoutesRecord),
}

/// The records a Helper leaves for a later one, and what an earlier one left for this one.
pub struct OperationStateManager;

impl OperationStateManager {
    /// Creates the manager with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the manager");
        // 1. Create the queue that holds the manager's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Makes what the manager needs before it can run.
    pub fn init(&self) {
        todo!("get the place the records are kept in ready");
        // The place has to exist, and only the Helper may use it.
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: OperationStateWork) {
        todo!("put the work into the work queue");
    }

    /// Takes the manager's work one at a time until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                OperationStateWork::CheckLeftovers => self.check_leftovers(),
                OperationStateWork::Save(record) => self.save(&record),
                OperationStateWork::Forget(record) => self.forget(&record),
                OperationStateWork::Stop => break,
            }
        }
    }

    fn next_work(&self) -> OperationStateWork {
        todo!("wait for the next work in the work queue");
    }

    fn check_leftovers(&self) {
        todo!("report what an earlier Helper left behind");
        // 1. Read the records an earlier Helper wrote. A record that cannot be read is skipped,
        //    and the others are still used.
        // 2. Report each session, each set of tunnel settings and each set of exception routes
        //    the records name.
        // 3. Report that the check is finished.
    }

    fn save(&self, _record: &OperationRecord) {
        todo!("write the record of something a feature created");
    }

    fn forget(&self, _record: &OperationRecord) {
        todo!("remove the record of something that was cleaned up");
    }
}
