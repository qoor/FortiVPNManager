use std::ptr;

use widestring::U16CString;
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_APPLICATION_DIR, LoadLibraryExW,
};

use crate::{Error, last_error};

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
