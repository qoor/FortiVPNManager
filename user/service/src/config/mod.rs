use fvm_common_base::protocol::{AppRequest, AppResponse};
use fvm_user_protocol::service::{AppState, ConfigAction, ServiceClient, SettingChange};

/// One piece of work for the config manager.
pub enum ConfigWork {
    /// What the user did about the settings.
    User(ConfigAction),
    /// Keep a value of the app state for the next run.
    SaveAppState(AppState),
    /// The settings file changed on disk.
    FileChanged,
    /// The Helper's answer to a request this manager made.
    HelperReplied(AppRequest, AppResponse),
    /// A request this manager made that the Helper did not answer.
    HelperUnanswered(AppRequest),
    Stop,
}

/// The app's settings in config.yaml, and the app state it keeps from one run to the next.
pub struct ConfigManager;

impl ConfigManager {
    /// Creates the manager with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the manager");
        // 1. Create the queue that holds the manager's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Makes what the manager needs before it can run.
    pub fn init(&self) {
        self.load();
        self.load_app_state();
        self.watch();
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: ConfigWork) {
        todo!("put the work into the work queue");
    }

    /// Takes the manager's work one at a time until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                ConfigWork::User(ConfigAction::Change(change)) => self.save(&change),
                ConfigWork::User(ConfigAction::LoadExisting) => self.load_existing(),
                ConfigWork::SaveAppState(state) => self.save_app_state(&state),
                ConfigWork::FileChanged => self.load(),
                ConfigWork::HelperReplied(request, response) => {
                    self.take_reply(&request, &response);
                }
                ConfigWork::HelperUnanswered(request) => self.take_unanswered(&request),
                ConfigWork::Stop => break,
            }
        }
    }

    fn next_work(&self) -> ConfigWork {
        todo!("wait for the next work in the work queue");
    }

    fn load(&self) {
        todo!("read config.yaml and report the settings");
        // 1. Read the file. A file that is not there gives the default settings.
        // 2. Report what is wrong in it: an error keeps the last good settings, and an unknown
        //    key or a wrong value is left out with a warning.
        // 3. Report all the settings as they are now, and have the exception rules sent to the
        //    Helper.
    }

    fn load_existing(&self) {
        todo!("load the settings of a VPN client that is already set up on the device");
        // 1. Read the settings of the existing VPN client. They are only read, never changed.
        // 2. Write the values that were found to config.yaml.
        // 3. Report what was loaded, or why nothing was, and report the settings.
    }

    fn save(&self, _change: &SettingChange) {
        todo!("write the changed setting to config.yaml");
        // 1. Read the file as it is now, so that a change made by hand meanwhile is kept.
        // 2. Put in the changed value only, and replace the file in one step.
        // 3. Report all the settings as they are now.
    }

    fn load_app_state(&self) {
        todo!("read app_state.json and report the app state kept from the last run");
        // A file that cannot be read starts an empty state.
        // A value that is unknown or wrong counts as not there, and the others are still used.
    }

    fn save_app_state(&self, _state: &AppState) {
        todo!("write the value to app_state.json for the next run");
    }

    fn watch(&self) {
        todo!("start watching config.yaml for changes made outside the app");
    }

    fn take_reply(&self, _request: &AppRequest, _response: &AppResponse) {
        todo!("take in the Helper's answer to a request this manager made");
        // An exception rule the Helper refused is reported as a problem in the settings.
    }

    fn take_unanswered(&self, _request: &AppRequest) {
        todo!("take in that the Helper did not answer a request this manager made");
    }
}
