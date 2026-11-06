//! Request checks, the config file and the launch, through `Backend::launch_spec` and
//! `Backend::check_otp`.

use std::path::{Path, PathBuf};

use fvm_backend_adapter::openfortivpn::OpenFortiVpn;
use fvm_contract::app_helper::RunRequest;
use fvm_contract::backend::{Backend, ClientExecutable, LaunchSpec};

const DIGEST_A: &str = "4d1b72371c42a69bd949048883897aa673059f2e7b1cc992a650584132aae8f3";
const DIGEST_B: &str = "e0ef56d6eb603d9ffe87e74c28b8210b0ea93839e2ea4fb64f1fc2c66e95e391";
const DIGEST_C: &str = "39610bca9443fdb6ae772d781954447c0bb23c05bcb2f08e72d6cc5f70209726";
const CLIENT_PATH: &str = "/opt/homebrew/Cellar/openfortivpn/1.24.1/bin/openfortivpn";
const CONFIG_PATH: &str = "/var/run/com.qoor.FortiVPNManager.Helper/openfortivpn-1.conf";

const INVALID_HOST: &str = "host must be a DNS hostname (letters, digits and hyphens, labels of 1-63) or a dotted-quad IPv4 address";

fn request() -> RunRequest {
    RunRequest {
        attempt_id: "6F9619FF-8B86-D011-B42D-00C04FC964FF".parse().unwrap(),
        host: "vpn.example.com".into(),
        port: 8443,
        username: "alice".into(),
        realm: None,
        trusted_certs: vec![DIGEST_A.into()],
        password: "s3cr3t".into(),
    }
}

fn with(change: impl FnOnce(&mut RunRequest)) -> RunRequest {
    let mut request = request();
    change(&mut request);
    request
}

fn launch(request: &RunRequest) -> Result<LaunchSpec, String> {
    let client = ClientExecutable {
        path: CLIENT_PATH.into(),
        version: Some("1.24.1".into()),
    };
    OpenFortiVpn.launch_spec(&client, request, Path::new(CONFIG_PATH))
}

fn config(request: &RunRequest) -> String {
    launch(request).unwrap().config_file
}

fn rejected(request: &RunRequest) -> String {
    launch(request).unwrap_err()
}

/// How openfortivpn 1.24.1's load_config() reads a file: lines split at LF only, an empty line or
/// one starting with '#' skipped, split at the first '=', and both sides trimmed of C-locale
/// isspace().
fn read_back(config: &str) -> Vec<(String, String)> {
    let trim = |text: &str| {
        text.trim_matches(|c: char| c == ' ' || ('\t'..='\r').contains(&c))
            .to_owned()
    };
    config
        .split_terminator('\n')
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (key, value) = line.split_once('=').unwrap();
            (trim(key), trim(value))
        })
        .collect()
}

// Launch

#[test]
fn runs_the_located_client_with_only_the_config_file_and_path() {
    let spec = launch(&request()).unwrap();
    assert_eq!(spec.program, PathBuf::from(CLIENT_PATH));
    assert_eq!(spec.args, ["-c", CONFIG_PATH]);
    assert_eq!(
        spec.env,
        [(
            "PATH".to_owned(),
            "/usr/bin:/bin:/usr/sbin:/sbin".to_owned()
        )]
    );
}

#[test]
fn renders_a_minimal_request_exactly() {
    assert_eq!(
        config(&request()),
        format!(
            "host = vpn.example.com\nport = 8443\nusername = alice\ntrusted-cert = {DIGEST_A}\n"
        )
    );
}

#[test]
fn renders_the_realm_and_one_line_per_digest_in_order() {
    let request = with(|r| {
        r.host = "203.0.113.10".into();
        r.port = 443;
        r.realm = Some("staff".into());
        r.trusted_certs = vec![DIGEST_B.into(), DIGEST_A.into(), DIGEST_C.into()];
    });
    assert_eq!(
        config(&request),
        format!(
            "host = 203.0.113.10\nport = 443\nusername = alice\nrealm = staff\n\
             trusted-cert = {DIGEST_B}\ntrusted-cert = {DIGEST_A}\ntrusted-cert = {DIGEST_C}\n"
        )
    );
}

