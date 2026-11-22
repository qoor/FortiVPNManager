#![cfg(windows)]
#![allow(unsafe_code)]

pub mod crypto;
pub mod library;
pub mod network;
pub mod process;

use std::fmt;
use std::io;

use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, ERROR_OBJECT_ALREADY_EXISTS, NO_ERROR, WIN32_ERROR,
};

/// A failed Windows API call: which call, and the Win32 error code it returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error {
    pub operation: &'static str,
    pub code: u32,
}

impl Error {
    /// The address or route to add is there already, which the caller checks rather than fails on.
    #[must_use]
    pub fn already_exists(self) -> bool {
        self.code == ERROR_OBJECT_ALREADY_EXISTS
    }

    #[must_use]
    pub fn not_found(self) -> bool {
        self.code == ERROR_NOT_FOUND || self.code == ERROR_FILE_NOT_FOUND
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} failed with Win32 error {}",
            self.operation, self.code
        )
    }
}

impl std::error::Error for Error {}

fn check(operation: &'static str, code: WIN32_ERROR) -> Result<(), Error> {
    if code == NO_ERROR {
        Ok(())
    } else {
        Err(Error { operation, code })
    }
}

fn last_error(operation: &'static str) -> Error {
    let code = io::Error::last_os_error()
        .raw_os_error()
        .expect("a failed call leaves an error code");
    Error {
        operation,
        code: code.cast_unsigned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_names_the_call_and_tells_an_existing_object() {
        let exists = Error {
            operation: "CreateIpForwardEntry2",
            code: ERROR_OBJECT_ALREADY_EXISTS,
        };
        assert!(exists.already_exists());
        assert_eq!(
            exists.to_string(),
            "CreateIpForwardEntry2 failed with Win32 error 5010"
        );
        let denied = Error {
            operation: "CreateIpForwardEntry2",
            code: 5,
        };
        assert!(!denied.already_exists());
    }
}
