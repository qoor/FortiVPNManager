use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;
use std::sync::mpsc::Sender;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW, NotifyUnicastIpAddressChange,
};
use windows_sys::Win32::Networking::WinSock::AF_UNSPEC;

use crate::{Error, check};

static CHANGED: OnceLock<Sender<()>> = OnceLock::new();

extern "system" fn address_changed(
    _: *const c_void,
    _: *const MIB_UNICASTIPADDRESS_ROW,
    _: MIB_NOTIFICATION_TYPE,
) {
    if let Some(changed) = CHANGED.get() {
        let _ = changed.send(());
    }
}

/// Sends to `changed` whenever an address of an interface is added, changed or removed, for as long
/// as the process runs.
///
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
    check("NotifyUnicastIpAddressChange", code)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn windows_reports_the_address_changes() {
        let (changed, _signals) = mpsc::channel();
        assert_eq!(watch(changed), Ok(()));
    }
}
