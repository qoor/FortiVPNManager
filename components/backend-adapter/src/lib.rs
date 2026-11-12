//! Backend adapters: what is particular to one VPN client, behind `fvm_contract::backend::Backend`.
//! Both compile on every OS: the app on either OS reads both clients' certificate values in
//! config.yaml. openfortivpn runs only on Unix, and openconnect is used on Windows.

pub mod openconnect;
pub mod openfortivpn;
mod values;
