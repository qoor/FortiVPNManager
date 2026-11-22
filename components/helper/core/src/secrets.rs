use zeroize::Zeroizing;

pub type StoredSecret = (String, Result<Zeroizing<Vec<u8>>, String>);

pub trait SecretStore: Send + Sync {
    /// Saves `secret` under `name`, replacing a secret of that name.
    ///
    /// # Errors
    ///
    /// Returns the store's reason when the secret cannot be saved.
    fn save(&self, name: &str, secret: &[u8]) -> Result<(), String>;

    /// Every stored secret with its name; a secret that cannot be read carries the reason.
    ///
    /// # Errors
    ///
    /// Returns the store's reason when the secrets cannot be listed.
    fn load_all(&self) -> Result<Vec<StoredSecret>, String>;

    /// Deletes the secret named `name`.
    ///
    /// # Errors
    ///
    /// Returns the store's reason when the secret cannot be deleted.
    fn delete(&self, name: &str) -> Result<(), String>;
}
