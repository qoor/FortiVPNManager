//! Shared helpers for the config.yaml tests.
#![allow(dead_code)]

use fvm_contract::app_internal::problem::{ConfigProblem, ConfigProblemKind, LineColumn};
use fvm_contract::app_internal::{OtpMethod, TimeOfDay, Weekday};
use fvm_core::config::{
    self, CertificateList, ClientRules, Ipv4Network, Loaded, Platform, Settings, Update,
    UpdateError,
};

/// The openfortivpn adapter's rule: a SHA-256 digest in hex, kept lowercase.
fn certificate_value(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    (lower.len() == 64
        && lower
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then_some(lower)
}

/// The openconnect adapter's rule, shortened: `pin-sha256:` and 44 base64 characters.
fn pin_value(text: &str) -> Option<String> {
    let encoded = text.strip_prefix("pin-sha256:")?;
    (encoded.len() == 44
        && encoded.ends_with('=')
        && encoded[..43]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/'))
    .then(|| text.to_owned())
}

/// The macOS app's rules: openfortivpn's digests.
pub fn client() -> ClientRules<'static> {
    ClientRules {
        name: "openfortivpn",
        platform: Platform::MacOs,
        certificate_list: CertificateList::Digests,
        digest_value: &certificate_value,
        pin_value: &pin_value,
    }
}

/// The Windows app's rules: openconnect's pins.
pub fn windows_client() -> ClientRules<'static> {
    ClientRules {
        name: "openconnect",
        platform: Platform::Windows,
        certificate_list: CertificateList::Pins,
        digest_value: &certificate_value,
        pin_value: &pin_value,
    }
}

pub fn read(yaml: &str) -> Result<Loaded, ConfigProblem> {
    config::read(yaml.as_bytes(), &client())
}

pub fn decode(yaml: &str) -> Loaded {
    read(yaml).unwrap_or_else(|problem| panic!("{yaml:?} did not read: {problem:?}"))
}

pub fn load_error(yaml: &str) -> Option<ConfigProblem> {
    read(yaml).err()
}

/// What a file written from `settings` holds; with no file yet, a change is always written.
pub fn render(settings: &Settings) -> String {
    match config::update(None, &client(), |current| *current = settings.clone()) {
        Ok(Update::Write { text, .. }) => text,
        other => panic!("{settings:?} did not render: {other:?}"),
    }
}

/// Why `settings` cannot be written.
pub fn validation_error(settings: &Settings) -> Option<ConfigProblemKind> {
    match config::update(Some(b"vpn: { port: 2 }\n"), &client(), |current| {
        *current = settings.clone()
    }) {
        Err(UpdateError::Settings(problem)) => {
            assert_eq!(problem.location, None);
            Some(problem.kind)
        }
        Err(UpdateError::File(problem)) => panic!("unexpected file problem {problem:?}"),
        Ok(_) => None,
    }
}

pub fn loc(line: u32, column: u32) -> Option<LineColumn> {
    Some(LineColumn { line, column })
}

pub fn time(hour: u8, minute: u8) -> TimeOfDay {
    TimeOfDay { hour, minute }
}

pub fn net(text: &str) -> Ipv4Network {
    text.parse().unwrap()
}

pub fn pin_a() -> String {
    format!("pin-sha256:{}=", "A".repeat(43))
}

pub fn pin_b() -> String {
    format!("pin-sha256:{}=", "B".repeat(43))
}

pub fn digest_a() -> String {
    "a".repeat(64)
}
pub fn digest_b() -> String {
    "0123456789abcdef".repeat(4)
}
pub fn digest_c() -> String {
    "c".repeat(63) + "d"
}
pub const GOOGLE_CLIENT_ID: &str =
    "123456789012-abcdefghijklmnopqrstuvwxyz012345.apps.googleusercontent.com";

/// A config with every field changed from the defaults.
pub fn non_default() -> Settings {
    let mut settings = Settings::default();
    settings.vpn.host = "vpn.example.com".into();
    settings.vpn.port = 8443;
    settings.vpn.username = "bob".into();
    settings.vpn.realm = "corp".into();
    settings.vpn.trusted_certs = vec![digest_a(), digest_b()];
    settings.vpn.trusted_cert_pins = vec![pin_a()];
    settings.otp.gmail_address = "otp@example.com".into();
    settings.otp.method = OtpMethod::GmailApi;
    settings.otp.google_client_id = GOOGLE_CLIENT_ID.into();
    let connection = &mut settings.connection;
    connection.auto_connect_on_launch = false;
    connection.session_max_minutes = 175;
    connection.proactive_reconnect.enabled = false;
    connection.proactive_reconnect.margin_minutes = 7;
    connection.reconnect_after_sleep.enabled = true;
    connection.reconnect_after_sleep.min_sleep_minutes = 45;
    connection.schedule.enabled = true;
    connection.schedule.days = vec![Weekday::Mon, Weekday::Wed, Weekday::Sat];
    connection.schedule.start = time(8, 30);
    connection.schedule.end = time(19, 5);
    connection.schedule.away_minutes = 90;
    settings.routes.networks = vec![net("10.20.161.0/24")];
    settings.routes.bypass_vpn = vec![net("10.30.161.16"), net("10.30.170.0/24")];
    settings
}