#[test]
fn renders_no_digest_for_the_first_connection() {
    let request = with(|r| r.trusted_certs.clear());
    assert_eq!(
        config(&request),
        "host = vpn.example.com\nport = 8443\nusername = alice\n"
    );
}

#[test]
fn leaves_out_a_missing_or_empty_realm() {
    for realm in [None, Some(String::new())] {
        let request = with(|r| r.realm = realm.clone());
        assert!(!config(&request).contains("realm"), "{realm:?}");
    }
}

#[test]
fn never_writes_the_password_or_another_key() {
    let password = "Pa55word-that-must-not-leak";
    let request = with(|r| {
        r.realm = Some("staff".into());
        r.trusted_certs = vec![DIGEST_A.into(), DIGEST_B.into()];
        r.password = password.into();
    });
    let spec = launch(&request).unwrap();
    assert!(!spec.config_file.contains(password));
    assert!(!spec.config_file.contains("password"));
    assert!(!spec.args.iter().any(|arg| arg.contains(password)));
    assert!(!spec.env.iter().any(|(_, value)| value.contains(password)));
    for (key, _) in read_back(&spec.config_file) {
        assert!(
            ["host", "port", "username", "realm", "trusted-cert"].contains(&key.as_str()),
            "{key}"
        );
    }
    assert!(spec.config_file.ends_with('\n'));
    assert!(!spec.config_file.contains('\r'));
}

#[test]
fn openfortivpn_reads_every_value_back_unchanged() {
    let requests = [
        request(),
        with(|r| {
            r.host = "203.0.113.10".into();
            r.port = 1;
            r.realm = Some("staff".into());
            r.trusted_certs = vec![DIGEST_A.into(), DIGEST_B.into()];
        }),
        with(|r| {
            r.host = "VPN-1.Corp.Example.COM".into();
            r.port = 65_535;
            r.username = "bob=x#y".into();
            r.realm = Some("r=1#2".into());
            r.trusted_certs.clear();
        }),
        with(|r| {
            r.username = "#alice smith".into();
            r.realm = Some("\u{C9C1}\u{C6D0}".into());
            r.trusted_certs = vec![DIGEST_C.into(), DIGEST_B.into(), DIGEST_A.into()];
        }),
        with(|r| {
            r.username = "a".repeat(64);
            r.realm = Some("r".repeat(63));
        }),
        with(|r| {
            r.host = format!("{}a", "a.".repeat(126));
            r.username = "\u{AC00}".repeat(21);
        }),
    ];
    for request in requests {
        let mut expected = vec![
            ("host".to_owned(), request.host.clone()),
            ("port".to_owned(), request.port.to_string()),
            ("username".to_owned(), request.username.clone()),
        ];
        expected.extend(
            request
                .realm
                .clone()
                .map(|realm| ("realm".to_owned(), realm)),
        );
        expected.extend(
            request
                .trusted_certs
                .iter()
                .map(|value| ("trusted-cert".to_owned(), value.clone())),
        );
        assert_eq!(read_back(&config(&request)), expected, "{request:?}");
    }
}

#[test]
fn allows_eight_digests() {
    let digests: Vec<String> = (0..8).map(|n| n.to_string().repeat(64)).collect();
    let request = with(|r| r.trusted_certs = digests);
    assert_eq!(config(&request).matches("trusted-cert = ").count(), 8);
}

// Host

#[test]
fn accepts_host_names_and_ipv4_addresses() {
    for host in [
        "vpn.example.com".to_owned(),
        "VPN.Example.COM".to_owned(),
        "vpn".to_owned(),
        "localhost".to_owned(),
        "vpn-1.example.com".to_owned(),
        "10.example.com".to_owned(),
        "203.0.113.10".to_owned(),
        "0.0.0.0".to_owned(),
        "255.255.255.255".to_owned(),
        "1.2.3.4".to_owned(),
        format!("{}.com", "a".repeat(63)),
        format!("{}a", "a.".repeat(126)), // 253 bytes
    ] {
        assert!(launch(&with(|r| r.host = host.clone())).is_ok(), "{host}");
    }
}

