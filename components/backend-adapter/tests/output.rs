//! What one run's output tells, line by line and prompt by prompt.
#![cfg(unix)] // openfortivpn runs only on Unix.

mod common;

use std::net::Ipv4Addr;

use common::Run;
use fvm_backend_adapter::openfortivpn::OpenFortiVpn;
use fvm_contract::app_helper::{Milestone, OutputSource};
use fvm_contract::backend::{Backend, Finding, OutputInterpreter, Prompt};

const DIGEST: &str = "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11";
const OTHER_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const PASSWORD_PROMPT: &str = "VPN account password: ";
const OTP_PROMPT: &str = "Two-factor authentication token: ";

fn findings<S: AsRef<str>>(lines: &[S]) -> Vec<Finding> {
    let mut interpreter = OpenFortiVpn.interpreter();
    lines
        .iter()
        .filter_map(|line| interpreter.line(OutputSource::Stdout, line.as_ref()))
        .collect()
}

fn milestone(milestone: Milestone) -> Finding {
    Finding::Milestone(milestone)
}

fn rejected(digest: &str) -> Finding {
    Finding::CertificateRejected(digest.into())
}

/// The rejection block openfortivpn prints when the gateway certificate is not trusted (tunnel.c).
fn certificate_block(digest: &str) -> Vec<String> {
    [
        "ERROR:  Gateway certificate validation failed, and the certificate digest is not in the local whitelist. If you trust it, rerun with:".to_owned(),
        format!("ERROR:      --trusted-cert {digest}"),
        "ERROR:  or add this line to your configuration file:".to_owned(),
        format!("ERROR:      trusted-cert = {digest}"),
        "ERROR:  Gateway certificate:".to_owned(),
        "ERROR:      subject:".to_owned(),
        "ERROR:          C=US".to_owned(),
        "ERROR:          O=Fortinet".to_owned(),
        "ERROR:          CN=FGT60FTK00000000".to_owned(),
        "ERROR:      issuer:".to_owned(),
        "ERROR:          CN=fortinet-subca2001".to_owned(),
        "ERROR:      sha256 digest:".to_owned(),
        format!("ERROR:          {digest}"),
    ]
    .into()
}

// Milestones and other single lines

#[test]
fn reads_milestones_and_the_tunnel_mode_refusal() {
    for (line, expected) in [
        (
            "INFO:   Connected to gateway.",
            milestone(Milestone::ConnectedToGateway),
        ),
        (
            "INFO:   Authenticated.",
            milestone(Milestone::Authenticated),
        ),
        (
            "INFO:   Tunnel is up and running.",
            milestone(Milestone::TunnelUp),
        ),
        (
            "ERROR:  Could not authenticate to the gateway. Please make sure tunnel mode is allowed by the gateway, check the realm, etc.",
            Finding::TunnelModeRejected,
        ),
    ] {
        assert_eq!(findings(&[line]), [expected], "{line}");
    }
}

#[test]
fn ignores_other_lines() {
    for line in [
        "",
        PASSWORD_PROMPT,
        OTP_PROMPT,
        "Connected to gateway.",
        "INFO:  Connected to gateway.",
        "INFO:   Connected to gateway. ",
        "INFO:   connected to gateway.",
        "INFO:   Remote gateway has allocated a VPN.",
        "INFO:   Negotiation complete.",
        "INFO:   Closed connection to gateway.",
        "INFO:   Logged out.",
        "INFO:   Terminated pppd.",
        "INFO:   Restoring routes...",
        "INFO:   Interface  is UP.",
        "INFO:   Interface ppp0 eth0 is UP.",
        "WARN:   Removing wrong route to vpn server...",
        "add host 203.0.113.10: gateway 192.168.1.1",
        "route: writing to routing socket: File exists",
        "  (raw) INFO:   Tunnel is up and running.",
        "ERROR:  /usr/sbin/pppd: No such file or directory.",
        "ERROR:  pppd: something new",
        // A failed login is not a tunnel mode refusal.
        "ERROR:  Could not authenticate to gateway. Please check the password, client certificate, etc.",
        "ERROR:  Could not authenticate to gateway.",
    ] {
        assert_eq!(findings(&[line]), [], "{line:?}");
    }
}

