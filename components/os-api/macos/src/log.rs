use std::ffi::{CString, c_char, c_void};

unsafe extern "C" {
    fn fvm_log_create(subsystem: *const c_char, category: *const c_char) -> *mut c_void;
    fn fvm_log_write(log: *mut c_void, kind: u8, message: *const c_char);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Debug,
    Info,
    Notice,
    Error,
    Fault,
}

pub struct Log(*mut c_void);

// SAFETY: an os_log_t may be used from any thread, and the system keeps it for the process.
unsafe impl Send for Log {}
// SAFETY: as above.
unsafe impl Sync for Log {}

impl Log {
    #[must_use]
    pub fn new(subsystem: &str, category: &str) -> Self {
        let subsystem = CString::new(subsystem).expect("a subsystem without NUL");
        let category = CString::new(category).expect("a category without NUL");
        // SAFETY: both are NUL-terminated strings that live through the call.
        Self(unsafe { fvm_log_create(subsystem.as_ptr(), category.as_ptr()) })
    }

    pub fn write(&self, level: Level, message: &str) {
        let kind = match level {
            Level::Debug => 0x02,
            Level::Info => 0x01,
            Level::Notice => 0x00,
            Level::Error => 0x10,
            Level::Fault => 0x11,
        };
        let message = message.split('\0').next().unwrap_or_default();
        let message = CString::new(message).expect("cut at the first NUL");
        // SAFETY: the handle came from os_log_create, and the message is NUL-terminated and lives
        // through the call.
        unsafe { fvm_log_write(self.0, kind, message.as_ptr()) }
    }
}
