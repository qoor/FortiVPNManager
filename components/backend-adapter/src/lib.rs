//! Backend adapters: what is particular to one VPN client, behind `fvm_contract::backend::Backend`.
//! Both compile on every OS: the app on either OS reads both clients' certificate values in
//! config.yaml. openfortivpn runs only on Unix; openconnect runs on Windows and macOS.

#[cfg(unix)]
mod homebrew;
pub mod openconnect;
pub mod openfortivpn;
mod values;