#[test]
fn reads_any_log_level() {
    for prefix in ["INFO:   ", "WARN:   ", "ERROR:  ", "DEBUG:  "] {
        assert_eq!(
            findings(&[&format!("{prefix}Authenticated.")]),
            [milestone(Milestone::Authenticated)],
            "{prefix}"
        );
    }
}

#[test]
fn reads_only_stdout() {
    let mut interpreter = OpenFortiVpn.interpreter();
    for source in [OutputSource::Stderr, OutputSource::Helper] {
        for line in [
            "INFO:   Connected to gateway.".to_owned(),
            format!("ERROR:      trusted-cert = {DIGEST}"),
            "INFO:   Setting new routes...".to_owned(),
            "Modem hangup".to_owned(),
        ] {
            assert_eq!(interpreter.line(source, &line), None, "{source:?} {line}");
        }
    }
    assert!(!interpreter.routes_left_unrestored());
}

// Tunnel IP

#[test]
fn reads_the_tunnel_ip_from_the_addresses_line() {
    for (addresses, expected) in [
        (
            "[10.212.134.200], ns [10.0.0.53, 10.0.0.54]",
            [10, 212, 134, 200],
        ),
        (
            "[10.212.134.200], ns [10.0.0.53, 10.0.0.54], ns_suffix [corp.example.com]",
            [10, 212, 134, 200],
        ),
        ("[10.1.2.3], ns [10.0.0.53, 0.0.0.0]", [10, 1, 2, 3]),
        (
            "[10.1.2.3], ns [0.0.0.0, 0.0.0.0], ns_suffix [a.example;b.example]",
            [10, 1, 2, 3],
        ),
        (
            "[10.1.2.3], ns [0.0.0.0, 0.0.0.0], ns_suffix []",
            [10, 1, 2, 3],
        ),
    ] {
        assert_eq!(
            findings(&[&format!("INFO:   Got addresses: {addresses}")]),
            [Finding::TunnelIp(Ipv4Addr::from(expected))],
            "{addresses}"
        );
    }
}

#[test]
fn ignores_a_malformed_addresses_line() {
    for addresses in [
        "[10.1.2], ns [10.0.0.53, 0.0.0.0]",
        "[999.1.2.3], ns [10.0.0.53, 0.0.0.0]",
        "[10.1.2.3]",
        "[10.1.2.3], ns [10.0.0.53, 0.0.0.0",
        "[10.1.2.3], ns [dns.example, 0.0.0.0]",
        "[10.1.2.3], ns [10.0.0.53, 0.0.0.0], something [x]",
        "10.1.2.3, ns [10.0.0.53, 0.0.0.0]",
    ] {
        assert_eq!(
            findings(&[&format!("INFO:   Got addresses: {addresses}")]),
            [],
            "{addresses}"
        );
    }
}

#[test]
fn reports_every_addresses_line() {
    assert_eq!(
        findings(&[
            "INFO:   Got addresses: [10.1.2.3], ns [0.0.0.0, 0.0.0.0]",
            "INFO:   Got addresses: [10.1.2.4], ns [0.0.0.0, 0.0.0.0]",
        ]),
        [
            Finding::TunnelIp(Ipv4Addr::new(10, 1, 2, 3)),
            Finding::TunnelIp(Ipv4Addr::new(10, 1, 2, 4)),
        ]
    );
}

// Certificate rejection

#[test]
fn takes_the_value_from_the_trusted_cert_line_only() {
    assert_eq!(findings(&certificate_block(DIGEST)), [rejected(DIGEST)]);
}

#[test]
fn reports_a_repeated_rejection_once() {
    // The rejection is printed again when openfortivpn reconnects to log out.
    let lines = [
        certificate_block(DIGEST),
        vec!["INFO:   Closed connection to gateway.".into()],
        certificate_block(DIGEST),
        vec!["INFO:   Could not log out.".into()],
    ]
    .concat();
    assert_eq!(findings(&lines), [rejected(DIGEST)]);
}

