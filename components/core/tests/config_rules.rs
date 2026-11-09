//! The exception rule lists and the schedule in config.yaml, and the rules a changed config must
//! pass before it is written (SPEC F-CF-01, F-CF-02, F-BR-01).

mod common;

use std::net::Ipv4Addr;

use common::*;
use fvm_contract::app_helper::AdditionalRoute;
use fvm_contract::app_internal::Weekday;
use fvm_contract::app_internal::problem::{
    AddressProblem, ConfigProblemKind, ConfigWarning, ConfigWarningKind, ValueKind,
};
use fvm_core::config::{Ipv4Network, Settings};

fn parse(text: &str) -> Result<Ipv4Network, AddressProblem> {
    text.parse()
}

fn out_of_range(length: u32) -> AddressProblem {
    AddressProblem::PrefixOutOfRange {
        length,
        minimum: 8,
        maximum: 32,
    }
}

fn route(destination: &str, mask: &str) -> AdditionalRoute {
    AdditionalRoute {
        destination: destination.into(),
        subnet_mask: mask.into(),
        gateway: "10.20.161.254".into(),
    }
}

fn bypass_host() -> Ipv4Network {
    net("10.30.161.16")
}
fn bypass_network() -> Ipv4Network {
    net("10.30.170.0/24")
}
fn office() -> Ipv4Network {
    net("10.20.161.0/24")
}

fn changed(change: impl FnOnce(&mut Settings)) -> Settings {
    let mut settings = Settings::default();
    change(&mut settings);
    settings
}

// MARK: Hosts and networks

#[test]
fn hosts_and_networks_in_canonical_spelling() {
    for (raw, canonical, prefix_length) in [
        ("10.30.161.16", "10.30.161.16", 32),
        ("10.30.161.16/32", "10.30.161.16", 32),
        ("10.30.170.0/24", "10.30.170.0/24", 24),
        ("10.0.0.0/8", "10.0.0.0/8", 8),
        ("192.168.50.128/25", "192.168.50.128/25", 25),
        ("223.255.255.255", "223.255.255.255", 32),
        ("1.0.0.0/8", "1.0.0.0/8", 8),
    ] {
        let network = parse(raw).unwrap();
        assert_eq!(network.to_string(), canonical);
        assert_eq!(u32::from(network.prefix_length()), prefix_length);
    }
}

#[test]
fn malformed_spellings() {
    for raw in [
        "",
        " 10.0.0.1",
        "10.0.0.1 ",
        "10.0.0",
        "10.0.0.1.2",
        "10.0.0.256",
        "010.0.0.1",
        "10.0.0.01",
        "10.0.0.1/",
        "10.0.0.1/024",
        "10.0.0.1/24/1",
        "10.0.0.1/+8",
        "0x0a.0.0.1",
        "10.0.0.-1",
        "10..0.1",
        "host.example.com",
        "10.0.0.1/33a",
        "\u{FF11}0.0.0.1",
        "::1",
        "10.0.0.1/100",
    ] {
        assert_eq!(parse(raw), Err(AddressProblem::Malformed {}), "{raw:?}");
    }
}

#[test]
fn prefix_lengths_outside_8_to_32() {
    assert_eq!(parse("10.0.0.0/7"), Err(out_of_range(7)));
    assert_eq!(parse("10.0.0.0/0"), Err(out_of_range(0)));
    assert_eq!(parse("10.0.0.1/33"), Err(out_of_range(33)));
}

#[test]
fn addresses_that_cannot_be_routed() {
    for raw in [
        "0.0.0.0",
        "0.1.2.3",
        "127.0.0.1",
        "127.0.0.0/8",
        "224.0.0.1",
        "239.255.255.250",
        "240.0.0.0/8",
        "255.255.255.255",
    ] {
        assert_eq!(parse(raw), Err(AddressProblem::Unroutable {}), "{raw}");
    }
}

#[test]
fn host_bits_after_the_prefix_suggest_the_network() {
    assert_eq!(
        parse("10.30.170.5/24"),
        Err(AddressProblem::HostBitsSet {
            network: "10.30.170.0/24".into()
        })
    );
}

