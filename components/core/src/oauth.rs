//! Google OAuth with PKCE and a loopback redirect, and the Desktop app client JSON.
//!
//! OAuth for installed apps (RFC 8252) with S256 PKCE (RFC 7636). Random bytes come from the
//! runtime, which also opens the listener, the browser and the connections.

mod loopback;

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fvm_contract::app_internal::SecretText;
use fvm_contract::app_internal::section::OAuthClientFileProblem;
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub use loopback::{CallbackDecision, HeadParse, LoopbackRequest, parse_request_head};

pub const AUTHORIZATION_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub const REVOCATION_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";
/// Reading, labeling and trashing messages; no permanent deletion and no settings.
pub const GMAIL_MODIFY_SCOPE: &str = "https://www.googleapis.com/auth/gmail.modify";

/// Random bytes behind a PKCE verifier, which becomes 64 characters.
pub const VERIFIER_RANDOM_BYTES: usize = 48;
/// Random bytes behind a `state`, which becomes 32 characters.
pub const STATE_RANDOM_BYTES: usize = 24;

/// `http://127.0.0.1:<port>`, which Google accepts for Desktop app clients on any port. The token
/// request repeats it exactly.
pub fn loopback_redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The secret the app keeps until the token exchange, and its challenge for the browser.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pkce {
    verifier: SecretText,
    challenge: String,
}

impl Pkce {
    pub fn from_random(bytes: &[u8; VERIFIER_RANDOM_BYTES]) -> Self {
        Self::with_verifier(URL_SAFE_NO_PAD.encode(bytes))
    }

    fn with_verifier(verifier: String) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier: SecretText(verifier),
            challenge,
        }
    }

    pub fn verifier(&self) -> &str {
        &self.verifier.0
    }

    /// BASE64URL(SHA256(verifier)), sent with the authorization request.
    pub fn challenge(&self) -> &str {
        &self.challenge
    }
}

/// The unguessable value that ties Google's answer to this sign-in.
pub fn state_from_random(bytes: &[u8; STATE_RANDOM_BYTES]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The browser URL that starts the consent. Offline access with forced consent makes Google return
/// a refresh token every time; `login_hint` preselects the expected account.
pub fn authorization_url(
    client_id: &str,
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
    login_hint: Option<&str>,
) -> String {
    let mut pairs = vec![
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", GMAIL_MODIFY_SCOPE),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    pairs.extend(
        login_hint
            .filter(|hint| !hint.is_empty())
            .map(|hint| ("login_hint", hint)),
    );
    format!("{AUTHORIZATION_ENDPOINT}?{}", form_encoded(&pairs))
}

/// The token request body that exchanges the authorization code.
pub fn authorization_code_body(
    code: &str,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> SecretText {
    SecretText(form_encoded(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("redirect_uri", redirect_uri),
        ("code_verifier", code_verifier),
    ]))
}

/// The token request body that gets a fresh access token.
pub fn refresh_body(refresh_token: &str, client_id: &str, client_secret: &str) -> SecretText {
    SecretText(form_encoded(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ]))
}

/// The revocation request body; revoking a refresh token revokes its access tokens too.
pub fn revocation_body(token: &str) -> SecretText {
    SecretText(form_encoded(&[("token", token)]))
}

/// `application/x-www-form-urlencoded`, with every byte outside the RFC 3986 unreserved set
/// escaped; Google reads an unescaped `+` as a space.
pub fn form_encoded(pairs: &[(&str, &str)]) -> String {
    let mut encoded = String::new();
    for (index, (name, value)) in pairs.iter().enumerate() {
        if index > 0 {
            encoded.push('&');
        }
        percent_encode(name, &mut encoded);
        encoded.push('=');
        percent_encode(value, &mut encoded);
    }
    encoded
}

fn percent_encode(text: &str, output: &mut String) {
    for byte in text.bytes() {
        if is_unreserved(byte) {
            output.push(char::from(byte));
        } else {
            write!(output, "%{byte:02X}").expect("writing to a String succeeds");
        }
    }
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

// MARK: Token endpoint

/// A successful answer of the token endpoint.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TokenGrant {
    pub access_token: SecretText,
    /// As Google reports it, typically 3599 seconds.
    pub expires_in: Duration,
    /// Present for the code exchange with offline access; usually absent on refresh.
    pub refresh_token: Option<SecretText>,
    /// Google may grant fewer scopes than asked when the user unticks one. A refresh answer
    /// without scopes keeps the original grant.
    pub scopes: BTreeSet<String>,
}

impl TokenGrant {
    pub fn grants(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }
}

/// Why the token endpoint refused (RFC 6749 §5.2) or answered something unusable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TokenError {
    /// `invalid_grant`: the refresh token was revoked or expired, or the code is invalid or used.
    InvalidGrant { description: Option<String> },
    /// `invalid_client` or `unauthorized_client`: a wrong client ID or secret, or a deleted client.
    InvalidClient { description: Option<String> },
    /// Any other OAuth error code, such as `invalid_request`.
    OAuth {
        code: String,
        description: Option<String>,
    },
    /// An error status without an OAuth error, such as a 5xx.
    Server { status: u16 },
    /// A success without a usable access token and lifetime.
    MalformedResponse,
}

