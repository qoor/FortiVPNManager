use fvm_contract::app_internal::SecretText;
use fvm_contract::validation::certificate_value;

use super::{ImportedSettings, is_digest, lines, strtol_port};

const DIGEST_A: &str = "4d1b72371c42a69bd949048883897aa673059f2e7b1cc992a650584132aae8f3";
const DIGEST_B: &str = "e0ef56d6eb603d9ffe87e74c28b8210b0ea93839e2ea4fb64f1fc2c66e95e391";
const PIN_A: &str = "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=";
const PIN_B: &str = "pin-sha256:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

fn read(text: &str) -> ImportedSettings {
    super::read(text.as_bytes())
}

fn empty() -> ImportedSettings {
    ImportedSettings {
        host: None,
        port: 443,
        username: None,
        password: None,
        realm: None,
        trusted_cert_pins: vec![],
        unknown_keys: vec![],
        ignored_keys: vec![],
        unreadable_lines: vec![],
    }
}

fn some(text: &str) -> Option<String> {
    Some(text.to_owned())
}

fn password(settings: &ImportedSettings) -> Option<&str> {
    settings.password.as_ref().map(|secret| secret.0.as_str())
}

/// Same shape as the user's real ~/.config/openfortivpn/config (values masked). `insecure-cert`
/// is not an openfortivpn key; it was harmless only because it was the last line.
fn user_file_shape() -> String {
    format!(
        "host = 203.0.113.10\nport = 8443\nusername = alice\npassword = s3cr3t\ntrusted-cert = {DIGEST_A}\ninsecure-cert = 1"
    )
}

#[test]
fn the_fixture_values_are_digests_and_pins() {
    for digest in [DIGEST_A, DIGEST_B] {
        assert!(is_digest(digest), "{digest}");
    }
    for pin in [PIN_A, PIN_B] {
        assert_eq!(certificate_value(pin), Some(pin.to_owned()));
    }
}

#[test]
fn every_value_of_the_user_file_shape_is_imported() {
    assert_eq!(
        read(&user_file_shape()),
        ImportedSettings {
            host: some("203.0.113.10"),
            port: 8443,
            username: some("alice"),
            password: Some(SecretText("s3cr3t".into())),
            unknown_keys: vec!["insecure-cert".into()],
            ..empty()
        }
    );
}

#[test]
fn an_unknown_key_before_other_keys_does_not_stop_reading() {
    let settings = read(&format!(
        "insecure-cert = 1\nhost = 203.0.113.10\nport = 8443\nusername = alice\npassword = s3cr3t\ntrusted-cert = {PIN_A}"
    ));
    assert_eq!(settings.host, some("203.0.113.10"));
    assert_eq!(settings.port, 8443);
    assert_eq!(settings.username, some("alice"));
    assert_eq!(password(&settings), Some("s3cr3t"));
    assert_eq!(settings.trusted_cert_pins, [PIN_A]);
    assert_eq!(settings.unknown_keys, ["insecure-cert"]);
}

#[test]
fn unknown_keys_in_the_middle_are_reported_once_in_order() {
    let settings = read(
        "host = vpn.example.com\ninsecure-cert = 1\nport = 10443\nbogus = x\ninsecure-cert = 0\nusername = alice",
    );
    assert_eq!(settings.host, some("vpn.example.com"));
    assert_eq!(settings.port, 10443);
    assert_eq!(settings.username, some("alice"));
    assert_eq!(settings.unknown_keys, ["insecure-cert", "bogus"]);
    assert_eq!(settings.unreadable_lines, Vec::<u32>::new());
}

#[test]
fn a_missing_port_is_443() {
    assert_eq!(read("host = vpn.example.com\nusername = alice\n").port, 443);
}

#[test]
fn digests_of_either_case_are_skipped_without_a_report() {
    let settings = read(&format!(
        "trusted-cert = {DIGEST_A}\ntrusted-cert = {}\n",
        DIGEST_B.to_uppercase()
    ));
    assert_eq!(settings.trusted_cert_pins, Vec::<String>::new());
    assert_eq!(settings.unreadable_lines, Vec::<u32>::new());
}

