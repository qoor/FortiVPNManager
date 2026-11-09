//! Reading and replacing the app's own small files: private to the user, written whole.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// The file's bytes, or `None` when there is none. Only a regular file of at most `limit` bytes is
/// read.
pub(crate) fn read_regular(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    Ok(Some(bytes))
}

/// Creates the directory and its missing parents, each private to the user.
pub(crate) fn make_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// Replaces `path` with `bytes` in one rename, so a reader sees the old file or the new one. The
/// file is private to the user.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let (temporary, mut file) = create_temporary(path)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    sync_directory(path);
    Ok(())
}

/// A new private file next to `path`, named so it never collides.
pub(crate) fn create_temporary(path: &Path) -> io::Result<(PathBuf, File)> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut random = [0u8; 6];
    for attempt in 0.. {
        getrandom::fill(&mut random).expect("random bytes from the OS");
        let suffix: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let temporary = directory.join(format!(".{name}.{suffix}"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&temporary) {
            Ok(file) => {
                set_private(&file)?;
                return Ok((temporary, file));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempt < 10 => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!("the loop returns")
}

/// 0600, whatever the umask made of the creation mode.
pub(crate) fn set_private(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

/// Makes the rename durable; a failure leaves the file written, so it is not reported.
fn sync_directory(path: &Path) {
    if let Some(directory) = path.parent()
        && let Ok(handle) = File::open(directory)
    {
        let _ = handle.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        std::env::temp_dir().join(format!("fvm-runtime-files-{name}"))
    }

    #[test]
    fn a_replaced_file_is_private_and_whole() {
        let directory = scratch();
        let path = directory.join("nested").join("state.json");
        make_directory(path.parent().unwrap()).unwrap();
        write_atomically(&path, b"first").unwrap();
        write_atomically(&path, b"second").unwrap();
        assert_eq!(
            read_regular(&path, 100).unwrap().as_deref(),
            Some(&b"second"[..])
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        assert_eq!(
            fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1,
            "no temporary file is left"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_missing_file_reads_as_none_and_a_large_one_fails() {
        let directory = scratch();
        make_directory(&directory).unwrap();
        assert_eq!(read_regular(&directory.join("missing"), 10).unwrap(), None);
        let large = directory.join("large");
        fs::write(&large, [0u8; 11]).unwrap();
        assert!(read_regular(&large, 10).is_err());
        assert!(
            read_regular(&directory, 10).is_err(),
            "a directory is not a regular file"
        );
        fs::remove_dir_all(directory).unwrap();
    }
}