#[test]
fn rejects_hosts() {
    let too_long = format!("{}ab", "a.".repeat(126));
    let label_too_long = format!("{}.com", "a".repeat(64));
    let cases: Vec<(&str, &str)> = vec![
        ("", "host must not be empty"),
        (&too_long, "host must be at most 253 UTF-8 bytes"),
        (
            " vpn.example.com",
            "host must not start or end with whitespace",
        ),
        (
            "vpn.example.com ",
            "host must not start or end with whitespace",
        ),
        (
            "vpn.example.com\u{A0}",
            "host must not start or end with whitespace",
        ),
        (
            "vpn.example.com\npppd-plugin = /tmp/x.so",
            "host contains forbidden character U+000A",
        ),
        (
            "vpn.example.com\r",
            "host contains forbidden character U+000D",
        ),
        (
            "vpn.example.com\t",
            "host contains forbidden character U+0009",
        ),
        (
            "vpn\0.example.com",
            "host contains forbidden character U+0000",
        ),
        (
            "vpn.example.com\u{85}",
            "host contains forbidden character U+0085",
        ),
        (
            "vpn.example.com\u{2028}x",
            "host contains forbidden character U+2028",
        ),
        (
            "vpn.example.com\u{2029}x",
            "host contains forbidden character U+2029",
        ),
        (
            "\u{FEFF}vpn.example.com",
            "host contains forbidden character U+FEFF",
        ),
        ("vpn example.com", INVALID_HOST),
        ("-vpn.example.com", INVALID_HOST),
        ("vpn-.example.com", INVALID_HOST),
        ("vpn..example.com", INVALID_HOST),
        (".vpn.example.com", INVALID_HOST),
        ("vpn.example.com.", INVALID_HOST),
        ("vpn_1.example.com", INVALID_HOST),
        (&label_too_long, INVALID_HOST),
        ("vpn.example.com:443", INVALID_HOST),
        ("vpn.ex#ample.com", INVALID_HOST),
        ("host=evil.example.com", INVALID_HOST),
        ("vpn.ex\u{E4}mple.com", INVALID_HOST),
        ("\u{FF56}\u{FF50}\u{FF4E}.example.com", INVALID_HOST),
        ("::1", INVALID_HOST),
        ("[::1]", INVALID_HOST),
        ("1.2.3", INVALID_HOST),
        ("127.1", INVALID_HOST),
        ("1.2.3.4.5", INVALID_HOST),
        ("256.1.1.1", INVALID_HOST),
        ("999.1.1.1", INVALID_HOST),
        ("010.0.0.1", INVALID_HOST),
        ("1.2.3.04", INVALID_HOST),
        ("0x7f000001", INVALID_HOST),
        ("0x7f.1", INVALID_HOST),
        ("10.0.0.0x1", INVALID_HOST),
        ("vpn.0x", INVALID_HOST),
        // A gateway pasted from the browser.
        ("https://vpn.example.com", INVALID_HOST),
        ("https://vpn.example.com/", INVALID_HOST),
        ("http://vpn.example.com:10443", INVALID_HOST),
        ("vpn.example.com:10443", INVALID_HOST),
        ("vpn.example.com/remote/login", INVALID_HOST),
    ];
    for (host, message) in cases {
        assert_eq!(
            rejected(&with(|r| r.host = host.into())),
            message,
            "{host:?}"
        );
    }
}

// Port

#[test]
fn accepts_ports_in_range() {
    for port in [1, 443, 8443, 65_535] {
        assert!(launch(&with(|r| r.port = port)).is_ok(), "{port}");
    }
}

#[test]
fn rejects_ports_out_of_range() {
    for port in [0, -1, 65_536, i64::MAX, i64::MIN] {
        assert_eq!(
            rejected(&with(|r| r.port = port)),
            "port must be between 1 and 65535",
            "{port}"
        );
    }
}

// Username

#[test]
fn accepts_usernames() {
    for username in [
        "a".to_owned(),
        "a".repeat(64),
        "\u{AC00}".repeat(21), // 63 bytes
        "alice smith".to_owned(),
        "bob=x".to_owned(),
        "#bob".to_owned(),
        "bob#1".to_owned(),
        "alice@corp.example".to_owned(),
        "\u{540D}\u{524D}".to_owned(),
        "DOMAIN\\alice".to_owned(),
    ] {
        assert!(
            launch(&with(|r| r.username = username.clone())).is_ok(),
            "{username}"
        );
    }
}

