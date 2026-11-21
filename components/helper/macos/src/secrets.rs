use fvm_helper_core::secrets::{SecretStore, StoredSecret};
use security_framework::base::Error;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, SearchResult};
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::passwords;
use zeroize::Zeroizing;

use crate::HELPER_ID;

const SYSTEM_KEYCHAIN: &str = "/Library/Keychains/System.keychain";
const ITEM_NOT_FOUND: i32 = -25_300;

pub(crate) struct Keychain;

impl SecretStore for Keychain {
    fn save(&self, name: &str, secret: &[u8]) -> Result<(), String> {
        passwords::set_generic_password(HELPER_ID, name, secret).map_err(describe)
    }

    fn load_all(&self) -> Result<Vec<StoredSecret>, String> {
        let system = system_keychain()?;
        let found = match items(&system)
            .load_attributes(true)
            .limit(Limit::All)
            .search()
        {
            Ok(found) => found,
            Err(error) if error.code() == ITEM_NOT_FOUND => return Ok(Vec::new()),
            Err(error) => return Err(describe(error)),
        };
        Ok(found
            .iter()
            .filter_map(|item| item.simplify_dict()?.remove("acct"))
            .map(|name| {
                let secret = read(&system, &name);
                (name, secret)
            })
            .collect())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        items(&system_keychain()?)
            .account(name)
            .delete()
            .map_err(describe)
    }
}

fn system_keychain() -> Result<SecKeychain, String> {
    SecKeychain::open(SYSTEM_KEYCHAIN).map_err(describe)
}

fn items(keychain: &SecKeychain) -> ItemSearchOptions {
    let mut options = ItemSearchOptions::new();
    options
        .class(ItemClass::generic_password())
        .service(HELPER_ID)
        .keychains(std::slice::from_ref(keychain));
    options
}

fn read(keychain: &SecKeychain, name: &str) -> Result<Zeroizing<Vec<u8>>, String> {
    let found = items(keychain)
        .account(name)
        .load_data(true)
        .search()
        .map_err(describe)?;
    match found.into_iter().next() {
        Some(SearchResult::Data(data)) => Ok(Zeroizing::new(data)),
        _ => Err("the keychain item has no data".to_owned()),
    }
}

fn describe(error: Error) -> String {
    format!("{error} (OSStatus {})", error.code())
}

#[cfg(test)]
mod tests {
    use std::process;

    use super::*;

    #[test]
    #[ignore = "needs root, whose default keychain is the System keychain"]
    fn the_system_keychain_keeps_a_secret_until_it_is_deleted() {
        let name = format!("fvm-test-{}", process::id());
        let named = |stored: Vec<StoredSecret>| -> Vec<Result<Vec<u8>, String>> {
            stored
                .into_iter()
                .filter(|(found, _)| *found == name)
                .map(|(_, secret)| secret.map(|bytes| bytes.to_vec()))
                .collect()
        };
        Keychain.save(&name, b"first").unwrap();
        Keychain.save(&name, b"second").unwrap();
        assert_eq!(
            named(Keychain.load_all().unwrap()),
            [Ok(b"second".to_vec())]
        );
        Keychain.delete(&name).unwrap();
        assert!(named(Keychain.load_all().unwrap()).is_empty());
        assert!(Keychain.delete(&name).is_err());
    }
}
