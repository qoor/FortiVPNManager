//! What the Helper entry needs from the OS it runs on. The OS's start passes it in, as it passes the
//! backend adapter, so the entry itself uses no OS-specific API.

use std::fs::File;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::run_directory::Failure;

/// A process told apart by pid, start time and executable path: a pid can be reused, so only a
/// process that matches all three is the one an earlier Helper recorded. As the pidfile records it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessIdentity {
    pub pid: i32,
    pub start_seconds: u64,
    pub start_microseconds: u64,
    pub path: String,
}

impl ProcessIdentity {
    /// Whether the recorded process still runs: same pid, start time and path, with the Helper's
    /// rights.
    pub(crate) fn is_running(&self, platform: &impl Platform) -> bool {
        platform.process(self.pid).as_ref() == Some(self)
    }
}

pub trait Platform {
    /// How the Helper's log names the forced stop.
    const FORCE_STOP: &'static str;

    /// The live process `pid` if it runs with the Helper's rights; `None` if it is gone, a zombie,
    /// unreadable or another user's.
    fn process(&self, pid: i32) -> Option<ProcessIdentity>;

    /// The processes that `parent` started and that still run with the Helper's rights.
    fn children(&self, parent: u32) -> Vec<ProcessIdentity>;

    /// Asks `process` to end, so that a client can restore the routes. It may have exited since it
    /// was checked.
    fn request_stop(&self, process: &ProcessIdentity);

    /// Ends `process` at once. It may have exited since it was checked.
    fn force_stop(&self, process: &ProcessIdentity);

    /// Creates the run directory if needed and makes sure it is a real directory that nobody but
    /// the Helper can place or read files in.
    fn prepare_run_directory(&self, directory: &Path) -> Result<(), Failure>;

    /// A new file that only the Helper can read; an existing file or symlink at `path` is an error.
    fn create_private_file(&self, path: &Path) -> io::Result<File>;
}
