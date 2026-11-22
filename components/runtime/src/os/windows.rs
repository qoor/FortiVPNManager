use std::net::IpAddr;

use fvm_contract::app_internal::port::SecretStoreFailure;
use fvm_core::connection::InterfaceAddress;
use fvm_os_api_windows::network::{self, Family};
use fvm_os_api_windows::{Error, credentials};

pub(super) fn unknown_error(code: i32) -> String {
    format!("Unknown error (0x{code:x})")
}

fn failure(error: Error) -> SecretStoreFailure {
    SecretStoreFailure {
        description: fvm_os_api_windows::message(error.code),
        code: error.code.cast_signed(),
    }
}

fn target(service: &str, account: &str) -> String {
    format!("{service}/{account}")
}

pub(super) fn read_secret(
    service: &str,
    account: &str,
) -> Result<Option<String>, SecretStoreFailure> {
    credentials::read(&target(service, account)).map_err(failure)
}

pub(super) fn has_secret(service: &str, account: &str) -> Result<bool, SecretStoreFailure> {
    read_secret(service, account).map(|secret| secret.is_some())
}

pub(super) fn write_secret(
    service: &str,
    account: &str,
    value: &str,
) -> Result<(), SecretStoreFailure> {
    credentials::write(&target(service, account), account, value).map_err(failure)
}

pub(super) fn delete_secret(service: &str, account: &str) -> Result<(), SecretStoreFailure> {
    credentials::delete(&target(service, account)).map_err(failure)
}

pub(super) fn interface_addresses() -> Vec<InterfaceAddress> {
    let Ok(addresses) = network::addresses(Family::Ipv4) else {
        return Vec::new();
    };
    addresses
        .into_iter()
        .filter_map(|address| {
            let IpAddr::V4(ipv4) = address.address else {
                return None;
            };
            let interface = network::interface(address.interface).ok()?;
            Some(InterfaceAddress {
                name: interface.alias,
                up: interface.up,
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
    #[ignore = "Credential Manager is open only in a session the user signed in to"]
    fn a_secret_is_written_read_replaced_and_deleted() {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).unwrap();
        let service = "com.qoor.FortiVPNManager.gmail";
        let account = format!("fvm-test-{}@example.invalid", u64::from_ne_bytes(random));
        assert_eq!(has_secret(service, &account), Ok(false));
        assert_eq!(read_secret(service, &account), Ok(None));
        // Not ASCII, so the bytes must be UTF-8 both ways.
        let written = write_secret(service, &account, "p@ss-éß");
        let read = read_secret(service, &account);
        let exists = has_secret(service, &account);
        let replaced = write_secret(service, &account, "second");
        let read_again = read_secret(service, &account);
        let emptied = write_secret(service, &account, "");
        let read_empty = read_secret(service, &account);
        assert_eq!(delete_secret(service, &account), Ok(()));
        assert_eq!(written, Ok(()));
        assert_eq!(read, Ok(Some("p@ss-éß".into())));
        assert_eq!(exists, Ok(true));
        assert_eq!(replaced, Ok(()));
        assert_eq!(read_again, Ok(Some("second".into())));
        assert_eq!(emptied, Ok(()));
        assert_eq!(read_empty, Ok(Some(String::new())));
        assert_eq!(has_secret(service, &account), Ok(false));
        assert_eq!(delete_secret(service, &account), Ok(()));
    }

    #[test]
    fn an_error_is_described_as_windows_describes_it() {
        let failure = failure(Error {
            operation: "CredReadW",
            code: 5,
        });
        assert_eq!(failure.code, 5);
        let text = failure.description.unwrap();
        assert!(!text.is_empty() && text.trim_end() == text, "{text:?}");
    }

    #[test]
    fn the_loopback_interface_has_its_address_and_is_up() {
        assert!(
            interface_addresses()
                .iter()
                .any(|address| address.ipv4 == Ipv4Addr::LOCALHOST && address.up)
        );
    }
}
