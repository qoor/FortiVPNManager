use fvm_contract::app_internal::port::SecretStoreFailure;
use fvm_core::connection::InterfaceAddress;
use nix::ifaddrs::getifaddrs;
use nix::net::if_::InterfaceFlags;
use security_framework::base::Error;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
use security_framework::passwords::{self, PasswordOptions};
use security_framework_sys::base::errSecItemNotFound;

pub(super) fn unknown_error(code: i32) -> String {
    format!("Keychain error {code}")
}

fn failure(error: Error) -> SecretStoreFailure {
    SecretStoreFailure {
        description: error.message(),
        code: error.code(),
    }
}

pub(super) fn read_secret(
    service: &str,
    account: &str,
) -> Result<Option<String>, SecretStoreFailure> {
    match passwords::generic_password(PasswordOptions::new_generic_password(service, account)) {
        Ok(data) => Ok(String::from_utf8(data).ok()),
        Err(error) if error.code() == errSecItemNotFound => Ok(None),
        Err(error) => Err(failure(error)),
    }
}

pub(super) fn has_secret(service: &str, account: &str) -> Result<bool, SecretStoreFailure> {
    let search = ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
        .account(account)
        .load_attributes(true)
        .limit(Limit::Max(1))
        .search();
    match search {
        Ok(_) => Ok(true),
        Err(error) if error.code() == errSecItemNotFound => Ok(false),
        Err(error) => Err(failure(error)),
    }
}

pub(super) fn write_secret(
    service: &str,
    account: &str,
    value: &str,
) -> Result<(), SecretStoreFailure> {
    passwords::set_generic_password(service, account, value.as_bytes()).map_err(failure)
}

pub(super) fn delete_secret(service: &str, account: &str) -> Result<(), SecretStoreFailure> {
    match passwords::delete_generic_password(service, account) {
        Err(error) if error.code() != errSecItemNotFound => Err(failure(error)),
        _ => Ok(()),
    }
}

pub(super) fn interface_addresses() -> Vec<InterfaceAddress> {
    let Ok(entries) = getifaddrs() else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let ipv4 = entry.address.as_ref()?.as_sockaddr_in()?.ip();
            Some(InterfaceAddress {
                up: entry.flags.contains(InterfaceFlags::IFF_UP),
                name: entry.interface_name,
                ipv4,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;

    #[test]
    #[ignore = "writes the login keychain of the user running the tests"]
    fn a_secret_is_written_read_replaced_and_deleted() {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let service = "com.qoor.FortiVPNManager.test";
        let account = format!("fvm-test-{}@example.invalid", u64::from_ne_bytes(random));
        assert_eq!(has_secret(service, &account), Ok(false));
        assert_eq!(read_secret(service, &account), Ok(None));
        let written = write_secret(service, &account, "p@ss-éß");
        let read = read_secret(service, &account);
        let exists = has_secret(service, &account);
        let replaced = write_secret(service, &account, "second");
        let read_again = read_secret(service, &account);
        assert_eq!(delete_secret(service, &account), Ok(()));
        assert_eq!(written, Ok(()));
        assert_eq!(read, Ok(Some("p@ss-éß".into())));
        assert_eq!(exists, Ok(true));
        assert_eq!(replaced, Ok(()));
        assert_eq!(read_again, Ok(Some("second".into())));
        assert_eq!(has_secret(service, &account), Ok(false));
        assert_eq!(delete_secret(service, &account), Ok(()));
    }

    #[test]
    fn the_loopback_interface_has_its_address_and_is_up() {
        assert!(interface_addresses().iter().any(|address| {
            address.name == "lo0" && address.ipv4 == Ipv4Addr::LOCALHOST && address.up
        }));
    }
}
