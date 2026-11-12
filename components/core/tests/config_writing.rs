//! The text the app writes to config.yaml.

mod common;

use common::*;
use fvm_contract::app_internal::{OtpMethod, TimeOfDay, Weekday};
use fvm_core::config::{self, Ipv4Network, Settings, Update};

#[test]
fn the_default_template_reads_back_as_the_defaults() {
    let loaded = decode(&render(&Settings::default()));
    assert_eq!(loaded.settings, Settings::default());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn the_default_template_body() {
    let text = render(&Settings::default());
    let body: Vec<&str> = text
        .split('\n')
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    assert_eq!(
        body.join("\n"),
        r#"schema_version: 1
vpn:
  host: ""
  port: 443
  username: ""
  realm: ""
  trusted_certs: []
  trusted_cert_pins: []
otp:
  gmail_address: ""
  method: "imap"
  google_client_id: ""
connection:
  auto_connect_on_launch: true
  session_max_minutes: 180
  proactive_reconnect: { enabled: true, margin_minutes: 5 }
  reconnect_after_sleep: { enabled: false, min_sleep_minutes: 30 }
  schedule: { enabled: false, days: [mon, tue, wed, thu, fri], start: "09:00", end: "18:00", away_minutes: 120 }
routes:
  networks: []
  bypass_vpn: []
"#
    );
    assert!(text.ends_with('\n'));
}

#[test]
fn every_key_has_a_comment() {
    let text = render(&Settings::default());
    // Every key has a comment line right above it except the schema version and nested flow keys.
    let lines: Vec<&str> = text.lines().collect();
    for key in [
        "host:",
        "port:",
        "username:",
        "realm:",
        "trusted_certs:",
        "trusted_cert_pins:",
        "gmail_address:",
        "method:",
        "google_client_id:",
        "auto_connect_on_launch:",
        "session_max_minutes:",
        "proactive_reconnect:",
        "reconnect_after_sleep:",
        "schedule:",
        "networks:",
        "bypass_vpn:",
    ] {
        let index = lines
            .iter()
            .position(|line| line.trim_start().starts_with(key))
            .unwrap();
        assert!(lines[index - 1].trim_start().starts_with('#'), "{key}");
    }
}

#[test]
fn certificate_values_are_a_block_list() {
    let text = render(&non_default());
    assert!(text.contains(&format!(
        "  trusted_certs:\n    - \"{}\"\n    - \"{}\"\n  #",
        digest_a(),
        digest_b()
    )));
    assert!(text.contains(&format!(
        "  trusted_cert_pins:\n    - \"{}\"\notp:\n",
        pin_a()
    )));
    let loaded = decode(&text);
    assert_eq!(loaded.settings, non_default());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn route_lists_are_block_lists() {
    let text = render(&non_default());
    assert!(text.contains("  networks:\n    - \"10.20.161.0/24\"\n  #"));
    assert!(text.ends_with("  bypass_vpn:\n    - \"10.30.161.16\"\n    - \"10.30.170.0/24\"\n"));
}

fn tricky_strings() -> Vec<String> {
    let mut strings: Vec<String> = [
        "",
        " ",
        "  leading",
        "trailing  ",
        "\"",
        "\\",
        "\\\"",
        "'",
        "''",
        "#",
        " # not a comment",
        "a: b",
        "a:b",
        "- item",
        "? key",
        "|",
        ">",
        "&anchor",
        "*alias",
        "!tag",
        "!!str",
        "%YAML",
        "@at",
        "`tick`",
        "{a: b}",
        "[1, 2]",
        "---",
        "...",
        "~",
        "null",
        "true",
        "no",
        "0443",
        "0x1BB",
        "2:30",
        "1e3",
        ".inf",
        "<<",
        "\u{D55C}\u{AE00} realm",
        "e\u{301}",
        "\u{1F680}\u{1F1F0}\u{1F1F7}",
        "\u{A0}nbsp\u{A0}",
        "\u{E000}",
        "\u{FFFD}",
        "\u{10FFFD}",
        "a,b",
        "${HOME}",
        "user@example.com",
        "DOMAIN\\user",
    ]
    .iter()
    .map(|text| text.to_string())
    .collect();
    strings.push("x".repeat(5000));
    strings
}

#[test]
fn string_fields_round_trip_exactly() {
    for value in tricky_strings() {
        let mut settings = Settings::default();
        settings.vpn.host = value.clone();
        settings.vpn.username = value.clone();
        settings.vpn.realm = value.clone();
        settings.otp.gmail_address = value.clone();
        settings.otp.google_client_id = value.clone();
        let loaded = decode(&render(&settings));
        assert_eq!(loaded.settings, settings, "{value:?}");
        assert!(loaded.warnings.is_empty());
    }
}

#[test]
fn writing_is_stable() {
    let once = render(&non_default());
    assert_eq!(render(&decode(&once).settings), once);
}

#[test]
fn double_quoted_escapes() {
    for (value, expected) in [
        ("plain", r#""plain""#),
        (r#"say "hi""#, r#""say \"hi\"""#),
        (r"C:\dir", r#""C:\\dir""#),
        ("\u{D55C}\u{1F680}", "\"\u{D55C}\u{1F680}\""),
    ] {
        let mut settings = Settings::default();
        settings.vpn.realm = value.into();
        assert!(
            render(&settings).contains(&format!("  realm: {expected}\n")),
            "{value}"
        );
    }
}

#[test]
fn an_unchanged_file_is_not_written() {
    let text = render(&non_default());
    let update = config::update(Some(text.as_bytes()), &client(), |_| {}).unwrap();
    assert_eq!(update, Update::Unchanged(non_default()));
    let update = config::update(Some(text.as_bytes()), &client(), |s| s.vpn.port = 10443).unwrap();
    let Update::Write { settings, text } = update else {
        panic!("expected a write")
    };
    assert_eq!(settings.vpn.port, 10443);
    assert_eq!(decode(&text).settings, settings);
}

#[test]
fn a_file_that_does_not_pass_is_never_written_over() {
    let newer = b"schema_version: 2\nvpn: { port: 1 }\n";
    assert!(matches!(
        config::update(Some(newer), &client(), |s| s.vpn.port = 10443),
        Err(config::UpdateError::File(_))
    ));
}

/// SplitMix64: a tiny deterministic generator for reproducible "random" tests.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn range(&mut self, low: i64, high: i64) -> i64 {
        low + (self.next() % (high - low + 1) as u64) as i64
    }
    fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

fn random_networks(rng: &mut Seeded) -> Vec<Ipv4Network> {
    let mut networks: Vec<Ipv4Network> = Vec::new();
    for _ in 0..rng.range(0, 10) {
        let prefix_length = rng.range(8, 32) as u32;
        let address = (rng.range(1, 223) as u32) << 24 | (rng.next() as u32 & 0xFF_FFFF);
        let network = address & (u32::MAX << (32 - prefix_length));
        let text = format!("{}/{prefix_length}", std::net::Ipv4Addr::from(network));
        let Ok(network) = text.parse::<Ipv4Network>() else {
            continue;
        };
        if !networks.iter().any(|other| other.overlaps(&network)) {
            networks.push(network);
        }
    }
    networks
}

/// Deterministic pseudo-random configs over the whole valid domain.
#[test]
fn random_valid_configs_round_trip() {
    let mut rng = Seeded(0x5EED_F0F1);
    let pool: Vec<String> = tricky_strings()
        .into_iter()
        .filter(|s| s.chars().count() < 100)
        .collect();
    let days = [
        Weekday::Mon,
        Weekday::Tue,
        Weekday::Wed,
        Weekday::Thu,
        Weekday::Fri,
        Weekday::Sat,
        Weekday::Sun,
    ];
    for _ in 0..300 {
        let mut string = || {
            (0..rng.range(0, 3))
                .map(|_| pool[rng.range(0, pool.len() as i64 - 1) as usize].clone())
                .collect::<String>()
        };
        let mut settings = Settings::default();
        settings.vpn.host = string();
        settings.vpn.username = string();
        settings.vpn.realm = string();
        settings.otp.gmail_address = string();
        settings.otp.google_client_id = string();
        settings.vpn.port = rng.range(1, 65535);
        for _ in 0..rng.range(0, 4) {
            let digest: String = (0..64)
                .map(|_| b"0123456789abcdef"[rng.range(0, 15) as usize] as char)
                .collect();
            if !settings.vpn.trusted_certs.contains(&digest) {
                settings.vpn.trusted_certs.push(digest);
            }
        }
        settings.otp.method = if rng.coin() {
            OtpMethod::Imap
        } else {
            OtpMethod::GmailApi
        };
        let session = rng.range(10, 1440);
        let connection = &mut settings.connection;
        connection.auto_connect_on_launch = rng.coin();
        connection.session_max_minutes = session;
        connection.proactive_reconnect.enabled = rng.coin();
        connection.proactive_reconnect.margin_minutes = rng.range(1, 60.min(session - 1));
        connection.reconnect_after_sleep.enabled = rng.coin();
        connection.reconnect_after_sleep.min_sleep_minutes = rng.range(1, 1440);
        let start = rng.range(0, 24 * 60 - 2);
        let end = rng.range(start + 1, 24 * 60 - 1);
        connection.schedule.enabled = rng.coin();
        connection.schedule.days = days.iter().copied().filter(|_| rng.coin()).collect();
        connection.schedule.start = TimeOfDay {
            hour: (start / 60) as u8,
            minute: (start % 60) as u8,
        };
        connection.schedule.end = TimeOfDay {
            hour: (end / 60) as u8,
            minute: (end % 60) as u8,
        };
        connection.schedule.away_minutes = rng.range(1, 1440);
        settings.routes.networks = random_networks(&mut rng);
        settings.routes.bypass_vpn = random_networks(&mut rng);
        let loaded = decode(&render(&settings));
        assert_eq!(loaded.settings, settings);
        assert!(loaded.warnings.is_empty());
    }
}

/// One file serves both OSes; the comments describe the OS of the app that wrote it.
#[test]
fn the_comments_describe_the_os_that_wrote_the_file() {
    let comments = |client| -> String {
        match config::update(None, &client, |current| *current = Settings::default()) {
            Ok(Update::Write { text, .. }) => text
                .lines()
                .filter(|line| line.trim_start().starts_with('#'))
                .collect::<Vec<_>>()
                .join("\n"),
            other => panic!("did not render: {other:?}"),
        }
    };
    let macos = comments(client());
    for wording in [
        "macOS login Keychain",
        "an existing openfortivpn config",
        "the Mac asleep",
        "networksetup -setadditionalroutes",
    ] {
        assert!(macos.contains(wording), "{wording}");
    }
    let windows = comments(windows_client());
    for wording in [
        "Windows Credential Manager",
        "in Credential Manager per 'username@server address'",
        "the PC asleep",
        "persistent routes of that interface",
        "gateway of the active network interface",
    ] {
        assert!(windows.contains(wording), "{wording}");
    }
    for wording in ["Keychain", "Mac", "networksetup", "openfortivpn"] {
        assert!(!windows.contains(wording), "{wording}");
    }
}

#[test]
fn a_file_written_on_one_os_reads_the_same_on_the_other() {
    let text = render(&non_default());
    let on_windows = config::read(text.as_bytes(), &windows_client()).unwrap();
    assert_eq!(on_windows.settings, non_default());
    assert!(on_windows.warnings.is_empty());
}
