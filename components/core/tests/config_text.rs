//! Reading config.yaml: duplicates, syntax, unknown keys, text checks, the schema version,
//! certificate values, nesting and the OTP method (SPEC F-CF-01).

mod common;

use common::*;
use fvm_contract::app_internal::OtpMethod;
use fvm_contract::app_internal::problem::{ConfigProblemKind, ConfigWarning, ConfigWarningKind};
use fvm_core::config::{self, Settings};

fn kind(yaml: &str) -> ConfigProblemKind {
    load_error(yaml)
        .unwrap_or_else(|| panic!("{yaml:?} read"))
        .kind
}

fn warning(kind: ConfigWarningKind, line: u32, column: u32) -> ConfigWarning {
    ConfigWarning {
        kind,
        location: loc(line, column),
    }
}

fn unknown(path: &str) -> ConfigWarningKind {
    ConfigWarningKind::UnknownKey { path: path.into() }
}

#[test]
fn duplicate_keys() {
    for (yaml, key, line) in [
        ("vpn:\n  port: 1\n  port: 2\n", "port", 2),
        ("vpn: {}\notp: {}\nvpn: {}\n", "vpn", 1),
        ("vpn:\n  port: 1\n  \"port\": 2\n", "port", 2),
    ] {
        let error = load_error(yaml).unwrap();
        assert_eq!(
            error.kind,
            ConfigProblemKind::DuplicateKey { key: key.into() },
            "{yaml:?}"
        );
        assert_eq!(
            error.location.map(|location| location.line),
            Some(line),
            "{yaml:?}"
        );
    }
}

#[test]
fn multiple_documents_are_refused() {
    let error = load_error("vpn: {}\n---\notp: {}\n").unwrap();
    assert_eq!(error.kind, ConfigProblemKind::MultipleDocuments {});
    assert_eq!(error.location, loc(2, 1));
}

#[test]
fn syntax_errors_carry_line_and_column() {
    for (yaml, location) in [
        ("vpn:\n  port: [1, 2\n", loc(3, 1)),
        // libyaml: 2:1 and 3:1. saphyr points at the tab and at the opening quote (REFACTOR-PLAN
        // §2 exception 2).
        ("vpn:\n\tport: 1\n", loc(2, 2)),
        ("vpn:\n  host: \"unterminated\n", loc(2, 9)),
        ("vpn:\n  host: a\n port: 1\n", loc(3, 2)),
        ("vpn: {host: a\n", loc(2, 1)),
        ("vpn:\n  host: a: b\n", loc(2, 10)),
    ] {
        let error = load_error(yaml).unwrap();
        assert!(
            matches!(error.kind, ConfigProblemKind::Syntax { .. }),
            "{yaml:?}: {:?}",
            error.kind
        );
        assert_eq!(error.location, location, "{yaml:?}");
    }
}

#[test]
fn unknown_keys_become_warnings_with_locations() {
    let loaded = decode(
        "schema_version: 1\ntheme: dark\nvpn:\n  host: gw\n  insecure-cert: 1\nconnection:\n  proactive_reconnect: { enabled: true, jitter: 3 }",
    );
    assert_eq!(loaded.settings.vpn.host, "gw");
    assert_eq!(
        loaded.warnings,
        [
            warning(unknown("theme"), 2, 1),
            warning(unknown("vpn.insecure-cert"), 5, 3),
            warning(unknown("connection.proactive_reconnect.jitter"), 7, 41),
        ]
    );
}

#[test]
fn values_of_unknown_keys_are_not_type_checked() {
    let loaded = decode("future:\n  port: \"443\"\n  list: [1, 2]\n");
    assert_eq!(loaded.settings, Settings::default());
    assert_eq!(
        loaded
            .warnings
            .into_iter()
            .map(|w| w.kind)
            .collect::<Vec<_>>(),
        [unknown("future")]
    );
}

// MARK: Text

#[test]
fn the_64_kib_limit() {
    let head = "vpn:\n  port: 8443\n# ";
    assert_eq!(config::MAX_FILE_BYTES, 65_536);
    let exact = format!(
        "{head}{}\n",
        "x".repeat(config::MAX_FILE_BYTES - head.len() - 1)
    );
    assert_eq!(exact.len(), config::MAX_FILE_BYTES);
    assert_eq!(decode(&exact).settings.vpn.port, 8443);
    let error = load_error(&format!("{exact}\n")).unwrap();
    assert_eq!(
        error.kind,
        ConfigProblemKind::FileTooLarge {
            byte_count: 65_537,
            limit: 65_536
        }
    );
    assert_eq!(error.location, None);
}