#[test]
fn networksetup_masks() {
    assert_eq!(
        Ipv4Network::from_route(&route("10.30.161.16", "255.255.255.255"))
            .unwrap()
            .to_string(),
        "10.30.161.16"
    );
    assert_eq!(
        Ipv4Network::from_route(&route("10.30.170.0", "255.255.255.0"))
            .unwrap()
            .to_string(),
        "10.30.170.0/24"
    );
    assert_eq!(
        Ipv4Network::from_route(&route("10.0.0.0", "255.0.255.0")),
        Err(AddressProblem::NonContiguousMask {})
    );
    assert_eq!(
        Ipv4Network::from_route(&route("10.0.0.0", "0.0.0.0")),
        Err(out_of_range(0))
    );
    assert_eq!(
        bypass_network().route_through("10.20.161.254"),
        route("10.30.170.0", "255.255.255.0")
    );
}

#[test]
fn containment() {
    assert!(office().contains_address(Ipv4Addr::new(10, 20, 161, 20)));
    assert!(!office().contains_address(Ipv4Addr::new(10, 20, 162, 20)));
    assert!(office().contains(&net("10.20.161.16")));
    assert!(office().contains(&office()));
    assert!(!office().contains(&net("10.20.0.0/16")));
    assert_eq!(
        Ipv4Network::enclosing("10.20.161.20", "255.255.255.0"),
        Ok(office())
    );
    assert_eq!(
        Ipv4Network::enclosing("10.20.161.20", "255.0.255.0"),
        Err(AddressProblem::NonContiguousMask {})
    );
}

// MARK: Lists in the file

#[test]
fn the_default_lists_are_empty() {
    assert!(Settings::default().routes.bypass_vpn.is_empty());
    assert!(decode("routes:\n").settings.routes.bypass_vpn.is_empty());
    assert!(
        decode("routes: { bypass_vpn: ~ }\n")
            .settings
            .routes
            .bypass_vpn
            .is_empty()
    );
}

#[test]
fn entries_are_read_in_canonical_spelling_and_repeats_dropped() {
    let loaded = decode(
        "routes:\n  bypass_vpn:\n    - 10.30.161.16/32\n    - '10.30.170.0/24'\n    - \"10.30.161.16\"",
    );
    assert_eq!(
        loaded.settings.routes.bypass_vpn,
        [bypass_host(), bypass_network()]
    );
    assert_eq!(
        loaded.warnings,
        [ConfigWarning {
            kind: ConfigWarningKind::DuplicateRouteEntry {
                path: "routes.bypass_vpn".into(),
                index: 2
            },
            location: loc(5, 7),
        }]
    );
}

#[test]
fn a_malformed_entry_fails_the_file_at_the_entry() {
    let error =
        load_error("routes:\n  bypass_vpn:\n    - 10.30.161.16\n    - 10.30.161\n").unwrap();
    assert_eq!(
        error.kind,
        ConfigProblemKind::InvalidRouteEntry {
            path: "routes.bypass_vpn".into(),
            index: 1,
            raw: "10.30.161".into(),
            problem: AddressProblem::Malformed {},
        }
    );
    assert_eq!(error.location, loc(4, 7));
}

#[test]
fn wrong_list_shapes() {
    assert_eq!(
        load_error("routes: { bypass_vpn: 10.0.0.1 }\n")
            .unwrap()
            .kind,
        ConfigProblemKind::TypeMismatch {
            path: "routes.bypass_vpn".into(),
            expected: ValueKind::Sequence
        }
    );
    assert_eq!(
        load_error("routes: { bypass_vpn: [[10.0.0.1]] }\n")
            .unwrap()
            .kind,
        ConfigProblemKind::TypeMismatch {
            path: "routes.bypass_vpn".into(),
            expected: ValueKind::String
        }
    );
    assert_eq!(
        load_error("routes: { bypass_vpn: [~] }\n").unwrap().kind,
        ConfigProblemKind::InvalidRouteEntry {
            path: "routes.bypass_vpn".into(),
            index: 0,
            raw: String::new(),
            problem: AddressProblem::Malformed {},
        }
    );
    assert_eq!(
        decode("routes: { bypass: [] }\n")
            .warnings
            .into_iter()
            .map(|w| w.kind)
            .collect::<Vec<_>>(),
        [ConfigWarningKind::UnknownKey {
            path: "routes.bypass".into()
        }]
    );
}

#[test]
fn at_most_a_hundred_entries() {
    let hosts: Vec<String> = (1..=101)
        .map(|n| format!("10.0.{}.{}", n / 256, n % 256))
        .collect();
    let yaml = format!(
        "routes:\n  bypass_vpn:\n{}",
        hosts
            .iter()
            .map(|h| format!("    - {h}\n"))
            .collect::<String>()
    );
    let too_many = ConfigProblemKind::TooManyRouteEntries {
        path: "routes.bypass_vpn".into(),
        count: 101,
        limit: 100,
    };
    assert_eq!(load_error(&yaml).unwrap().kind, too_many);
    let mut settings = Settings::default();
    settings.routes.bypass_vpn = hosts.iter().map(|h| net(h)).collect();
    assert_eq!(validation_error(&settings), Some(too_many));
    settings.routes.bypass_vpn.pop();
    assert_eq!(validation_error(&settings), None);
}

