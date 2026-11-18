//! Backend adapter: what is particular to one VPN client, behind
//! `fvm_helper_core::session::backend::Backend`. openconnect runs on Windows and macOS.

pub mod openconnect;
