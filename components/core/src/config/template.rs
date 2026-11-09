//! The text the app writes: always the same commented template, so the user's comments and the
//! keys this version does not know are lost on a write (DESIGN §3).

use std::fmt;
use std::ops::RangeInclusive;

use fvm_contract::app_internal::TimeOfDay;
use fvm_contract::app_internal::problem::ConfigProblemKind;

use super::text::disallowed_in_value;
use super::{
    AWAY_MINUTES, MARGIN_MINUTES, MAX_FILE_BYTES, MAX_ROUTE_ENTRIES, MAX_TRUSTED_CERTS,
    MIN_SLEEP_MINUTES, OTP_METHODS, PORTS, SCHEMA_VERSION, SESSION_MAX_MINUTES, Settings, WEEKDAYS,
    minutes, normalized_days,
};
use crate::bypass_rules::{MAX_PREFIX_LENGTH, MIN_PREFIX_LENGTH};

/// Settings that pass come back unchanged when the text is read. The certificate values are not
/// checked again: only the client's rule produces them.
pub(super) fn write(settings: &Settings, client_name: &str) -> Result<String, ConfigProblemKind> {
    let strings = [
        ("vpn.host", &settings.vpn.host),
        ("vpn.username", &settings.vpn.username),
        ("vpn.realm", &settings.vpn.realm),
        ("otp.gmail_address", &settings.otp.gmail_address),
        ("otp.google_client_id", &settings.otp.google_client_id),
    ];
    for (path, value) in strings {
        if let Some(character) = disallowed_in_value(value) {
            return Err(ConfigProblemKind::InvalidCharacter {
                path: path.to_owned(),
                scalar: u32::from(character),
            });
        }
    }
    let connection = &settings.connection;
    let ranges = [
        ("vpn.port", settings.vpn.port, PORTS),
        (
            "connection.session_max_minutes",
            connection.session_max_minutes,
            SESSION_MAX_MINUTES,
        ),
        (
            "connection.proactive_reconnect.margin_minutes",
            connection.proactive_reconnect.margin_minutes,
            MARGIN_MINUTES,
        ),
        (
            "connection.reconnect_after_sleep.min_sleep_minutes",
            connection.reconnect_after_sleep.min_sleep_minutes,
            MIN_SLEEP_MINUTES,
        ),
        (
            "connection.schedule.away_minutes",
            connection.schedule.away_minutes,
            AWAY_MINUTES,
        ),
    ];
    for (path, value, range) in ranges {
        if !range.contains(&value) {
            return Err(ConfigProblemKind::OutOfRange {
                path: path.to_owned(),
                raw: value.to_string(),
                minimum: *range.start(),
                maximum: *range.end(),
            });
        }
    }
    let margin = connection.proactive_reconnect.margin_minutes;
    let session = connection.session_max_minutes;
    if margin >= session {
        return Err(ConfigProblemKind::MarginNotLessThanSession { margin, session });
    }
    let schedule = &connection.schedule;
    if minutes(schedule.start) >= minutes(schedule.end) {
        return Err(ConfigProblemKind::ScheduleEndNotAfterStart {
            start: schedule.start,
            end: schedule.end,
        });
    }
    let lists = [
        ("routes.networks", &settings.routes.networks),
        ("routes.bypass_vpn", &settings.routes.bypass_vpn),
    ];
    for (path, list) in lists {
        if list.len() > MAX_ROUTE_ENTRIES {
            return Err(ConfigProblemKind::TooManyRouteEntries {
                path: path.to_owned(),
                count: list.len() as u32,
                limit: MAX_ROUTE_ENTRIES as u32,
            });
        }
        for (index, entry) in list.iter().enumerate() {
            let earlier = &list[..index];
            if earlier.contains(entry) {
                return Err(ConfigProblemKind::DuplicateRouteEntry {
                    path: path.to_owned(),
                    index: index as u32,
                });
            }
            if let Some(other) = earlier.iter().find(|other| other.overlaps(entry)) {
                return Err(ConfigProblemKind::OverlappingRouteEntry {
                    path: path.to_owned(),
                    index: index as u32,
                    other: other.to_string(),
                });
            }
        }
    }
    let text = render(settings, client_name);
    if text.len() > MAX_FILE_BYTES {
        return Err(ConfigProblemKind::FileTooLarge {
            byte_count: text.len() as u64,
            limit: MAX_FILE_BYTES as u64,
        });
    }
    Ok(text)
}

