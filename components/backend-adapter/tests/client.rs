//! The client's name, executable problems, stop grace period and leftover executables.
#![cfg(unix)] // openfortivpn runs only on Unix.

use std::path::{Path, PathBuf};
use std::time::Duration;

use fvm_backend_adapter::openfortivpn::OpenFortiVpn;
use fvm_contract::backend::{Backend, ClientProblem, ClientProblemKind, UntestedVersion};

const RESOLVED: &str = "/opt/homebrew/Cellar/openfortivpn/1.24.1/bin/openfortivpn";

fn describe(kind: ClientProblemKind, path: &str) -> String {
    OpenFortiVpn.describe_problem(&ClientProblem {
        kind,
        path: PathBuf::from(path),
    })
}

#[test]
fn versions_outside_the_tested_series_are_untested() {
    for tested in ["1.24", "1.24.1", "1.24.10"] {
        assert_eq!(OpenFortiVpn.untested_version(tested), None, "{tested}");
    }
    for untested in ["1.25.0", "1.240.1", "1.2", "2.0.0"] {
        assert_eq!(
            OpenFortiVpn.untested_version(untested),
            Some(UntestedVersion {
                tested_series: "1.24".into()
            }),
            "{untested}"
        );
    }
}

#[test]
fn is_named_openfortivpn() {
    assert_eq!(OpenFortiVpn.client_name(), "openfortivpn");
}

#[test]
fn describes_problems_as_the_current_app_shows_them() {
    assert_eq!(
        describe(
            ClientProblemKind::NotFound {},
            "/opt/homebrew/bin/openfortivpn"
        ),
        "openfortivpn not found at /opt/homebrew/bin/openfortivpn; install it with: brew install openfortivpn"
    );
    assert_eq!(
        describe(
            ClientProblemKind::OutsideAllowedLocation {},
            "/usr/local/bin/openfortivpn"
        ),
        "openfortivpn resolves outside the Homebrew Cellar (/opt/homebrew/Cellar/openfortivpn/): /usr/local/bin/openfortivpn"
    );
    assert_eq!(
        describe(ClientProblemKind::NotRegularFile {}, RESOLVED),
        format!("openfortivpn is not a regular file: {RESOLVED}")
    );
    assert_eq!(
        describe(
            ClientProblemKind::WritableByOthers { mode: 0o775 },
            RESOLVED
        ),
        format!(
            "openfortivpn is writable by other users (mode 775); refusing to run it: {RESOLVED}"
        )
    );
    assert_eq!(
        describe(
            ClientProblemKind::WritableByOthers { mode: 0o4757 },
            RESOLVED
        ),
        format!(
            "openfortivpn is writable by other users (mode 4757); refusing to run it: {RESOLVED}"
        )
    );
    assert_eq!(
        describe(ClientProblemKind::NotExecutable {}, RESOLVED),
        format!("openfortivpn is not executable: {RESOLVED}")
    );
}

#[test]
fn gives_the_client_ten_seconds_to_stop() {
    assert_eq!(OpenFortiVpn.stop_grace_period(), Duration::from_secs(10));
}

#[test]
fn owns_openfortivpn_from_the_cellar_and_apple_pppd() {
    for path in [
        RESOLVED,
        "/opt/homebrew/Cellar/openfortivpn/1.24.1_1/bin/openfortivpn",
        "/opt/homebrew/Cellar/openfortivpn/1.23.0/bin/openfortivpn",
        "/usr/sbin/pppd",
    ] {
        assert!(OpenFortiVpn.is_own_executable(Path::new(path)), "{path}");
    }
    for path in [
        "/opt/homebrew/bin/openfortivpn",
        "/opt/homebrew/Cellar/openfortivpn",
        "/opt/homebrew/Cellar/openfortivpn-other/1.0/bin/openfortivpn",
        "/opt/homebrew/Cellar/openconnect/9.12/bin/openconnect",
        "/usr/local/bin/openfortivpn",
        "/usr/local/sbin/pppd",
        "/usr/sbin/pppd2",
        "/sbin/route",
        "/bin/sh",
    ] {
        assert!(!OpenFortiVpn.is_own_executable(Path::new(path)), "{path}");
    }
}
