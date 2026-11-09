//! Runtime: the app's orchestration and every outside job that is the same on every OS. It takes
//! all inputs in one queue (UI commands, port responses, system events, Helper messages, timers),
//! asks the judgment core what to do and carries it out: Helper requests in order, timers on the
//! wall clock, OTP sessions, the Google account, config.yaml, state.json and the log file, syncing
//! the exception routes, and the sections and log lines the UI shows. It uses the backend adapter
//! only to import the VPN client's settings and to check certificate and account values. Whatever
//! differs per OS goes through ports; how each OS starts the runtime lives in that OS's module.

mod bypass_sync;
mod config_file;
mod google_account;
mod log_file;
#[cfg(target_os = "macos")]
pub mod macos;
mod orchestration;
mod otp_session;
mod state_file;
