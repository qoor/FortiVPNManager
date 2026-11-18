//! The values openconnect takes, checked again by the Helper, since it does not trust the app.

use fvm_contract::app_helper::RunRequest;
use fvm_contract::validation::{
    self, MAX_HOST_BYTES, MAX_REALM_BYTES, MAX_USERNAME_BYTES, certificate_value,
    is_realm_character, is_username_character,
};

const MAX_TRUSTED_PINS: usize = 8;

pub(super) fn check(request: &RunRequest) -> Result<(), String> {
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
