use std::{ptr, slice};

use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
use windows_sys::Win32::Security::Credentials::{
    CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
    CredWriteW,
};
use zeroize::Zeroizing;

use crate::{Error, last_error};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The secret of the user's generic credential `target`, decoded as UTF-8 with invalid bytes
/// replaced; `None` when there is no such credential.
///
/// # Errors
///
/// Returns the Windows error code when Credential Manager fails, such as in a session the user did
/// not sign in to.
pub fn read(target: &str) -> Result<Option<String>, Error> {
    let target = wide(target);
    let mut found: *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: `target` is NUL-terminated and live for the call, which points `found` to a
    // credential it allocates.
    if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &raw mut found) } == 0 {
        let error = last_error("CredReadW");
        return if error.code == ERROR_NOT_FOUND {
            Ok(None)
        } else {
            Err(error)
        };
    }
    // SAFETY: After a success, `found` points to a credential whose blob holds `CredentialBlobSize`
    // bytes, none for an empty secret, and stays valid until it is freed once, after the bytes were
    // copied.
    let bytes = unsafe {
        let credential = &*found;
        let size = credential.CredentialBlobSize as usize;
        let bytes = Zeroizing::new(if size == 0 {
            Vec::new()
        } else {
            slice::from_raw_parts(credential.CredentialBlob, size).to_vec()
        });
        CredFree(found.cast());
        bytes
    };
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Saves `secret` as the user's generic credential `target`, for `user_name`, on this PC only,
/// replacing the one there.
///
/// # Errors
///
/// Returns the Windows error code when Credential Manager fails, such as in a session the user did
/// not sign in to.
///
/// # Panics
///
/// Panics if `secret` is 4 GiB or longer.
pub fn write(target: &str, user_name: &str, secret: &str) -> Result<(), Error> {
    let mut target = wide(target);
    let mut user_name = wide(user_name);
    let mut blob = Zeroizing::new(secret.as_bytes().to_vec());
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: u32::try_from(blob.len()).expect("a secret is shorter than 4 GiB"),
        CredentialBlob: if blob.is_empty() {
            ptr::null_mut()
        } else {
            blob.as_mut_ptr()
        },
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: user_name.as_mut_ptr(),
        ..CREDENTIALW::default()
    };
    // SAFETY: `credential` and the strings and bytes it points to are live for the call, which only
    // reads them.
    if unsafe { CredWriteW(&raw const credential, 0) } == 0 {
        return Err(last_error("CredWriteW"));
    }
    Ok(())
}

/// Deletes the user's generic credential `target`; deleting a missing one succeeds.
///
/// # Errors
///
/// Returns the Windows error code when Credential Manager fails, such as in a session the user did
/// not sign in to.
pub fn delete(target: &str) -> Result<(), Error> {
    let target = wide(target);
    // SAFETY: `target` is NUL-terminated and live for the call, which only reads it.
    if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
        let error = last_error("CredDeleteW");
        if error.code != ERROR_NOT_FOUND {
            return Err(error);
        }
    }
    Ok(())
}
