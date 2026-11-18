//! Checks of the values the VPN client takes: the gateway's host, text fields and the OTP. Error
//! texts name the field but never its value, and reach the app as they are.

use std::net::Ipv4Addr;

use crate::app_internal::problem::{AccountField, AccountValueProblem};
use crate::app_internal::section::AccountSaveError;

// openfortivpn's limits.
pub const MAX_HOST_BYTES: usize = 253;
pub const MAX_USERNAME_BYTES: usize = 64;
pub const MAX_REALM_BYTES: usize = 63;
const MAX_PASSWORD_BYTES: usize = 255;

pub const PIN_PREFIX: &str = "pin-sha256:";

pub fn check_otp(code: &str) -> Result<(), String> {
    if (6..=8).contains(&code.len()) && code.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(())
    } else {
        Err("OTP must be 6 to 8 ASCII digits".to_owned())
    }
}

pub fn check_host(host: &str, max_bytes: usize) -> Result<(), String> {
    check_text("host", host, max_bytes, any_character)?;
    if is_host(host) {
        Ok(())
    } else {
        Err("host must be a DNS hostname (letters, digits and hyphens, labels of 1-63) or a dotted-quad IPv4 address".to_owned())
    }
}

/// An IPv4 address by the contract's rule (`Ipv4Addr`'s), or a host name; never a scheme, port or
/// path.
fn is_host(host: &str) -> bool {
    host.parse::<Ipv4Addr>().is_ok() || is_host_name(host)
}

/// RFC 1123 labels. The last one must not be a number (RFC 3696 §2), since getaddrinfo reads names
/// such as `127.1`, `1.2.3` or `0x7f000001` as IPv4 addresses.
fn is_host_name(host: &str) -> bool {
    let labels_valid = host.split('.').all(|label| {
        (1..=63).contains(&label.len())
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    });
    if !labels_valid {
        return false;
    }
    let last = host
        .rsplit_once('.')
        .map_or(host, |(_, last)| last)
        .as_bytes();
    let decimal = last.iter().all(u8::is_ascii_digit);
    let hex = last.len() >= 2
        && last[0] == b'0'
        && last[1].eq_ignore_ascii_case(&b'x')
        && last[2..].iter().all(u8::is_ascii_hexdigit);
    !decimal && !hex
}

/// Why a text value cannot be given to the client.
enum TextProblem {
    Empty,
    TooLong,
    ForbiddenCharacter(char),
    SurroundingWhitespace,
}

/// The first problem of `value`; `allowed` narrows the characters a client takes further.
fn text_problem(
    value: &str,
    max_bytes: usize,
    allowed: impl Fn(char) -> bool,
) -> Option<TextProblem> {
    if value.is_empty() {
        return Some(TextProblem::Empty);
    }
    if value.len() > max_bytes {
        return Some(TextProblem::TooLong);
    }
    if let Some(character) = value.chars().find(|&c| is_forbidden(c) || !allowed(c)) {
        return Some(TextProblem::ForbiddenCharacter(character));
    }
    // Refused rather than trimmed: a client that trims the value itself would use another one than
    // the value checked.
    if value.starts_with(char::is_whitespace) || value.ends_with(char::is_whitespace) {
        return Some(TextProblem::SurroundingWhitespace);
    }
    None
}

/// A line break would end the value where a client reads it line by line. The other controls (NEL
/// included), the Unicode line and paragraph separators and the BOM are never part of a real value.
pub fn is_forbidden(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}')
}

fn any_character(_: char) -> bool {
    true
}

pub fn check_text(
    field: &str,
    value: &str,
    max_bytes: usize,
    allowed: impl Fn(char) -> bool,
) -> Result<(), String> {
    match text_problem(value, max_bytes, allowed) {
        None => Ok(()),
        Some(TextProblem::Empty) => Err(format!("{field} must not be empty")),
        Some(TextProblem::TooLong) => {
            Err(format!("{field} must be at most {max_bytes} UTF-8 bytes"))
        }
        Some(TextProblem::ForbiddenCharacter(character)) => {
            let code = u32::from(character);
            Err(format!("{field} contains forbidden character U+{code:04X}"))
        }
        Some(TextProblem::SurroundingWhitespace) => {
            Err(format!("{field} must not start or end with whitespace"))
        }
    }
}

/// The app's check of one account value before it is saved or used. An empty value is left to the
/// caller: an account not set up yet, or no realm.
fn account_value(
    field: AccountField,
    value: &str,
    max_bytes: usize,
    allowed: impl Fn(char) -> bool,
) -> Result<(), AccountValueProblem> {
    match text_problem(value, max_bytes, allowed) {
        None | Some(TextProblem::Empty) => Ok(()),
        Some(TextProblem::TooLong) => Err(AccountValueProblem::TooLong {
            field,
            max_bytes: u32::try_from(max_bytes).expect("value limits fit in u32"),
        }),
        Some(TextProblem::ForbiddenCharacter(_)) => {
            Err(AccountValueProblem::DisallowedCharacters { field })
        }
        Some(TextProblem::SurroundingWhitespace) => {
            Err(AccountValueProblem::LeadingOrTrailingSpace { field })
        }
    }
}

