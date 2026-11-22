#![cfg(windows)]
#![allow(unsafe_code)]

pub mod credentials;
pub mod crypto;
pub mod library;
pub mod network;
pub mod process;

use std::{fmt, io, ptr, slice};

use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, ERROR_OBJECT_ALREADY_EXISTS, LocalFree, NO_ERROR,
    WIN32_ERROR,
};
use windows_sys::Win32::System::Diagnostics::Debug::{
    FORMAT_MESSAGE_ALLOCATE_BUFFER, FORMAT_MESSAGE_ARGUMENT_ARRAY, FORMAT_MESSAGE_FROM_SYSTEM,
    FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
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

/// Windows's description of the error `code` in the user's language, trimmed as .NET's
/// `Win32Exception` trims it; `None` when Windows has none.
#[must_use]
pub fn message(code: u32) -> Option<String> {
    let mut buffer: *mut u16 = ptr::null_mut();
    // SAFETY: With ALLOCATE_BUFFER, the buffer argument is the address of `buffer`, which the call
    // points to a string it allocates of the length it returns.
    let length = unsafe {
        FormatMessageW(
            FORMAT_MESSAGE_ALLOCATE_BUFFER
                | FORMAT_MESSAGE_FROM_SYSTEM
                | FORMAT_MESSAGE_IGNORE_INSERTS
                | FORMAT_MESSAGE_ARGUMENT_ARRAY,
            ptr::null(),
            code,
            0,
            (&raw mut buffer).cast(),
            0,
            ptr::null(),
        )
    };
    if length == 0 {
        return None;
    }
    // SAFETY: After a success, `buffer` holds `length` UTF-16 units and is freed once, after they
    // were copied.
    let text = unsafe {
        let text = String::from_utf16_lossy(slice::from_raw_parts(buffer, length as usize));
        LocalFree(buffer.cast());
        text
    };
    Some(text.trim_end_matches(|c: char| c <= ' ').to_owned())
}

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
