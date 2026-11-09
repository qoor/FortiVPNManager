//! Shared contract: what the app and the Helper exchange, what a backend adapter provides, and how
//! Swift and Rust talk inside each process.

pub mod app_helper;
pub mod app_internal;
pub mod backend;
pub mod helper_internal;
pub mod stream;