impl TokenError {
    /// How Google describes the refusal, such as `invalid_request: Missing code verifier.`;
    /// never holds a secret.
    pub fn detail(&self) -> String {
        let with = |code: &str, description: &Option<String>| match description {
            Some(description) => format!("{code}: {description}"),
            None => code.to_owned(),
        };
        match self {
            TokenError::InvalidGrant { description } => with("invalid_grant", description),
            TokenError::InvalidClient { description } => with("invalid_client", description),
            TokenError::OAuth { code, description } => with(code, description),
            TokenError::Server { status } => format!("HTTP {status}"),
            TokenError::MalformedResponse => "malformed token response".to_owned(),
        }
    }
}

/// Reads an answer of the token endpoint, for the code exchange and for a refresh.
pub fn read_token_response(status: u16, body: &[u8]) -> Result<TokenGrant, TokenError> {
    #[derive(Deserialize)]
    struct Body {
        access_token: Option<String>,
        expires_in: Option<f64>,
        refresh_token: Option<String>,
        scope: Option<String>,
        error: Option<String>,
        error_description: Option<String>,
    }

    let body = serde_json::from_slice::<Body>(body).ok();
    if let Some(body) = &body
        && let Some(code) = body.error.as_ref().filter(|code| !code.is_empty())
    {
        let description = body.error_description.clone();
        return Err(match code.as_str() {
            "invalid_grant" => TokenError::InvalidGrant { description },
            "invalid_client" | "unauthorized_client" => TokenError::InvalidClient { description },
            _ => TokenError::OAuth {
                code: code.clone(),
                description,
            },
        });
    }
    if !(200..=299).contains(&status) {
        return Err(TokenError::Server { status });
    }
    let body = body.ok_or(TokenError::MalformedResponse)?;
    let access_token = body
        .access_token
        .filter(|token| !token.is_empty())
        .ok_or(TokenError::MalformedResponse)?;
    let expires_in = body
        .expires_in
        .filter(|seconds| *seconds > 0.0)
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .ok_or(TokenError::MalformedResponse)?;
    Ok(TokenGrant {
        access_token: SecretText(access_token),
        expires_in,
        refresh_token: body
            .refresh_token
            .filter(|token| !token.is_empty())
            .map(SecretText),
        scopes: body
            .scope
            .unwrap_or_default()
            .split(' ')
            .filter(|scope| !scope.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    })
}

// MARK: Client JSON

/// The parts of a Google Cloud "Desktop app" OAuth client the app uses. The secret is not
/// confidential for such a client (PKCE protects the sign-in) but stays out of debug output.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: SecretText,
}

/// Real client IDs and secrets are about 75 and 35 characters.
const MAX_CLIENT_FIELD_BYTES: usize = 256;
const CLIENT_ID_SUFFIX: &str = ".apps.googleusercontent.com";

/// Reads the `installed` section of a client JSON downloaded from the Google Cloud console, or
/// built into the app.
pub fn parse_client_file(bytes: &[u8]) -> Result<OAuthClient, OAuthClientFileProblem> {
    let object = serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(bytes)
        .map_err(|_| OAuthClientFileProblem::NotJson {})?;
    let Some(serde_json::Value::Object(installed)) = object.get("installed") else {
        return Err(if object.contains_key("web") {
            OAuthClientFileProblem::WebClient {}
        } else {
            OAuthClientFileProblem::NoDesktopClient {}
        });
    };
    let field = |name: &str| -> Result<String, OAuthClientFileProblem> {
        match installed.get(name) {
            None => Err(OAuthClientFileProblem::MissingField { name: name.into() }),
            Some(serde_json::Value::String(text)) if !text.is_empty() => Ok(text.clone()),
            Some(_) => Err(OAuthClientFileProblem::InvalidField { name: name.into() }),
        }
    };
    let client_id = field("client_id")?;
    if !is_valid_client_id(&client_id) {
        return Err(OAuthClientFileProblem::InvalidField {
            name: "client_id".into(),
        });
    }
    let client_secret = field("client_secret")?;
    let secret_valid = client_secret.len() <= MAX_CLIENT_FIELD_BYTES
        && client_secret.bytes().all(|byte| byte.is_ascii_graphic());
    if !secret_valid {
        return Err(OAuthClientFileProblem::InvalidField {
            name: "client_secret".into(),
        });
    }
    Ok(OAuthClient {
        client_id,
        client_secret: SecretText(client_secret),
    })
}

/// `<number>-<id>.apps.googleusercontent.com` in `A-Z a-z 0-9 . _ -`.
fn is_valid_client_id(value: &str) -> bool {
    value.len() <= MAX_CLIENT_FIELD_BYTES
        && value.len() > CLIENT_ID_SUFFIX.len()
        && value.ends_with(CLIENT_ID_SUFFIX)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_'))
}
