use std::net::IpAddr;

use fvm_core::connection::InterfaceAddress;
use fvm_os_api_windows::network::{self, Family};

pub(super) fn interface_addresses() -> Vec<InterfaceAddress> {
    let Ok(addresses) = network::addresses(Family::Ipv4) else {
        return Vec::new();
    };
    addresses
        .into_iter()
        .filter_map(|address| {
            let IpAddr::V4(ipv4) = address.address else {
                return None;
            };
            let interface = network::interface(address.interface).ok()?;
            Some(InterfaceAddress {
                name: interface.alias,
                up: interface.up,
                ipv4,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    fn the_loopback_interface_has_its_address_and_is_up() {
        assert!(
            interface_addresses()
                .iter()
                .any(|address| address.ipv4 == Ipv4Addr::LOCALHOST && address.up)
        );
    }
}