#[test]
fn a_changed_config_refuses_repeats() {
    let settings =
        changed(|s| s.routes.bypass_vpn = vec![bypass_host(), bypass_network(), bypass_host()]);
    assert_eq!(
        validation_error(&settings),
        Some(ConfigProblemKind::DuplicateRouteEntry {
            path: "routes.bypass_vpn".into(),
            index: 2
        })
    );
}

#[test]
fn networks_are_read_like_targets() {
    let loaded = decode(
        "routes:\n  networks: [10.20.161.0/24, \"10.30.161.0/24\", 10.20.161.0/24]\n  bypass_vpn: [10.30.161.16]",
    );
    assert_eq!(
        loaded.settings.routes.networks,
        [office(), net("10.30.161.0/24")]
    );
    assert_eq!(
        loaded
            .warnings
            .into_iter()
            .map(|w| w.kind)
            .collect::<Vec<_>>(),
        [ConfigWarningKind::DuplicateRouteEntry {
            path: "routes.networks".into(),
            index: 2
        }]
    );
    assert_eq!(
        load_error("routes:\n  networks: [10.20.161.5/24]\n")
            .unwrap()
            .kind,
        ConfigProblemKind::InvalidRouteEntry {
            path: "routes.networks".into(),
            index: 0,
            raw: "10.20.161.5/24".into(),
            problem: AddressProblem::HostBitsSet {
                network: "10.20.161.0/24".into()
            },
        }
    );
    let settings = changed(|s| s.routes.networks = vec![bypass_network(), bypass_network()]);
    assert_eq!(
        validation_error(&settings),
        Some(ConfigProblemKind::DuplicateRouteEntry {
            path: "routes.networks".into(),
            index: 1
        })
    );
}

#[test]
fn overlapping_entries_fail_the_file() {
    let error =
        load_error("routes:\n  bypass_vpn:\n    - 10.30.161.0/24\n    - 10.30.161.16\n").unwrap();
    assert_eq!(
        error.kind,
        ConfigProblemKind::OverlappingRouteEntry {
            path: "routes.bypass_vpn".into(),
            index: 1,
            other: "10.30.161.0/24".into()
        }
    );
    assert_eq!(error.location, loc(4, 7));
    assert_eq!(
        load_error("routes:\n  networks: [10.20.161.0/24, 10.20.0.0/16]\n")
            .unwrap()
            .kind,
        ConfigProblemKind::OverlappingRouteEntry {
            path: "routes.networks".into(),
            index: 1,
            other: "10.20.161.0/24".into()
        }
    );
    let settings = changed(|s| s.routes.bypass_vpn = vec![bypass_network(), net("10.30.0.0/16")]);
    assert_eq!(
        validation_error(&settings),
        Some(ConfigProblemKind::OverlappingRouteEntry {
            path: "routes.bypass_vpn".into(),
            index: 1,
            other: "10.30.170.0/24".into()
        })
    );
    // The two lists may overlap each other.
    let settings = changed(|s| {
        s.routes.bypass_vpn = vec![bypass_host()];
        s.routes.networks = vec![net("10.30.161.0/24")];
    });
    assert_eq!(validation_error(&settings), None);
}

// MARK: Schedule

#[test]
fn times_are_text_in_h_mm_or_hh_mm_quoted_or_plain() {
    for (raw, minutes) in [
        ("\"09:00\"", 540),
        ("'9:00'", 540),
        ("9:05", 545),
        ("18:00", 1080),
        ("0:00", 0),
        ("\"23:58\"", 1438),
    ] {
        let schedule = decode(&format!(
            "connection:\n  schedule: {{ start: {raw}, end: 23:59 }}\n"
        ))
        .settings
        .connection
        .schedule;
        assert_eq!(
            u32::from(schedule.start.hour) * 60 + u32::from(schedule.start.minute),
            minutes,
            "{raw}"
        );
        assert_eq!(schedule.end, time(23, 59));
    }
}

