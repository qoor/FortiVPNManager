//! Reading an existing openfortivpn config file, to import its account and certificate values. The
//! file is only read.
//!
//! The reader follows openfortivpn 1.24.1's load_config() (src/config.c) byte for byte:
//! - Lines come from getline(): split at LF only, so a CR before the LF stays in the line and is
//!   then trimmed as whitespace.
//! - A line is split at its first `=`, so values may hold `=` and `#`; there are no inline
//!   comments.
//! - Key and value are trimmed of the C locale's isspace() (SP, HT, LF, VT, FF, CR); openfortivpn
//!   never calls setlocale.
//! - Everything is a C string, so a NUL byte would end the line early.
//!
//! Unlike load_config(), it reads every known key to the end of the file instead of stopping at
//! the first unknown one, and it also takes indented `#` lines as comments.

use std::path::{Path, PathBuf};

use fvm_contract::app_internal::SecretText;

use super::request::{is_certificate_value, is_forbidden};

/// The port openfortivpn uses when the file sets none.
const DEFAULT_PORT: u16 = 443;

const IMPORTED_KEYS: [&str; 6] = [
    "host",
    "port",
    "username",
    "password",
    "realm",
    "trusted-cert",
];

/// Every other key load_config() accepts in the macOS build of 1.24.1. It is built with
/// /usr/sbin/pppd, so the `pppd` key of the build without it is unknown there.
const OTHER_KNOWN_KEYS: [&str; 34] = [
    "otp",
    "otp-prompt",
    "otp-delay",
    "cookie",
    "cookie-on-stdin",
    "no-ftm-push",
    "pinentry",
    "set-dns",
    "sni",
    "set-routes",
    "half-internet-routes",
    "persistent",
    "pppd-use-peerdns",
    "pppd-log",
    "pppd-plugin",
    "pppd-ipparam",
    "pppd-ifname",
    "pppd-call",
    "pppd-accept-remote",
    "ppp-system",
    "use-resolvconf",
    "use-syslog",
    "ca-file",
    "user-cert",
    "saml-login",
    "user-key",
    "pem-passphrase",
    "insecure-ssl",
    "cipher-list",
    "min-tls",
    "seclevel-1",
    "user-agent",
    "hostcheck",
    "check-virtual-desktop",
];

/// What an openfortivpn config file holds for the app. A later line overrides an earlier one, as
/// in openfortivpn, and an empty value clears the field.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportedSettings {
    pub host: Option<String>,
    /// 443 when the file has no usable `port` line.
    pub port: u16,
    pub username: Option<String>,
    /// Written in the file in plain text, to go to the secret storage.
    pub password: Option<SecretText>,
    pub realm: Option<String>,
    /// Valid certificate values, lowercased (openfortivpn compares lowercase, so an uppercase one
    /// never matched there), each once, in file order.
    pub trusted_certs: Vec<String>,
    /// Keys openfortivpn does not know, such as `insecure-cert`, each once, in file order;
    /// openfortivpn stops reading at the first one.
    pub unknown_keys: Vec<String>,
    /// openfortivpn options the app does not carry over, such as `set-dns`.
    pub ignored_keys: Vec<String>,
    /// 1-based numbers of lines that could not be used: without `=`, with an empty key, with bytes
    /// that are not UTF-8, with a control character or NUL, with a port openfortivpn refuses, or
    /// with an invalid certificate value.
    pub unreadable_lines: Vec<u32>,
}

/// The files openfortivpn reads by default, in the order to try: the user's, then Homebrew's.
pub(super) fn default_files(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".config/openfortivpn/config"),
        PathBuf::from("/opt/homebrew/etc/openfortivpn/openfortivpn/config"),
    ]
}

