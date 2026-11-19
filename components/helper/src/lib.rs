//! The Helper's common part, the same on every OS, in three modules by role:
//!
//! - `entry`, the Helper's front: it admits the app connections the OS side passes on, decides who
//!   may do what with the one run, answers the app's requests, and decides when the Helper exits.
//! - `session`, the VPN sessions: checking a request, running a session with the password, OTP and
//!   network settings, its log lines, the time limits and stopping it.
//! - `recovery`, what a run leaves for a later Helper: the run record, the session secrets, and
//!   cleaning up after an earlier Helper.
//!
//! The OS's start hands over the VPN client's backend and the store for the session secrets
//! (`secrets`) in a `SessionManager` with the run directory it chose; `entry` gets the
//! `SessionManager` but cannot reach them. The OS side, the Helper's Swift side on macOS and the
//! service's own part on Windows, takes the app connections and does what the entry asks of it;
//! the two exchange the messages in `fvm_contract::helper_internal`.

pub mod entry;
pub mod recovery;
pub mod secrets;
pub mod session;
mod watchdog;
