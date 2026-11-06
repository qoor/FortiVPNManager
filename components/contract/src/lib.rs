//! Shared contract: what the app and the Helper exchange, what a backend adapter provides, and how
//! Swift and Rust talk inside one process.

pub mod app_helper;
pub mod backend;
pub mod helper_internal;
pub mod stream;