#[test]
fn invalid_utf8_is_located() {
    let mut bytes = b"vpn:\n  host: \"a".to_vec();
    bytes.push(0xFF);
    bytes.extend(b"b\"\n");
    let error = config::read(&bytes, &client()).unwrap_err();
    assert_eq!(error.kind, ConfigProblemKind::InvalidUtf8 {});
    assert_eq!(error.location, loc(2, 11));
    // A truncated multi-byte sequence after non-ASCII text on the first line.
    let mut truncated = "# \u{D55C}\u{AE00}".as_bytes().to_vec();
    truncated.extend([0xED, 0x95]);
    assert_eq!(
        config::read(&truncated, &client()).unwrap_err().location,
        loc(1, 5)
    );
    // An encoded surrogate (invalid in UTF-8).
    assert_eq!(
        config::read(&[0x61, 0x3A, 0x20, 0xED, 0xA0, 0x80], &client())
            .unwrap_err()
            .kind,
        ConfigProblemKind::InvalidUtf8 {}
    );
}

#[test]
fn raw_characters_the_reader_refuses_are_located() {
    for (yaml, scalar, location) in [
        ("vpn:\n  host: a\u{01}b\n", 0x01, loc(2, 10)),
        ("# comment\u{7F}\nvpn: {}\n", 0x7F, loc(1, 10)),
        ("# \u{D55C}\u{AE00}\u{01}\n", 0x01, loc(1, 5)),
        ("vpn: {}\r\n# \u{00}\r\n", 0x00, loc(2, 3)),
        ("vpn:\n  host: \"a\u{80}\"\n", 0x80, loc(2, 11)),
        ("vpn:\n  realm: x\u{FFFE}\n", 0xFFFE, loc(2, 11)),
        ("vpn:\n  realm: e\u{301}\u{1B}\n", 0x1B, loc(2, 12)),
    ] {
        let error = load_error(yaml).unwrap();
        assert_eq!(
            error.kind,
            ConfigProblemKind::DisallowedCharacter { scalar },
            "{yaml:?}"
        );
        assert_eq!(error.location, location, "{yaml:?}");
    }
}

#[test]
fn raw_next_line_and_separators_are_refused() {
    // REFACTOR-PLAN §2 exception 1: YAML 1.1 and 1.2 disagree on whether they break lines.
    for (raw, scalar) in [("\u{85}", 0x85), ("\u{2028}", 0x2028), ("\u{2029}", 0x2029)] {
        let error = load_error(&format!("vpn:\n  realm: a{raw}b\n")).unwrap();
        assert_eq!(
            error.kind,
            ConfigProblemKind::DisallowedCharacter { scalar }
        );
        assert_eq!(error.location, loc(2, 11));
    }
}

#[test]
fn string_fields_refuse_control_separator_bom_and_noncharacter_code_points() {
    for (yaml, path, scalar) in [
        ("vpn:\n  host: \"a\\tb\"\n", "vpn.host", 0x09),
        ("vpn:\n  host: \"a\\x01b\"\n", "vpn.host", 0x01),
        ("vpn:\n  username: \"a\\nb\"\n", "vpn.username", 0x0A),
        ("vpn:\n  username: \"a\\Nb\"\n", "vpn.username", 0x85),
        ("vpn:\n  realm: \"a\\x9Fb\"\n", "vpn.realm", 0x9F),
        ("vpn:\n  realm: \"a\\Lb\"\n", "vpn.realm", 0x2028),
        ("vpn:\n  realm: \"a\\Pb\"\n", "vpn.realm", 0x2029),
        (
            "otp:\n  gmail_address: \"a\\uFEFFb\"\n",
            "otp.gmail_address",
            0xFEFF,
        ),
        (
            "otp:\n  gmail_address: \"\\uFDD0\"\n",
            "otp.gmail_address",
            0xFDD0,
        ),
        (
            "otp:\n  gmail_address: \"\\uFFFF\"\n",
            "otp.gmail_address",
            0xFFFF,
        ),
        (
            "otp:\n  gmail_address: \"\\U0001FFFE\"\n",
            "otp.gmail_address",
            0x1FFFE,
        ),
        ("vpn:\n  host: \"a\u{FEFF}b\"\n", "vpn.host", 0xFEFF),
        ("vpn:\n  host: \"a\tb\"\n", "vpn.host", 0x09),
        ("vpn:\n  host: |\n    gw\n", "vpn.host", 0x0A),
    ] {
        let error = load_error(yaml).unwrap();
        assert_eq!(
            error.kind,
            ConfigProblemKind::InvalidCharacter {
                path: path.into(),
                scalar
            },
            "{yaml:?}"
        );
        assert!(
            matches!(error.location.map(|l| l.line), Some(2 | 3)),
            "{yaml:?}"
        );
    }
}

