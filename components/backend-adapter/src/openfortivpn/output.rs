//! Reading one openfortivpn run's output. openfortivpn logs to stdout with a log.c level prefix;
//! stderr carries pppd's messages, and unprefixed stdout is /sbin/route output, a prompt or raw
//! tunnel data, so only prefixed stdout lines are read.

use std::net::Ipv4Addr;

use fvm_contract::app_helper::{Milestone, OutputSource};
use fvm_contract::backend::{Finding, OutputInterpreter, Prompt};

use super::request::certificate_value;

// read_password() prints these with no line break and then blocks reading stdin.
const PASSWORD_PROMPT: &[u8] = b"VPN account password: "; // main.c
const OTP_PROMPT: &[u8] = b"Two-factor authentication token: "; // http.c

#[derive(Default)]
pub struct Interpreter {
    /// In the order printed. openfortivpn prints a rejection again when it reconnects to log out.
    rejected_certificates: Vec<String>,
    routes: Routes,
    /// After "Received bad header from gateway:" openfortivpn copies up to 250 bytes of tunnel data
    /// to stdout, line breaks included (io.c). That text can look like log lines, so from then on
    /// only route restoration, which happens at teardown, is still followed.
    after_raw_dump: bool,
}

#[derive(Default)]
enum Routes {
    #[default]
    Unchanged,
    Changed,
    Restoring,
    Restored,
}

enum Level {
    Info,
    Warn,
    Error,
    Debug,
}

impl OutputInterpreter for Interpreter {
    fn prompt(&mut self, source: OutputSource, partial_line: &[u8]) -> Option<Prompt> {
        if source != OutputSource::Stdout {
            None
        } else if partial_line.ends_with(PASSWORD_PROMPT) {
            Some(Prompt::Password)
        } else if partial_line.ends_with(OTP_PROMPT) {
            Some(Prompt::Otp)
        } else {
            None
        }
    }

    /// openfortivpn asks for the password at startup, before it connects to the gateway.
    fn password_expected(&self, milestone: Milestone) -> bool {
        milestone < Milestone::ConnectedToGateway
    }

    fn line(&mut self, source: OutputSource, text: &str) -> Option<Finding> {
        if source != OutputSource::Stdout {
            return None;
        }
        let (level, message) = split_level(text)?;
        // Restoration logs only WARN and DEBUG lines and /sbin/route's own output (ipv4.c), apart
        // from "/sbin/route: <error>." when the binary is missing. The next INFO or ERROR line is
        // printed after it returned (tunnel.c).
        if matches!(self.routes, Routes::Restoring)
            && matches!(level, Level::Info | Level::Error)
            && !message.starts_with("/sbin/route: ")
        {
            self.routes = Routes::Restored;
        }
        match message {
            "Restoring routes..." => {
                if matches!(self.routes, Routes::Changed) {
                    self.routes = Routes::Restoring;
                }
                return None;
            }
            "Received bad header from gateway:" => {
                self.after_raw_dump = true;
                return None;
            }
            _ if self.after_raw_dump => return None,
            "Connected to gateway." => {
                return Some(Finding::Milestone(Milestone::ConnectedToGateway));
            }
            "Authenticated." => return Some(Finding::Milestone(Milestone::Authenticated)),
            "Tunnel is up and running." => return Some(Finding::Milestone(Milestone::TunnelUp)),
            "Setting new routes..." => {
                self.routes = Routes::Changed;
                return None;
            }
            _ => {}
        }
        if let Some(addresses) = message.strip_prefix("Got addresses: ") {
            return tunnel_ip(addresses).map(Finding::TunnelIp);
        }
        if let Some(value) = message.strip_prefix("    trusted-cert = ") {
            return self.certificate_rejected(value);
        }
        // "Could not authenticate to the gateway. Please make sure tunnel mode is allowed ..." (io.c);
        // the other variant (tunnel.c) is a failed login.
        if message.starts_with("Could not authenticate to ") && message.contains("tunnel mode") {
            return Some(Finding::TunnelModeRejected);
        }
        None
    }

    fn routes_left_unrestored(&self) -> bool {
        matches!(self.routes, Routes::Changed | Routes::Restoring)
    }
}

impl Interpreter {
    /// `value` is the SHA-256 digest in hex, from the advice line openfortivpn prints with the
    /// rejection (tunnel.c).
    fn certificate_rejected(&mut self, value: &str) -> Option<Finding> {
        // In the form the next run accepts as a trusted value.
        let value = certificate_value(value.trim_matches(' '))?;
        if self.rejected_certificates.contains(&value) {
            return None;
        }
        self.rejected_certificates.push(value.clone());
        Some(Finding::CertificateRejected(value))
    }
}

/// log.c's prefixes, all eight bytes long.
fn split_level(text: &str) -> Option<(Level, &str)> {
    [
        ("INFO:   ", Level::Info),
        ("WARN:   ", Level::Warn),
        ("ERROR:  ", Level::Error),
        ("DEBUG:  ", Level::Debug),
    ]
    .into_iter()
    .find_map(|(prefix, level)| Some((level, text.strip_prefix(prefix)?)))
}

/// `[ip], ns [ns1, ns2]`, optionally followed by `, ns_suffix [suffix]` (io.c).
fn tunnel_ip(addresses: &str) -> Option<Ipv4Addr> {
    let (ip, rest) = addresses.strip_prefix('[')?.split_once(']')?;
    let (name_servers, rest) = rest.strip_prefix(", ns [")?.split_once(']')?;
    let name_servers_valid = name_servers
        .split(',')
        .all(|server| server.trim_matches(' ').parse::<Ipv4Addr>().is_ok());
    let suffix_valid =
        rest.is_empty() || (rest.starts_with(", ns_suffix [") && rest.ends_with(']'));
    if name_servers_valid && suffix_valid {
        ip.parse().ok()
    } else {
        None
    }
}
