use std::sync::mpsc::Receiver;

use fvm_common_base::protocol::{LogLine, SessionId};

/// The level `OpenConnect` gave a line.
pub enum VpnLogLevel {
    Info,
    Error,
}

/// The level of a line in the OS log.
pub enum SystemLogLevel {
    Info,
    Notice,
    Error,
    Fault,
}

/// The Helper's logs. Every feature records through it; it does not depend on the main service.
pub struct LogManager;

impl LogManager {
    /// Makes what the logs need before a line can be recorded. The Helper calls it once.
    pub fn init() {
        todo!("get the Helper's logs ready");
        // 1. Open the Helper's log file, starting a new one when it is over its size.
        // 2. Connect to the OS log.
        //
        // A log file that cannot be opened does not stop the Helper.
    }

    /// Records a line `OpenConnect` wrote for a session.
    pub fn record_vpn(_session: SessionId, _level: VpnLogLevel, _text: &str) {
        todo!("record a line OpenConnect wrote");
        // 1. Put the time and the session ID before the text, which stays as OpenConnect wrote it.
        // 2. Write the line to the Helper's log file.
        // 3. Hand the line to the subscriber, when there is one.
    }

    /// Records a line the Helper writes about a session.
    pub fn record_helper(_session: SessionId, _text: &str) {
        todo!("record a line the Helper writes about a session");
        // 1. Put the time and the session ID before the text.
        // 2. Write the line to the Helper's log file.
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
        // The app link manager subscribes, to send the lines to the apps.
        // Lines recorded while nobody subscribes are in the Helper's log file only.
    }
}
