//! Root processes told apart by pid, start time and executable path: a pid can be reused, so only a
//! process that matches all three is the one an earlier Helper recorded.

use libproc::libproc::bsd_info::BSDInfo;
use libproc::libproc::proc_pid::{pidinfo, pidpath};
use libproc::processes::{ProcFilter, pids_by_type};
use serde::{Deserialize, Serialize};

/// As the pidfile records it; the field names are the Swift Helper's.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProcessIdentity {
    pub pid: i32,
    pub start_seconds: u64,
    pub start_microseconds: u64,
    pub path: String,
}

impl ProcessIdentity {
    /// The live process `pid` if it runs as root; `None` if it is gone, a zombie, unreadable or not
    /// root.
    pub fn of_root_process(pid: i32) -> Option<Self> {
        // The pid may come from a pidfile; 0 would name the kernel.
        if pid <= 0 {
            return None;
        }
        let info = pidinfo::<BSDInfo>(pid, 0).ok()?;
        if info.pbi_uid != 0 {
            return None;
        }
        // A zombie has no executable path any more.
        let path = pidpath(pid).ok()?;
        Some(Self {
            pid,
            start_seconds: info.pbi_start_tvsec,
            start_microseconds: info.pbi_start_tvusec,
            path,
        })
    }

    /// The root processes that `parent` started and that are still running.
    pub fn root_children(parent: u32) -> Vec<Self> {
        // Listing fails when there are none.
        let children =
            pids_by_type(ProcFilter::ByParentProcess { ppid: parent }).unwrap_or_default();
        children
            .into_iter()
            .filter_map(|pid| Self::of_root_process(i32::try_from(pid).ok()?))
            .collect()
    }

    /// Whether the recorded process still runs: same pid, start time and path, as root.
    pub fn is_running(&self) -> bool {
        Self::of_root_process(self.pid).as_ref() == Some(self)
    }
}