#[test]
fn other_time_spellings_are_errors_at_the_value() {
    for raw in [
        "\"24:00\"",
        "\"9:60\"",
        "\"09:0\"",
        "\"0900\"",
        "\"9am\"",
        "\"\"",
        "\" 9:00\"",
        "\"09:00:00\"",
        "\"-1:00\"",
        "\"\u{FF19}:00\"",
    ] {
        let error = load_error(&format!("connection:\n  schedule:\n    start: {raw}\n")).unwrap();
        assert!(
            matches!(&error.kind, ConfigProblemKind::InvalidTime { path, .. } if path == "connection.schedule.start"),
            "{raw}: {:?}",
            error.kind
        );
        assert_eq!(error.location, loc(3, 12), "{raw}");
    }
}

#[test]
fn numbers_are_not_times() {
    assert_eq!(
        load_error("connection:\n  schedule: { end: 1080 }\n")
            .unwrap()
            .kind,
        ConfigProblemKind::InvalidTime {
            path: "connection.schedule.end".into(),
            raw: "1080".into()
        }
    );
}

#[test]
fn the_end_must_be_later_than_the_start() {
    let error =
        load_error("connection:\n  schedule: { start: \"18:00\", end: \"09:00\" }").unwrap();
    assert_eq!(
        error.kind,
        ConfigProblemKind::ScheduleEndNotAfterStart {
            start: time(18, 0),
            end: time(9, 0)
        }
    );
    assert_eq!(error.location, loc(2, 36));
    assert_eq!(
        load_error("connection:\n  schedule: { start: \"18:00\" }\n")
            .unwrap()
            .kind,
        ConfigProblemKind::ScheduleEndNotAfterStart {
            start: time(18, 0),
            end: time(18, 0)
        }
    );
    assert_eq!(
        load_error("connection:\n  schedule: { end: \"09:01\" }\n"),
        None
    );
}

#[test]
fn days_are_normalized() {
    let days = |yaml: &str| decode(yaml).settings.connection.schedule.days;
    assert_eq!(
        days("connection:\n  schedule: { days: [sun, fri, mon, \"fri\"] }\n"),
        [Weekday::Mon, Weekday::Fri, Weekday::Sun]
    );
    assert!(days("connection:\n  schedule: { days: [] }\n").is_empty());
    assert_eq!(
        days("connection:\n  schedule: { days: ~ }\n"),
        Settings::default().connection.schedule.days
    );
}

#[test]
fn unknown_day_names_are_errors_at_the_item() {
    for raw in ["Mon", "monday", "1", "~", "true"] {
        let error = load_error(&format!(
            "connection:\n  schedule: {{ days: [mon, {raw}] }}\n"
        ))
        .unwrap();
        let ConfigProblemKind::InvalidChoice { path, allowed, .. } = &error.kind else {
            panic!("{raw}: expected invalidChoice, got {:?}", error.kind)
        };
        assert_eq!(path, "connection.schedule.days");
        assert_eq!(allowed, &["mon", "tue", "wed", "thu", "fri", "sat", "sun"]);
        assert_eq!(error.location, loc(2, 27), "{raw}");
    }
}

#[test]
fn days_must_be_a_list() {
    assert_eq!(
        load_error("connection:\n  schedule: { days: mon }\n")
            .unwrap()
            .kind,
        ConfigProblemKind::TypeMismatch {
            path: "connection.schedule.days".into(),
            expected: ValueKind::Sequence
        }
    );
}

#[test]
fn unknown_schedule_keys_are_warnings() {
    let loaded = decode("connection:\n  schedule: { enabled: true, holidays: [] }\n");
    assert!(loaded.settings.connection.schedule.enabled);
    assert_eq!(
        loaded.warnings,
        [ConfigWarning {
            kind: ConfigWarningKind::UnknownKey {
                path: "connection.schedule.holidays".into()
            },
            location: loc(2, 30),
        }]
    );
}

#[test]
fn a_changed_config_checks_the_schedule() {
    let settings = changed(|s| s.connection.schedule.end = s.connection.schedule.start);
    assert_eq!(
        validation_error(&settings),
        Some(ConfigProblemKind::ScheduleEndNotAfterStart {
            start: time(9, 0),
            end: time(9, 0)
        })
    );
    let settings = changed(|s| s.connection.schedule.away_minutes = 0);
    assert_eq!(
        validation_error(&settings),
        Some(ConfigProblemKind::OutOfRange {
            path: "connection.schedule.away_minutes".into(),
            raw: "0".into(),
            minimum: 1,
            maximum: 1440
        })
    );
}

