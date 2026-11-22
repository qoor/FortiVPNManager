use std::ptr;

use widestring::U16CString;
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LoadLibraryExW,
};

use crate::{Error, last_error};

/// Loads the library `name` from the application's directory and keeps it loaded for the life of
/// the process.
///
/// # Errors
///
/// Returns the Windows error code when the library cannot be loaded.
///
/// # Panics
///
/// Panics if `name` holds a NUL character.
pub fn keep_library_loaded(name: &str) -> Result<(), Error> {
    let name = U16CString::from_str(name).expect("a library name has no NUL");
    // SAFETY: `name` is NUL-terminated and live for the call, which only reads it; the reserved
    // file handle is null.
    let module = unsafe {
        LoadLibraryExW(
            name.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_APPLICATION_DIR,
        )
    };
    if module.is_null() {
        return Err(last_error("LoadLibraryExW"));
    }
    Ok(())
}
