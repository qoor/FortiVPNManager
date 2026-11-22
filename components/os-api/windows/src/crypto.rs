use std::{ptr, slice};

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

use crate::{Error, last_error};

pub fn protect(data: &[u8]) -> Result<Vec<u8>, Error> {
    let input = blob(data);
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` points to `data`, which the call only reads, and `output` is live for it; the
    // other pointers may be null.
    let done = unsafe {
        CryptProtectData(
            &raw const input,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &raw mut output,
        )
    };
    if done == 0 {
        return Err(last_error("CryptProtectData"));
    }
    Ok(take(output))
}

pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, Error> {
    let input = blob(data);
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: As in `protect`; the description pointer may be null too.
    let done = unsafe {
        CryptUnprotectData(
            &raw const input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &raw mut output,
        )
    };
    if done == 0 {
        return Err(last_error("CryptUnprotectData"));
    }
    Ok(take(output))
}

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).expect("a secret is far smaller than 4 GiB"),
        pbData: data.as_ptr().cast_mut(),
    }
}

fn take(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let length = output.cbData as usize;
    // SAFETY: DPAPI's buffer holds `cbData` bytes and is this function's alone.
    let bytes = unsafe { slice::from_raw_parts_mut(output.pbData, length) };
    let copy = bytes.to_vec();
    for byte in bytes.iter_mut() {
        // SAFETY: `byte` is a valid byte of the buffer.
        unsafe { ptr::write_volatile(byte, 0) };
    }
    // SAFETY: DPAPI allocated the buffer with LocalAlloc, and it is freed once, after its last use.
    unsafe { LocalFree(output.pbData.cast()) };
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_data_is_unprotected_to_the_same_bytes() {
        let secret = br#"{"cookie":"SVPNCOOKIE=Ab12"}"#;
        let protected = protect(secret).unwrap();
        assert!(
            !protected
                .windows(secret.len())
                .any(|window| window == secret),
            "the protected bytes hold the secret"
        );
        assert_eq!(unprotect(&protected).unwrap(), secret);
        let mut damaged = protected;
        let last = damaged.len() - 1;
        damaged[last] ^= 0xff;
        assert_eq!(
            unprotect(&damaged).unwrap_err().operation,
            "CryptUnprotectData"
        );
    }
}