#[test]
fn allowed_unicode_in_strings() {
    let settings =
        decode("vpn:\n  realm: \"\u{C601}\u{C5C5}\u{BD80} \u{1F680} \\u00A0x\"\n").settings;
    assert_eq!(
        settings.vpn.realm,
        "\u{C601}\u{C5C5}\u{BD80} \u{1F680} \u{A0}x"
    );
}

// MARK: Schema version

#[test]
fn a_missing_version_means_one() {
    assert_eq!(decode("vpn: {}\n").settings, Settings::default());
    assert_eq!(decode("schema_version: 1\n").settings, Settings::default());
}

#[test]
fn a_newer_version_is_refused() {
    let error = load_error("schema_version: 2\n").unwrap();
    assert_eq!(
        error.kind,
        ConfigProblemKind::NewerSchemaVersion { version: 2 }
    );
    assert_eq!(error.location, loc(1, 17));
}

#[test]
fn a_newer_version_wins_over_everything_else() {
    // A future file may use keys, values or ranges this version refuses.
    assert_eq!(
        kind(
            "vpn:\n  port: 0443\n  trusted_certs: { primary: x }\nschema_version: 3\nconnection:\n  session_max_minutes: \"forever\"\n"
        ),
        ConfigProblemKind::NewerSchemaVersion { version: 3 }
    );
}

#[test]
fn invalid_versions() {
    for (yaml, expected) in [
        ("schema_version: 0\n", "out"),
        ("schema_version: 99999999999999999999999\n", "out"),
        ("schema_version: \"1\"\n", "quoted"),
        ("schema_version: 01\n", "invalid"),
        ("schema_version: v1\n", "invalid"),
    ] {
        let error = load_error(yaml).unwrap();
        assert_eq!(error.location, loc(1, 17), "{yaml:?}");
        let matched = match (&error.kind, expected) {
            (
                ConfigProblemKind::OutOfRange {
                    path,
                    minimum: 1,
                    maximum: 1,
                    ..
                },
                "out",
            ) => path == "schema_version",
            (ConfigProblemKind::QuotedNumber { path, raw }, "quoted") => {
                path == "schema_version" && raw == "1"
            }
            (ConfigProblemKind::InvalidInteger { path, .. }, "invalid") => path == "schema_version",
            _ => false,
        };
        assert!(matched, "{yaml:?}: {:?}", error.kind);
    }
}

// MARK: Certificate values

#[test]
fn an_uppercase_certificate_value_is_lowercased() {
    let settings = decode(&format!(
        "vpn:\n  trusted_certs:\n    - {}\n",
        digest_b().to_uppercase()
    ))
    .settings;
    assert_eq!(settings.vpn.trusted_certs, [digest_b()]);
}

#[test]
fn invalid_certificate_values_are_dropped_with_warnings() {
    let yaml = format!(
        "vpn:\n  trusted_certs:\n    - {a}\n    - not-a-digest\n    - {a63}\n    - {f65}\n    - \"{b} \"\n    - ~\n    - {{ sha: {c} }}\n    - [{c}]\n    - {upper_c}\n    - \"{upper_a}\"\n    - \u{FF10}{zeros}\n",
        a = digest_a(),
        a63 = "a".repeat(63),
        f65 = "f".repeat(65),
        b = digest_b(),
        c = digest_c(),
        upper_c = digest_c().to_uppercase(),
        upper_a = digest_a().to_uppercase(),
        zeros = "0".repeat(63),
    );
    let loaded = decode(&yaml);
    assert_eq!(loaded.settings.vpn.trusted_certs, [digest_a(), digest_c()]);
    let invalid = |index| ConfigWarningKind::InvalidTrustedCert { index };
    assert_eq!(
        loaded.warnings,
        [
            warning(invalid(1), 4, 7),
            warning(invalid(2), 5, 7),
            warning(invalid(3), 6, 7),
            warning(invalid(4), 7, 7),
            warning(invalid(5), 8, 7),
            warning(invalid(6), 9, 7),
            warning(invalid(7), 10, 7),
            warning(ConfigWarningKind::DuplicateTrustedCert { index: 9 }, 12, 7),
            warning(invalid(10), 13, 7),
        ]
    );
}

#[test]
fn an_all_digit_certificate_value_stays_text() {
    let digits = "0123456789".repeat(6) + "0123";
    assert_eq!(
        decode(&format!("vpn:\n  trusted_certs: [{digits}]\n"))
            .settings
            .vpn
            .trusted_certs,
        [digits]
    );
}

