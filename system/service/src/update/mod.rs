use fvm_common_base::protocol::{
    PreparedUpdateDiscardRequest, PreparedUpdateReadRequest, UpdateInstallRequest,
    UpdatePrepareRequest,
};
use fvm_system_protocol::service::{Requester, ServiceClient};

/// One piece of work for the update manager.
pub enum UpdateWork {
    /// Make a downloaded installer ready to install.
    Prepare(Requester, UpdatePrepareRequest),
    /// Install the prepared update.
    Install(Requester, UpdateInstallRequest),
    /// Answer with the prepared update.
    ReadPrepared(Requester, PreparedUpdateReadRequest),
    /// Delete the prepared update.
    DiscardPrepared(Requester, PreparedUpdateDiscardRequest),
    Stop,
}

/// The Helper's part of an update: checking the installer, keeping it and running it.
pub struct UpdateManager;

impl UpdateManager {
    /// Creates the manager with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the manager");
        // 1. Create the queue that holds the manager's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Makes what the manager needs before it can run.
    pub fn init(&self) {
        todo!("get the place the prepared update is kept in ready");
        // The place has to exist, and only the Helper may write to it.
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: UpdateWork) {
        todo!("put the work into the work queue");
    }

    /// Takes the manager's work one at a time until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                UpdateWork::Prepare(requester, request) => self.prepare(&requester, &request),
                UpdateWork::Install(requester, request) => self.install(&requester, &request),
                UpdateWork::ReadPrepared(requester, request) => {
                    self.read_prepared(&requester, &request);
                }
                UpdateWork::DiscardPrepared(requester, request) => {
                    self.discard_prepared(&requester, &request);
                }
                UpdateWork::Stop => break,
            }
        }
    }

    fn next_work(&self) -> UpdateWork {
        todo!("wait for the next work in the work queue");
    }

    fn prepare(&self, _requester: &Requester, _request: &UpdatePrepareRequest) {
        todo!("make the downloaded installer ready to install");
        // 1. Copy the installer the app downloaded to a place only the Helper can write to.
        // 2. Check its signature.
        // 3. Check its hash.
        // 4. Check its version.
        // 5. Answer that the update is ready, or why it is not.
    }

    fn install(&self, _requester: &Requester, _request: &UpdateInstallRequest) {
        todo!("install the prepared update");
        // 1. Answer that the installation starts, or why it cannot.
        // 2. Write down that the installation started.
        // 3. Run the installer program of the OS on the prepared installer.
        // 4. Write down how it ended, with the installer's exit code when it could be seen.
    }

    fn read_prepared(&self, _requester: &Requester, _request: &PreparedUpdateReadRequest) {
        todo!("answer with the version of the prepared update, or that there is none");
    }

    fn discard_prepared(&self, _requester: &Requester, _request: &PreparedUpdateDiscardRequest) {
        todo!("delete the prepared update and answer");
    }
}
