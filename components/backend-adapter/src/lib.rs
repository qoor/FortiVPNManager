//! Backend adapter: what is particular to one VPN client, behind `fvm_contract::backend::Backend`.
//! openconnect runs on Windows and macOS.

#[cfg(unix)]
mod homebrew;
pub mod openconnect;
mod values;

pub use values::is_forbidden;
