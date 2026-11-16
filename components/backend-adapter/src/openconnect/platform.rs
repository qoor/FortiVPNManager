//! What differs per OS for openconnect: where the Helper may run it from, the environment it and its
//! route script need, and the curl that sends the logout request. Everything else about openconnect
//! is the same on every OS.

#[cfg(any(windows, target_os = "macos"))]
pub(super) const INSTALL_HINT: &str = "it comes with FortiVPNManager, so reinstall FortiVPNManager";

#[cfg(windows)]
mod values {
    use std::fs;
    use std::path::{Path, PathBuf};

    use fvm_contract::backend::{ClientProblem, ClientProblemKind};

    /// The copy that ships in the app's folder; only administrators can change it.
    const CLIENT: &str = r"C:\Program Files\FortiVPNManager\OpenConnect\openconnect.exe";
    pub(in super::super) const ROUTE_SCRIPT: Option<&str> = None;
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

    pub(in super::super) fn executable() -> Result<PathBuf, ClientProblem> {
        let path = PathBuf::from(CLIENT);
        let problem = |kind| ClientProblem {
            kind,
            path: path.clone(),
        };
        // Not followed: a link swapped in would point the Helper at something else.
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| problem(ClientProblemKind::NotFound {}))?;
        if !metadata.is_file() {
            return Err(problem(ClientProblemKind::NotRegularFile {}));
        }
        Ok(path)
    }

    /// Windows paths ignore case.
    pub(in super::super) fn is_own_executable(path: &Path) -> bool {
        path.to_str()
            .is_some_and(|path| path.eq_ignore_ascii_case(CLIENT))
    }
}

#[cfg(target_os = "macos")]
mod values {
    use std::path::{Path, PathBuf};

    use fvm_contract::backend::ClientProblem;

    use crate::homebrew;

    const CLIENT: &str = "/Applications/FortiVPNManager.app/Contents/MacOS/openconnect";
    pub(in super::super) const ROUTE_SCRIPT: Option<&str> =
        Some("/Applications/FortiVPNManager.app/Contents/Resources/vpnc-script");
    pub(in super::super) const CURL: &str = "/usr/bin/curl";
    pub(in super::super) const NULL_DEVICE: &str = "/dev/null";
    pub(in super::super) const SYSTEM_VARIABLES: &[&str] = &["PATH"];

    pub(in super::super) fn executable() -> Result<PathBuf, ClientProblem> {
        homebrew::check_file(Path::new(CLIENT))?;
        if let Some(script) = ROUTE_SCRIPT {
            homebrew::check_file(Path::new(script))?;
        }
        Ok(PathBuf::from(CLIENT))
    }

    pub(in super::super) fn is_own_executable(path: &Path) -> bool {
        path == Path::new(CLIENT)
    }
}

/// Linux gets its client location when openconnect is supported there. Until then the client is
/// never found, and the values below only let the OS-neutral parts be built and tested.
#[cfg(not(any(windows, target_os = "macos")))]
mod values {
    use std::path::{Path, PathBuf};

    use fvm_contract::backend::{ClientProblem, ClientProblemKind};

    pub(in super::super) const INSTALL_HINT: &str = "openconnect is not supported on this OS yet";
    pub(in super::super) const ROUTE_SCRIPT: Option<&str> = None;
    pub(in super::super) const CURL: &str = "/usr/bin/curl";
    pub(in super::super) const NULL_DEVICE: &str = "/dev/null";
    pub(in super::super) const SYSTEM_VARIABLES: &[&str] = &["PATH"];

    pub(in super::super) fn executable() -> Result<PathBuf, ClientProblem> {
        Err(ClientProblem {
            kind: ClientProblemKind::NotFound {},
            path: PathBuf::from("openconnect"),
        })
    }

    pub(in super::super) fn is_own_executable(_: &Path) -> bool {
        false
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(super) use values::INSTALL_HINT;
pub(super) use values::{
    CURL, NULL_DEVICE, ROUTE_SCRIPT, SYSTEM_VARIABLES, executable, is_own_executable,
};
