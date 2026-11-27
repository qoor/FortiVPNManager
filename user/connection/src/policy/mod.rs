use fvm_user_protocol::service::{ServiceClient, Settings};

/// One piece of work for the connection policy manager.
pub enum PolicyWork {
    SettingsChanged(Settings),
    /// Something the OS told of.
    Os(OsEvent),
    Stop,
}

/// What the OS told of.
pub enum OsEvent {
    Slept,
    Woke,
    ClockChanged,
}

/// The facts a connection is judged by that come from time and use: the schedule and idleness.
pub struct ConnectionPolicyManager;

impl ConnectionPolicyManager {
    /// Creates the manager with the client it tells the main service through.
    #[must_use]
    pub fn new(_service: ServiceClient) -> Self {
        todo!("create the manager");
        // 1. Create the queue that holds the manager's work.
        // 2. Keep the client for reporting to the main service.
    }

    /// Makes what the manager needs before it can run.
    pub fn init(&self) {
        todo!("ask the OS to tell of sleep, of waking and of changes of the clock or time zone");
    }

    /// Takes the manager's work one at a time, and follows the time, until it is told to stop.
    pub fn run(&self) {
        loop {
            match self.next_work() {
                Some(PolicyWork::SettingsChanged(settings)) => self.apply_settings(&settings),
                Some(PolicyWork::Os(event)) => self.follow_os_event(&event),
                Some(PolicyWork::Stop) => break,
                None => self.follow_time(),
            }
        }
    }

    /// Puts one piece of work into the manager's queue.
    pub fn push(&self, _work: PolicyWork) {
        todo!("put the work into the work queue");
    }

    fn next_work(&self) -> Option<PolicyWork> {
        todo!("wait for the next work in the work queue, or for the next time to look at");
        // The next time to look at is the nearer of the next schedule time and the next look at
        // whether the user is away.
        // When that time comes before any work does, there is no work to give.
    }

    fn apply_settings(&self, _settings: &Settings) {
        todo!("take in the schedule and the idle settings that changed");
        // Whether the time is in the schedule, and the next start, are worked out again and
        // reported when they changed.
    }

    fn follow_os_event(&self, _event: &OsEvent) {
        todo!("report what the OS told of");
        // Sleep and waking are reported as they are.
        // A change of the clock or the time zone has the schedule times worked out again.
    }

    fn follow_time(&self) {
        todo!("report what changed with the time");
        // 1. When a schedule time came, report whether the time is in the schedule and when the
        //    next start is.
        // 2. Read from the OS how long the device has not been used, and report when the user
        //    went away or came back.
    }
}