#[test]
fn rejects_usernames() {
    let too_long = "a".repeat(65);
    let too_long_in_bytes = "\u{AC00}".repeat(22); // 66 bytes
    let cases: Vec<(&str, &str)> = vec![
        ("", "username must not be empty"),
        (&too_long, "username must be at most 64 UTF-8 bytes"),
        (
            &too_long_in_bytes,
            "username must be at most 64 UTF-8 bytes",
        ),
        (
            "bob\npppd-plugin = /tmp/x.so",
            "username contains forbidden character U+000A",
        ),
        (
            "bob\r\npppd-plugin = /tmp/x.so",
            "username contains forbidden character U+000D",
        ),
        ("bob\0", "username contains forbidden character U+0000"),
        ("bob\u{0B}x", "username contains forbidden character U+000B"),
        ("bob\u{0C}x", "username contains forbidden character U+000C"),
        ("bob\tx", "username contains forbidden character U+0009"),
        (
            "bob\u{1B}[31m",
            "username contains forbidden character U+001B",
        ),
        ("bob\u{7F}", "username contains forbidden character U+007F"),
        ("bob\u{85}x", "username contains forbidden character U+0085"),
        ("bob\u{9F}x", "username contains forbidden character U+009F"),
        (
            "bob\u{2028}x",
            "username contains forbidden character U+2028",
        ),
        (
            "bob\u{2029}x",
            "username contains forbidden character U+2029",
        ),
        (
            "\u{FEFF}bob",
            "username contains forbidden character U+FEFF",
        ),
        (" bob", "username must not start or end with whitespace"),
        ("bob ", "username must not start or end with whitespace"),
        (
            "\u{A0}bob",
            "username must not start or end with whitespace",
        ),
        (
            "bob\u{3000}",
            "username must not start or end with whitespace",
        ),
        (
            "bob\u{2003}",
            "username must not start or end with whitespace",
        ),
    ];
    for (username, message) in cases {
        assert_eq!(
            rejected(&with(|r| r.username = username.into())),
            message,
            "{username:?}"
        );
    }
}

// Realm

#[test]
fn accepts_realms() {
    for realm in [
        None,
        Some(String::new()),
        Some("staff".into()),
        Some("r".repeat(63)),
    ] {
        assert!(
            launch(&with(|r| r.realm = realm.clone())).is_ok(),
            "{realm:?}"
        );
    }
}

#[test]
fn rejects_realms() {
    let too_long = "r".repeat(64);
    let cases: Vec<(&str, &str)> = vec![
        (&too_long, "realm must be at most 63 UTF-8 bytes"),
        (
            "staff\nhost = evil.example.com",
            "realm contains forbidden character U+000A",
        ),
        ("staff\0", "realm contains forbidden character U+0000"),
        ("staff\u{2028}", "realm contains forbidden character U+2028"),
        (" staff", "realm must not start or end with whitespace"),
        ("staff ", "realm must not start or end with whitespace"),
    ];
    for (realm, message) in cases {
        assert_eq!(
            rejected(&with(|r| r.realm = Some(realm.into()))),
            message,
            "{realm:?}"
        );
    }
}

// Trusted certificate values

#[test]
fn rejects_trusted_cert_values() {
    let not_hex =
        |index: usize| format!("trusted-cert entry {index} is not 64 lowercase hex characters");
    let cases: Vec<(Vec<String>, String)> = vec![
        (vec![DIGEST_A.to_uppercase()], not_hex(0)),
        (vec![DIGEST_A[..63].into()], not_hex(0)),
        (vec![format!("{DIGEST_A}0")], not_hex(0)),
        (vec![format!("{}g", &DIGEST_A[..63])], not_hex(0)),
        (vec![format!("{DIGEST_A} ")], not_hex(0)),
        (vec![format!(" {}", &DIGEST_A[1..])], not_hex(0)),
        (vec![String::new()], not_hex(0)),
        (
            vec![format!("{DIGEST_A}\nhost = evil.example.com")],
            not_hex(0),
        ),
        (vec![format!("{}\n", &DIGEST_A[..63])], not_hex(0)),
        (
            vec![
                DIGEST_A.into(),
                DIGEST_B.into(),
                format!("sha256:{DIGEST_C}"),
            ],
            not_hex(2),
        ),
        (vec![DIGEST_A.into(), "\u{FF10}".repeat(64)], not_hex(1)),
        (
            vec![DIGEST_A.into(); 9],
            "trusted-cert allows at most 8 digests".into(),
        ),
    ];
    for (values, message) in cases {
        assert_eq!(
            rejected(&with(|r| r.trusted_certs = values.clone())),
            message,
            "{values:?}"
        );
    }
}

