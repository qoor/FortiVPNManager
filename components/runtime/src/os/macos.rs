use fvm_core::connection::InterfaceAddress;
use nix::ifaddrs::getifaddrs;
use nix::net::if_::InterfaceFlags;

pub(super) fn interface_addresses() -> Vec<InterfaceAddress> {
    let Ok(entries) = getifaddrs() else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let ipv4 = entry.address.as_ref()?.as_sockaddr_in()?.ip();
            Some(InterfaceAddress {
                up: entry.flags.contains(InterfaceFlags::IFF_UP),
                name: entry.interface_name,
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
        assert!(interface_addresses().iter().any(|address| {
            address.name == "lo0" && address.ipv4 == Ipv4Addr::LOCALHOST && address.up
        }));
    }
}