#[test]
fn the_template_writes_normalized_days_and_quoted_times() {
    let mut settings = Settings::default();
    let schedule = &mut settings.connection.schedule;
    schedule.enabled = true;
    schedule.days = vec![Weekday::Sun, Weekday::Sat];
    schedule.start = time(7, 5);
    schedule.end = time(23, 59);
    schedule.away_minutes = 5;
    let text = render(&settings);
    assert!(text.contains("  schedule: { enabled: true, days: [sat, sun], start: \"07:05\", end: \"23:59\", away_minutes: 5 }\n"));
    settings.connection.schedule.days = vec![Weekday::Sat, Weekday::Sun];
    assert_eq!(decode(&text).settings, settings);
    settings.connection.schedule.days = vec![];
    let text = render(&settings);
    assert!(text.contains("days: [], "));
    assert_eq!(decode(&text).settings, settings);
}

// MARK: Rules for a changed config

#[test]
fn valid_configs_pass() {
    assert_eq!(validation_error(&Settings::default()), None);
    assert_eq!(validation_error(&non_default()), None);
}

#[test]
fn each_rule() {
    let range = |path: &str, raw: &str, minimum, maximum| ConfigProblemKind::OutOfRange {
        path: path.into(),
        raw: raw.into(),
        minimum,
        maximum,
    };
    let character = |path: &str, scalar| ConfigProblemKind::InvalidCharacter {
        path: path.into(),
        scalar,
    };
    type Change = Box<dyn Fn(&mut Settings)>;
    let cases: Vec<(Change, ConfigProblemKind)> = vec![
        (
            Box::new(|s| s.vpn.port = 0),
            range("vpn.port", "0", 1, 65535),
        ),
        (
            Box::new(|s| s.vpn.port = 70000),
            range("vpn.port", "70000", 1, 65535),
        ),
        (
            Box::new(|s| s.vpn.host = "gw\n".into()),
            character("vpn.host", 0x0A),
        ),
        (
            Box::new(|s| s.vpn.username = "\u{2028}".into()),
            character("vpn.username", 0x2028),
        ),
        (
            Box::new(|s| s.vpn.realm = "\u{FEFF}".into()),
            character("vpn.realm", 0xFEFF),
        ),
        (
            Box::new(|s| s.otp.gmail_address = "a\u{FFFE}".into()),
            character("otp.gmail_address", 0xFFFE),
        ),
        (
            Box::new(|s| s.otp.google_client_id = "id\u{0B}".into()),
            character("otp.google_client_id", 0x0B),
        ),
        (
            Box::new(|s| s.connection.session_max_minutes = 9),
            range("connection.session_max_minutes", "9", 10, 1440),
        ),
        (
            Box::new(|s| s.connection.proactive_reconnect.margin_minutes = 61),
            range("connection.proactive_reconnect.margin_minutes", "61", 1, 60),
        ),
        (
            Box::new(|s| s.connection.reconnect_after_sleep.min_sleep_minutes = 0),
            range(
                "connection.reconnect_after_sleep.min_sleep_minutes",
                "0",
                1,
                1440,
            ),
        ),
        (
            Box::new(|s| {
                s.connection.session_max_minutes = 20;
                s.connection.proactive_reconnect.margin_minutes = 20;
            }),
            ConfigProblemKind::MarginNotLessThanSession {
                margin: 20,
                session: 20,
            },
        ),
    ];
    for (change, expected) in cases {
        let settings = changed(change);
        assert_eq!(validation_error(&settings), Some(expected));
    }
}

/// A config whose written file the reader would refuse must not be written.
#[test]
fn the_written_file_must_fit_the_size_limit() {
    let limit = fvm_core::config::MAX_FILE_BYTES;
    let overhead = render(&Settings::default()).len(); // with an empty realm
    let fits = changed(|s| s.vpn.realm = "r".repeat(limit - overhead));
    assert_eq!(render(&fits).len(), limit);
    assert_eq!(validation_error(&fits), None);
    assert_eq!(decode(&render(&fits)).settings, fits);
    let mut too_large = fits.clone();
    too_large.vpn.realm.push('r');
    assert_eq!(
        validation_error(&too_large),
        Some(ConfigProblemKind::FileTooLarge {
            byte_count: limit as u64 + 1,
            limit: limit as u64
        })
    );
    // Multi-byte text counts in UTF-8 bytes.
    let korean = changed(|s| s.vpn.realm = "\u{D55C}".repeat(70_000 / 3));
    assert!(matches!(
        validation_error(&korean),
        Some(ConfigProblemKind::FileTooLarge { .. })
    ));
}
