use std::fs::{self, Metadata};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use fvm_contract::backend::{ClientProblem, ClientProblemKind};

pub(crate) fn check_file(path: &Path) -> Result<(), ClientProblem> {
    let problem = |kind| ClientProblem {
        kind,
        path: path.to_owned(),
    };
    match fs::symlink_metadata(path) {
        Ok(metadata) => file_problem(&metadata).map_or(Ok(()), |kind| Err(problem(kind))),
        Err(_) => Err(problem(ClientProblemKind::NotFound {})),
    }
}

fn file_problem(metadata: &Metadata) -> Option<ClientProblemKind> {
    let mode = metadata.permissions().mode() & 0o7777;
    if !metadata.is_file() {
        Some(ClientProblemKind::NotRegularFile {})
    } else if mode & 0o022 != 0 {
        Some(ClientProblemKind::WritableByOthers { mode })
    } else if mode & 0o111 == 0 {
        Some(ClientProblemKind::NotExecutable {})
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::{env, process};

    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                env::temp_dir().join(format!("fvm-backend-adapter-{}-{name}", process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn file_with_mode(&self, name: &str, mode: u32) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, b"#!/bin/sh\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn kind(path: &Path) -> Option<ClientProblemKind> {
        check_file(path).err().map(|problem| {
            assert_eq!(problem.path, path);
            problem.kind
        })
    }

    #[test]
    fn a_regular_file_runs_if_executable_and_writable_only_by_its_owner() {
        let scratch = Scratch::new("modes");
        let problem = |mode: u32| kind(&scratch.file_with_mode(&format!("{mode:o}"), mode));
        for mode in [0o755, 0o555, 0o700, 0o100, 0o4755] {
            assert_eq!(problem(mode), None, "{mode:o}");
        }
        for mode in [0o775, 0o757, 0o777, 0o620, 0o602] {
            assert_eq!(
                problem(mode),
                Some(ClientProblemKind::WritableByOthers { mode }),
                "{mode:o}"
            );
        }
        for mode in [0o644, 0o600, 0o444] {
            assert_eq!(
                problem(mode),
                Some(ClientProblemKind::NotExecutable {}),
                "{mode:o}"
            );
        }
    }

    #[test]
    fn a_directory_or_a_symlink_is_not_a_regular_file_and_nothing_is_not_found() {
        let scratch = Scratch::new("kinds");
        assert_eq!(kind(&scratch.0), Some(ClientProblemKind::NotRegularFile {}));
        let target = scratch.file_with_mode("target", 0o755);
        let link = scratch.0.join("link");
        symlink(&target, &link).unwrap();
        assert_eq!(kind(&link), Some(ClientProblemKind::NotRegularFile {}));
        assert_eq!(
            kind(&scratch.0.join("missing")),
            Some(ClientProblemKind::NotFound {})
        );
    }
}
