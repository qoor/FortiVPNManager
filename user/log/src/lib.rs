use std::sync::mpsc::Receiver;

use fvm_common_base::protocol::{LogLine, SessionId};

/// The level of a line in the OS log.
pub enum SystemLogLevel {
    Info,
    Notice,
    Error,
}

/// The app's logs. Every feature records through it; it does not depend on the main service.
pub struct LogManager;

impl LogManager {
    /// Makes what the logs need before a line can be recorded. The app calls it once.
    pub fn init() {
        todo!("get the app's logs ready");
        // 1. Open the app's log file, starting a new one when it is over its size.
        // 2. Connect to the OS log.
        //
        // A log file that cannot be opened does not stop the app.
    }

    /// Records a line the Helper sent on the log channel.
    pub fn record_helper(_line: LogLine) {
        todo!("record a line the Helper sent");
        // 1. Write the line to the app's log file as the Helper sent it; the time and the
        //    session ID are already in it.
        // 2. Hand the line to the subscriber, when there is one.
    }

    /// Records a line the app writes about a session.
    pub fn record_app(_session: SessionId, _text: &str) {
        todo!("record a line the app writes about a session");
        // 1. Put the time and the session ID before the text.
        // 2. Write the line to the app's log file.
        // 3. Hand the line to the subscriber, when there is one.
    }

    /// Records a line in the OS log.
    pub fn record_system(_level: SystemLogLevel, _text: &str) {
        todo!("record a line in the OS log");
        // The line goes to the unified log on macOS and to the event log on Windows.
    }

    /// Gives the lines recorded from now on to the caller.
    #[must_use]
    pub fn subscribe() -> Receiver<LogLine> {
        todo!("give the lines recorded from now on to the caller");
        // The app's entry subscribes, to show the lines in the log pane.
    }
}
