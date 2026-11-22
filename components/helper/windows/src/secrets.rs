use std::fs;
use std::io::Write;
use std::path::PathBuf;

use fvm_helper_core::recovery::create_private_file;
use fvm_helper_core::secrets::{SecretStore, StoredSecret};
use fvm_os_api_windows::crypto;
use zeroize::Zeroizing;

const PREFIX: &str = "session-";

pub struct SecretFiles {
    directory: PathBuf,
}

impl SecretFiles {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(format!("{PREFIX}{name}"))
    }
}

impl SecretStore for SecretFiles {
    fn save(&self, name: &str, secret: &[u8]) -> Result<(), String> {
        let protected = crypto::protect(secret).map_err(|error| error.to_string())?;
        let path = self.path(name);
        let mut file = create_private_file(&path)
            .map_err(|error| format!("open {}: {error}", path.display()))?;
        file.write_all(&protected).map_err(|error| {
            let _ = fs::remove_file(&path);
            format!("write {}: {error}", path.display())
        })
    }

    fn load_all(&self) -> Result<Vec<StoredSecret>, String> {
        let entries = fs::read_dir(&self.directory)
            .map_err(|error| format!("read {}: {error}", self.directory.display()))?;
        Ok(entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().to_str()?.strip_prefix(PREFIX)?.to_owned();
                let secret = fs::read(entry.path())
                    .map_err(|error| format!("read {}: {error}", entry.path().display()))
                    .and_then(|protected| {
                        crypto::unprotect(&protected)
                            .map(Zeroizing::new)
                            .map_err(|error| error.to_string())
                    });
                Some((name, secret))
            })
            .collect())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        let path = self.path(name);
        fs::remove_file(&path).map_err(|error| format!("delete {}: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::{env, process};

    use super::*;

    #[test]
    fn a_secret_is_kept_protected_until_it_is_deleted() {
        let directory = env::temp_dir().join(format!("fvm-secret-files-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("run.json"), "{}").unwrap();
        fs::write(directory.join("session-damaged"), "not protected").unwrap();
        let files = SecretFiles::new(directory.clone());
        let secret = br#"{"cookie":"SVPNCOOKIE=Ab12"}"#;
        files
            .save("E621E1F8-C36C-495A-93FC-0C247A3E6E5F", secret)
            .unwrap();
        let written =
            fs::read(directory.join("session-E621E1F8-C36C-495A-93FC-0C247A3E6E5F")).unwrap();
        assert!(!written.windows(secret.len()).any(|window| window == secret));
        assert!(
            files
                .save("E621E1F8-C36C-495A-93FC-0C247A3E6E5F", secret)
                .is_err()
        );

        let mut stored = files.load_all().unwrap();
        stored.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].0, "E621E1F8-C36C-495A-93FC-0C247A3E6E5F");
        assert_eq!(stored[0].1.as_deref().unwrap().as_slice(), secret);
        assert_eq!(stored[1].0, "damaged");
        assert!(stored[1].1.is_err());

        files
            .delete("E621E1F8-C36C-495A-93FC-0C247A3E6E5F")
            .unwrap();
        files.delete("damaged").unwrap();
        assert_eq!(files.load_all().unwrap(), []);
        assert!(files.delete("damaged").is_err());
        fs::remove_dir_all(&directory).unwrap();
    }
}
