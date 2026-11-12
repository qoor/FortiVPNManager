//! Checks of the values both VPN clients take: the gateway's host, text fields, the password and
//! the OTP. Each client adds its own limits on top. Error texts name the field but never its value,
//! and reach the app as they are.

use std::net::Ipv4Addr;

use fvm_contract::app_internal::problem::{AccountField, AccountValueProblem};

pub(crate) fn check_otp(code: &str) -> Result<(), String> {
    if (6..=8).contains(&code.len()) && code.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(())
    } else {
        Err("OTP must be 6 to 8 ASCII digits".to_owned())
    }
}

pub(crate) fn check_host(host: &str, max_bytes: usize) -> Result<(), String> {
    check_text("host", host, max_bytes, any_character)?;
    if is_host(host) {
        Ok(())
    } else {
        Err("host must be a DNS hostname (letters, digits and hyphens, labels of 1-63) or a dotted-quad IPv4 address".to_owned())
    }
}

/// An IPv4 address by the contract's rule (`Ipv4Addr`'s), or a host name; never a scheme, port or
/// path.
pub(crate) fn is_host(host: &str) -> bool {
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
pub(crate) fn is_forbidden(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}')
}

pub(crate) fn any_character(_: char) -> bool {
    true
}

pub(crate) fn check_text(
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
pub(crate) fn account_value(
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

/// The password goes to stdin, followed by '\n', so spaces at either end are kept. '\n' ends the
/// client's read early, '\r' would become part of the password (a pipe has no tty translation),
/// and '\0' ends the C string.
pub(crate) fn forbidden_in_password(password: &str) -> Option<char> {
    password.chars().find(|c| matches!(c, '\n' | '\r' | '\0'))
}
