//! The Helper's common part, the same on every OS, in three modules by role:
//!
//! - `entry`, the Helper's front: it admits the app connections the OS side passes on, decides who
//!   may do what with the one run, answers the app's requests, and decides when the Helper exits.
//! - `process`, the VPN client: checking it, running it with the password, OTP and session input,
//!   its output, the time limits, stopping it and the logout.
//! - `recovery`, what a run leaves for a later Helper: the run record, stopping what an earlier
//!   Helper left, and the route repair requests.
//!
//! Only `process` and `recovery` use the backend adapter, which the OS's start hands over in a
//! `Watchdog` with the run directory it chose; `entry` gets the `Watchdog` but cannot reach the
//! backend in it. The OS side, the Helper's Swift side on macOS and the service's own part on
//! Windows, takes the app connections and does what the entry asks of it; the two exchange the
//! messages in `fvm_contract::helper_internal`. OS-specific code is only in `process`: running,
//! finding and stopping processes, and the run directory's permissions.

pub mod entry;
pub mod process;
pub mod recovery;