#[test]
fn pins_are_kept_in_order_once_and_invalid_values_reported() {
    let settings = read(&format!(
        "trusted-cert = {pin_b}\ntrusted-cert = {pin_a}\ntrusted-cert = {b}\ntrusted-cert = {short}\n\
         trusted-cert = {a}0\ntrusted-cert = {short}g\ntrusted-cert = {head} {tail}\ntrusted-cert =\n\
         trusted-cert = {pin_a}",
        a = DIGEST_A,
        b = DIGEST_B,
        pin_a = PIN_A,
        pin_b = PIN_B,
        short = &DIGEST_A[..63],
        head = &DIGEST_A[..32],
        tail = &DIGEST_A[33..],
    ));
    assert_eq!(settings.trusted_cert_pins, [PIN_B, PIN_A]);
    assert_eq!(settings.unreadable_lines, [4, 5, 6, 7, 8]);
}

#[test]
fn spacing_variants_crlf_and_bom_are_accepted() {
    for text in [
        "host=vpn.example.com\nport=8443\nusername=alice\n",
        "host = vpn.example.com\nport = 8443\nusername = alice\n",
        "  host   =   vpn.example.com   \n\tport\t=\t8443\t\nusername= alice\n",
        "host = vpn.example.com\r\nport = 8443\r\nusername = alice\r\n",
        "host = vpn.example.com\r\n\r\nport = 8443\r\n# comment\r\nusername = alice",
        "\u{FEFF}host = vpn.example.com\nport = 8443\nusername = alice\n",
    ] {
        assert_eq!(
            read(text),
            ImportedSettings {
                host: some("vpn.example.com"),
                port: 8443,
                username: some("alice"),
                ..empty()
            },
            "{text:?}"
        );
    }
}

#[test]
fn comments_and_blank_lines_are_skipped_but_a_hash_inside_a_value_stays() {
    let settings = read(
        "# openfortivpn config\n#host = commented.example.com\n   # indented comment = with equals sign\n\t\n\
         host = vpn.example.com\npassword = p#ss # not a comment\nrealm = staff",
    );
    assert_eq!(settings.host, some("vpn.example.com"));
    assert_eq!(password(&settings), Some("p#ss # not a comment"));
    assert_eq!(settings.realm, some("staff"));
    assert_eq!(settings.unknown_keys, Vec::<String>::new());
    assert_eq!(settings.unreadable_lines, Vec::<u32>::new());
}

#[test]
fn a_line_splits_at_the_first_equals_sign() {
    let settings = read("password = a=b==\nusername==alice\n");
    assert_eq!(password(&settings), Some("a=b=="));
    assert_eq!(settings.username, some("=alice"));
}

#[test]
fn malformed_lines_are_reported_with_one_based_numbers() {
    let settings = read(
        "host = vpn.example.com\nthis line has no equals sign\n= value without key\nport = 8443",
    );
    assert_eq!(settings.host, some("vpn.example.com"));
    assert_eq!(settings.port, 8443);
    assert_eq!(settings.unreadable_lines, [2, 3]);
}

#[test]
fn later_lines_override_earlier_ones_and_empty_values_clear() {
    let settings = read(
        "host = first.example.com\nhost = second.example.com\nusername = alice\nusername =\nrealm =\npassword =\t",
    );
    assert_eq!(settings.host, some("second.example.com"));
    assert_eq!(settings.username, None);
    assert_eq!(settings.realm, None);
    assert_eq!(settings.password, None);
    assert_eq!(settings.unreadable_lines, Vec::<u32>::new());
}

#[test]
fn a_rejected_port_keeps_the_previous_value() {
    let settings = read("port = 8443\nport = 0\nport = 70000\n");
    assert_eq!(settings.port, 8443);
    assert_eq!(settings.unreadable_lines, [2, 3]);
}

#[test]
fn ports_are_read_like_strtol_base_0() {
    for (value, expected) in [
        ("8443", 8443),
        ("1", 1),
        ("65535", 65535),
        ("+443", 443),
        ("8443 # office", 8443), // strtol stops at the first non-digit
        ("8443abc", 8443),
        ("1e3", 1),
        ("0x20FB", 8443), // strtol base 0: hex
        ("0X20fb", 8443),
        ("010", 8), // strtol base 0: octal
        ("0443", 291),
        ("  443", 443),
        ("0x1bbz", 443),
        ("077", 63),
    ] {
        let settings = read(&format!("port = {value}\n"));
        assert_eq!(settings.port, expected, "{value}");
        assert!(settings.unreadable_lines.is_empty(), "{value}");
    }
}

