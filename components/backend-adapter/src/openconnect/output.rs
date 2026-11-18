//! Reading an attempt's openconnect output, its login run and then its tunnel run. The tunnel run
//! writes its progress to stdout and its prompts and errors to stderr, each line as it happens; the
//! login run writes its progress to stderr too, and only its session lines to stdout. The route
//! script's output reaches them on macOS only. Some text, such as dates and Windows error
//! messages, is in the system's language and code page.

use std::net::Ipv4Addr;

use fvm_contract::app_helper::{Milestone, OutputSource};
use fvm_contract::backend::{Finding, OutputInterpreter, Prompt, SessionInputs};
use fvm_contract::validation::certificate_value;
use zeroize::Zeroizing;

// The fortinet login form's fields, each printed alone on a line without a line break.
const PASSWORD_PROMPT: &[u8] = b"Password: ";
const OTP_PROMPT: &[u8] = b"Code: ";

const CERTIFICATE_REJECTED: &str = " specified via --servercert match server's certificate: ";
/// The login run's last lines, shell-quoted: `COOKIE='SVPNCOOKIE=…'`, then HOST, CONNECT_URL,
/// FINGERPRINT and RESOLVE, which hold no secret.
const COOKIE_LINE: &str = "COOKIE=";
pub(super) const GATEWAY_ROUTE_REQUESTED: &str = "FortiVPNManager: gateway route requested";
const SCRIPT_LEFT_RUNNING: &str = "Script did not complete within 10 seconds.";
const ADDRESS_IN_USE: &str = " is UP and using our IPv4 address. Cannot resolve.";

#[derive(Default)]
pub struct Interpreter {
    /// In the order printed.
    rejected_certificates: Vec<String>,
}

impl OutputInterpreter for Interpreter {
    fn prompt(&mut self, source: OutputSource, partial_line: &[u8]) -> Option<Prompt> {
        if source != OutputSource::Stderr {
            None
        } else if partial_line == PASSWORD_PROMPT {
            Some(Prompt::Password)
        } else if partial_line == OTP_PROMPT {
            Some(Prompt::Otp)
        } else {
            None
        }
    }

    /// openconnect asks for the password only once it verified the gateway's certificate, so the
    /// milestone that shows it need not have been read yet: the two pipes are read apart.
    fn password_expected(&self, milestone: Milestone) -> bool {
        milestone < Milestone::OtpPrompted
    }

    fn line(&mut self, source: OutputSource, text: &str) -> Option<Finding> {
        // On stderr in the login run, on stdout in the tunnel run, and again when the tunnel run
        // reconnects to log out; the milestone only moves forward.
        if source != OutputSource::Helper && text.starts_with("Connected to HTTPS on ") {
            return Some(Finding::Milestone(Milestone::ConnectedToGateway));
        }
        match source {
            OutputSource::Stdout => self.stdout_line(text),
            OutputSource::Stderr => self.stderr_line(text),
            OutputSource::Helper => None,
        }
    }

    fn routes_left_unrestored(&self) -> bool {
        false
    }
}

impl Interpreter {
    fn stdout_line(&mut self, text: &str) -> Option<Finding> {
        // Looked for anywhere in the line: text without a line break before it, such as a prompt,
        // would otherwise carry the cookie into the log.
        if let Some(at) = text.find(COOKIE_LINE) {
            return Some(session(&text[at + COOKIE_LINE.len()..]));
        }
        if text == GATEWAY_ROUTE_REQUESTED {
            return Some(Finding::GatewayRouteRequested);
        }
        if text.starts_with("Got Legacy IP address ") {
            return Some(Finding::Milestone(Milestone::Authenticated));
        }
        // "Configured as 10.212.133.105, with SSL disconnected and DTLS established"
        let ip = text
            .strip_prefix("Configured as ")?
            .split(',')
            .next()?
            .split(" + ")
            .next()?;
        let ip = ip.parse::<Ipv4Addr>().ok()?;
        Some(Finding::TunnelIp(ip))
    }

    fn stderr_line(&mut self, text: &str) -> Option<Finding> {
        if text.contains(SCRIPT_LEFT_RUNNING) {
            return Some(Finding::ScriptLeftRunning);
        }
        if text.starts_with("Adapter \"") && text.ends_with(ADDRESS_IN_USE) {
            return Some(Finding::TunnelAddressInUse);
        }
        // "None of the 1 fingerprint(s) specified via --servercert match server's certificate:
        // pin-sha256:…"
        let (start, value) = text.split_once(CERTIFICATE_REJECTED)?;
        if !start.starts_with("None of the ") {
            return None;
        }
        let value = certificate_value(value)?;
        if self.rejected_certificates.contains(&value) {
            return None;
        }
        self.rejected_certificates.push(value.clone());
        Some(Finding::CertificateRejected(value))
    }
}

/// The tunnel run reads the cookie as its stdin line, curl as a header line. A cookie that is not
/// one quoted word cannot be passed on unchanged, so it only keeps the line out of the log.
fn session(quoted: &str) -> Finding {
    let Some(cookie) = quoted
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
        .filter(|cookie| {
            !cookie.is_empty()
                && cookie
                    .bytes()
                    .all(|byte| byte.is_ascii_graphic() && byte != b'\'')
        })
    else {
        return Finding::Hidden;
    };
    Finding::Session(SessionInputs {
        tunnel: Zeroizing::new(format!("{cookie}\n").into_bytes()),
        logout: Zeroizing::new(format!("Cookie: {cookie}\n").into_bytes()),
    })
}
