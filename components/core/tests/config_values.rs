//! Reading config.yaml values: defaults, scalars, ranges and structure (SPEC F-CF-01).

mod common;

use common::*;
use fvm_contract::app_internal::problem::{
    ConfigProblemKind, ConfigWarning, ConfigWarningKind, ValueKind,
};
use fvm_core::config::Settings;

fn kind(yaml: &str) -> ConfigProblemKind {
    load_error(yaml)
        .unwrap_or_else(|| panic!("{yaml:?} read"))
        .kind
}

// MARK: Defaults

#[test]
fn empty_comment_only_and_null_documents_give_the_defaults() {
    for yaml in [
        "",
        "\n\n",
        "   \n",
        "# only a comment\n# another\n",
        "---\n",
        "---\n...\n",
        "~\n",
        "null\n",
    ] {
        let loaded = decode(yaml);
        assert_eq!(loaded.settings, Settings::default(), "{yaml:?}");
        assert!(loaded.warnings.is_empty(), "{yaml:?}");
    }
}

#[test]
fn a_full_document_in_block_style() {
    let yaml = format!(
        "schema_version: 1
vpn:
  host: vpn.example.com
  port: 8443
  username: bob
  realm: corp
  trusted_certs:
    - {a}
    - \"{b}\"
otp:
  gmail_address: otp@example.com
  method: gmail_api
  google_client_id: {GOOGLE_CLIENT_ID}
connection:
  auto_connect_on_launch: false
  session_max_minutes: 175
  proactive_reconnect:
    enabled: false
    margin_minutes: 7
  reconnect_after_sleep:
    enabled: true
    min_sleep_minutes: 45
  schedule:
    enabled: true
    days:
      - sat
      - mon
      - wed
    start: \"08:30\"
    end: 19:05
    away_minutes: 90
routes:
  networks:
    - 10.20.161.0/24
  bypass_vpn:
    - 10.30.161.16
    - \"10.30.170.0/24\"",
        a = digest_a(),
        b = digest_b()
    );
    let loaded = decode(&yaml);
    assert_eq!(loaded.settings, non_default());
    assert!(loaded.warnings.is_empty());
}

