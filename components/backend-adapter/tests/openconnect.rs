//! openconnect's launch, value checks and output, with output taken from openconnect 9.21 runs
//! against a FortiGate (2026-11-10, 11-11 and 11-12); addresses, names, keys and cookies are
//! replaced.

mod common;

use std::net::Ipv4Addr;
use std::path::Path;

use common::Run;
use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_contract::app_helper::{AttemptId, Milestone, OutputSource, RunRequest};
use fvm_contract::app_internal::problem::{AccountField, AccountValueProblem};
use fvm_contract::app_internal::section::AccountSaveError;
use fvm_contract::backend::{
    Backend, ClientExecutable, Finding, LaunchSpec, OutputInterpreter, Prompt, SessionInputs,
};
use zeroize::Zeroizing;

const CLIENT_PATH: &str = r"C:\Program Files\OpenConnect\openconnect.exe";
const PIN: &str = "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=";
const OTHER_PIN: &str = "pin-sha256:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const PLACEHOLDER_PIN: &str = "pin-sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSWORD: &str = " Pa55 w#rd ";

fn request() -> RunRequest {
    RunRequest {
        attempt_id: "6B1D9B3E-2C1F-4E55-9C0B-3F0A2B4C5D6E"
            .parse::<AttemptId>()
            .unwrap(),
        host: "203.0.113.10".into(),
        port: 8443,
        username: "alice".into(),
        realm: None,
        trusted_certs: vec![PIN.into()],
        password: PASSWORD.into(),
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
        version: "9.21".into(),
    };
    OpenConnect.launch_spec(&client, request, Path::new("unused"))
}

mod launching {
    use super::*;

    /// The login run prints the session; the tunnel run reads it on stdin and gives up reconnecting
    /// at once.
    #[test]
    fn logs_in_then_opens_the_tunnel_with_the_session() {
        let spec = launch(&request()).unwrap();
        assert_eq!(spec.program, Path::new(CLIENT_PATH));
        assert_eq!(
            spec.args,
            [
                "--protocol=fortinet",
                "--user=alice",
                &format!("--servercert={PIN}"),
                "--authenticate",
                "https://203.0.113.10:8443/",
            ]
        );
        assert_eq!(spec.config_file, None);
        let later = spec.session.expect("the runs after the login");
        assert_eq!(later.tunnel.program, Path::new(CLIENT_PATH));
        assert_eq!(
            later.tunnel.args,
            [
                "--protocol=fortinet",
                &format!("--servercert={PIN}"),
                "--reconnect-timeout=0",
                "--cookie-on-stdin",
                "https://203.0.113.10:8443/",
            ]
        );
        assert_eq!(later.tunnel.env, spec.env);
        assert_eq!(
            (later.tunnel.config_file, later.tunnel.session),
            (None, None)
        );
    }

    /// curl with the gateway's pins, the cookie as a header on stdin and only the status printed.
    #[test]
    fn the_logout_request_checks_the_pins_and_reads_the_cookie_from_stdin() {
        let spec = launch(&with(|r| {
            r.trusted_certs = vec![PIN.into(), OTHER_PIN.into()]
        }))
        .unwrap();
        let logout = spec.session.unwrap().logout;
        let pins = format!(
            "sha256//{};sha256//{}",
            PIN.strip_prefix("pin-sha256:").unwrap(),
            OTHER_PIN.strip_prefix("pin-sha256:").unwrap()
        );
        assert!(logout.program.ends_with("curl.exe") || logout.program.ends_with("curl"));
        let args: Vec<&str> = logout.args.iter().map(String::as_str).collect();
        assert_eq!(
            args[..7],
            ["-q", "-s", "-k", "--noproxy", "*", "--max-time", "10"]
        );
        assert_eq!(args[7..11], ["--pinnedpubkey", &pins, "-H", "@-"]);
        // The body goes to the OS's null device.
        assert_eq!(args[11], "-o");
        assert_eq!(
            args[13..],
            [
                "-w",
                "HTTP %{http_code}",
                "https://203.0.113.10:8443/remote/logout"
            ]
        );
        assert_eq!((logout.config_file, logout.session), (None, None));
        assert!(!logout.args.iter().any(|arg| arg.contains(PASSWORD.trim())));
    }