pub fn check_password(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("password must not be empty".to_owned());
    }
    if let Some(character) = password.chars().find(|&c| !is_password_character(c)) {
        let code = u32::from(character);
        return Err(format!(
            "password contains forbidden character U+{code:04X}"
        ));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(format!(
            "password must be at most {MAX_PASSWORD_BYTES} bytes"
        ));
    }
    Ok(())
}

/// openconnect for Windows reads its arguments and stdin in the system's legacy code page, so
/// anything outside printable ASCII may reach the gateway changed.
pub fn is_username_character(c: char) -> bool {
    c.is_ascii() && !c.is_ascii_control()
}

/// Printable ASCII and the space, as for the username; a line break would end the read early.
fn is_password_character(c: char) -> bool {
    is_username_character(c)
}

/// Characters that stay as they are in a URL's path.
pub fn is_realm_character(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')
}

/// The app's check of the account before it is saved or used, with the Helper's rules: the first
/// value openconnect would refuse, in the order server address, username, realm. An empty server
/// address or username is left to the caller, as an account not set up yet; an empty realm means
/// none.
pub fn check_account(host: &str, username: &str, realm: &str) -> Result<(), AccountValueProblem> {
    account_value(AccountField::Host, host, MAX_HOST_BYTES, any_character)?;
    if !host.is_empty() && !is_host(host) {
        return Err(AccountValueProblem::InvalidHost {});
    }
    account_value(
        AccountField::Username,
        username,
        MAX_USERNAME_BYTES,
        is_username_character,
    )?;
    account_value(
        AccountField::Realm,
        realm,
        MAX_REALM_BYTES,
        is_realm_character,
    )
}

/// The app's check of a typed VPN password before it is saved. An empty one is left to the
/// caller, for which it keeps the stored password.
pub fn check_vpn_password(password: &str) -> Result<(), AccountSaveError> {
    if !password.chars().all(is_password_character) || password.len() > MAX_PASSWORD_BYTES {
        return Err(AccountSaveError::InvalidVpnPassword {
            max_bytes: u32::try_from(MAX_PASSWORD_BYTES).expect("the limit fits in u32"),
        });
    }
    Ok(())
}

/// `text` as openconnect compares it: `pin-sha256:` and the standard base64 of a SHA-256 hash of
/// the gateway's public key, as openconnect prints it when it rejects the gateway.
pub fn certificate_value(text: &str) -> Option<String> {
    let encoded = text.strip_prefix(PIN_PREFIX)?.as_bytes();
    // 32 bytes: 43 characters, the last of which carries 2 unused zero bits, and one '='.
    let (data, padding) = encoded.split_at_checked(43)?;
    let valid = padding == b"="
        && data.iter().all(|&byte| base64_value(byte).is_some())
        && base64_value(data[42]).is_some_and(|value| value & 0b11 == 0);
    valid.then(|| text.to_owned())
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=";
    const OTHER_PIN: &str = "pin-sha256:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
    const PLACEHOLDER_PIN: &str = "pin-sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    const PASSWORD: &str = " Pa55 w#rd ";

    #[test]
    fn otp_is_six_to_eight_digits() {
        for code in ["008056", "1234567", "12345678"] {
            assert_eq!(check_otp(code), Ok(()), "{code}");
        }
        for code in ["12345", "123456789", "12345a", "١٢٣٤٥٦"] {
            assert!(check_otp(code).is_err(), "{code}");
        }
    }

    #[test]
    fn certificate_values_are_canonical_sha256_pins() {
        for pin in [PIN, OTHER_PIN, PLACEHOLDER_PIN] {
            assert_eq!(certificate_value(pin), Some(pin.to_owned()));
        }
        for text in [
            "",
            "VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx34=",
            "PIN-SHA256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx34=",
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx34",
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A==",
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A= ",
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu-T+caKs4wthkccwXx3A=",
            // The last character carries bits past the 32 bytes.
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx35=",
            "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11",
        ] {
            assert_eq!(certificate_value(text), None, "{text}");
        }
    }

    #[test]
    fn account_values_follow_the_launch_rules() {
        assert_eq!(check_account("", "", ""), Ok(()));
        assert_eq!(
            check_account("203.0.113.10", "alice", "staff-1.a_b~c"),
            Ok(())
        );
        assert_eq!(
            check_account("vpn.example.com:8443", "alice", ""),
            Err(AccountValueProblem::InvalidHost {})
        );
        assert_eq!(
            check_account("vpn.example.com", "alicë", ""),
            Err(AccountValueProblem::DisallowedCharacters {
                field: AccountField::Username
            })
        );
        assert_eq!(
            check_account("vpn.example.com", "alice", "a/b"),
            Err(AccountValueProblem::DisallowedCharacters {
                field: AccountField::Realm
            })
        );
        assert_eq!(
            check_account("vpn.example.com", " alice", ""),
            Err(AccountValueProblem::LeadingOrTrailingSpace {
                field: AccountField::Username
            })
        );
    }

    #[test]
    fn a_typed_password_must_be_printable_ascii() {
        assert_eq!(check_vpn_password(""), Ok(()));
        assert_eq!(check_vpn_password(PASSWORD), Ok(()));
        for refused in ["pässword".to_owned(), "a\nb".to_owned(), "p".repeat(256)] {
            assert_eq!(
                check_vpn_password(&refused),
                Err(AccountSaveError::InvalidVpnPassword { max_bytes: 255 }),
                "{refused:?}"
            );
        }
    }
}
