//! What differs per OS for openconnect: where the Helper may run it from, the environment it and its
//! route script need, and the curl that sends the logout request. Everything else about openconnect
//! is the same on every OS. Only Windows is supported so far.

#[cfg(windows)]
mod values {
    /// Where the OpenConnect installer puts it; only administrators can change it.
    pub(in super::super) const CLIENT: Option<&str> =
        Some(r"C:\Program Files\OpenConnect\openconnect.exe");
    /// Windows' own curl, in System32, which only administrators can change.
    pub(in super::super) const CURL: &str = r"C:\Windows\System32\curl.exe";
    pub(in super::super) const NULL_DEVICE: &str = "NUL";
    /// The Windows variables the client, its route script (vpnc-script-win.js, run by cscript and
    /// cmd) and curl need, taken from the Helper's own environment. Proxy variables are left out.
    pub(in super::super) const SYSTEM_VARIABLES: &[&str] = &[
        "SystemRoot",
        "SystemDrive",
        "windir",
        "ComSpec",
        "PATH",
        "PATHEXT",
        "TEMP",
        "TMP",
    ];
}

/// macOS and Linux get their client location, and the line that shows the tunnel's interface
/// exists, when openconnect is supported there. Until then the client is never found, and the
/// values below only let the OS-neutral parts be built and tested.
#[cfg(not(windows))]
mod values {
    pub(in super::super) const CLIENT: Option<&str> = None;
    pub(in super::super) const CURL: &str = "/usr/bin/curl";
    pub(in super::super) const NULL_DEVICE: &str = "/dev/null";
    pub(in super::super) const SYSTEM_VARIABLES: &[&str] = &["PATH"];
}

pub(super) use values::{CLIENT, CURL, NULL_DEVICE, SYSTEM_VARIABLES};