#[test]
fn reports_distinct_values_in_order() {
    let lines = [
        certificate_block(OTHER_DIGEST),
        certificate_block(DIGEST),
        certificate_block(OTHER_DIGEST),
    ]
    .concat();
    assert_eq!(findings(&lines), [rejected(OTHER_DIGEST), rejected(DIGEST)]);
}

#[test]
fn reads_a_rejection_after_authentication() {
    // For example on the second TLS connection after the VPN is allocated.
    let lines = [
        vec![
            "INFO:   Connected to gateway.".into(),
            "INFO:   Authenticated.".into(),
            "INFO:   Remote gateway has allocated a VPN.".into(),
        ],
        certificate_block(DIGEST),
    ]
    .concat();
    assert_eq!(
        findings(&lines),
        [
            milestone(Milestone::ConnectedToGateway),
            milestone(Milestone::Authenticated),
            rejected(DIGEST),
        ]
    );
}

#[test]
fn reports_an_uppercase_value_in_lowercase() {
    assert_eq!(
        findings(&[&format!(
            "ERROR:      trusted-cert = {}",
            DIGEST.to_uppercase()
        )]),
        [rejected(DIGEST)]
    );
}

#[test]
fn ignores_a_malformed_value() {
    for value in [
        DIGEST[..63].to_owned(),
        format!("{DIGEST}0"),
        format!("{}g", &DIGEST[..63]),
        String::new(),
    ] {
        assert_eq!(
            findings(&[&format!("ERROR:      trusted-cert = {value}")]),
            [],
            "{value}"
        );
    }
}

// Routes

fn interpreter_after(lines: &[&str]) -> impl OutputInterpreter {
    let mut interpreter = OpenFortiVpn.interpreter();
    for line in lines {
        interpreter.line(OutputSource::Stdout, line);
    }
    interpreter
}

#[test]
fn routes_count_as_restored_only_once_restoration_returned() {
    let mut interpreter = interpreter_after(&["INFO:   Setting new routes..."]);
    assert!(interpreter.routes_left_unrestored());
    for line in [
        "INFO:   Setting ppp0 interface down.",
        "INFO:   Restoring routes...",
        "delete host 203.0.113.10: gateway 192.168.1.1",
        "DEBUG:  /sbin/route -n add -net 0.0.0.0 -netmask 0.0.0.0 192.168.1.1",
        "WARN:   Could not restore default route (File exists). Already restored?",
        "ERROR:  /sbin/route: No such file or directory.",
    ] {
        assert_eq!(interpreter.line(OutputSource::Stdout, line), None, "{line}");
        assert!(interpreter.routes_left_unrestored(), "{line}");
    }
    interpreter.line(OutputSource::Stdout, "INFO:   Removing VPN nameservers...");
    assert!(!interpreter.routes_left_unrestored());
}

#[test]
fn restoration_returns_on_the_next_info_or_error_line() {
    // Without set-dns there is no "Removing VPN nameservers..." line.
    for line in [
        "ERROR:  pppd: The link was terminated by the modem hanging up.",
        "INFO:   Terminated pppd.",
        "ERROR:  waitpid: No child processes",
    ] {
        let mut interpreter = interpreter_after(&[
            "INFO:   Setting new routes...",
            "INFO:   Restoring routes...",
        ]);
        assert!(interpreter.routes_left_unrestored());
        interpreter.line(OutputSource::Stdout, line);
        assert!(!interpreter.routes_left_unrestored(), "{line}");
    }
}

#[test]
fn routes_are_left_unrestored_when_the_run_ends_mid_restoration() {
    let interpreter = interpreter_after(&[
        "INFO:   Setting new routes...",
        "INFO:   Tunnel is up and running.",
        "INFO:   Restoring routes...",
    ]);
    assert!(interpreter.routes_left_unrestored());
}

#[test]
fn routes_are_left_unrestored_when_the_tunnel_never_came_up() {
    // SIGTERM while the interface comes up: openfortivpn skips restoration (tunnel.c).
    let interpreter = interpreter_after(&[
        "INFO:   Interface ppp0 is UP.",
        "INFO:   Setting new routes...",
        "INFO:   Cancelling threads...",
        "INFO:   Cleanup, joining threads...",
        "INFO:   pppd: The link was terminated by the modem hanging up.",
        "INFO:   Terminated pppd.",
    ]);
    assert!(interpreter.routes_left_unrestored());
}

