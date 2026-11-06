//! Checking the app's values again and turning them into openfortivpn's launch. Error texts name the
//! field but never its value, and reach the app as they are.

use std::net::Ipv4Addr;
use std::path::Path;

use fvm_contract::app_helper::RunRequest;
use fvm_contract::backend::{ClientExecutable, LaunchSpec};

// openfortivpn's buffer sizes (src/config.h): load_config() silently cuts longer values, so the
// client would use a different value than the one checked.
const MAX_HOST_BYTES: usize = 253; // GATEWAY_HOST_SIZE
const MAX_USERNAME_BYTES: usize = 64; // USERNAME_SIZE
const MAX_REALM_BYTES: usize = 63; // REALM_SIZE

const MAX_TRUSTED_CERTS: usize = 8;

/// read_password() reads at most PASSWORD_SIZE (256) bytes and stops early only at '\n'. A longer
/// password would leave its '\n' in stdin, and the OTP prompt would read that as an empty answer.
const MAX_PASSWORD_BYTES: usize = 255;

/// The only environment variable openfortivpn gets: it reads https_proxy and all_proxy.
const PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

pub(super) fn launch_spec(
    client: &ClientExecutable,
    request: &RunRequest,
    config_path: &Path,
) -> Result<LaunchSpec, String> {
    check(request)?;
    let config_path = config_path
        .to_str()
        .expect("the Helper names its config file in UTF-8");
    Ok(LaunchSpec {
        program: client.path.clone(),
        // No verbosity option, which would print the cookie, OTP or password, and no --persistent,
        // which would send the used OTP again.
        args: vec!["-c".to_owned(), config_path.to_owned()],
        env: vec![("PATH".to_owned(), PATH.to_owned())],
        config_file: config_file(request),
    })
}

pub(super) fn check_otp(code: &str) -> Result<(), String> {
    if (6..=8).contains(&code.len()) && code.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(())
    } else {
        Err("OTP must be 6 to 8 ASCII digits".to_owned())
    }
}

/// Checks the fields in the order the config file lists them, then the password.
fn check(request: &RunRequest) -> Result<(), String> {
    check_host(&request.host)?;
    if !(1..=65_535).contains(&request.port) {
        return Err("port must be between 1 and 65535".to_owned());
    }
    check_text("username", &request.username, MAX_USERNAME_BYTES)?;
    if let Some(realm) = realm(request) {
        check_text("realm", realm, MAX_REALM_BYTES)?;
    }
    if request.trusted_certs.len() > MAX_TRUSTED_CERTS {
        return Err(format!(
            "trusted-cert allows at most {MAX_TRUSTED_CERTS} digests"
        ));
    }
    if let Some(index) = request
        .trusted_certs
        .iter()
        .position(|value| !is_certificate_value(value))
    {
        return Err(format!(
            "trusted-cert entry {index} is not 64 lowercase hex characters"
        ));
    }
    check_password(&request.password)
}

fn check_host(host: &str) -> Result<(), String> {
    check_text("host", host, MAX_HOST_BYTES)?;
    if host.parse::<Ipv4Addr>().is_ok() || is_host_name(host) {
        Ok(())
    } else {
        Err("host must be a DNS hostname (letters, digits and hyphens, labels of 1-63) or a dotted-quad IPv4 address".to_owned())
    }
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

fn check_text(field: &str, value: &str, max_bytes: usize) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if value.len() > max_bytes {
        return Err(format!("{field} must be at most {max_bytes} UTF-8 bytes"));
    }
    // A line break would add a config line. The other controls (NEL included), the Unicode line
    // and paragraph separators and the BOM are never part of a real value.
    if let Some(character) = value
        .chars()
        .find(|&c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}'))
    {
        return Err(forbidden(field, character));
    }
    // Stricter than load_config(), which trims only ASCII whitespace.
    if value.starts_with(char::is_whitespace) || value.ends_with(char::is_whitespace) {
        return Err(format!("{field} must not start or end with whitespace"));
    }
    Ok(())
}

/// The password goes to stdin, followed by '\n', so spaces at either end are kept.
fn check_password(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("password must not be empty".to_owned());
    }
    // '\n' ends read_password() early, '\r' would become part of the password (a pipe has no tty
    // translation), and '\0' ends the C string.
    if let Some(character) = password.chars().find(|c| matches!(c, '\n' | '\r' | '\0')) {
        return Err(forbidden("password", character));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(format!(
            "password must be at most {MAX_PASSWORD_BYTES} UTF-8 bytes"
        ));
    }
    Ok(())
}

fn forbidden(field: &str, character: char) -> String {
    format!(
        "{field} contains forbidden character U+{:04X}",
        u32::from(character)
    )
}

/// openfortivpn compares the gateway's SHA-256 digest as lowercase hex.
fn is_certificate_value(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// An empty realm means none.
fn realm(request: &RunRequest) -> Option<&str> {
    request.realm.as_deref().filter(|realm| !realm.is_empty())
}

/// Checked values hold no line break and neither start nor end with whitespace, so load_config(),
/// which splits each line at its first '=' and trims both sides, reads every value back unchanged.
fn config_file(request: &RunRequest) -> String {
    let port = request.port.to_string();
    let mut entries = vec![
        ("host", request.host.as_str()),
        ("port", port.as_str()),
        ("username", request.username.as_str()),
    ];
    entries.extend(realm(request).map(|realm| ("realm", realm)));
    entries.extend(
        request
            .trusted_certs
            .iter()
            .map(|value| ("trusted-cert", value.as_str())),
    );
    entries
        .iter()
        .map(|(key, value)| format!("{key} = {value}\n"))
        .collect()
}
