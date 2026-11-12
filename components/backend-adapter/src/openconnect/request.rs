//! The values openconnect takes: checked by the app before saving them, checked again by the
//! Helper, and turned into the attempt's runs. openconnect has no config file; the password and
//! the session cookie go to stdin.

use std::env;
use std::path::PathBuf;

use fvm_contract::app_helper::RunRequest;
use fvm_contract::app_internal::problem::{AccountField, AccountValueProblem};
use fvm_contract::app_internal::section::AccountSaveError;
use fvm_contract::backend::{ClientExecutable, LaunchSpec, SessionLaunch};

use super::platform::{CURL, NULL_DEVICE, SYSTEM_VARIABLES};
use crate::values;

// openfortivpn's limits, so that one config.yaml and one password work with both clients.
const MAX_HOST_BYTES: usize = 253;
const MAX_USERNAME_BYTES: usize = 64;
const MAX_REALM_BYTES: usize = 63;
const MAX_PASSWORD_BYTES: usize = 255;

const MAX_TRUSTED_PINS: usize = 8;

const PIN_PREFIX: &str = "pin-sha256:";
/// The SHA-256 of nothing the gateway presents. openconnect run without any pin stops at the
/// unknown certificate and waits for an answer on its console, printing nothing on the pipes; with
/// a pin that does not match, it prints the gateway's pin and exits by itself.
const PLACEHOLDER_PIN: &str = "pin-sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

/// curl's whole wait for the gateway's answer to the logout request.
const LOGOUT_SECONDS: &str = "10";

/// Three runs: the login, which prints the session cookie and exits; the tunnel run, which reads
/// the cookie on stdin; and the logout request for when the Helper has to end the tunnel run by
/// force.
pub(super) fn launch_spec(
    client: &ClientExecutable,
    request: &RunRequest,
) -> Result<LaunchSpec, String> {
    check(request)?;
    let pins: Vec<&str> = if request.trusted_certs.is_empty() {
        vec![PLACEHOLDER_PIN]
    } else {
        request.trusted_certs.iter().map(String::as_str).collect()
    };
    let servercert = pins.iter().map(|pin| format!("--servercert={pin}"));
    let url = url(request);
    let env: Vec<(String, String)> = SYSTEM_VARIABLES
        .iter()
        .filter_map(|&name| Some((name.to_owned(), env::var(name).ok()?)))
        .collect();
    // No verbosity option, which would print the cookie, OTP or password; no password option,
    // since the certificate question would take the password line as its answer; no
    // non-interactive option, which also stops the OTP prompt.
    let login = [
        "--protocol=fortinet".to_owned(),
        format!("--user={}", request.username),
    ]
    .into_iter()
    .chain(servercert.clone())
    .chain(["--authenticate".to_owned(), url.clone()])
    .collect();
    // The gateway refuses a dropped session, so waiting to reconnect only hides the drop: with 0
    // openconnect gives up after its first attempt fails and the app logs in again.
    let tunnel = ["--protocol=fortinet".to_owned()]
        .into_iter()
        .chain(servercert)
        .chain([
            "--reconnect-timeout=0".to_owned(),
            "--cookie-on-stdin".to_owned(),
            url,
        ])
        .collect();
    let run = |args| LaunchSpec {
        program: client.path.clone(),
        args,
        env: env.clone(),
        config_file: None,
        session: None,
    };
    Ok(LaunchSpec {
        session: Some(Box::new(SessionLaunch {
            tunnel: run(tunnel),
            logout: logout(request, &pins, env.clone()),
        })),
        ..run(login)
    })
}

/// What openconnect itself sends when it logs out. curl checks the gateway's public key against the
/// pins even with -k (needed for the self-signed certificate), reads the cookie as a header from
/// stdin and prints only the status code; -q skips any curlrc and --noproxy any proxy setting.
fn logout(request: &RunRequest, pins: &[&str], env: Vec<(String, String)>) -> LaunchSpec {
    let pinned = pins
        .iter()
        .map(|pin| {
            let hash = pin.strip_prefix(PIN_PREFIX).expect("the pins were checked");
            format!("sha256//{hash}")
        })
        .collect::<Vec<_>>()
        .join(";");
    let args = [
        "-q",
        "-s",
        "-k",
        "--noproxy",
        "*",
        "--max-time",
        LOGOUT_SECONDS,
        "--pinnedpubkey",
        &pinned,
        "-H",
        "@-",
        "-o",
        NULL_DEVICE,
        "-w",
        "HTTP %{http_code}",
        &format!("https://{}:{}/remote/logout", request.host, request.port),
    ]
    .map(str::to_owned)
    .to_vec();
    LaunchSpec {
        program: PathBuf::from(CURL),
        args,
        env,
        config_file: None,
        session: None,
    }
}

