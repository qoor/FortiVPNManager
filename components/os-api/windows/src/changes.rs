use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;
use std::sync::mpsc::Sender;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    MIB_IPFORWARD_ROW2, MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW, NotifyRouteChange2,
    NotifyUnicastIpAddressChange,
};
use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

use crate::{Error, check};

static CHANGED: OnceLock<Sender<()>> = OnceLock::new();

fn signal() {
    if let Some(changed) = CHANGED.get() {
        let _ = changed.send(());
    }
}

extern "system" fn address_changed(
    _: *const c_void,
    _: *const MIB_UNICASTIPADDRESS_ROW,
    _: MIB_NOTIFICATION_TYPE,
) {
    signal();
}

extern "system" fn route_changed(
    _: *const c_void,
    _: *const MIB_IPFORWARD_ROW2,
    _: MIB_NOTIFICATION_TYPE,
) {
    signal();
}

/// # Errors
///
/// Returns the Windows error code when Windows cannot report the changes.
///
/// # Panics
///
/// Panics if called more than once.
pub fn watch(changed: Sender<()>) -> Result<(), Error> {
    assert!(CHANGED.set(changed).is_ok(), "the changes are watched once");
    let mut address_handle: HANDLE = ptr::null_mut();
    // SAFETY: The callback reads none of its arguments; the registration `address_handle` receives
    // is never cancelled.
    let code = unsafe {
        NotifyUnicastIpAddressChange(
            AF_UNSPEC,
            Some(address_changed),
            ptr::null(),
            false,
            &raw mut address_handle,
        )
    };
    check("NotifyUnicastIpAddressChange", code)?;
    let mut route_handle: HANDLE = ptr::null_mut();
    // SAFETY: As above, for the route callback and `route_handle`.
    let code = unsafe {
        NotifyRouteChange2(
            AF_UNSPEC,
            Some(route_changed),
            ptr::null(),
            false,
            &raw mut route_handle,
        )
    };
    check("NotifyRouteChange2", code)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::network::{self, Route};

    #[test]
    fn windows_reports_a_route_that_comes_and_goes() {
        let (changed, signals) = mpsc::channel();
        assert_eq!(watch(changed), Ok(()));
        while signals.try_recv().is_ok() {}
        let (default, _) = network::best_route(IpAddr::V4(Ipv4Addr::UNSPECIFIED)).unwrap();
        let route = Route {
            interface: default.interface,
            destination: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 77)),
            prefix_length: 32,
            next_hop: default.next_hop,
            metric: 256,
        };
        network::add_route(&route).unwrap();
        let added = signals.recv_timeout(Duration::from_secs(5));
        while signals.try_recv().is_ok() {}
        network::delete_route(&route).unwrap();
        let removed = signals.recv_timeout(Duration::from_secs(5));
        assert_eq!(added, Ok(()));
        assert_eq!(removed, Ok(()));
    }
}
