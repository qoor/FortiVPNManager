//! Which openfortivpn executable the Helper may run as root, and which leftover processes are ours.

use std::path::Path;

use fvm_contract::backend::{ClientExecutable, ClientProblem, ClientProblemKind, UntestedVersion};

use crate::homebrew::{self, inside_cellar};

const HOMEBREW_LINK: &str = "/opt/homebrew/bin/openfortivpn";
const CELLAR: &str = "/opt/homebrew/Cellar/openfortivpn/";
/// The pppd openfortivpn starts; it can outlive openfortivpn when the Helper dies mid-run.
const PPPD: &str = "/usr/sbin/pppd";
/// The release series whose output wording the adapter was checked against.
const TESTED_SERIES: &str = "1.24";

pub(super) fn locate() -> Result<ClientExecutable, ClientProblem> {
    let path = homebrew::resolve(HOMEBREW_LINK, CELLAR)?;
    let version = version(inside_cellar(&path, CELLAR).expect("resolved inside the Cellar"));
    Ok(ClientExecutable { path, version })
}

/// The English text the current app shows as it is.
pub(super) fn describe(problem: &ClientProblem) -> String {
    let path = problem.path.display();
    match problem.kind {
        ClientProblemKind::NotFound {} => {
            format!("openfortivpn not found at {path}; install it with: brew install openfortivpn")
        }
        ClientProblemKind::OutsideAllowedLocation {} => {
            format!("openfortivpn resolves outside the Homebrew Cellar ({CELLAR}): {path}")
        }
        ClientProblemKind::NotRegularFile {} => {
            format!("openfortivpn is not a regular file: {path}")
        }
        ClientProblemKind::WritableByOthers { mode } => format!(
            "openfortivpn is writable by other users (mode {mode:o}); refusing to run it: {path}"
        ),
        ClientProblemKind::NotExecutable {} => format!("openfortivpn is not executable: {path}"),
    }
}

pub(super) fn untested_version(version: &str) -> Option<UntestedVersion> {
    let tested = version
        .strip_prefix(TESTED_SERIES)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'));
    (!tested).then(|| UntestedVersion {
        tested_series: TESTED_SERIES.into(),
    })
}

pub(super) fn is_own_executable(path: &Path) -> bool {
    path == Path::new(PPPD) || inside_cellar(path, CELLAR).is_some()
}

/// The Cellar directory name without Homebrew's revision suffix: `1.24.1_1/bin/openfortivpn` gives
/// `1.24.1`.
fn version(in_cellar: &Path) -> String {
    let directory = in_cellar
        .iter()
        .next()
        .expect("a path inside the Cellar has a first component")
        .to_string_lossy();
    match directory.rsplit_once('_') {
        Some((version, revision))
            if !revision.is_empty() && revision.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            version.to_owned()
        }
        _ => directory.into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_drops_only_a_numeric_revision() {
        for (in_cellar, expected) in [
            ("1.24.1/bin/openfortivpn", "1.24.1"),
            ("1.24.1_1/bin/openfortivpn", "1.24.1"),
            ("1.24.1_12/bin/openfortivpn", "1.24.1"),
            ("1.24.1_/bin/openfortivpn", "1.24.1_"),
            ("1.24.1_rc/bin/openfortivpn", "1.24.1_rc"),
            ("HEAD-abc1234_2/bin/openfortivpn", "HEAD-abc1234"),
            ("openfortivpn", "openfortivpn"),
        ] {
            assert_eq!(version(Path::new(in_cellar)), expected, "{in_cellar}");
        }
    }
}