    #[test]
    fn the_realm_is_the_url_path() {
        let spec = launch(&with(|r| {
            r.host = "vpn.example.com".into();
            r.port = 443;
            r.realm = Some("staff".into());
        }))
        .unwrap();
        assert_eq!(
            spec.args.last().unwrap(),
            "https://vpn.example.com:443/staff"
        );
        let spec = launch(&with(|r| r.realm = Some(String::new()))).unwrap();
        assert_eq!(spec.args.last().unwrap(), "https://203.0.113.10:8443/");
    }

    /// Without any pin openconnect waits for an answer that never comes.
    #[test]
    fn always_passes_at_least_one_pin() {
        let pins = |request: &RunRequest| -> Vec<String> {
            launch(request)
                .unwrap()
                .args
                .into_iter()
                .filter_map(|arg| Some(arg.strip_prefix("--servercert=")?.to_owned()))
                .collect()
        };
        assert_eq!(pins(&with(|r| r.trusted_certs.clear())), [PLACEHOLDER_PIN]);
        assert_eq!(
            pins(&with(
                |r| r.trusted_certs = vec![PIN.into(), OTHER_PIN.into()]
            )),
            [PIN, OTHER_PIN]
        );
    }

    #[test]
    fn never_passes_the_password_or_a_proxy() {
        let spec = launch(&request()).unwrap();
        assert!(!spec.args.iter().any(|arg| arg.contains(PASSWORD.trim())));
        assert!(
            !spec
                .env
                .iter()
                .any(|(_, value)| value.contains(PASSWORD.trim()))
        );
        let allowed = [
            "SystemRoot",
            "SystemDrive",
            "windir",
            "ComSpec",
            "PATH",
            "PATHEXT",
            "TEMP",
            "TMP",
        ];
        for (name, _) in &spec.env {
            assert!(allowed.contains(&name.as_str()), "{name}");
        }
    }

    #[test]
    fn refuses_values_openconnect_would_not_take_as_given() {
        for (request, reason) in [
            (
                with(|r| r.host = "https://vpn.example.com".into()),
                "host must be a DNS hostname (letters, digits and hyphens, labels of 1-63) or a dotted-quad IPv4 address",
            ),
            (with(|r| r.port = 0), "port must be between 1 and 65535"),
            (
                with(|r| r.username = "alicë".into()),
                "username contains forbidden character U+00EB",
            ),
            (
                with(|r| r.username = "ali\tce".into()),
                "username contains forbidden character U+0009",
            ),
            (
                with(|r| r.realm = Some("a/b".into())),
                "realm contains forbidden character U+002F",
            ),
            (
                with(|r| r.realm = Some("a b".into())),
                "realm contains forbidden character U+0020",
            ),
            (
                with(|r| r.trusted_certs = vec![PIN.into(), "VU1WXtWq".into()]),
                "servercert entry 1 is not a pin-sha256: value",
            ),
            (
                with(|r| r.trusted_certs = vec![PIN.into(); 9]),
                "servercert allows at most 8 pins",
            ),
            (
                with(|r| r.password = String::new()),
                "password must not be empty",
            ),
            (
                with(|r| r.password = "pässword".into()),
                "password contains forbidden character U+00E4",
            ),
            (
                with(|r| r.password = "pass\nword".into()),
                "password contains forbidden character U+000A",
            ),
            (
                with(|r| r.password = "p".repeat(256)),
                "password must be at most 255 bytes",
            ),
        ] {
            assert_eq!(launch(&request), Err(reason.to_owned()), "{request:?}");
        }
        assert!(launch(&with(|r| r.password = "p".repeat(255))).is_ok());
    }

