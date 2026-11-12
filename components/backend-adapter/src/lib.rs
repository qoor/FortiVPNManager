//! Backend adapters: what is particular to one VPN client, behind `fvm_contract::backend::Backend`.
//! The account and certificate value checks, which the app uses, compile on every OS; finding and
//! running a client only where it runs.

pub mod openfortivpn;
mod values;