#[test]
fn routes_are_not_unrestored_when_never_changed() {
    let interpreter = interpreter_after(&[
        "INFO:   Connected to gateway.",
        "INFO:   Restoring routes...",
        "INFO:   Logged out.",
    ]);
    assert!(!interpreter.routes_left_unrestored());
}

#[test]
fn a_new_route_change_needs_its_own_restoration() {
    let interpreter = interpreter_after(&[
        "INFO:   Setting new routes...",
        "INFO:   Restoring routes...",
        "INFO:   Terminated pppd.",
        "INFO:   Setting new routes...",
    ]);
    assert!(interpreter.routes_left_unrestored());
}

// Raw tunnel data

#[test]
fn follows_only_route_restoration_after_raw_tunnel_data() {
    let mut interpreter = OpenFortiVpn.interpreter();
    let mut read = |lines: &[String]| -> Vec<Finding> {
        lines
            .iter()
            .filter_map(|line| interpreter.line(OutputSource::Stdout, line))
            .collect()
    };
    let setup = [
        "INFO:   Connected to gateway.",
        "INFO:   Authenticated.",
        "INFO:   Setting new routes...",
        "INFO:   Tunnel is up and running.",
    ]
    .map(String::from);
    assert_eq!(
        read(&setup),
        [
            milestone(Milestone::ConnectedToGateway),
            milestone(Milestone::Authenticated),
            milestone(Milestone::TunnelUp),
        ]
    );
    // Up to 250 raw bytes, line breaks included, follow on stdout (io.c).
    let dump = [
        "ERROR:  Received bad header from gateway:".to_owned(),
        "  (hex) 48 54 54 50 2f 31".to_owned(),
        "  (raw) HTTP/1.1 200 OK".to_owned(),
        format!("ERROR:      trusted-cert = {OTHER_DIGEST}"),
        "ERROR:  Could not authenticate to the gateway. Please make sure tunnel mode is allowed by the gateway, check the realm, etc.".to_owned(),
        "INFO:   Got addresses: [10.9.9.9], ns [10.9.9.53, 0.0.0.0]".to_owned(),
        "INFO:   Authenticated.".to_owned(),
        "INFO:   Tunnel is up and running.".to_owned(),
        "INFO:   Setting new routes...".to_owned(),
    ];
    assert_eq!(read(&dump), []);
    let teardown = [
        "INFO:   Cancelling threads...",
        "INFO:   Setting ppp0 interface down.",
        "INFO:   Restoring routes...",
        "INFO:   Removing VPN nameservers...",
        "ERROR:  pppd: The link was terminated by the modem hanging up.",
    ]
    .map(String::from);
    assert_eq!(read(&teardown), []);
    assert!(!interpreter.routes_left_unrestored());
}

#[test]
fn a_route_change_in_raw_tunnel_data_is_ignored() {
    let interpreter = interpreter_after(&[
        "ERROR:  Received bad header from gateway:",
        "INFO:   Setting new routes...",
    ]);
    assert!(!interpreter.routes_left_unrestored());
}

// Prompts

#[test]
fn reports_an_unfinished_prompt_at_once() {
    for (text, prompt) in [
        (PASSWORD_PROMPT, Prompt::Password),
        (OTP_PROMPT, Prompt::Otp),
    ] {
        assert_eq!(
            Run::new().read(OutputSource::Stdout, text.as_bytes()),
            Some(prompt)
        );
    }
}

#[test]
fn reports_a_prompt_split_at_any_offset() {
    for (text, prompt) in [
        (PASSWORD_PROMPT, Prompt::Password),
        (OTP_PROMPT, Prompt::Otp),
    ] {
        let bytes = text.as_bytes();
        for cut in 1..bytes.len() {
            let mut run = Run::new();
            assert_eq!(run.read(OutputSource::Stdout, &bytes[..cut]), None, "{cut}");
            assert_eq!(
                run.read(OutputSource::Stdout, &bytes[cut..]),
                Some(prompt),
                "{cut}"
            );
        }
    }
}

