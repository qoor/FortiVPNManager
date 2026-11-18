use zeroize::Zeroizing;

pub type StoredSecret = (String, Result<Zeroizing<Vec<u8>>, String>);

pub trait SecretStore: Send + Sync {
    fn save(&self, name: &str, secret: &[u8]) -> Result<(), String>;

    fn load_all(&self) -> Result<Vec<StoredSecret>, String>;

    fn delete(&self, name: &str) -> Result<(), String>;
}
