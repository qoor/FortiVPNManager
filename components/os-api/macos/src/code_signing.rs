use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use security_framework_sys::base::errSecSuccess;
use security_framework_sys::code_signing::{SecCodeCopySelf, SecCodeRef, SecStaticCodeRef};

const SIGNING_INFORMATION: u32 = 1 << 1;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecCodeCopyStaticCode(
        code: SecCodeRef,
        flags: u32,
        static_code: *mut SecStaticCodeRef,
    ) -> i32;
    fn SecCodeCopySigningInformation(
        code: SecStaticCodeRef,
        flags: u32,
        information: *mut CFDictionaryRef,
    ) -> i32;
    static kSecCodeInfoTeamIdentifier: CFStringRef;
}

/// The Team ID (certificate OU) this process is signed with; `None` when it is unsigned or ad-hoc
/// signed.
#[must_use]
pub fn team_id() -> Option<String> {
    let mut code: SecCodeRef = std::ptr::null_mut();
    // SAFETY: `code` receives a +1 reference on success.
    if unsafe { SecCodeCopySelf(0, &raw mut code) } != errSecSuccess || code.is_null() {
        return None;
    }
    // SAFETY: a +1 reference from SecCodeCopySelf.
    let code = unsafe { CFType::wrap_under_create_rule(code.cast()) };
    let mut static_code: SecStaticCodeRef = std::ptr::null_mut();
    // SAFETY: `code` is a valid SecCode; `static_code` receives a +1 reference on success.
    let status =
        unsafe { SecCodeCopyStaticCode(code.as_CFTypeRef() as _, 0, &raw mut static_code) };
    if status != errSecSuccess || static_code.is_null() {
        return None;
    }
    // SAFETY: a +1 reference from SecCodeCopyStaticCode.
    let static_code = unsafe { CFType::wrap_under_create_rule(static_code.cast()) };
    let mut information: CFDictionaryRef = std::ptr::null();
    // SAFETY: `static_code` is a valid SecStaticCode; `information` receives a +1 reference.
    let status = unsafe {
        SecCodeCopySigningInformation(
            static_code.as_CFTypeRef() as _,
            SIGNING_INFORMATION,
            &raw mut information,
        )
    };
    if status != errSecSuccess || information.is_null() {
        return None;
    }
    // SAFETY: a +1 reference to the signing information, whose keys are strings.
    let information: CFDictionary<CFString, CFType> =
        unsafe { CFDictionary::wrap_under_create_rule(information) };
    // SAFETY: a constant of the Security framework.
    let key = unsafe { CFString::wrap_under_get_rule(kSecCodeInfoTeamIdentifier) };
    information
        .find(&key)
        .and_then(|value| value.downcast::<CFString>())
        .map(|team| team.to_string())
}