#[test]
fn a_full_document_in_flow_style() {
    let yaml = format!(
        "{{schema_version: 1, vpn: {{host: vpn.example.com, port: 8443, username: bob, realm: corp,
 trusted_certs: [{a}, '{b}']}},
 otp: {{gmail_address: otp@example.com, method: \"gmail_api\", google_client_id: '{GOOGLE_CLIENT_ID}'}},
 connection: {{auto_connect_on_launch: false, session_max_minutes: 175,
 proactive_reconnect: {{enabled: false, margin_minutes: 7}},
 reconnect_after_sleep: {{enabled: true, min_sleep_minutes: 45}},
 schedule: {{enabled: true, days: [mon, wed, sat, wed], start: 8:30, end: '19:05', away_minutes: 90}}}},
 routes: {{networks: [\"10.20.161.0/24\"], bypass_vpn: [10.30.161.16/32, '10.30.170.0/24']}}}}",
        a = digest_a(),
        b = digest_b()
    );
    assert_eq!(decode(&yaml).settings, non_default());
}

#[test]
fn missing_keys_are_merged_with_defaults() {
    let loaded = decode(
        "vpn:\n  host: gw.example.com\nconnection:\n  proactive_reconnect:\n    margin_minutes: 10",
    );
    let mut expected = Settings::default();
    expected.vpn.host = "gw.example.com".into();
    expected.connection.proactive_reconnect.margin_minutes = 10;
    assert_eq!(loaded.settings, expected);
    assert!(loaded.warnings.is_empty());
}

#[test]
fn null_values_and_sections_fall_back_to_defaults() {
    for yaml in [
        "vpn:\n",
        "vpn: ~\n",
        "vpn: null\n",
        "vpn:\n  port:\n",
        "vpn:\n  port: ~\n",
        "vpn:\n  host: NULL\n",
        "connection:\n  proactive_reconnect:\n",
        "connection:\n  auto_connect_on_launch: Null\n",
        "schema_version:\n",
        "vpn:\n  trusted_certs:\n",
    ] {
        assert_eq!(decode(yaml).settings, Settings::default(), "{yaml:?}");
    }
}

#[test]
fn string_fields_keep_their_text_as_written() {
    let settings = decode(
        "vpn:\n  host: 10.0.0.1\n  username: 0123\n  realm: true\notp:\n  gmail_address: '\u{D55C}\u{AE00}@example.com'\n",
    )
    .settings;
    assert_eq!(settings.vpn.host, "10.0.0.1");
    assert_eq!(settings.vpn.username, "0123");
    assert_eq!(settings.vpn.realm, "true");
    assert_eq!(settings.otp.gmail_address, "\u{D55C}\u{AE00}@example.com");
}

#[test]
fn booleans_accept_the_yaml_1_2_core_spellings() {
    for (raw, value) in [
        ("true", true),
        ("True", true),
        ("TRUE", true),
        ("false", false),
        ("False", false),
        ("FALSE", false),
    ] {
        let settings = decode(&format!("connection:\n  auto_connect_on_launch: {raw}\n")).settings;
        assert_eq!(settings.connection.auto_connect_on_launch, value, "{raw}");
    }
}

#[test]
fn crlf_line_endings_and_a_leading_bom() {
    let settings = decode("\u{FEFF}vpn:\r\n  host: gw\r\n  port: 8443\r\n").settings;
    assert_eq!(settings.vpn.host, "gw");
    assert_eq!(settings.vpn.port, 8443);
    // The BOM is not counted in columns.
    assert_eq!(
        load_error("\u{FEFF}vpn:\n  port: 0443\n").unwrap().location,
        loc(2, 9)
    );
    // CRLF counts as one line break.
    assert_eq!(
        load_error("vpn:\r\n  host: gw\r\n  port: 0443\r\n")
            .unwrap()
            .location,
        loc(3, 9)
    );
}

// MARK: Integers and booleans

#[test]
fn integers_must_be_canonical() {
    for raw in [
        "0443",
        "2:30",
        "0x1BB",
        "0X1bb",
        "1_0",
        "+443",
        "-1",
        "443.0",
        "1e3",
        "0o12",
        "0b1",
        "00",
        "abc",
        "1 2",
        ".inf",
        "\u{FF14}\u{FF14}\u{FF13}",
    ] {
        let error = load_error(&format!("vpn:\n  port: {raw}\n")).unwrap();
        assert_eq!(
            error.kind,
            ConfigProblemKind::InvalidInteger {
                path: "vpn.port".into(),
                raw: raw.into()
            },
            "{raw}"
        );
        assert_eq!(error.location, loc(2, 9), "{raw}");
    }
}

#[test]
fn every_integer_field_uses_the_strict_rule() {
    for (yaml, path, location) in [
        ("schema_version: 0443\n", "schema_version", loc(1, 17)),
        (
            "connection:\n  session_max_minutes: 2:30\n",
            "connection.session_max_minutes",
            loc(2, 24),
        ),
        (
            "connection:\n  proactive_reconnect: { enabled: true, margin_minutes: 0x1BB }\n",
            "connection.proactive_reconnect.margin_minutes",
            loc(2, 57),
        ),
        (
            "connection:\n  reconnect_after_sleep:\n    min_sleep_minutes: 1_0\n",
            "connection.reconnect_after_sleep.min_sleep_minutes",
            loc(3, 24),
        ),
    ] {
        let error = load_error(yaml).unwrap();
        let ConfigProblemKind::InvalidInteger {
            path: error_path, ..
        } = &error.kind
        else {
            panic!("unexpected {:?}", error.kind)
        };
        assert_eq!(error_path, path);
        assert_eq!(error.location, location, "{yaml:?}");
    }
}

#[test]
fn quoted_numbers_have_their_own_problem() {
    for literal in ["\"443\"", "'443'", "\"0x1BB\"", "\" 8443 \"", "'-1'"] {
        let error = load_error(&format!("vpn:\n  port: {literal}\n")).unwrap();
        assert!(
            matches!(&error.kind, ConfigProblemKind::QuotedNumber { path, .. } if path == "vpn.port"),
            "{literal}: {:?}",
            error.kind
        );
        assert_eq!(error.location, loc(2, 9));
    }
}

#[test]
fn a_quoted_non_number_is_a_type_mismatch() {
    let error = load_error("vpn:\n  port: \"https\"\n").unwrap();
    assert_eq!(
        error.kind,
        ConfigProblemKind::TypeMismatch {
            path: "vpn.port".into(),
            expected: ValueKind::Integer
        }
    );
    assert_eq!(error.location, loc(2, 9));
}

#[test]
fn quoted_booleans_have_their_own_problem() {
    for literal in ["\"true\"", "'false'", "\"yes\"", "'Off'"] {
        let error = load_error(&format!(
            "connection:\n  auto_connect_on_launch: {literal}\n"
        ))
        .unwrap();
        assert!(
            matches!(&error.kind, ConfigProblemKind::QuotedBoolean { path, .. } if path == "connection.auto_connect_on_launch"),
            "{literal}: {:?}",
            error.kind
        );
        assert_eq!(error.location, loc(2, 27));
    }
}

#[test]
fn a_quoted_non_boolean_is_a_type_mismatch() {
    assert_eq!(
        kind("connection:\n  auto_connect_on_launch: \"maybe\"\n"),
        ConfigProblemKind::TypeMismatch {
            path: "connection.auto_connect_on_launch".into(),
            expected: ValueKind::Boolean
        }
    );
}

#[test]
fn yaml_1_1_booleans_and_numbers_are_not_booleans() {
    for raw in ["yes", "no", "on", "off", "y", "n", "1", "0"] {
        let error = load_error(&format!(
            "connection:\n  reconnect_after_sleep: {{ enabled: {raw} }}\n"
        ))
        .unwrap();
        assert_eq!(
            error.kind,
            ConfigProblemKind::InvalidBoolean {
                path: "connection.reconnect_after_sleep.enabled".into(),
                raw: raw.into()
            }
        );
        assert_eq!(error.location, loc(2, 37));
    }
}

#[test]
fn range_limits() {
    for (yaml, path, valid) in [
        ("vpn:\n  port: 0\n", "vpn.port", false),
        ("vpn:\n  port: 1\n", "vpn.port", true),
        ("vpn:\n  port: 65535\n", "vpn.port", true),
        ("vpn:\n  port: 65536\n", "vpn.port", false),
        ("vpn:\n  port: 99999999999999999999999\n", "vpn.port", false),
        (
            "connection:\n  session_max_minutes: 9\n",
            "connection.session_max_minutes",
            false,
        ),
        (
            "connection:\n  session_max_minutes: 10\n",
            "connection.session_max_minutes",
            true,
        ),
        (
            "connection:\n  session_max_minutes: 1440\n",
            "connection.session_max_minutes",
            true,
        ),
        (
            "connection:\n  session_max_minutes: 1441\n",
            "connection.session_max_minutes",
            false,
        ),
        (
            "connection:\n  proactive_reconnect: { margin_minutes: 0 }\n",
            "connection.proactive_reconnect.margin_minutes",
            false,
        ),
        (
            "connection:\n  proactive_reconnect: { margin_minutes: 1 }\n",
            "connection.proactive_reconnect.margin_minutes",
            true,
        ),
        (
            "connection:\n  proactive_reconnect: { margin_minutes: 60 }\n",
            "connection.proactive_reconnect.margin_minutes",
            true,
        ),
        (
            "connection:\n  proactive_reconnect: { margin_minutes: 61 }\n",
            "connection.proactive_reconnect.margin_minutes",
            false,
        ),
        (
            "connection:\n  reconnect_after_sleep: { min_sleep_minutes: 0 }\n",
            "connection.reconnect_after_sleep.min_sleep_minutes",
            false,
        ),
        (
            "connection:\n  reconnect_after_sleep: { min_sleep_minutes: 1 }\n",
            "connection.reconnect_after_sleep.min_sleep_minutes",
            true,
        ),
        (
            "connection:\n  reconnect_after_sleep: { min_sleep_minutes: 1440 }\n",
            "connection.reconnect_after_sleep.min_sleep_minutes",
            true,
        ),
        (
            "connection:\n  reconnect_after_sleep: { min_sleep_minutes: 1441 }\n",
            "connection.reconnect_after_sleep.min_sleep_minutes",
            false,
        ),
        (
            "connection:\n  schedule: { away_minutes: 0 }\n",
            "connection.schedule.away_minutes",
            false,
        ),
        (
            "connection:\n  schedule: { away_minutes: 1 }\n",
            "connection.schedule.away_minutes",
            true,
        ),
        (
            "connection:\n  schedule: { away_minutes: 1440 }\n",
            "connection.schedule.away_minutes",
            true,
        ),
        (
            "connection:\n  schedule: { away_minutes: 1441 }\n",
            "connection.schedule.away_minutes",
            false,
        ),
    ] {
        let error = load_error(yaml);
        if valid {
            assert_eq!(error, None, "{yaml:?}");
        } else {
            let error = error.unwrap_or_else(|| panic!("{yaml:?} read"));
            assert!(
                matches!(&error.kind, ConfigProblemKind::OutOfRange { path: error_path, .. } if error_path == path),
                "{yaml:?}: {:?}",
                error.kind
            );
            assert_eq!(
                error.location.map(|location| location.line),
                Some(yaml.lines().count() as u32)
            );
        }
    }
}

#[test]
fn the_margin_must_be_less_than_the_session() {
    let error = load_error(
        "connection:\n  session_max_minutes: 30\n  proactive_reconnect: { enabled: true, margin_minutes: 30 }",
    )
    .unwrap();
    assert_eq!(
        error.kind,
        ConfigProblemKind::MarginNotLessThanSession {
            margin: 30,
            session: 30
        }
    );
    assert_eq!(error.location, loc(3, 57));
    // Margin key order does not matter, and the default margin fits the smallest session.
    assert_eq!(
        kind(
            "connection:\n  proactive_reconnect: { margin_minutes: 45 }\n  session_max_minutes: 30\n"
        ),
        ConfigProblemKind::MarginNotLessThanSession {
            margin: 45,
            session: 30
        }
    );
    assert_eq!(load_error("connection:\n  session_max_minutes: 10\n"), None);
    assert_eq!(
        load_error(
            "connection:\n  session_max_minutes: 10\n  proactive_reconnect: { margin_minutes: 9 }\n"
        ),
        None
    );
}

// MARK: Structure

#[test]
fn anchors_and_aliases_are_refused_at_the_anchor() {
    for (yaml, location) in [
        ("vpn: &v\n  host: a\n".to_owned(), loc(1, 6)),
        ("base: &p 8443\nvpn:\n  port: *p\n".to_owned(), loc(1, 7)),
        (
            "vpn:\n  host: &h gw\n  username: *h\n".to_owned(),
            loc(2, 9),
        ),
        (
            format!("vpn:\n  trusted_certs: [&d {}]\n", "a".repeat(64)),
            loc(2, 19),
        ),
        ("&root\nvpn: {}\n".to_owned(), loc(1, 1)),
    ] {
        let error = load_error(&yaml).unwrap();
        assert_eq!(error.kind, ConfigProblemKind::AnchorOrAlias {}, "{yaml:?}");
        assert_eq!(error.location, location, "{yaml:?}");
    }
}

#[test]
fn an_undefined_alias_is_reported_as_an_alias() {
    let error = load_error("vpn:\n  port: *nope\n").unwrap();
    assert_eq!(error.kind, ConfigProblemKind::AnchorOrAlias {});
    assert_eq!(error.location, loc(2, 9));
}

#[test]
fn merge_keys_are_refused() {
    for (yaml, location) in [
        ("vpn:\n  <<: { port: 8443 }\n", loc(2, 3)),
        (
            "defaults: { port: 8443 }\nvpn:\n  host: gw\n  <<: {}\n",
            loc(4, 3),
        ),
        ("<<: { vpn: {} }\n", loc(1, 1)),
    ] {
        let error = load_error(yaml).unwrap();
        assert_eq!(error.kind, ConfigProblemKind::MergeKey {}, "{yaml:?}");
        assert_eq!(error.location, location, "{yaml:?}");
    }
}

#[test]
fn a_quoted_merge_lookalike_is_just_an_unknown_key() {
    assert_eq!(
        decode("vpn:\n  \"<<\": 1\n").warnings,
        [ConfigWarning {
            kind: ConfigWarningKind::UnknownKey {
                path: "vpn.<<".into()
            },
            location: loc(2, 3)
        }]
    );
}

#[test]
fn explicit_tags_are_refused() {
    for (yaml, tag, location) in [
        (
            "vpn:\n  port: !!int 443\n",
            "tag:yaml.org,2002:int",
            loc(2, 9),
        ),
        (
            "vpn:\n  host: !!str gw\n",
            "tag:yaml.org,2002:str",
            loc(2, 9),
        ),
        ("vpn:\n  host: !custom gw\n", "!custom", loc(2, 9)),
        (
            "vpn: !!map\n  host: gw\n",
            "tag:yaml.org,2002:map",
            loc(1, 6),
        ),
        (
            "vpn:\n  trusted_certs: !!seq []\n",
            "tag:yaml.org,2002:seq",
            loc(2, 18),
        ),
        (
            "vpn:\n  port: !!int \"443\"\n",
            "tag:yaml.org,2002:int",
            loc(2, 9),
        ),
        (
            "schema_version: !!int 1\n",
            "tag:yaml.org,2002:int",
            loc(1, 17),
        ),
    ] {
        let error = load_error(yaml).unwrap();
        assert_eq!(
            error.kind,
            ConfigProblemKind::ExplicitTag { tag: tag.into() },
            "{yaml:?}"
        );
        assert_eq!(error.location, location, "{yaml:?}");
    }
}

#[test]
fn the_root_must_be_a_mapping() {
    for (yaml, location) in [
        ("- a\n- b\n", loc(1, 1)),
        ("just a string\n", loc(1, 1)),
        ("42\n", loc(1, 1)),
        ("[]\n", loc(1, 1)),
        ("\n\n\"quoted\"\n", loc(3, 1)),
    ] {
        let error = load_error(yaml).unwrap();
        assert_eq!(error.kind, ConfigProblemKind::RootNotMapping {}, "{yaml:?}");
        assert_eq!(error.location, location, "{yaml:?}");
    }
}

#[test]
fn wrong_node_kinds() {
    let a64 = "a".repeat(64);
    for (yaml, path, expected, location) in [
        ("vpn: 5\n".to_owned(), "vpn", ValueKind::Mapping, loc(1, 6)),
        ("vpn: []\n".to_owned(), "vpn", ValueKind::Mapping, loc(1, 6)),
        (
            "connection:\n  proactive_reconnect: on\n".to_owned(),
            "connection.proactive_reconnect",
            ValueKind::Mapping,
            loc(2, 24),
        ),
        (
            "vpn:\n  host: [a]\n".to_owned(),
            "vpn.host",
            ValueKind::String,
            loc(2, 9),
        ),
        (
            "otp:\n  gmail_address: { a: b }\n".to_owned(),
            "otp.gmail_address",
            ValueKind::String,
            loc(2, 18),
        ),
        (
            "otp:\n  method: [imap]\n".to_owned(),
            "otp.method",
            ValueKind::String,
            loc(2, 11),
        ),
        (
            "otp:\n  google_client_id: { a: b }\n".to_owned(),
            "otp.google_client_id",
            ValueKind::String,
            loc(2, 21),
        ),
        (
            "vpn:\n  port: { a: 1 }\n".to_owned(),
            "vpn.port",
            ValueKind::Integer,
            loc(2, 9),
        ),
        // The parser places a block scalar at its content, not at `|` (DESIGN §7.2 approved change 2).
        (
            "vpn:\n  port: |\n    443\n".to_owned(),
            "vpn.port",
            ValueKind::Integer,
            loc(3, 5),
        ),
        (
            "connection:\n  auto_connect_on_launch: [true]\n".to_owned(),
            "connection.auto_connect_on_launch",
            ValueKind::Boolean,
            loc(2, 27),
        ),
        (
            format!("vpn:\n  trusted_certs: {a64}\n"),
            "vpn.trusted_certs",
            ValueKind::Sequence,
            loc(2, 18),
        ),
        (
            "vpn:\n  trusted_certs: { a: b }\n".to_owned(),
            "vpn.trusted_certs",
            ValueKind::Sequence,
            loc(2, 18),
        ),
        (
            "schema_version: [1]\n".to_owned(),
            "schema_version",
            ValueKind::Integer,
            loc(1, 17),
        ),
    ] {
        let error = load_error(&yaml).unwrap();
        assert_eq!(
            error.kind,
            ConfigProblemKind::TypeMismatch {
                path: path.into(),
                expected
            },
            "{yaml:?}"
        );
        assert_eq!(error.location, location, "{yaml:?}");
    }
}

#[test]
fn complex_keys_are_refused() {
    let error = load_error("vpn:\n  ? [a, b]\n  : c\n").unwrap();
    assert_eq!(error.kind, ConfigProblemKind::ComplexKey {});
    assert_eq!(error.location, loc(2, 5));
}