    #[test]
    fn otp_is_six_to_eight_digits() {
        for code in ["008056", "1234567", "12345678"] {
            assert_eq!(OpenConnect.check_otp(code), Ok(()), "{code}");
        }
        for code in ["12345", "123456789", "12345a", "١٢٣٤٥٦"] {
            assert!(OpenConnect.check_otp(code).is_err(), "{code}");
        }
    }
}

mod values {
    use super::*;

    #[test]
    fn certificate_values_are_canonical_sha256_pins() {
        for pin in [PIN, OTHER_PIN, PLACEHOLDER_PIN] {
            assert_eq!(OpenConnect.certificate_value(pin), Some(pin.to_owned()));
        }
        for text in [
            "",
            "VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx34=",
            "PIN-SHA256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx34=",
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx34",
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A==",
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A= ",
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu-T+caKs4wthkccwXx3A=",
            // The last character carries bits past the 32 bytes.
            "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx35=",
            "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11",
        ] {
            assert_eq!(OpenConnect.certificate_value(text), None, "{text}");
        }
    }

    #[test]
    fn account_values_follow_the_launch_rules() {
        assert_eq!(OpenConnect.check_account("", "", ""), Ok(()));
        assert_eq!(
            OpenConnect.check_account("203.0.113.10", "alice", "staff-1.a_b~c"),
            Ok(())
        );
        assert_eq!(
            OpenConnect.check_account("vpn.example.com:8443", "alice", ""),
            Err(AccountValueProblem::InvalidHost {})
        );
        assert_eq!(
            OpenConnect.check_account("vpn.example.com", "alicë", ""),
            Err(AccountValueProblem::DisallowedCharacters {
                field: AccountField::Username
            })
        );
        assert_eq!(
            OpenConnect.check_account("vpn.example.com", "alice", "a/b"),
            Err(AccountValueProblem::DisallowedCharacters {
                field: AccountField::Realm
            })
        );
        assert_eq!(
            OpenConnect.check_account("vpn.example.com", " alice", ""),
            Err(AccountValueProblem::LeadingOrTrailingSpace {
                field: AccountField::Username
            })
        );
    }