#[test]
fn ports_openfortivpn_refuses_are_reported() {
    for value in [
        "0",
        "-1",
        "-8443",
        "65536",
        "99999999999999999999999",
        "abc",
        "0x",
        "08443",
        "",
        "-0",
        "0xg",
        "00",
        "09",
        "+",
    ] {
        let settings = read(&format!("port = {value}\n"));
        assert_eq!(settings.port, 443, "{value:?}");
        assert_eq!(settings.unreadable_lines, [1], "{value:?}");
    }
}

#[test]
fn ignored_openfortivpn_options_differ_from_unknown_keys() {
    let settings = read(
        "set-dns = 0\npppd-use-peerdns = 1\notp = 123456\npppd = 1\nHost = upper.example.com\nset-dns = 1\nhost = vpn.example.com",
    );
    assert_eq!(
        settings.ignored_keys,
        ["set-dns", "pppd-use-peerdns", "otp"]
    );
    // `pppd` only exists in non-pppd builds; keys are case-sensitive (strcmp).
    assert_eq!(settings.unknown_keys, ["pppd", "Host"]);
    assert_eq!(settings.host, some("vpn.example.com"));
}

#[test]
fn control_characters_in_imported_values_are_reported_not_imported() {
    let settings = read(
        "host = vpn\u{1B}[31m.example.com\nusername = ali\u{85}ce\nrealm = st\u{2028}aff\npassword = tab\tinside",
    );
    assert_eq!(settings.host, None);
    assert_eq!(settings.username, None);
    assert_eq!(settings.realm, None);
    assert_eq!(password(&settings), Some("tab\tinside"));
    assert_eq!(settings.unreadable_lines, [1, 2, 3]);
}

#[test]
fn classic_mac_line_endings_do_not_give_a_host_with_carriage_returns() {
    let settings = read("host = vpn.example.com\rport = 8443\r");
    assert_eq!(settings.host, None);
    assert_eq!(settings.port, 443);
    assert_eq!(settings.unreadable_lines, [1]);
}

#[test]
fn a_nul_byte_makes_the_line_unreadable() {
    let settings = read("host = vpn\0.example.com\nport = 8443\n");
    assert_eq!(settings.host, None);
    assert_eq!(settings.port, 8443);
    assert_eq!(settings.unreadable_lines, [1]);
}

#[test]
fn invalid_utf8_is_reported_per_line_instead_of_decoded_lossily() {
    let mut bytes = b"host = vpn.example.com\npassword = caf".to_vec();
    bytes.push(0xE9); // Latin-1 "é"
    bytes.extend("\n\u{FF}key = 1\n".as_bytes());
    bytes.extend([0xFF, 0x3D, 0x31, 0x0A]); // "\xFF=1"
    bytes.extend(b"username = alice\n");
    let settings = super::read(&bytes);
    assert_eq!(settings.host, some("vpn.example.com"));
    assert_eq!(settings.password, None);
    assert_eq!(settings.username, some("alice"));
    assert_eq!(settings.unknown_keys, ["\u{FF}key"]);
    assert_eq!(settings.unreadable_lines, [2, 4]);
}

#[test]
fn empty_or_comment_only_files_import_nothing() {
    for text in ["", "\n", "\n\n\n", "# only a comment\n", "   "] {
        assert_eq!(read(text), empty(), "{text:?}");
    }
}

#[test]
fn the_last_line_without_a_newline_is_read() {
    assert_eq!(
        read("host = vpn.example.com\nrealm = staff").realm,
        some("staff")
    );
}

#[test]
fn debug_output_redacts_the_password() {
    let debug = format!("{:?}", read(&user_file_shape()));
    assert!(!debug.contains("s3cr3t"));
    assert!(debug.contains("203.0.113.10"));
}

#[test]
fn lines_follow_getline() {
    assert_eq!(lines(b"a\n\nb"), [b"a".as_slice(), b"", b"b"]);
    assert_eq!(lines(b"a\n").len(), 1);
    assert_eq!(lines(b""), Vec::<&[u8]>::new());
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
