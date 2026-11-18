//! The values openconnect takes: checked by the app before saving them, checked again by the
//! Helper, and turned into the attempt's runs. openconnect has no config file; the password and
//! the session cookie go to stdin.

use std::env;
use std::path::{Path, PathBuf};

use fvm_contract::app_helper::RunRequest;
use fvm_contract::backend::{ClientExecutable, LaunchSpec, SessionLaunch};
use fvm_contract::validation::{
    self, MAX_HOST_BYTES, MAX_REALM_BYTES, MAX_USERNAME_BYTES, PIN_PREFIX, certificate_value,
    is_realm_character, is_username_character,
};

use super::platform::{self, CURL, NULL_DEVICE, SYSTEM_VARIABLES};

const MAX_TRUSTED_PINS: usize = 8;

/// The SHA-256 of nothing the gateway presents. openconnect run without any pin stops at the
/// unknown certificate and waits for an answer on its console, printing nothing on the pipes; with
/// a pin that does not match, it prints the gateway's pin and exits by itself.
const PLACEHOLDER_PIN: &str = "pin-sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

/// curl's whole wait for the gateway's answer to the logout request.
const LOGOUT_SECONDS: &str = "10";

pub(super) const VALUES_VARIABLE: &str = "FVM_VALUES_FILE";

/// Three runs: the login, which prints the session cookie and exits; the tunnel run, which reads
/// the cookie on stdin; and the logout request for when the Helper has to end the tunnel run by
/// force.
pub(super) fn launch_spec(
    client: &ClientExecutable,
    request: &RunRequest,
    values_path: &Path,
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
    let script = platform::script();
    // The gateway refuses a dropped session, so waiting to reconnect only hides the drop: with 0
    // openconnect gives up after its first attempt fails and the app logs in again.
    let tunnel = ["--protocol=fortinet".to_owned()]
        .into_iter()
        .chain(servercert)
        .chain(
            script
                .as_ref()
                .map(|(argument, _)| format!("--script={argument}")),
        )
        .chain((!request.dtls).then(|| "--no-dtls".to_owned()))
        .chain([
            "--reconnect-timeout=0".to_owned(),
            "--cookie-on-stdin".to_owned(),
            url,
        ])
        .collect();
    let values = values_path
        .to_str()
        .expect("the Helper's run directory has a UTF-8 path")
        .to_owned();
    let mut tunnel_env = env.clone();
    tunnel_env.push((VALUES_VARIABLE.to_owned(), values));
    let login = LaunchSpec {
        program: client.path.clone(),
        args: login,
        env: env.clone(),
        config_file: None,
        session: None,
        script_runner: None,
    };
    let tunnel = LaunchSpec {
        args: tunnel,
        env: tunnel_env,
        script_runner: script.map(|(_, runner)| runner),
        ..login.clone()
    };
    Ok(LaunchSpec {
        session: Some(Box::new(SessionLaunch {
            tunnel,
            logout: logout(request, &pins, env),
        })),
        ..login
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
        script_runner: None,
    }
}

/// The fortinet protocol takes the realm as the URL's path: the gateway redirects `/<realm>` to its
/// login page for that realm.
fn url(request: &RunRequest) -> String {
    let realm = request.realm.as_deref().unwrap_or_default();
    format!("https://{}:{}/{realm}", request.host, request.port)
}

fn check(request: &RunRequest) -> Result<(), String> {
    validation::check_host(&request.host, MAX_HOST_BYTES)?;
    if !(1..=65_535).contains(&request.port) {
        return Err("port must be between 1 and 65535".to_owned());
    }
    validation::check_text(
        "username",
        &request.username,
        MAX_USERNAME_BYTES,
        is_username_character,
    )?;
    if let Some(realm) = request.realm.as_deref().filter(|realm| !realm.is_empty()) {
        validation::check_text("realm", realm, MAX_REALM_BYTES, is_realm_character)?;
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
    validation::check_password(&request.password)
}