/// The fortinet protocol takes the realm as the URL's path: the gateway redirects `/<realm>` to its
/// login page for that realm.
fn url(request: &RunRequest) -> String {
    let realm = request.realm.as_deref().unwrap_or_default();
    format!("https://{}:{}/{realm}", request.host, request.port)
}

fn check(request: &RunRequest) -> Result<(), String> {
    values::check_host(&request.host, MAX_HOST_BYTES)?;
    if !(1..=65_535).contains(&request.port) {
        return Err("port must be between 1 and 65535".to_owned());
    }
    values::check_text(
        "username",
        &request.username,
        MAX_USERNAME_BYTES,
        is_username_character,
    )?;
    if let Some(realm) = request.realm.as_deref().filter(|realm| !realm.is_empty()) {
        values::check_text("realm", realm, MAX_REALM_BYTES, is_realm_character)?;
    }
    if request.trusted_certs.len() > MAX_TRUSTED_PINS {
        return Err(format!("servercert allows at most {MAX_TRUSTED_PINS} pins"));
    }
    if let Some(index) = request
        .trusted_certs
        .iter()
        .position(|value| certificate_value(value).is_none())
    {
        return Err(format!(
            "servercert entry {index} is not a pin-sha256: value"
        ));
    }
    check_password(&request.password)
}

fn check_password(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("password must not be empty".to_owned());
    }
    if let Some(character) = password.chars().find(|&c| !is_password_character(c)) {
        let code = u32::from(character);
        return Err(format!(
            "password contains forbidden character U+{code:04X}"
        ));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(format!(
            "password must be at most {MAX_PASSWORD_BYTES} bytes"
        ));
    }
    Ok(())
}

/// openconnect for Windows reads its arguments and stdin in the system's legacy code page, so
/// anything outside printable ASCII may reach the gateway changed.
fn is_username_character(c: char) -> bool {
    c.is_ascii() && !c.is_ascii_control()
}

/// Printable ASCII and the space, as for the username; a line break would end the read early.
fn is_password_character(c: char) -> bool {
    is_username_character(c)
}

/// Characters that stay as they are in a URL's path.
fn is_realm_character(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')
}

/// The app's check of the account before it is saved or used, with the Helper's rules: the first
/// value openconnect would refuse, in the order server address, username, realm. An empty server
/// address or username is left to the caller, as an account not set up yet; an empty realm means
/// none.
pub(super) fn check_account(
    host: &str,
    username: &str,
    realm: &str,
) -> Result<(), AccountValueProblem> {
    values::account_value(
        AccountField::Host,
        host,
        MAX_HOST_BYTES,
        values::any_character,
    )?;
    if !host.is_empty() && !values::is_host(host) {
        return Err(AccountValueProblem::InvalidHost {});
    }
    values::account_value(
        AccountField::Username,
        username,
        MAX_USERNAME_BYTES,
        is_username_character,
    )?;
    values::account_value(
        AccountField::Realm,
        realm,
        MAX_REALM_BYTES,
        is_realm_character,
    )
}

/// The app's check of a typed VPN password before it is saved. An empty one is left to the
/// caller, for which it keeps the stored password.
pub(super) fn check_vpn_password(password: &str) -> Result<(), AccountSaveError> {
    if !password.chars().all(is_password_character) || password.len() > MAX_PASSWORD_BYTES {
        return Err(AccountSaveError::InvalidVpnPassword {
            max_bytes: u32::try_from(MAX_PASSWORD_BYTES).expect("the limit fits in u32"),
        });
    }
    Ok(())
}

/// `text` as openconnect compares it: `pin-sha256:` and the standard base64 of a SHA-256 hash of
/// the gateway's public key, as openconnect prints it when it rejects the gateway.
pub(super) fn certificate_value(text: &str) -> Option<String> {
    let encoded = text.strip_prefix(PIN_PREFIX)?.as_bytes();
    // 32 bytes: 43 characters, the last of which carries 2 unused zero bits, and one '='.
    let (data, padding) = encoded.split_at_checked(43)?;
    let valid = padding == b"="
        && data.iter().all(|&byte| base64_value(byte).is_some())
        && base64_value(data[42]).is_some_and(|value| value & 0b11 == 0);
    valid.then(|| text.to_owned())
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
