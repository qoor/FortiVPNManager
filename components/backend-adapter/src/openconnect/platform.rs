//! What differs per OS for openconnect: where the Helper may run it from, the environment it and its
//! route script need, and the curl that sends the logout request. Everything else about openconnect
//! is the same on every OS.

#[cfg(any(windows, target_os = "macos"))]
pub(super) const INSTALL_HINT: &str = "it comes with FortiVPNManager, so reinstall FortiVPNManager";

#[cfg(windows)]
mod values {
    use std::fs;
    use std::os::windows::io::AsHandle;
    use std::path::{Path, PathBuf};
    use std::process::Child;
    use std::time::Duration;

    use fvm_common::os_api;
    use fvm_contract::backend::{ClientProblem, ClientProblemKind};

    /// The copy that ships in the app's folder; only administrators can change it.
    const CLIENT: &str = r"C:\Program Files\FortiVPNManager\OpenConnect\openconnect.exe";
    pub(in super::super) fn script() -> Option<(String, PathBuf)> {
        Some((
            r"C:\Program Files\FortiVPNManager\vpnc-values.js".to_owned(),
            PathBuf::from(r"C:\Windows\System32\cscript.exe"),
        ))
    }
    /// Windows' own curl, in System32, which only administrators can change.
    pub(in super::super) const CURL: &str = r"C:\Windows\System32\curl.exe";
    pub(in super::super) const NULL_DEVICE: &str = "NUL";
    /// The Windows variables the client, its route script (vpnc-values.js, run by cscript) and curl
    /// need, taken from the Helper's own environment. Proxy variables are left out.
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

    pub(in super::super) fn wait_for_exit(child: &Child, limit: Option<Duration>) -> bool {
        os_api::wait_for_exit(child.as_handle(), limit)
    }
}

#[cfg(target_os = "macos")]
mod values {
    use std::path::{Path, PathBuf};
    use std::process::Child;
    use std::time::Duration;

    use fvm_common::process_exit::ProcessExit;
    use fvm_contract::backend::ClientProblem;

    use super::super::output::GATEWAY_ROUTE_REQUESTED;
    use super::super::request::VALUES_VARIABLE;
    use crate::homebrew;

    const CLIENT: &str = "/Applications/FortiVPNManager.app/Contents/MacOS/openconnect";
    pub(in super::super) fn script() -> Option<(String, PathBuf)> {
        let file = format!("\"${VALUES_VARIABLE}\"");
        let draft = format!("\"${VALUES_VARIABLE}.tmp\"");
        let command = format!(
            "case \"$reason\" in connect) /usr/bin/env -0 > {draft} && /bin/mv -f {draft} {file};; attempt-reconnect) echo '{GATEWAY_ROUTE_REQUESTED}';; esac; exit 0"
        );
        Some((command, PathBuf::from("/bin/sh")))
    }
    pub(in super::super) const CURL: &str = "/usr/bin/curl";
    pub(in super::super) const NULL_DEVICE: &str = "/dev/null";
    pub(in super::super) const SYSTEM_VARIABLES: &[&str] = &["PATH"];

    pub(in super::super) fn executable() -> Result<PathBuf, ClientProblem> {
        homebrew::check_file(Path::new(CLIENT))?;
        Ok(PathBuf::from(CLIENT))
    }

    pub(in super::super) fn is_own_executable(path: &Path) -> bool {
        path == Path::new(CLIENT)
    }

    pub(in super::super) fn wait_for_exit(child: &Child, limit: Option<Duration>) -> bool {
        ProcessExit::watch(child.id()).is_none_or(|exit| exit.wait(limit))
    }
}

/// Linux gets its client location when openconnect is supported there. Until then the client is
/// never found, and the values below only let the OS-neutral parts be built and tested.
#[cfg(not(any(windows, target_os = "macos")))]
mod values {
    use std::path::{Path, PathBuf};
    use std::process::Child;
    use std::time::Duration;

    use fvm_contract::backend::{ClientProblem, ClientProblemKind};

    pub(in super::super) const INSTALL_HINT: &str = "openconnect is not supported on this OS yet";
    pub(in super::super) fn script() -> Option<(String, PathBuf)> {
        None
    }
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

    pub(in super::super) fn wait_for_exit(_: &Child, _: Option<Duration>) -> bool {
        unreachable!("openconnect is never found on this OS")
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(super) use values::INSTALL_HINT;
pub(super) use values::{
    CURL, NULL_DEVICE, SYSTEM_VARIABLES, executable, is_own_executable, script, wait_for_exit,
};