pub(super) fn read(bytes: &[u8]) -> ImportedSettings {
    let mut settings = ImportedSettings {
        host: None,
        port: DEFAULT_PORT,
        username: None,
        password: None,
        realm: None,
        trusted_certs: Vec::new(),
        unknown_keys: Vec::new(),
        ignored_keys: Vec::new(),
        unreadable_lines: Vec::new(),
    };
    let mut port = None;
    let body = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    for (index, line) in lines(body).into_iter().enumerate() {
        let number = u32::try_from(index + 1).expect("a file the size of a config has few lines");
        let content = trimmed(line);
        if content.is_empty() || content[0] == b'#' {
            continue;
        }
        let Some((key, value)) = split(line).filter(|_| !line.contains(&0)) else {
            settings.unreadable_lines.push(number);
            continue;
        };
        let Some(key) = std::str::from_utf8(key).ok().filter(|key| !key.is_empty()) else {
            settings.unreadable_lines.push(number);
            continue;
        };
        if !IMPORTED_KEYS.contains(&key) {
            let list = if OTHER_KNOWN_KEYS.contains(&key) {
                &mut settings.ignored_keys
            } else {
                &mut settings.unknown_keys
            };
            push_once(list, key.to_owned());
            continue;
        }
        let Some(text) = std::str::from_utf8(value)
            .ok()
            .filter(|text| !text.chars().any(|c| c != '\t' && is_forbidden(c)))
        else {
            settings.unreadable_lines.push(number);
            continue;
        };
        let text = (!text.is_empty()).then(|| text.to_owned());
        match key {
            "host" => settings.host = text,
            "username" => settings.username = text,
            "password" => settings.password = text.map(SecretText),
            "realm" => settings.realm = text,
            // Like openfortivpn, a refused port leaves the previous one in place.
            "port" => match strtol_port(value) {
                Some(value) => port = Some(value),
                None => settings.unreadable_lines.push(number),
            },
            "trusted-cert" => {
                let value = text.unwrap_or_default().to_lowercase();
                if is_certificate_value(&value) {
                    push_once(&mut settings.trusted_certs, value);
                } else {
                    settings.unreadable_lines.push(number);
                }
            }
            _ => unreachable!("every imported key is handled"),
        }
    }
    settings.port = port.unwrap_or(DEFAULT_PORT);
    settings
}

fn push_once(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// The lines getline() returns, without their LF; a final LF starts no new line.
fn lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    if lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines
}

fn is_space(byte: u8) -> bool {
    byte == b' ' || (0x09..=0x0D).contains(&byte)
}

fn trimmed(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !is_space(*byte))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !is_space(*byte))
        .map_or(start, |last| last + 1);
    &bytes[start..end]
}

/// Splits at the first `=` and trims both sides; `None` without one, which openfortivpn reports
/// as a bad line.
fn split(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let equals = line.iter().position(|byte| *byte == b'=')?;
    Some((trimmed(&line[..equals]), trimmed(&line[equals + 1..])))
}

/// `strtol(value, NULL, 0)` and load_config()'s 1 to 65535 check: decimal, `0x` hex or octal with a
/// leading zero, an optional sign, and reading stops at the first character that is not a digit
/// (`8443 # x` is 8443). `None` when openfortivpn would print "Bad port" and ignore the line.
fn strtol_port(value: &[u8]) -> Option<u16> {
    let mut rest = &value[value.iter().take_while(|byte| is_space(**byte)).count()..];
    let mut negative = false;
    if let [sign @ (b'+' | b'-'), tail @ ..] = rest {
        negative = *sign == b'-';
        rest = tail;
    }
    let base = match rest {
        [b'0', b'x' | b'X', digit, ..] if char::from(*digit).is_ascii_hexdigit() => {
            rest = &rest[2..];
            16
        }
        // The leading zero itself reads as an octal digit.
        [b'0', ..] => 8,
        _ => 10,
    };
    let digits: Vec<u32> = rest
        .iter()
        .map_while(|byte| char::from(*byte).to_digit(base))
        .collect();
    if digits.is_empty() {
        return None;
    }
    // Anything above 65535 is refused anyway, so the value saturates just past it.
    let magnitude = digits
        .iter()
        .fold(0u32, |value, digit| (value * base + digit).min(65_536));
    if negative || magnitude == 0 {
        return None;
    }
    u16::try_from(magnitude).ok()
}

#[cfg(test)]
mod tests {
    use super::{lines, strtol_port};

    #[test]
    fn lines_follow_getline() {
        assert_eq!(lines(b"a\n\nb"), [b"a".as_slice(), b"", b"b"]);
        assert_eq!(lines(b"a\n").len(), 1);
        assert!(lines(b"").is_empty());
        assert_eq!(lines(b"\n"), [b"".as_slice()]);
    }

    #[test]
    fn ports_follow_strtol() {
        for (value, expected) in [
            ("443", Some(443)),
            ("  443", Some(443)),
            ("-0", None),
            ("0x1BB", Some(443)),
            ("0x1bbz", Some(443)),
            ("0xg", None),
            ("00", None),
            ("0", None),
            ("077", Some(63)),
            ("09", None),
            ("65535", Some(65_535)),
            ("65536", None),
            ("", None),
            ("+", None),
        ] {
            assert_eq!(strtol_port(value.as_bytes()), expected, "{value:?}");
        }
    }
}