#[test]
fn empty_certificate_lists() {
    assert!(
        decode("vpn:\n  trusted_certs: []\n")
            .settings
            .vpn
            .trusted_certs
            .is_empty()
    );
    assert!(
        decode("vpn:\n  trusted_certs:\n")
            .settings
            .vpn
            .trusted_certs
            .is_empty()
    );
}

// MARK: Nesting

#[test]
fn deep_flow_nesting_within_the_cap_reads() {
    // saphyr's flow nesting stops at 255 levels (REFACTOR-PLAN §2 exception 2); libyaml's at 1000.
    for depth in [100, 254] {
        let yaml = format!("notes: {}{}\n", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(
            decode(&yaml)
                .warnings
                .into_iter()
                .map(|w| w.kind)
                .collect::<Vec<_>>(),
            [unknown("notes")],
            "{depth}"
        );
    }
}

#[test]
fn nesting_deeper_than_allowed_is_an_error() {
    let depth = 1_200;
    let error = load_error(&format!(
        "vpn:\n  x: {}{}\n",
        "[".repeat(depth),
        "]".repeat(depth)
    ))
    .unwrap();
    assert!(
        matches!(
            error.kind,
            ConfigProblemKind::NestingTooDeep {} | ConfigProblemKind::Syntax { .. }
        ),
        "{:?}",
        error.kind
    );
    assert_eq!(error.location.map(|l| l.line), Some(2));
}

#[test]
fn deep_block_sequences_read_too() {
    // Compact nested block sequences: `- - - … x`, one level per two bytes.
    let yaml = format!("notes:\n{}x\n", "- ".repeat(990));
    assert_eq!(
        decode(&yaml)
            .warnings
            .into_iter()
            .map(|w| w.kind)
            .collect::<Vec<_>>(),
        [unknown("notes")]
    );
}

// MARK: OTP method

#[test]
fn accepted_otp_methods() {
    for (raw, expected) in [
        ("imap", OtpMethod::Imap),
        ("\"imap\"", OtpMethod::Imap),
        ("gmail_api", OtpMethod::GmailApi),
        ("'gmail_api'", OtpMethod::GmailApi),
        ("~", OtpMethod::Imap),
        ("", OtpMethod::Imap),
    ] {
        let loaded = decode(&format!("otp:\n  method: {raw}\n"));
        assert_eq!(loaded.settings.otp.method, expected, "{raw}");
        assert!(loaded.warnings.is_empty());
    }
}

#[test]
fn other_otp_methods_are_errors_at_the_value() {
    for raw in [
        "IMAP",
        "Gmail_API",
        "gmail-api",
        "pop3",
        "\"\"",
        "true",
        "1",
    ] {
        let error = load_error(&format!(
            "otp:\n  gmail_address: a@example.com\n  method: {raw}\n"
        ))
        .unwrap();
        let text = if raw == "\"\"" { "" } else { raw };
        assert_eq!(
            error.kind,
            ConfigProblemKind::InvalidChoice {
                path: "otp.method".into(),
                raw: text.into(),
                allowed: vec!["imap".into(), "gmail_api".into()],
            },
            "{raw}"
        );
        assert_eq!(error.location, loc(3, 11), "{raw}");
    }
}

#[test]
fn missing_otp_keys_keep_the_defaults() {
    let otp = decode("otp:\n  gmail_address: a@example.com\n")
        .settings
        .otp;
    assert_eq!(otp.gmail_address, "a@example.com");
    assert_eq!(otp.method, OtpMethod::Imap);
    assert_eq!(otp.google_client_id, "");
}

#[test]
fn the_client_id_is_text_as_written() {
    let otp = decode("otp:\n  google_client_id: 123-abc.apps.googleusercontent.com\n")
        .settings
        .otp;
    assert_eq!(otp.google_client_id, "123-abc.apps.googleusercontent.com");
    assert_eq!(otp.method, OtpMethod::Imap);
}

#[test]
fn the_client_id_refuses_control_characters() {
    for (literal, scalar) in [
        ("\"id\\n\"", 0x0A),
        ("\"\\tid\"", 0x09),
        ("\"id\\u2028\"", 0x2028),
        ("\"\\x7F\"", 0x7F),
    ] {
        let error = load_error(&format!("otp:\n  google_client_id: {literal}\n")).unwrap();
        assert_eq!(
            error.kind,
            ConfigProblemKind::InvalidCharacter {
                path: "otp.google_client_id".into(),
                scalar
            },
            "{literal}"
        );
        assert_eq!(error.location, loc(2, 21));
    }
}
