use fvm_common_base::protocol::{AppRequest, AppResponse};
use fvm_user_protocol::service::{AppState, ServiceClient, UpdateAction};

/// One piece of work for the update manager.
pub enum UpdateWork {
    /// What the user did about updates.
    User(UpdateAction),
    /// The Helper is connected.
    HelperConnected,
    /// The Helper's answer to a request this manager made.
    HelperReplied(AppRequest, AppResponse),
    /// A request this manager made that the Helper did not answer.
    HelperUnanswered(AppRequest),
    /// The app state kept from the last run.
    AppStateLoaded(AppState),
    /// The connection ended.
    ConnectionEnded,
    Stop,
}

/// The app's part of an update: finding a new version, downloading it and having it installed.
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
        todo!("get the place downloaded installers are kept in ready");
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: UpdateWork) {
        todo!("put the work into the work queue");
    }

    /// Takes the manager's work one at a time, and looks for a new version when its time comes,
    /// until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                Some(UpdateWork::User(UpdateAction::Check)) | None => self.check(),
                Some(UpdateWork::User(UpdateAction::StartDownload)) => self.download(),
                Some(UpdateWork::User(UpdateAction::StartInstall)) => self.install(),
                Some(UpdateWork::HelperConnected) => self.ask_prepared(),
                Some(UpdateWork::AppStateLoaded(state)) => self.apply_app_state(&state),
                Some(UpdateWork::Stop) => break,
                Some(other) => self.handle(&other),
            }
        }
    }

    fn next_work(&self) -> Option<UpdateWork> {
        todo!("wait for the next work, or for the next time to look for a new version");
        // When that time comes before any work does, there is no work to give.
    }

    fn check(&self) {
        todo!("look for a version newer than the running one");
        // 1. Ask the release page for the latest version.
        // 2. Report what the UI shows about updates.
        // 3. For a new version the user was not yet told of, raise a notice once, and have the
        //    version kept so that the next run does not tell again.
        // 4. When the latest version is higher than the update the Helper holds ready, ask the
        //    Helper to delete that update.
    }

    fn ask_prepared(&self) {
        todo!("ask the Helper which update it holds ready");
    }

    fn download(&self) {
        todo!("download the new version's installer and have the Helper make it ready");
        // 1. Download the installer and its checksum file, reporting the progress.
        // 2. Ask the Helper to make the installer ready, and wait for its answer.
        // 3. Report that the update is ready, or why it is not.
        //
        // While downloading or waiting:
        // - The user cancelling ends the download and deletes what was downloaded.
        // - A stop does the same as cancelling, and ends the run.
        // - Looking for a new version and starting an installation are not taken.
        // - The Helper refusing, or the request coming back unanswered, is reported as why the
        //   update is not ready.
        // - Any other work is handled right away, the same way as while nothing is being
        //   downloaded.
    }

    fn install(&self) {
        todo!("have the Helper install the update that is ready");
        // 1. Have the version to be installed kept for the next run.
        // 2. Ask for the connection to be ended, and wait until it ended.
        // 3. Arrange for the app to be opened again after the installation.
        // 4. Ask the Helper to install the update, and wait for its answer.
        // 5. When the Helper answers that the installation starts, report that the app can quit
        //    now.
        //
        // When the Helper refuses, or does not answer, the failure is reported, the version
        // kept in the first step is cleared, and that the installation was given up is reported
        // so that the connection is judged again.
        //
        // While waiting:
        // - A command of the user is not taken.
        // - A stop ends the wait and the run.
    }

    fn apply_app_state(&self, _state: &AppState) {
        todo!("take in the app state kept from the last run");
        // 1. Take in the version the user was already told of, so that looking for a new
        //    version does not tell of it again.
        // 2. Work out how the installation that was started in the last run ended: the running
        //    version is compared with the version that was to be installed, and a running
        //    version that is lower is reported as a failed installation.
        // 3. Have the version that was to be installed cleared, so that the next run does not
        //    work it out again.
    }

    fn handle(&self, _work: &UpdateWork) {
        todo!("handle work that arrives while nothing is being downloaded or installed");
        // An answer of the Helper about the update that is ready changes what the UI shows.
        // A request the Helper did not answer leaves what the UI shows as it is.
        // A cancelled download or an ended connection finds nothing waiting for it.
    }
}
