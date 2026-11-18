//! openconnect's launch, value checks and output, with output taken from openconnect 9.21 runs
//! against a FortiGate (2026-11-10, 11-11 and 11-12); addresses, names, keys and cookies are
//! replaced.

mod common;

use std::net::Ipv4Addr;
use std::path::Path;

use common::Run;
use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_contract::app_helper::{AttemptId, Milestone, OutputSource, RunRequest, SessionId};
use fvm_contract::backend::{
    Backend, ClientExecutable, Finding, LaunchSpec, Prompt, SessionInputs,
};
use zeroize::Zeroizing;

const CLIENT_PATH: &str = r"C:\Program Files\FortiVPNManager\OpenConnect\openconnect.exe";
const PIN: &str = "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=";
const OTHER_PIN: &str = "pin-sha256:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const PLACEHOLDER_PIN: &str = "pin-sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const PASSWORD: &str = " Pa55 w#rd ";
const VALUES_PATH: &str = "/var/run/com.qoor.FortiVPNManager.Helper/values.3";

fn request() -> RunRequest {
    RunRequest {
        attempt_id: "6B1D9B3E-2C1F-4E55-9C0B-3F0A2B4C5D6E"
            .parse::<AttemptId>()
            .unwrap(),
        replacing: None,
        session_id: SessionId(277_070_470),
        host: "203.0.113.10".into(),
        port: 8443,
        username: "alice".into(),
        realm: None,
        trusted_certs: vec![PIN.into()],
        dtls: false,
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
    OpenConnect.launch_spec(
        &client,
        request,
        Path::new("unused"),
        Path::new(VALUES_PATH),
    )
}

mod launching {
    use super::*;

    #[cfg(target_os = "macos")]
    const SCRIPT: Option<(&str, &str)> = Some((
        "--script=case \"$reason\" in connect) /usr/bin/env -0 > \"$FVM_VALUES_FILE.tmp\" && /bin/mv -f \"$FVM_VALUES_FILE.tmp\" \"$FVM_VALUES_FILE\";; attempt-reconnect) echo 'FortiVPNManager: gateway route requested';; esac; exit 0",
        "/bin/sh",
    ));
    #[cfg(windows)]
    const SCRIPT: Option<(&str, &str)> = Some((
        r"--script=C:\Program Files\FortiVPNManager\vpnc-values.js",
        r"C:\Windows\System32\cscript.exe",
    ));
    #[cfg(not(any(windows, target_os = "macos")))]
    const SCRIPT: Option<(&str, &str)> = None;

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
        let servercert = format!("--servercert={PIN}");
        let mut tunnel = vec!["--protocol=fortinet", servercert.as_str()];
        tunnel.extend(SCRIPT.map(|(argument, _)| argument));
        tunnel.extend([
            "--no-dtls",
            "--reconnect-timeout=0",
            "--cookie-on-stdin",
            "https://203.0.113.10:8443/",
        ]);
        assert_eq!(later.tunnel.args, tunnel);
        let mut tunnel_env = spec.env.clone();
        tunnel_env.push(("FVM_VALUES_FILE".to_owned(), VALUES_PATH.to_owned()));
        assert_eq!(later.tunnel.env, tunnel_env);
        assert!(!spec.env.iter().any(|(name, _)| name == "FVM_VALUES_FILE"));
        assert_eq!(
            (later.tunnel.config_file, later.tunnel.session),
            (None, None)
        );
        assert_eq!(
            later.tunnel.script_runner.as_deref(),
            SCRIPT.map(|(_, runner)| Path::new(runner))
        );
        assert_eq!(spec.script_runner, None);
        assert_eq!(later.logout.script_runner, None);
        assert!(
            !later
                .logout
                .env
                .iter()
                .any(|(name, _)| name == "FVM_VALUES_FILE")
        );
    }

    #[test]
    fn only_the_tunnel_run_turns_dtls_off() {
        let off = launch(&request()).unwrap();
        let on = launch(&with(|r| r.dtls = true)).unwrap();
        assert_eq!(on.args, off.args);
        let (off, on) = (off.session.unwrap(), on.session.unwrap());
        assert_eq!(on.logout, off.logout);
        let mut tunnel = on.tunnel.args;
        let position = tunnel
            .iter()
            .position(|arg| arg == "--reconnect-timeout=0")
            .unwrap();
        tunnel.insert(position, "--no-dtls".to_owned());
        assert_eq!(off.tunnel.args, tunnel);
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
}

mod client {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn only_the_installed_client_is_ours() {
        assert_eq!(OpenConnect.client_name(), "openconnect");
        for path in [
            CLIENT_PATH,
            r"c:\program files\fortivpnmanager\openconnect\OPENCONNECT.EXE",
        ] {
            assert!(OpenConnect.is_own_executable(Path::new(path)), "{path}");
        }
        for path in [
            r"C:\Program Files\FortiVPNManager\OpenConnect\other.exe",
            r"C:\Program Files\OpenConnect\openconnect.exe",
            r"C:\Users\alice\openconnect.exe",
            "/opt/homebrew/bin/openconnect",
        ] {
            assert!(!OpenConnect.is_own_executable(Path::new(path)), "{path}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_the_installed_client_is_ours() {
        assert_eq!(OpenConnect.client_name(), "openconnect");
        assert!(OpenConnect.is_own_executable(Path::new(
            "/Applications/FortiVPNManager.app/Contents/MacOS/openconnect"
        )));
        for path in [
            "/Applications/FortiVPNManager.app/Contents/MacOS/FortiVPNManager",
            "/Applications/FortiVPNManager.app/Contents/Resources/vpnc-script",
            "/Users/alice/Downloads/FortiVPNManager.app/Contents/MacOS/openconnect",
            "/opt/homebrew/Cellar/openconnect/9.21/bin/openconnect",
            "/opt/homebrew/bin/openconnect",
            CLIENT_PATH,
        ] {
            assert!(!OpenConnect.is_own_executable(Path::new(path)), "{path}");
        }
    }

    /// openconnect is not supported on this OS yet.
    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn no_client_is_ours_here_yet() {
        assert!(!OpenConnect.is_own_executable(Path::new(CLIENT_PATH)));
        assert!(!OpenConnect.is_own_executable(Path::new("/opt/homebrew/bin/openconnect")));
        let problem = OpenConnect
            .locate_client(fvm_contract::backend::ClientCheckWait::Limited)
            .unwrap_err();
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

    fn run() -> Run {
        Run::with(OpenConnect.interpreter())
    }

    /// Reads in order; returns the prompts reported.
    fn replay(run: &mut Run, reads: &[(OutputSource, &[u8])]) -> Vec<Prompt> {
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
            b"Script did not complete within 10 seconds.\nFailed to spawn script 'C:\\Program Files\\FortiVPNManager\\OpenConnect\\vpnc-script-win.js' for connect: \xc0\xdf\xb8\xf8\xb5\xc8 \xb1\xe2\xb4\xc9\xc0\xd4\xb4\xcf\xb4\xd9.\nUnexpected pre-PPP packet header for encap 5.\n",
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
        let mut expected = logged_in();
        expected.push(Finding::ScriptLeftRunning);
        assert_eq!(run.findings, expected);
        assert!(!run.interpreter.routes_left_unrestored());
    }

    #[test]
    fn the_gateway_route_request_line_is_reported() {
        let mut run = run();
        replay(&mut run, &LOGIN_RUN);
        let tunnel_run: [(OutputSource, &[u8]); 3] = [
            (
                Stdout,
                b"GET https://203.0.113.10:8443/remote/fortisslvpn_xml?dual_stack=1\nConnected to HTTPS on 203.0.113.10 with ciphersuite (TLS1.3)-(ECDHE-SECP384R1)-(RSA-PSS-RSAE-SHA256)-(AES-256-GCM)\nGot Legacy IP address 10.10.0.5\n",
            ),
            (
                Stdout,
                b"Configured as 10.10.0.5, with SSL connected and DTLS in progress\nFortiVPNManager: gateway route requested.\n echo FortiVPNManager: gateway route requested\n",
            ),
            (Stdout, b"FortiVPNManager: gateway route requested\n"),
        ];
        assert_eq!(replay(&mut run, &tunnel_run), []);
        let mut expected = logged_in();
        expected.push(Finding::GatewayRouteRequested);
        assert_eq!(run.findings, expected);
        assert!(!run.interpreter.routes_left_unrestored());
    }

    #[test]
    fn a_tunnel_with_ipv6_reports_its_ipv4_address() {
        let mut run = run();
        run.read(
            Stdout,
            b"Configured as 10.10.0.5 + 2001:db8::5/64, with SSL disconnected and DTLS established\n",
        );
        assert_eq!(
            run.findings,
            [Finding::TunnelIp(Ipv4Addr::new(10, 10, 0, 5))]
        );
    }

    #[test]
    fn another_adapter_up_with_the_tunnels_address_is_reported() {
        let mut run = run();
        replay(&mut run, &LOGIN_RUN);
        replay(&mut run, &TUNNEL_RUN[..5]);
        let mut conflict = b"0: Creating adapter\nAdapter \"".to_vec();
        conflict.extend_from_slice(CP949_TEXT);
        conflict.extend_from_slice(b"\" / 54 is UP and using our IPv4 address. Cannot resolve.\n");
        assert_eq!(run.read(Stderr, &conflict), None);
        let mut expected = logged_in();
        expected.push(Finding::TunnelAddressInUse);
        assert_eq!(run.findings, expected);
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
        expected.push(Finding::ScriptLeftRunning);
        // Reconnecting to log out; the watchdog keeps the furthest milestone.
        expected.push(Finding::Milestone(Milestone::ConnectedToGateway));
        assert_eq!(run.findings, expected);
        assert!(!run.interpreter.routes_left_unrestored());
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

mod network_values {
    use std::net::IpAddr;

    use fvm_contract::backend::{
        FamilySettings, IpNetwork, NetworkSettings, TunnelDevice, TunnelRoutes,
    };

    use super::*;

    fn values(entries: &[&str]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for entry in entries {
            bytes.extend_from_slice(entry.as_bytes());
            bytes.push(0);
        }
        bytes
    }

    fn network(address: &str, prefix_length: u8) -> IpNetwork {
        IpNetwork {
            address: address.parse().unwrap(),
            prefix_length,
        }
    }

    fn ip(address: &str) -> IpAddr {
        address.parse().unwrap()
    }

    const SPLIT_TUNNEL: &[&str] = &[
        "PATH=/usr/bin:/bin:/usr/sbin:/sbin",
        "FVM_VALUES_FILE=/var/run/com.qoor.FortiVPNManager.Helper/values.3",
        "reason=connect",
        "VPNGATEWAY=203.0.113.10",
        "VPNPID=4242",
        "TUNDEV=utun5",
        "LOG_LEVEL=1",
        "IDLE_TIMEOUT=3000",
        "INTERNAL_IP4_ADDRESS=10.10.0.5",
        "INTERNAL_IP4_MTU=1351",
        "INTERNAL_IP4_DNS=10.20.0.53 10.20.0.54 10.20.0.53",
        "INTERNAL_IP4_NBNS=10.20.0.60",
        "CISCO_DEF_DOMAIN=corp.example.com",
        "CISCO_SPLIT_DNS=corp.example.com,example.net",
        "CISCO_SPLIT_INC=2",
        "CISCO_SPLIT_INC_0_ADDR=10.20.161.0",
        "CISCO_SPLIT_INC_0_MASK=255.255.255.0",
        "CISCO_SPLIT_INC_0_MASKLEN=24",
        "CISCO_SPLIT_INC_0_PROTOCOL=0",
        "CISCO_SPLIT_INC_1_ADDR=192.168.50.7",
        "CISCO_SPLIT_INC_1_MASK=255.255.255.0",
        "CISCO_SPLIT_INC_1_MASKLEN=24",
        "CISCO_SPLIT_EXC=4",
        "CISCO_SPLIT_EXC_0_ADDR=10.20.161.16",
        "CISCO_SPLIT_EXC_0_MASK=255.255.255.255",
        "CISCO_SPLIT_EXC_1_ADDR=0.0.0.0",
        "CISCO_SPLIT_EXC_1_MASK=255.0.0.0",
        "CISCO_SPLIT_EXC_2_ADDR=127.0.0.0",
        "CISCO_SPLIT_EXC_2_MASK=255.0.0.0",
        "CISCO_SPLIT_EXC_3_ADDR=169.254.0.0",
        "CISCO_SPLIT_EXC_3_MASK=255.255.0.0",
    ];

    fn split_tunnel() -> NetworkSettings {
        NetworkSettings {
            tunnel: TunnelDevice {
                name: "utun5".into(),
                index: None,
            },
            gateway: ip("203.0.113.10"),
            mtu: Some(1351),
            ipv4: FamilySettings {
                address: network("10.10.0.5", 32),
                routes: TunnelRoutes::Split {
                    include: vec![network("10.20.161.0", 24), network("192.168.50.0", 24)],
                },
            },
            ipv6: None,
            split_exclude: vec![network("10.20.161.16", 32)],
            dns_servers: vec![ip("10.20.0.53"), ip("10.20.0.54")],
            default_domains: vec!["corp.example.com".into()],
            split_dns_domains: vec!["corp.example.com".into(), "example.net".into()],
            wins_servers: vec![Ipv4Addr::new(10, 20, 0, 60)],
        }
    }

    #[test]
    fn a_split_tunnel_is_read_without_the_unroutable_excludes() {
        assert_eq!(
            OpenConnect.network_settings(&values(SPLIT_TUNNEL)),
            Ok(split_tunnel())
        );
    }

    #[test]
    fn a_full_tunnel_with_ipv6_is_read_from_windows_variables() {
        let mut banner = b"CISCO_BANNER=".to_vec();
        banner.extend_from_slice(b"\xb3\xd7\xc6\xae\xbf\xf6\xc5\xa9");
        banner.push(0);
        banner.extend(values(&[
            "=C:=C:\\Windows\\System32",
            "SystemRoot=C:\\Windows",
            "reason=connect",
            "VPNGATEWAY=2001:db8::10",
            "TUNDEV=203.0.113.10",
            "TUNIDX=54",
            "INTERNAL_IP4_ADDRESS=10.10.0.5",
            "INTERNAL_IP4_NETMASK=255.255.255.0",
            "INTERNAL_IP4_DNS=10.20.0.53",
            "INTERNAL_IP6_ADDRESS=fd00:10::5",
            "INTERNAL_IP6_NETMASK=fd00:10::/64",
            "INTERNAL_IP6_DNS=fd00:10::53",
            "CISCO_SPLIT_INC=2",
            "CISCO_SPLIT_INC_0_ADDR=10.20.161.0",
            "CISCO_SPLIT_INC_0_MASK=255.255.255.0",
            "CISCO_SPLIT_INC_1_ADDR=0.0.0.0",
            "CISCO_SPLIT_INC_1_MASK=0.0.0.0",
            "CISCO_IPV6_SPLIT_INC=1",
            "CISCO_IPV6_SPLIT_INC_0_ADDR=fd00:20::1",
            "CISCO_IPV6_SPLIT_INC_0_MASKLEN=48",
            "CISCO_IPV6_SPLIT_EXC=1",
            "CISCO_IPV6_SPLIT_EXC_0_ADDR=2001:db8:5::",
            "CISCO_IPV6_SPLIT_EXC_0_MASKLEN=64",
        ]));
        assert_eq!(
            OpenConnect.network_settings(&banner),
            Ok(NetworkSettings {
                tunnel: TunnelDevice {
                    name: "203.0.113.10".into(),
                    index: Some(54),
                },
                gateway: ip("2001:db8::10"),
                mtu: None,
                ipv4: FamilySettings {
                    address: network("10.10.0.5", 24),
                    routes: TunnelRoutes::Full {},
                },
                ipv6: Some(FamilySettings {
                    address: network("fd00:10::5", 64),
                    routes: TunnelRoutes::Split {
                        include: vec![network("fd00:20::", 48)],
                    },
                }),
                split_exclude: vec![network("2001:db8:5::", 64)],
                dns_servers: vec![ip("10.20.0.53"), ip("fd00:10::53")],
                default_domains: Vec::new(),
                split_dns_domains: Vec::new(),
                wins_servers: Vec::new(),
            })
        );
    }

    #[test]
    fn no_split_include_is_a_full_tunnel_for_each_family() {
        let settings = OpenConnect
            .network_settings(&values(&[
                "reason=connect",
                "VPNGATEWAY=203.0.113.10",
                "TUNDEV=utun5",
                "INTERNAL_IP4_ADDRESS=10.10.0.5",
                "INTERNAL_IP6_ADDRESS=fd00:10::5",
                "CISCO_SPLIT_INC=0",
                "CISCO_IPV6_SPLIT_INC=1",
                "CISCO_IPV6_SPLIT_INC_0_ADDR=::",
                "CISCO_IPV6_SPLIT_INC_0_MASKLEN=0",
            ]))
            .unwrap();
        assert_eq!(settings.ipv4.routes, TunnelRoutes::Full {});
        assert_eq!(
            settings.ipv6,
            Some(FamilySettings {
                address: network("fd00:10::5", 128),
                routes: TunnelRoutes::Full {},
            })
        );
    }

    #[test]
    fn values_that_cannot_be_applied_are_refused() {
        let with = |name: &str, value: Option<&str>| {
            let mut entries: Vec<String> = SPLIT_TUNNEL
                .iter()
                .filter(|entry| !entry.starts_with(&format!("{name}=")))
                .map(|entry| (*entry).to_owned())
                .collect();
            entries.extend(value.map(|value| format!("{name}={value}")));
            let entries: Vec<&str> = entries.iter().map(String::as_str).collect();
            OpenConnect.network_settings(&values(&entries))
        };
        let refused = [
            (
                "reason",
                Some("disconnect"),
                "reason is \"disconnect\", not connect",
            ),
            (
                "INTERNAL_IP4_ADDRESS",
                None,
                "INTERNAL_IP4_ADDRESS is missing",
            ),
            (
                "VPNGATEWAY",
                Some("vpn.example.com"),
                "VPNGATEWAY has an unreadable value: \"vpn.example.com\"",
            ),
            (
                "TUNDEV",
                Some("-utun5"),
                "TUNDEV is not a device name: \"-utun5\"",
            ),
            (
                "INTERNAL_IP4_MTU",
                Some("0"),
                "INTERNAL_IP4_MTU 0 is outside 68 to 65535",
            ),
            (
                "INTERNAL_IP4_NETMASK",
                Some("255.0.255.0"),
                "INTERNAL_IP4_NETMASK is not a subnet mask: \"255.0.255.0\"",
            ),
            (
                "CISCO_SPLIT_INC",
                Some("3"),
                "CISCO_SPLIT_INC_2_ADDR is missing",
            ),
            (
                "CISCO_SPLIT_INC_1_MASK",
                Some("255.0.255.0"),
                "CISCO_SPLIT_INC_1_MASK is not a subnet mask: \"255.0.255.0\"",
            ),
            (
                "INTERNAL_IP4_DNS",
                Some("10.20.0.53 dns"),
                "INTERNAL_IP4_DNS has an unreadable value: \"dns\"",
            ),
            (
                "CISCO_SPLIT_DNS",
                Some("example.net,bad domain!"),
                "CISCO_SPLIT_DNS has a value that is not a domain: \"domain!\"",
            ),
            (
                "INTERNAL_IP6_NETMASK",
                Some("fd00::/129"),
                "INTERNAL_IP6_NETMASK is not an address/length: \"fd00::/129\"",
            ),
        ];
        for (name, value, error) in refused {
            assert_eq!(with(name, value), Err(error.to_owned()), "{name}");
        }
        let mut not_utf8 = values(SPLIT_TUNNEL);
        not_utf8.extend_from_slice(b"CISCO_DEF_DOMAIN=\xff\0");
        assert_eq!(
            OpenConnect.network_settings(&not_utf8),
            Err("CISCO_DEF_DOMAIN is not UTF-8".to_owned())
        );
        assert_eq!(
            OpenConnect.network_settings(b""),
            Err("reason is missing".to_owned())
        );
    }
}

#[cfg(target_os = "macos")]
mod value_script {
    use std::fs;
    use std::process::{self, Command};

    use super::*;

    #[test]
    fn only_connect_passes_the_values_and_attempt_reconnect_asks_for_the_gateway_route() {
        let spec = launch(&request()).unwrap();
        let tunnel = spec.session.unwrap().tunnel;
        let script = tunnel
            .args
            .iter()
            .find_map(|arg| arg.strip_prefix("--script="))
            .unwrap()
            .to_owned();
        assert_eq!(tunnel.script_runner.as_deref(), Some(Path::new("/bin/sh")));
        let directory = std::env::temp_dir().join(format!("fvm-value-script-{}", process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let file = directory.join("values.3");
        let draft = directory.join("values.3.tmp");
        for reason in [
            "pre-init",
            "connect",
            "disconnect",
            "reconnect",
            "attempt-reconnect",
        ] {
            let _ = fs::remove_file(&file);
            let output = Command::new("/bin/sh")
                .arg("-c")
                .arg(&script)
                .env_clear()
                .env("reason", reason)
                .env("FVM_VALUES_FILE", &file)
                .env("CISCO_BANNER", "a\nb=c")
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(0), "{reason}");
            assert_eq!(String::from_utf8_lossy(&output.stderr), "", "{reason}");
            let expected_output = if reason == "attempt-reconnect" {
                "FortiVPNManager: gateway route requested\n"
            } else {
                ""
            };
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                expected_output,
                "{reason}"
            );
            assert!(!draft.exists(), "{reason}");
            if reason == "connect" {
                let written = fs::read(&file).unwrap();
                let mut entries: Vec<&[u8]> = written.split(|&byte| byte == 0).collect();
                assert_eq!(entries.pop(), Some(&b""[..]));
                let values_entry = format!("FVM_VALUES_FILE={}", file.display());
                let given: [&[u8]; 3] = [
                    b"CISCO_BANNER=a\nb=c",
                    values_entry.as_bytes(),
                    b"reason=connect",
                ];
                for entry in given {
                    assert!(entries.contains(&entry), "{entries:?}");
                }
                assert!(OpenConnect.network_settings(&written).is_err());
            } else {
                assert!(!file.exists(), "{reason}");
            }
        }
        fs::remove_dir_all(&directory).unwrap();
    }
}