    #[test]
    fn a_typed_password_must_be_printable_ascii() {
        assert_eq!(OpenConnect.check_vpn_password(""), Ok(()));
        assert_eq!(OpenConnect.check_vpn_password(PASSWORD), Ok(()));
        for refused in ["pässword".to_owned(), "a\nb".to_owned(), "p".repeat(256)] {
            assert_eq!(
                OpenConnect.check_vpn_password(&refused),
                Err(AccountSaveError::InvalidVpnPassword { max_bytes: 255 }),
                "{refused:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn only_the_installed_client_is_ours() {
        assert_eq!(OpenConnect.client_name(), "openconnect");
        for path in [CLIENT_PATH, r"c:\program files\openconnect\OPENCONNECT.EXE"] {
            assert!(OpenConnect.is_own_executable(Path::new(path)), "{path}");
        }
        for path in [
            r"C:\Program Files\OpenConnect\other.exe",
            r"C:\Users\alice\openconnect.exe",
            "/opt/homebrew/bin/openconnect",
        ] {
            assert!(!OpenConnect.is_own_executable(Path::new(path)), "{path}");
        }
    }

    /// openconnect is not supported on this OS yet.
    #[cfg(not(windows))]
    #[test]
    fn no_client_is_ours_here_yet() {
        assert!(!OpenConnect.is_own_executable(Path::new(CLIENT_PATH)));
        assert!(!OpenConnect.is_own_executable(Path::new("/opt/homebrew/bin/openconnect")));
        let problem = OpenConnect.locate_client().unwrap_err();
        assert_eq!(problem.path, Path::new("openconnect"));
    }

    #[test]
    fn versions_outside_9_21_are_untested() {
        for tested in ["9.21", "9.21.1", "9.21-12-gabcdef"] {
            assert_eq!(OpenConnect.untested_version(tested), None, "{tested}");
        }
        for untested in ["9.20", "9.210", "9.12", "10.0"] {
            assert_eq!(
                OpenConnect
                    .untested_version(untested)
                    .map(|version| version.tested_series),
                Some("9.21".to_owned()),
                "{untested}"
            );
        }
    }
}

mod output {
    use super::*;

    use OutputSource::{Stderr, Stdout};

    /// "네트워크" in the PC's code page (CP949), as Windows error messages arrive.
    const CP949_TEXT: &[u8] = b"\xb3\xd7\xc6\xae\xbf\xf6\xc5\xa9";

    fn run() -> Run<fvm_backend_adapter::openconnect::Interpreter> {
        Run::with(OpenConnect.interpreter())
    }

    /// Reads in order; returns the prompts reported.
    fn replay(
        run: &mut Run<fvm_backend_adapter::openconnect::Interpreter>,
        reads: &[(OutputSource, &[u8])],
    ) -> Vec<Prompt> {
        reads
            .iter()
            .filter_map(|&(source, bytes)| run.read(source, bytes))
            .collect()
    }

    const COOKIE: &str = "SVPNCOOKIE=Ab12Cd34Ef56";

    /// The login run prints its progress on stderr and only the session lines on stdout.
    const LOGIN_RUN: [(OutputSource, &[u8]); 10] = [
        (Stderr, b"GET https://203.0.113.10:8443/\n"),
        (Stderr, b"Connected to 203.0.113.10:8443\nSSL negotiation with 203.0.113.10\n"),
        (Stderr, b"Server certificate verify failed: signer not found\n"),
        (
            Stderr,
            b"Connected to HTTPS on 203.0.113.10 with ciphersuite (TLS1.3)-(ECDHE-SECP384R1)-(RSA-PSS-RSAE-SHA256)-(AES-256-GCM)\n",
        ),
        (Stderr, b"GET https://203.0.113.10:8443/remote/login\n"),
        (Stderr, b"Password: "),
        (Stderr, b"POST https://203.0.113.10:8443/remote/logincheck\n\n"),
        (Stderr, b"Code: "),
        (Stderr, b"POST https://203.0.113.10:8443/remote/logincheck\n"),
        (
            Stdout,
            b"COOKIE='SVPNCOOKIE=Ab12Cd34Ef56'\nHOST='203.0.113.10'\nCONNECT_URL='https://203.0.113.10:8443/'\nFINGERPRINT='pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A='\nRESOLVE='203.0.113.10:203.0.113.10'",
        ),
    ];

    /// The tunnel run, with the session on stdin, until the tunnel's adapter exists and the route
    /// script outlived openconnect's 10-second limit.
    const TUNNEL_RUN: [(OutputSource, &[u8]); 9] = [
        (
            Stdout,
            b"GET https://203.0.113.10:8443/remote/fortisslvpn_xml?dual_stack=1\nConnected to 203.0.113.10:8443\nSSL negotiation with 203.0.113.10\nServer certificate verify failed: signer not found\n",
        ),
        (
            Stdout,
            b"Connected to HTTPS on 203.0.113.10 with ciphersuite (TLS1.3)-(ECDHE-SECP384R1)-(RSA-PSS-RSAE-SHA256)-(AES-256-GCM)\nDTLS is enabled on port 8443\nServer reports that reconnect-after-drop is not allowed. OpenConnect will not\nbe able to reconnect if dead peer is detected. If reconnection DOES work,\nplease report to <openconnect-devel@lists.infradead.org>\n",
        ),
        (
            Stdout,
            b"Got Legacy IP address 10.10.0.5\nGot IPv4 route 10.20.161.0/255.255.255.0\nIdle timeout is 50 minutes.\nReceived split routes; not setting default Legacy IP route\n",
        ),
        (
            Stdout,
            b"Established DTLS connection (using GnuTLS). Ciphersuite (DTLS1.2)-(ECDHE-SECP384R1)-(RSA-PSS-RSAE-SHA256)-(AES-256-GCM).\nRequesting calculated MTU of 1351\n",
        ),
        (
            Stdout,
            b"Configured as 10.10.0.5, with SSL disconnected and DTLS established\n",
        ),
        (
            Stdout,
            b"Session authentication will expire at \xbf\xc0\xc0\xfc, 12 11 2026 17:57:05\n",
        ),
        (Stdout, b"0: Using existing driver 0.14\n0: Creating adapter\n"),
        (
            Stderr,
            b"Using Wintun device '203.0.113.10', index 54\nWARNING: Support for Wintun is experimental and may be unstable. If you\n  encounter problems, install the TAP-Windows driver instead. See\n  https://www.infradead.org/openconnect/building.html\n",
        ),
        (
            Stderr,
            b"Script did not complete within 10 seconds.\nFailed to spawn script 'C:\\Program Files\\OpenConnect\\vpnc-script-win.js' for connect: \xc0\xdf\xb8\xf8\xb5\xc8 \xb1\xe2\xb4\xc9\xc0\xd4\xb4\xcf\xb4\xd9.\nUnexpected pre-PPP packet header for encap 5.\n",
        ),
    ];

    fn session() -> Finding {
        Finding::Session(SessionInputs {
            tunnel: Zeroizing::new(format!("{COOKIE}\n").into_bytes()),
            logout: Zeroizing::new(format!("Cookie: {COOKIE}\n").into_bytes()),
        })
    }

    fn logged_in() -> Vec<Finding> {
        vec![
            Finding::Milestone(Milestone::ConnectedToGateway),
            session(),
            // The tunnel run connects again; the watchdog keeps the furthest milestone.
            Finding::Milestone(Milestone::ConnectedToGateway),
            Finding::Milestone(Milestone::Authenticated),
            Finding::TunnelIp(Ipv4Addr::new(10, 10, 0, 5)),
            Finding::Milestone(Milestone::TunnelUp),
        ]
    }

    #[test]
    fn a_login_reports_each_step_and_the_prompts() {
        let mut run = run();
        assert_eq!(
            replay(&mut run, &LOGIN_RUN),
            [Prompt::Password, Prompt::Otp]
        );
        assert_eq!(replay(&mut run, &TUNNEL_RUN), []);
        assert_eq!(run.findings, logged_in());
        assert!(run.interpreter.routes_left_unrestored());
    }

    /// Text without a line break before the session line, such as a prompt, does not carry the
    /// cookie into the log; a session line that cannot be passed on is hidden all the same.
    #[test]
    fn a_session_line_is_never_shown() {
        let mut run = run();
        run.read(Stdout, format!("Code: COOKIE='{COOKIE}'\n").as_bytes());
        run.read(Stdout, format!("COOKIE='{COOKIE}\n").as_bytes());
        run.read(Stdout, b"COOKIE='a'b'\nCOOKIE=''\n");
        assert_eq!(
            run.findings,
            [session(), Finding::Hidden, Finding::Hidden, Finding::Hidden]
        );
    }

    /// The gateway refuses the dropped session; openconnect logs out and exits.
    #[test]
    fn a_drop_after_the_tunnel_reports_nothing_new() {
        let mut run = run();
        replay(&mut run, &LOGIN_RUN);
        replay(&mut run, &TUNNEL_RUN);
        let mut refused = b"Failed to reconnect to host 203.0.113.10: ".to_vec();
        refused.extend_from_slice(CP949_TEXT);
        refused.push(b'\n');
        let tail: [(OutputSource, &[u8]); 9] = [
            (
                Stderr,
                b"Write error on DTLS session: Error in the push function.\nConnect UDP socket: No error\nFailed to connect DTLS tunnel; using HTTPS instead (state 3).\n",
            ),
            (
                Stdout,
                b"GET https://203.0.113.10:8443/remote/fortisslvpn_xml?dual_stack=1\n",
            ),
            (Stderr, &refused),
            (
                Stderr,
                b"Failed to open HTTPS connection to 203.0.113.10\n",
            ),
            (
                Stdout,
                b"SSL negotiation with 203.0.113.10\nServer certificate verify failed: signer not found\nConnected to HTTPS on 203.0.113.10 with ciphersuite (TLS1.3)-(ECDHE-SECP384R1)-(RSA-PSS-RSAE-SHA256)-(AES-256-GCM)\nGot HTTP response: HTTP/1.1 403 Forbidden\n",
            ),
            (
                Stderr,
                b"Unexpected 403 result from server\nAncient Fortinet server (<v5?) only supports ancient HTML config, which is not implemented by OpenConnect.\nCookie is no longer valid, ending session\nEstablishing PPP tunnel over TLS failed\n",
            ),
            (Stdout, b"GET https://203.0.113.10:8443/remote/logout\n"),
            (Stdout, b"Logout successful.\n"),
            (Stderr, b"Cookie was rejected by server; exiting.\n"),
        ];
        assert_eq!(replay(&mut run, &tail), []);
        let mut expected = logged_in();
        // Reconnecting to log out; the watchdog keeps the furthest milestone.
        expected.push(Finding::Milestone(Milestone::ConnectedToGateway));
        assert_eq!(run.findings, expected);
        assert!(run.interpreter.routes_left_unrestored());
    }

    #[test]
    fn a_pin_that_does_not_match_reports_the_gateways_pin() {
        let mut run = run();
        let reads: [(OutputSource, &[u8]); 5] = [
            (
                Stdout,
                b"GET https://203.0.113.10:8443/\nConnected to 203.0.113.10:8443\nSSL negotiation with 203.0.113.10\n",
            ),
            (Stdout, b"Server certificate verify failed: signer not found\n"),
            (
                Stderr,
                b"None of the 1 fingerprint(s) specified via --servercert match server's certificate: pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=\n",
            ),
            (
                Stderr,
                b"SSL connection failure: Error in the certificate.\nFailed to open HTTPS connection to 203.0.113.10\n",
            ),
            (Stderr, b"Failed to complete authentication\n"),
        ];
        assert_eq!(replay(&mut run, &reads), []);
        assert_eq!(run.findings, [Finding::CertificateRejected(PIN.into())]);
        assert!(!run.interpreter.routes_left_unrestored());
    }

    #[test]
    fn each_rejected_pin_is_reported_once() {
        let mut run = run();
        let line = format!(
            "None of the 2 fingerprint(s) specified via --servercert match server's certificate: {OTHER_PIN}\n"
        );
        for _ in 0..2 {
            run.read(Stderr, line.as_bytes());
        }
        run.read(
            Stderr,
            b"None of the 1 fingerprint(s) specified via --servercert match server's certificate: not-a-pin\n",
        );
        assert_eq!(
            run.findings,
            [Finding::CertificateRejected(OTHER_PIN.into())]
        );
    }

    /// A second prompt is the gateway's rejection; the watchdog ends the run on it.
    #[test]
    fn prompts_are_recognised_each_time_and_only_on_stderr() {
        let mut run = run();
        assert_eq!(run.read(Stderr, b"Pass"), None);
        assert_eq!(run.read(Stderr, b"word: "), Some(Prompt::Password));
        assert_eq!(run.read(Stderr, b"\nCode: "), Some(Prompt::Otp));
        assert_eq!(run.read(Stderr, b"\nCode: "), Some(Prompt::Otp));
        assert_eq!(run.read(Stderr, b"\nInvalid Code: "), None);
        assert_eq!(run.read(Stdout, b"Password: "), None);
        assert_eq!(run.read(Stdout, b"\nCode: "), None);
    }

    /// openconnect asks only after it verified the gateway, and its stderr prompt may be read
    /// before the stdout line that shows the connection.
    #[test]
    fn the_password_is_expected_until_the_otp_prompt() {
        let interpreter = OpenConnect.interpreter();
        for milestone in [Milestone::Spawned, Milestone::ConnectedToGateway] {
            assert!(interpreter.password_expected(milestone), "{milestone:?}");
        }
        for milestone in [Milestone::OtpPrompted, Milestone::TunnelUp] {
            assert!(!interpreter.password_expected(milestone), "{milestone:?}");
        }
    }
}
