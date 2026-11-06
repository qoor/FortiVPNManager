//! Which openfortivpn executable the Helper may run as root, and which leftover processes are ours.

use std::fs::{self, Metadata};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use fvm_contract::backend::{ClientExecutable, ClientProblem, ClientProblemKind};

const HOMEBREW_LINK: &str = "/opt/homebrew/bin/openfortivpn";
/// Only a file inside it is run, so the link cannot point root at something else.
const CELLAR: &str = "/opt/homebrew/Cellar/openfortivpn/";
/// The pppd openfortivpn starts; it can outlive openfortivpn when the Helper dies mid-run.
const PPPD: &str = "/usr/sbin/pppd";

pub(super) fn locate() -> Result<ClientExecutable, ClientProblem> {
    let resolved = fs::canonicalize(HOMEBREW_LINK).map_err(|_| ClientProblem {
        kind: ClientProblemKind::NotFound,
        path: HOMEBREW_LINK.into(),
    })?;
    let problem = |kind| ClientProblem {
        kind,
        path: resolved.clone(),
    };
    let in_cellar = inside_cellar(&resolved)
        .ok_or_else(|| problem(ClientProblemKind::OutsideAllowedLocation))?;
    let version = version(in_cellar);
    // lstat: a symlink swapped in after resolving is refused rather than followed.
    let metadata =
        fs::symlink_metadata(&resolved).map_err(|_| problem(ClientProblemKind::NotRegularFile))?;
    if let Some(kind) = file_problem(&metadata) {
        return Err(problem(kind));
    }
    Ok(ClientExecutable {
        path: resolved,
        version: Some(version),
    })
}

/// The English text the current app shows as it is.
pub(super) fn describe(problem: &ClientProblem) -> String {
    let path = problem.path.display();
    match problem.kind {
        ClientProblemKind::NotFound => {
            format!("openfortivpn not found at {path}; install it with: brew install openfortivpn")
        }
        ClientProblemKind::OutsideAllowedLocation => {
            format!("openfortivpn resolves outside the Homebrew Cellar ({CELLAR}): {path}")
        }
        ClientProblemKind::NotRegularFile => format!("openfortivpn is not a regular file: {path}"),
        ClientProblemKind::WritableByOthers { mode } => format!(
            "openfortivpn is writable by other users (mode {mode:o}); refusing to run it: {path}"
        ),
        ClientProblemKind::NotExecutable => format!("openfortivpn is not executable: {path}"),
    }
}

pub(super) fn is_own_executable(path: &Path) -> bool {
    path == Path::new(PPPD) || inside_cellar(path).is_some()
}

/// The part of `path` below the Cellar directory, if `path` lies inside it.
fn inside_cellar(path: &Path) -> Option<&Path> {
    path.strip_prefix(CELLAR)
        .ok()
        .filter(|rest| !rest.as_os_str().is_empty())
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

/// Why the resolved file must not run as root, if it must not.
fn file_problem(metadata: &Metadata) -> Option<ClientProblemKind> {
    let mode = metadata.permissions().mode() & 0o7777;
    if !metadata.is_file() {
        Some(ClientProblemKind::NotRegularFile)
    } else if mode & 0o022 != 0 {
        Some(ClientProblemKind::WritableByOthers { mode })
    } else if mode & 0o111 == 0 {
        Some(ClientProblemKind::NotExecutable)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::{env, process};

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

    #[test]
    fn only_paths_below_the_cellar_are_inside_it() {
        assert_eq!(
            inside_cellar(Path::new(
                "/opt/homebrew/Cellar/openfortivpn/1.24.1/bin/openfortivpn"
            )),
            Some(Path::new("1.24.1/bin/openfortivpn"))
        );
        for outside in [
            "/opt/homebrew/Cellar/openfortivpn",
            "/opt/homebrew/Cellar/openfortivpn-evil/1.0/bin/openfortivpn",
            "/opt/homebrew/Cellar/other/1.0/bin/openfortivpn",
            "/usr/local/bin/openfortivpn",
            "opt/homebrew/Cellar/openfortivpn/1.24.1/bin/openfortivpn",
        ] {
            assert_eq!(inside_cellar(Path::new(outside)), None, "{outside}");
        }
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                env::temp_dir().join(format!("fvm-backend-adapter-{}-{name}", process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn file_with_mode(&self, mode: u32) -> Metadata {
            let path = self.0.join(format!("{mode:o}"));
            fs::write(&path, b"#!/bin/sh\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            fs::symlink_metadata(&path).unwrap()
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_regular_file_runs_if_executable_and_writable_only_by_its_owner() {
        let scratch = Scratch::new("modes");
        for mode in [0o755, 0o555, 0o700, 0o100, 0o4755] {
            assert_eq!(
                file_problem(&scratch.file_with_mode(mode)),
                None,
                "{mode:o}"
            );
        }
        for mode in [0o775, 0o757, 0o777, 0o620, 0o602] {
            assert_eq!(
                file_problem(&scratch.file_with_mode(mode)),
                Some(ClientProblemKind::WritableByOthers { mode }),
                "{mode:o}"
            );
        }
        for mode in [0o644, 0o600, 0o444] {
            assert_eq!(
                file_problem(&scratch.file_with_mode(mode)),
                Some(ClientProblemKind::NotExecutable),
                "{mode:o}"
            );
        }
    }

    #[test]
    fn a_directory_is_not_a_regular_file() {
        let scratch = Scratch::new("directory");
        let metadata = fs::symlink_metadata(&scratch.0).unwrap();
        assert_eq!(
            file_problem(&metadata),
            Some(ClientProblemKind::NotRegularFile)
        );
    }

    #[test]
    fn a_symlink_is_not_a_regular_file() {
        let scratch = Scratch::new("symlink");
        let target = scratch.0.join("target");
        fs::write(&target, b"").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let link = scratch.0.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(
            file_problem(&fs::symlink_metadata(&link).unwrap()),
            Some(ClientProblemKind::NotRegularFile)
        );
    }
}