fn render(settings: &Settings, client_name: &str) -> String {
    let vpn = &settings.vpn;
    let otp = &settings.otp;
    let connection = &settings.connection;
    let proactive = &connection.proactive_reconnect;
    let after_sleep = &connection.reconnect_after_sleep;
    let schedule = &connection.schedule;
    let method = OTP_METHODS
        .iter()
        .find(|&&(_, method)| method == otp.method)
        .map(|&(spelling, _)| spelling)
        .expect("every method has a spelling");
    let days = normalized_days(&schedule.days)
        .iter()
        .map(|&day| {
            let &(spelling, _) = WEEKDAYS
                .iter()
                .find(|&&(_, weekday)| weekday == day)
                .expect("every day has a spelling");
            spelling
        })
        .collect::<Vec<_>>()
        .join(", ");
    let time = |time: TimeOfDay| quoted(&format!("{:02}:{:02}", time.hour, time.minute));
    let trusted_certs = list(&vpn.trusted_certs);
    let networks = list(&settings.routes.networks);
    let bypass_vpn = list(&settings.routes.bypass_vpn);
    let (shortest_prefix, longest_prefix) = (MIN_PREFIX_LENGTH, MAX_PREFIX_LENGTH);
    format!(
        r#"# FortiVPNManager configuration file
#
# - Passwords (VPN password, Gmail app password) and the Google sign-in live in the macOS login Keychain, not in this file.
# - The app rewrites this file when you save in Settings, when it records a new server certificate, and when it adds routes found on the network service to routes.bypass_vpn. Comments you added and unknown keys are removed then.
# - Do not write leading zeros in numbers (0443 is an error). Write numbers and true/false without quotes.
# - This file contains the server and account details, so do not commit it to a public repository (dotfiles etc.).
# - Changes take effect from the next connection. The session max time and proactive reconnect margin are recalculated immediately, and the routes section is applied right away.
schema_version: {SCHEMA_VERSION}
vpn:
  # VPN gateway address (host name or IPv4 address).
  host: {host}
  # Filled in on first launch from an existing {client_name} config, if any (e.g. 8443). 443 is the default when the file has no value.
  port: {port}
  # VPN username. The password is stored in the Keychain per 'username@server address'.
  username: {username}
  # Authentication realm. Leave it empty ("") if not needed.
  realm: {realm}
  # SHA-256 digests of the server certificate. The app fills these in on the first connection and manages them; no need to edit by hand.
  # When the certificate changes, the app automatically adds the new value at the front (up to {MAX_TRUSTED_CERTS} entries).
  trusted_certs:{trusted_certs}
otp:
  # Gmail (Google Workspace) address that receives the OTP emails. The app password is in the Keychain.
  gmail_address: {gmail_address}
  # How the app reads the OTP emails: "imap" (Gmail app password) or "gmail_api" (Gmail API with Google sign-in; falls back to IMAP when the sign-in fails and an app password is stored).
  method: {method}
  # OAuth client ID of the Google Cloud desktop app used by "gmail_api". Set by importing the client JSON in Settings; its client secret is in the Keychain.
  google_client_id: {google_client_id}
connection:
  # Connect automatically when the app starts.
  auto_connect_on_launch: {auto_connect_on_launch}
  # Time until the server forcibly ends the session (minutes, {session_range}).
  session_max_minutes: {session_max_minutes}
  # When enabled is true, reconnect ahead of time, margin_minutes minutes ({margin_range}, less than session_max_minutes) before the session max time runs out.
  proactive_reconnect: {{ enabled: {proactive_enabled}, margin_minutes: {margin_minutes} }}
  # When enabled is true, reconnect on wake after sleeping for at least min_sleep_minutes minutes ({min_sleep_range}) while connected.
  reconnect_after_sleep: {{ enabled: {after_sleep_enabled}, min_sleep_minutes: {min_sleep_minutes} }}
  # Scheduled hours. When enabled is true, connect at start ("HH:mm") on the listed days (mon, tue, wed, thu, fri, sat, sun); start must be earlier than end.
  # The end never disconnects. After it, reconnects wait while you are away (screen locked or off, or the Mac asleep) and stop once you have been away for away_minutes minutes ({away_range}).
  schedule: {{ enabled: {schedule_enabled}, days: [{days}], start: {start}, end: {end}, away_minutes: {away_minutes} }}
routes:
  # Networks where the exception rules apply (e.g. "10.20.161.0/24", /{shortest_prefix}-/{longest_prefix}, up to {MAX_ROUTE_ENTRIES} entries). The rules are used only while this Mac's IPv4 address is inside one of them; on any other network everything goes through the VPN. Entries must not overlap. The app never adds entries here.
  networks:{networks}
  # Exception targets: IPv4 hosts (e.g. "10.30.161.16") or networks (e.g. "10.30.170.0/24") that skip the VPN and go out directly through the router of the active network service (up to {MAX_ROUTE_ENTRIES} entries, which must not overlap). Targets on this Mac's own network need no route and are skipped.
  # The app keeps them as that service's additional routes (networksetup -setadditionalroutes) whether or not the VPN is connected, so they also survive a reboot. On a network listed above, routes added there outside the app through the same router are added here unless they overlap a target; routes through other gateways are left alone.
  bypass_vpn:{bypass_vpn}
"#,
        host = quoted(&vpn.host),
        port = vpn.port,
        username = quoted(&vpn.username),
        realm = quoted(&vpn.realm),
        gmail_address = quoted(&otp.gmail_address),
        method = quoted(method),
        google_client_id = quoted(&otp.google_client_id),
        auto_connect_on_launch = connection.auto_connect_on_launch,
        session_range = range(SESSION_MAX_MINUTES),
        session_max_minutes = connection.session_max_minutes,
        margin_range = range(MARGIN_MINUTES),
        proactive_enabled = proactive.enabled,
        margin_minutes = proactive.margin_minutes,
        min_sleep_range = range(MIN_SLEEP_MINUTES),
        after_sleep_enabled = after_sleep.enabled,
        min_sleep_minutes = after_sleep.min_sleep_minutes,
        away_range = range(AWAY_MINUTES),
        schedule_enabled = schedule.enabled,
        start = time(schedule.start),
        end = time(schedule.end),
        away_minutes = schedule.away_minutes,
    )
}

fn range(range: RangeInclusive<i64>) -> String {
    format!("{}-{}", range.start(), range.end())
}

/// A block list, or `[]` when empty.
fn list<T: fmt::Display>(values: &[T]) -> String {
    if values.is_empty() {
        return " []".to_owned();
    }
    values
        .iter()
        .map(|value| format!("\n    - {}", quoted(&value.to_string())))
        .collect()
}

/// A double-quoted scalar. Values that pass hold no character needing an escape other than the
/// quote and the backslash.
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