// Password

#[test]
fn accepts_passwords() {
    for password in [
        "s3cr3t".to_owned(),
        "a".repeat(255),
        "\u{AC00}".repeat(85), // 255 bytes
        " spaced password ".to_owned(),
        "p=ss#word".to_owned(),
        "tab\tinside".to_owned(),
        "\u{FC}n\u{EF}c\u{F6}d\u{E9}-p\u{E4}ssw\u{F6}rd".to_owned(),
        "\u{2028}".to_owned(),
    ] {
        assert!(
            launch(&with(|r| r.password = password.clone())).is_ok(),
            "{password:?}"
        );
    }
}

#[test]
fn rejects_passwords() {
    let too_long = "a".repeat(256);
    let too_long_in_bytes = "\u{AC00}".repeat(86); // 258 bytes
    let too_long_with_line_break = format!("{}\n", "a".repeat(300));
    let cases: Vec<(&str, &str)> = vec![
        ("", "password must not be empty"),
        ("pass\nword", "password contains forbidden character U+000A"),
        ("password\n", "password contains forbidden character U+000A"),
        ("pass\rword", "password contains forbidden character U+000D"),
        (
            "pass\r\nword",
            "password contains forbidden character U+000D",
        ),
        ("pass\0word", "password contains forbidden character U+0000"),
        (&too_long, "password must be at most 255 UTF-8 bytes"),
        (
            &too_long_in_bytes,
            "password must be at most 255 UTF-8 bytes",
        ),
        (
            &too_long_with_line_break,
            "password contains forbidden character U+000A",
        ),
    ];
    for (password, message) in cases {
        assert_eq!(
            rejected(&with(|r| r.password = password.into())),
            message,
            "{password:?}"
        );
    }
}

// Errors

#[test]
fn reports_the_first_invalid_field_in_file_order() {
    let request = with(|r| {
        r.host = "bad host".into();
        r.port = 0;
        r.username = String::new();
        r.trusted_certs = vec!["x".into()];
        r.password = String::new();
    });
    assert_eq!(rejected(&request), INVALID_HOST);
    let request = with(|r| {
        r.trusted_certs = vec!["x".into()];
        r.password = String::new();
    });
    assert_eq!(
        rejected(&request),
        "trusted-cert entry 0 is not 64 lowercase hex characters"
    );
}

#[test]
fn errors_name_the_field_but_never_the_value() {
    let secret = "hunter2-very-secret";
    let cases = [
        with(|r| r.password = format!("{secret}\n")),
        with(|r| r.password = secret.repeat(20)),
        with(|r| r.username = format!("{secret}\npppd-plugin = /tmp/x.so")),
        with(|r| r.username = format!("{secret}\u{2028}")),
        with(|r| r.host = format!("{secret}_host")),
        with(|r| r.realm = Some(format!(" {secret}"))),
        with(|r| r.trusted_certs = vec![DIGEST_A.into(), secret.into()]),
    ];
    for request in cases {
        assert!(!rejected(&request).contains(secret));
    }
}

// OTP

#[test]
fn accepts_six_to_eight_ascii_digits_as_otp() {
    for code in ["123456", "0000000", "12345678"] {
        assert_eq!(OpenFortiVpn.check_otp(code), Ok(()), "{code}");
    }
}

#[test]
fn rejects_other_otp() {
    for code in [
        "",
        "12345",
        "123456789",
        "12345a",
        " 123456",
        "123456\n",
        "\u{FF11}\u{FF12}",                 // 6 bytes
        "\u{0661}\u{0662}\u{0663}\u{0664}", // 8 bytes
    ] {
        assert_eq!(
            OpenFortiVpn.check_otp(code),
            Err("OTP must be 6 to 8 ASCII digits".to_owned()),
            "{code:?}"
        );
    }
}
