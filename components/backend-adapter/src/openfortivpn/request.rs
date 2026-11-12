//! A run's values checked again by the Helper and turned into openfortivpn's launch. The error
//! texts name the field but never its value, and reach the app as they are.

use std::path::Path;

use fvm_contract::app_helper::RunRequest;
use fvm_contract::backend::{ClientExecutable, LaunchSpec};

use super::account::{
    MAX_HOST_BYTES, MAX_PASSWORD_BYTES, MAX_REALM_BYTES, MAX_USERNAME_BYTES, is_certificate_value,
};
use crate::values;

const MAX_TRUSTED_CERTS: usize = 8;

/// The only environment variable openfortivpn gets: it reads https_proxy and all_proxy.
const PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

pub(super) fn launch_spec(
    client: &ClientExecutable,
    request: &RunRequest,
    config_path: &Path,
) -> Result<LaunchSpec, String> {
    check(request)?;
    let config_path = config_path
        .to_str()
        .expect("the Helper names its config file in UTF-8");
    Ok(LaunchSpec {
        program: client.path.clone(),
        // No verbosity option, which would print the cookie, OTP or password, and no --persistent,
        // which would send the used OTP again.
        args: vec!["-c".to_owned(), config_path.to_owned()],
        env: vec![("PATH".to_owned(), PATH.to_owned())],
        config_file: Some(config_file(request)),
        session: None,
    })
}

/// Checks the fields in the order the config file lists them, then the password.
fn check(request: &RunRequest) -> Result<(), String> {
    values::check_host(&request.host, MAX_HOST_BYTES)?;
    if !(1..=65_535).contains(&request.port) {
        return Err("port must be between 1 and 65535".to_owned());
    }
    values::check_text(
        "username",
        &request.username,
        MAX_USERNAME_BYTES,
        values::any_character,
    )?;
    if let Some(realm) = realm(request) {
        values::check_text("realm", realm, MAX_REALM_BYTES, values::any_character)?;
    }
    if request.trusted_certs.len() > MAX_TRUSTED_CERTS {
        return Err(format!(
            "trusted-cert allows at most {MAX_TRUSTED_CERTS} digests"
        ));
    }
    if let Some(index) = request
        .trusted_certs
        .iter()
        .position(|value| !is_certificate_value(value))
    {
        return Err(format!(
            "trusted-cert entry {index} is not 64 lowercase hex characters"
        ));
    }
    check_password(&request.password)
}

/// The password goes to stdin, followed by '\n', so spaces at either end are kept.
fn check_password(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("password must not be empty".to_owned());
    }
    if let Some(character) = values::forbidden_in_password(password) {
        let code = u32::from(character);
        return Err(format!(
            "password contains forbidden character U+{code:04X}"
        ));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(format!(
            "password must be at most {MAX_PASSWORD_BYTES} UTF-8 bytes"
        ));
    }
    Ok(())
}

/// An empty realm means none.
fn realm(request: &RunRequest) -> Option<&str> {
    request.realm.as_deref().filter(|realm| !realm.is_empty())
}

/// Checked values hold no line break and neither start nor end with whitespace, so load_config(),
/// which splits each line at its first '=' and trims both sides, reads every value back unchanged.
fn config_file(request: &RunRequest) -> String {
    let port = request.port.to_string();
    let mut entries = vec![
        ("host", request.host.as_str()),
        ("port", port.as_str()),
        ("username", request.username.as_str()),
    ];
    entries.extend(realm(request).map(|realm| ("realm", realm)));
    entries.extend(
        request
            .trusted_certs
            .iter()
            .map(|value| ("trusted-cert", value.as_str())),
    );
    entries
        .iter()
        .map(|(key, value)| format!("{key} = {value}\n"))
        .collect()
}