#[test]
fn reports_a_prompt_read_byte_by_byte_once() {
    for (text, prompt) in [
        (PASSWORD_PROMPT, Prompt::Password),
        (OTP_PROMPT, Prompt::Otp),
    ] {
        let mut run = Run::new();
        let reports: Vec<Prompt> = text
            .as_bytes()
            .iter()
            .filter_map(|byte| run.read(OutputSource::Stdout, &[*byte]))
            .collect();
        assert_eq!(reports, [prompt]);
    }
}

#[test]
fn reports_each_prompt_as_its_line_arrives() {
    let mut run = Run::new();
    let mut read = |text: &str| run.read(OutputSource::Stdout, text.as_bytes());
    assert_eq!(read(PASSWORD_PROMPT), Some(Prompt::Password));
    // The line break printed after reading stdin arrives with the next log line.
    assert_eq!(read("\nINFO:   Connected to gateway.\n"), None);
    assert_eq!(read(OTP_PROMPT), Some(Prompt::Otp));
    assert_eq!(read("\nINFO:   Authenticated.\n"), None);
    assert_eq!(read(PASSWORD_PROMPT), Some(Prompt::Password));
    assert_eq!(read(PASSWORD_PROMPT), Some(Prompt::Password));
}

#[test]
fn reports_a_prompt_that_ends_an_unfinished_line() {
    assert_eq!(
        Run::new().read(
            OutputSource::Stdout,
            b"WARN:   noise without newline; VPN account password: "
        ),
        Some(Prompt::Password)
    );
}

#[test]
fn reports_no_prompt_for_other_output() {
    for text in [
        "INFO:   Connected to gateway.\n",
        "ERROR:  Could not authenticate to gateway. Please check the password, client certificate, etc.\n",
        "WARN:   Adding route table is incomplete. Please check route table.\n",
        "VPN account password: \n",
        "VPN account password:",
        "Two-factor authentication token:",
        "VPN account password: x",
        "add net 0.0.0.0: gateway ppp0\n",
    ] {
        assert_eq!(
            Run::new().read(OutputSource::Stdout, text.as_bytes()),
            None,
            "{text:?}"
        );
    }
}

#[test]
fn reports_no_prompt_broken_by_a_line_break() {
    let mut run = Run::new();
    for chunk in ["VPN account pass", "\n", "word: "] {
        assert_eq!(
            run.read(OutputSource::Stdout, chunk.as_bytes()),
            None,
            "{chunk:?}"
        );
    }
}

#[test]
fn reports_a_prompt_after_long_output() {
    let noise = "x".repeat(10_000);
    let mut run = Run::new();
    assert_eq!(run.read(OutputSource::Stdout, noise.as_bytes()), None);
    assert_eq!(
        run.read(OutputSource::Stdout, PASSWORD_PROMPT.as_bytes()),
        Some(Prompt::Password)
    );
    let mut run = Run::new();
    let lines = format!("{noise}\n{noise}\n");
    assert_eq!(run.read(OutputSource::Stdout, lines.as_bytes()), None);
    assert_eq!(
        run.read(OutputSource::Stdout, OTP_PROMPT.as_bytes()),
        Some(Prompt::Otp)
    );
}

#[test]
fn reports_no_prompt_on_stderr() {
    for text in [PASSWORD_PROMPT, OTP_PROMPT] {
        assert_eq!(Run::new().read(OutputSource::Stderr, text.as_bytes()), None);
    }
}

#[test]
fn expects_the_password_only_before_the_gateway_connection() {
    let interpreter = OpenFortiVpn.interpreter();
    assert!(interpreter.password_expected(Milestone::Spawned));
    for milestone in [
        Milestone::ConnectedToGateway,
        Milestone::OtpPrompted,
        Milestone::OtpSubmitted,
        Milestone::Authenticated,
        Milestone::TunnelUp,
    ] {
        assert!(!interpreter.password_expected(milestone), "{milestone:?}");
    }
}
