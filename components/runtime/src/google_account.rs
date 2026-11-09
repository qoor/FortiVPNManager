//! The Google account for the Gmail API (SPEC F-GA-01, F-GA-02): the OAuth client (built in, or
//! imported with its secret in the secret storage), the browser sign-in with a loopback redirect and
//! PKCE, access tokens from the stored refresh token, and disconnecting.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

use fvm_contract::app_internal::port::{
    LogCategory, LogLevel, PortRequest, PortResponse, SecretItem, SecretStoreFailure,
};
use fvm_contract::app_internal::problem::FileProblem;
use fvm_contract::app_internal::section::{GoogleError, OAuthClientSource};
use fvm_contract::app_internal::{SecretText, SignInPages};
use fvm_core::gmail_api::{self, ApiError, Profile};
use fvm_core::oauth::{self, CallbackDecision, HeadParse, OAuthClient, Pkce, TokenError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

use crate::files;
use crate::net;
use crate::orchestration::{Queue, Runtime};

const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// A browser connection that sends nothing in this time is dropped.
const LOOPBACK_CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CLIENT_FILE_BYTES: u64 = 64 * 1024;
/// A cached access token is used while it has at least this long left.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

/// Why no access token could be had.
#[derive(Clone, Debug)]
pub(crate) enum TokenFailure {
    NoClient,
    NotSignedIn,
    Revoked,
    ClientRejected,
    SecretStore(SecretStoreFailure),
    Network(String),
    Server(String),
}

impl TokenFailure {
    /// No retry can help; a session may fall back to IMAP.
    pub(crate) fn is_authentication(&self) -> bool {
        matches!(
            self,
            Self::NoClient
                | Self::NotSignedIn
                | Self::Revoked
                | Self::ClientRejected
                | Self::SecretStore(_)
        )
    }

    pub(crate) fn is_transient(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Server(_))
    }

    pub(crate) fn describe(&self) -> String {
        match self {
            Self::NoClient => "no OAuth client".into(),
            Self::NotSignedIn => "not signed in with Google".into(),
            Self::Revoked => "the Google authorization was revoked or has expired".into(),
            Self::ClientRejected => "Google refused the OAuth client".into(),
            Self::SecretStore(failure) => format!("keychain: {}", failure.description),
            Self::Network(detail) => format!("network error: {detail}"),
            Self::Server(detail) => format!("Google token endpoint: {detail}"),
        }
    }
}

#[derive(Default)]
pub(crate) struct Google {
    pub(crate) error: Option<GoogleError>,
    /// Ends the running browser sign-in.
    cancel_sign_in: Option<oneshot::Sender<()>>,
    pub(crate) disconnecting: bool,
    /// Access tokens by address and client ID.
    tokens: HashMap<(String, String), (SecretText, SystemTime)>,
    /// What the last lookup found.
    pub(crate) imported_secret_stored: bool,
    pub(crate) authorized: bool,
}

/// Addresses are compared and stored without regard to case.
pub(crate) fn normalized(address: &str) -> String {
    address.trim().to_lowercase()
}

impl Google {
    pub(crate) fn signing_in(&self) -> bool {
        self.cancel_sign_in.is_some()
    }
}

impl Runtime {
    fn built_in_client(&self) -> Option<OAuthClient> {
        let json = self.facts.built_in_oauth_client.as_ref()?;
        oauth::parse_client_file(json.0.as_bytes()).ok()
    }

    fn imported_client_id(&self) -> String {
        self.config.settings().otp.google_client_id.clone()
    }

    /// The client in use: the imported one while its secret is stored, else the built-in one.
    pub(crate) fn oauth_client_source(&self) -> OAuthClientSource {
        let imported = self.imported_client_id();
        if !imported.is_empty() && self.google.imported_secret_stored {
            return OAuthClientSource::Imported {
                client_id: imported,
            };
        }
        match self.built_in_client() {
            Some(client) => OAuthClientSource::BuiltIn {
                client_id: client.client_id,
            },
            None => OAuthClientSource::NotConfigured {},
        }
    }

    /// Looks up whether the imported client's secret and the saved address's refresh token are
    /// stored, without reading them.
    pub(crate) fn refresh_google_state(&mut self) {
        let imported = self.imported_client_id();
        let item = (!imported.is_empty()).then_some(SecretItem::GoogleClientSecret {
            client_id: imported,
        });
        self.has_secret(item, |runtime, stored| {
            runtime.google.imported_secret_stored = stored;
            let address = normalized(&runtime.config.settings().otp.gmail_address);
            runtime.gmail_api_authorized(&address, |runtime, authorized| {
                runtime.google.authorized = authorized
            });
        });
    }

    /// Whether the Gmail API can be used for `address`: a usable client and a refresh token.
    pub(crate) fn gmail_api_authorized(
        &mut self,
        address: &str,
        then: impl FnOnce(&mut Runtime, bool) + 'static,
    ) {
        let address = normalized(address);
        if address.is_empty()
            || matches!(
                self.oauth_client_source(),
                OAuthClientSource::NotConfigured {}
            )
        {
            return then(self, false);
        }
        self.has_secret(Some(SecretItem::GoogleRefreshToken { address }), then);
    }

    /// The client in use with its secret.
    fn client_credentials(
        &mut self,
        then: impl FnOnce(&mut Runtime, Result<OAuthClient, TokenFailure>) + 'static,
    ) {
        match self.oauth_client_source() {
            OAuthClientSource::Imported { client_id } => {
                let item = SecretItem::GoogleClientSecret {
                    client_id: client_id.clone(),
                };
                self.request(
                    PortRequest::ReadSecret { item },
                    move |runtime, response| {
                        let result = match response {
                            PortResponse::Secret {
                                value: Some(secret),
                            } if !secret.0.is_empty() => Ok(OAuthClient {
                                client_id,
                                client_secret: secret,
                            }),
                            PortResponse::Secret { .. } => Err(TokenFailure::NoClient),
                            PortResponse::SecretStoreFailed { failure } => {
                                Err(TokenFailure::SecretStore(failure))
                            }
                            other => panic!("a secret read answered with {other:?}"),
                        };
                        then(runtime, result);
                    },
                );
            }
            OAuthClientSource::BuiltIn { .. } => {
                let client = self.built_in_client().expect("the source is built in");
                then(self, Ok(client));
            }
            OAuthClientSource::NotConfigured {} => then(self, Err(TokenFailure::NoClient)),
        }
    }

    /// An access token for `address`: the cached one, or a new one from the refresh token.
    pub(crate) fn access_token(
        &mut self,
        address: &str,
        then: impl FnOnce(&mut Runtime, Result<SecretText, TokenFailure>) + Send + 'static,
    ) {
        let address = normalized(address);
        let client_id = match self.oauth_client_source() {
            OAuthClientSource::Imported { client_id }
            | OAuthClientSource::BuiltIn { client_id } => client_id,
            OAuthClientSource::NotConfigured {} => return then(self, Err(TokenFailure::NoClient)),
        };
        // Checked before the client secret is read: a session asks several times a second.
        let key = (address.clone(), client_id);
        if let Some((token, expires)) = self.google.tokens.get(&key)
            && expires
                .duration_since(SystemTime::now())
                .unwrap_or_default()
                > EXPIRY_MARGIN
        {
            let token = token.clone();
            return then(self, Ok(token));
        }
        self.client_credentials(move |runtime, client| {
            let client = match client {
                Ok(client) => client,
                Err(failure) => return then(runtime, Err(failure)),
            };
            let item = SecretItem::GoogleRefreshToken {
                address: address.clone(),
            };
            runtime.request(
                PortRequest::ReadSecret { item },
                move |runtime, response| {
                    let refresh_token = match response {
                        PortResponse::Secret { value: Some(token) } if !token.0.is_empty() => token,
                        PortResponse::Secret { .. } => {
                            return then(runtime, Err(TokenFailure::NotSignedIn));
                        }
                        PortResponse::SecretStoreFailed { failure } => {
                            return then(runtime, Err(TokenFailure::SecretStore(failure)));
                        }
                        other => panic!("a secret read answered with {other:?}"),
                    };
                    let body = oauth::refresh_body(
                        &refresh_token.0,
                        &client.client_id,
                        &client.client_secret.0,
                    );
                    let queue = runtime.queue();
                    runtime.tasks.spawn(async move {
                        let reply = net::post_form(oauth::TOKEN_ENDPOINT, &body).await;
                        queue.run(move |runtime| {
                            runtime.token_refreshed(key, refresh_token, reply, then)
                        });
                    });
                },
            );
        });
    }

    fn token_refreshed(
        &mut self,
        key: (String, String),
        refresh_token: SecretText,
        reply: Result<net::Reply, net::NetworkError>,
        then: impl FnOnce(&mut Runtime, Result<SecretText, TokenFailure>) + Send + 'static,
    ) {
        let reply = match reply {
            Ok(reply) => reply,
            Err(detail) => return then(self, Err(TokenFailure::Network(detail))),
        };
        match oauth::read_token_response(reply.status, &reply.body) {
            Ok(grant) => {
                self.google.tokens.insert(
                    key,
                    (
                        grant.access_token.clone(),
                        SystemTime::now() + grant.expires_in,
                    ),
                );
                then(self, Ok(grant.access_token));
            }
            Err(TokenError::InvalidGrant { .. }) => {
                self.system_log(
                    LogLevel::Error,
                    LogCategory::Oauth,
                    "Google refused the refresh token (invalid_grant); deleting it",
                );
                let address = key.0.clone();
                let item = SecretItem::GoogleRefreshToken {
                    address: address.clone(),
                };
                // Only the token that was refused is deleted, not one a newer sign-in stored.
                self.request(PortRequest::ReadSecret { item: item.clone() }, move |runtime, response| {
                    if matches!(response, PortResponse::Secret { value: Some(ref current) } if *current == refresh_token) {
                        runtime.request(PortRequest::DeleteSecret { item }, |runtime, _| {
                            runtime.google.tokens.clear();
                            runtime.refresh_google_state();
                        });
                    }
                    then(runtime, Err(TokenFailure::Revoked));
                });
            }
            Err(TokenError::InvalidClient { .. }) => {
                self.system_log(
                    LogLevel::Error,
                    LogCategory::Oauth,
                    "Google refused the OAuth client during a token refresh",
                );
                then(self, Err(TokenFailure::ClientRejected));
            }
            Err(error) => then(self, Err(TokenFailure::Server(error.detail()))),
        }
    }

    pub(crate) fn invalidate_access_token(&mut self, address: &str) {
        let address = normalized(address);
        self.google
            .tokens
            .retain(|(cached, _), _| *cached != address);
    }

    // MARK: Sign-in

    /// Why Connect cannot run now, as the Gmail section shows it.
    pub(crate) fn google_connect_blocker(
        &self,
    ) -> Option<fvm_contract::app_internal::section::GoogleConnectBlocker> {
        use fvm_contract::app_internal::section::GoogleConnectBlocker;
        if matches!(
            self.oauth_client_source(),
            OAuthClientSource::NotConfigured {}
        ) {
            return Some(GoogleConnectBlocker::NoOAuthClient);
        }
        let typed = self.forms.typed_gmail_address();
        if typed.is_empty() {
            return Some(GoogleConnectBlocker::EnterAddress);
        }
        if typed != self.config.settings().otp.gmail_address {
            return Some(GoogleConnectBlocker::SaveAddressFirst);
        }
        None
    }

    /// The browser sign-in for the saved Gmail address; closing Settings does not stop it.
    pub(crate) fn connect_google_account(&mut self) {
        if self.google_connect_blocker().is_some()
            || self.google.disconnecting
            || self.google.signing_in()
        {
            return;
        }
        self.google.error = None;
        self.reset_gmail_test();
        let address = normalized(&self.config.settings().otp.gmail_address);
        let (cancel, cancelled) = oneshot::channel();
        self.google.cancel_sign_in = Some(cancel);
        self.client_credentials(move |runtime, client| {
            let client = match client {
                Ok(client) => client,
                Err(failure) => return runtime.sign_in_failed(token_error(failure)),
            };
            let pages = runtime.facts.sign_in_pages.clone();
            let queue = runtime.queue();
            runtime.tasks.spawn(async move {
                let outcome = sign_in(&queue, client, address, pages, cancelled).await;
                queue.run(move |runtime| match outcome {
                    Ok(()) => {
                        runtime.google.cancel_sign_in = None;
                        runtime.system_log(
                            LogLevel::Notice,
                            LogCategory::Oauth,
                            "Google sign-in completed; refresh token stored",
                        );
                        runtime.refresh_google_state();
                        runtime.feed(fvm_core::connection::Input::RecheckReadiness);
                    }
                    Err(None) => runtime.google.cancel_sign_in = None,
                    Err(Some(error)) => runtime.sign_in_failed(error),
                });
            });
        });
    }

    fn sign_in_failed(&mut self, error: GoogleError) {
        self.google.cancel_sign_in = None;
        self.system_log(
            LogLevel::Error,
            LogCategory::Oauth,
            format!("Google sign-in failed: {error:?}"),
        );
        self.google.error = Some(error);
    }

    pub(crate) fn cancel_google_sign_in(&mut self) {
        if let Some(cancel) = self.google.cancel_sign_in.take() {
            let _ = cancel.send(());
        }
    }

    /// Deletes the saved address's refresh token and revokes it at Google. Connect waits for the
    /// revocation, so it cannot revoke a newer sign-in.
    pub(crate) fn disconnect_google_account(&mut self) {
        let address = normalized(&self.config.settings().otp.gmail_address);
        if address.is_empty() || self.google.disconnecting {
            return;
        }
        self.google.disconnecting = true;
        self.google.error = None;
        self.reset_gmail_test();
        let item = SecretItem::GoogleRefreshToken { address };
        self.request(
            PortRequest::ReadSecret { item: item.clone() },
            move |runtime, response| {
                let token = match response {
                    PortResponse::Secret { value } => value,
                    _ => None,
                };
                runtime.request(
                    PortRequest::DeleteSecret { item },
                    move |runtime, response| {
                        if let PortResponse::SecretStoreFailed { failure } = response {
                            runtime.google.disconnecting = false;
                            runtime.google.error = Some(GoogleError::SecretStore { failure });
                            return;
                        }
                        runtime.google.tokens.clear();
                        runtime.system_log(
                            LogLevel::Notice,
                            LogCategory::Oauth,
                            "Google authorization removed",
                        );
                        runtime.refresh_google_state();
                        runtime.feed(fvm_core::connection::Input::RecheckReadiness);
                        let Some(token) = token else {
                            runtime.google.disconnecting = false;
                            return;
                        };
                        let queue = runtime.queue();
                        runtime.tasks.spawn(async move {
                            let reply = net::post_form(
                                oauth::REVOCATION_ENDPOINT,
                                &oauth::revocation_body(&token.0),
                            )
                            .await;
                            queue.run(move |runtime| {
                                runtime.google.disconnecting = false;
                                let message = match reply {
                                    Ok(reply) => format!(
                                        "Google token revocation answered HTTP {}",
                                        reply.status
                                    ),
                                    Err(detail) => {
                                        format!("Google token revocation failed: {detail}")
                                    }
                                };
                                runtime.system_log(LogLevel::Notice, LogCategory::Oauth, message);
                            });
                        });
                    },
                );
            },
        );
    }

    // MARK: The OAuth client

    /// Imports a Desktop app client file: its secret goes to the secret storage, its ID to the
    /// settings file. A different client makes the stored sign-in unusable, so it is deleted.
    pub(crate) fn import_oauth_client(&mut self, file: &str) {
        self.google.error = None;
        self.reset_gmail_test();
        let bytes = match files::read_regular(Path::new(file), MAX_CLIENT_FILE_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                return self.google.error = Some(GoogleError::ClientFileUnreadable {
                    problem: FileProblem::NotFound {},
                });
            }
            Err(error) => {
                let problem = match error.kind() {
                    std::io::ErrorKind::InvalidInput => FileProblem::NotRegularFile {},
                    std::io::ErrorKind::InvalidData => FileProblem::TooLarge {
                        limit_bytes: MAX_CLIENT_FILE_BYTES,
                    },
                    _ => FileProblem::Io {
                        operation: "read".into(),
                        path: file.into(),
                        errno: error.raw_os_error().unwrap_or(0),
                    },
                };
                return self.google.error = Some(GoogleError::ClientFileUnreadable { problem });
            }
        };
        let client = match oauth::parse_client_file(&bytes) {
            Ok(client) => client,
            Err(problem) => {
                return self.google.error = Some(GoogleError::ClientFileInvalid { problem });
            }
        };
        let previous = match self.oauth_client_source() {
            OAuthClientSource::Imported { client_id }
            | OAuthClientSource::BuiltIn { client_id } => client_id,
            OAuthClientSource::NotConfigured {} => String::new(),
        };
        let previous_import = self.imported_client_id();
        let item = SecretItem::GoogleClientSecret {
            client_id: client.client_id.clone(),
        };
        self.request(
            PortRequest::WriteSecret {
                item,
                value: client.client_secret.clone(),
            },
            move |runtime, response| {
                if let PortResponse::SecretStoreFailed { failure } = response {
                    runtime.google.error = Some(GoogleError::SecretStore { failure });
                    return;
                }
                let id = client.client_id.clone();
                match runtime
                    .config
                    .update(|settings| settings.otp.google_client_id.clone_from(&id))
                {
                    Ok(_) => runtime.config_reloaded(),
                    Err(error) => {
                        runtime.google.error = Some(GoogleError::Config { error });
                        return;
                    }
                }
                if previous != client.client_id {
                    let address = normalized(&runtime.config.settings().otp.gmail_address);
                    if !address.is_empty() {
                        runtime.request(
                            PortRequest::DeleteSecret {
                                item: SecretItem::GoogleRefreshToken { address },
                            },
                            |_, _| {},
                        );
                    }
                    if !previous_import.is_empty() && previous_import != client.client_id {
                        let item = SecretItem::GoogleClientSecret {
                            client_id: previous_import,
                        };
                        runtime.request(PortRequest::DeleteSecret { item }, |_, _| {});
                    }
                    runtime.google.tokens.clear();
                }
                runtime.system_log(
                    LogLevel::Notice,
                    LogCategory::Oauth,
                    format!("Imported Google OAuth client {}", client.client_id),
                );
                runtime.refresh_google_state();
                runtime.feed(fvm_core::connection::Input::RecheckReadiness);
            },
        );
    }

    /// Forgets the imported client. Its sign-in goes too, unless the built-in client is the same.
    pub(crate) fn use_built_in_oauth_client(&mut self) {
        self.google.error = None;
        self.reset_gmail_test();
        let imported = self.imported_client_id();
        if imported.is_empty() {
            return;
        }
        let keep_authorization = self
            .built_in_client()
            .is_some_and(|client| client.client_id == imported);
        if let Err(error) = self
            .config
            .update(|settings| settings.otp.google_client_id.clear())
        {
            self.system_log(
                LogLevel::Error,
                LogCategory::Oauth,
                format!("clearing the imported OAuth client failed: {error:?}"),
            );
            return;
        }
        self.config_reloaded();
        self.request(
            PortRequest::DeleteSecret {
                item: SecretItem::GoogleClientSecret {
                    client_id: imported,
                },
            },
            |_, _| {},
        );
        let address = normalized(&self.config.settings().otp.gmail_address);
        if !keep_authorization && !address.is_empty() {
            self.request(
                PortRequest::DeleteSecret {
                    item: SecretItem::GoogleRefreshToken { address },
                },
                |_, _| {},
            );
        }
        self.google.tokens.clear();
        self.refresh_google_state();
        self.feed(fvm_core::connection::Input::RecheckReadiness);
    }
}

pub(crate) fn token_error(failure: TokenFailure) -> GoogleError {
    match failure {
        TokenFailure::NoClient | TokenFailure::NotSignedIn => GoogleError::ClientRejected {},
        TokenFailure::Revoked => GoogleError::TokenRejected {
            detail: "invalid_grant".into(),
        },
        TokenFailure::ClientRejected => GoogleError::ClientRejected {},
        TokenFailure::SecretStore(failure) => GoogleError::SecretStore { failure },
        TokenFailure::Network(detail) => GoogleError::Network { detail },
        TokenFailure::Server(detail) => GoogleError::TokenRejected { detail },
    }
}

/// The whole browser sign-in. `Err(None)` when the user cancelled it in the app.
async fn sign_in(
    queue: &Queue,
    client: OAuthClient,
    address: String,
    pages: SignInPages,
    cancelled: oneshot::Receiver<()>,
) -> Result<(), Option<GoogleError>> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|error| {
        Some(GoogleError::ListenerFailed {
            detail: error.to_string(),
        })
    })?;
    let port = listener
        .local_addr()
        .map_err(|error| {
            Some(GoogleError::ListenerFailed {
                detail: error.to_string(),
            })
        })?
        .port();
    let mut random = [0u8; oauth::VERIFIER_RANDOM_BYTES];
    getrandom::fill(&mut random).expect("random bytes from the OS");
    let pkce = Pkce::from_random(&random);
    let mut state_random = [0u8; oauth::STATE_RANDOM_BYTES];
    getrandom::fill(&mut state_random).expect("random bytes from the OS");
    let state = oauth::state_from_random(&state_random);
    let redirect_uri = oauth::loopback_redirect_uri(port);
    let url = oauth::authorization_url(
        &client.client_id,
        &redirect_uri,
        pkce.challenge(),
        &state,
        Some(&address),
    );
    let opened = queue
        .ask(move |runtime, reply| {
            runtime.request(PortRequest::OpenBrowser { url }, move |_, response| {
                reply(matches!(
                    response,
                    PortResponse::BrowserOpened { opened: true }
                ));
            });
        })
        .await;
    if !opened {
        return Err(Some(GoogleError::BrowserUnavailable {}));
    }
    let code = tokio::select! {
        code = wait_for_callback(&listener, &state, &pages) => code?,
        _ = tokio::time::sleep(SIGN_IN_TIMEOUT) => {
            return Err(Some(GoogleError::TimedOut { after_seconds: SIGN_IN_TIMEOUT.as_secs() }));
        }
        _ = cancelled => return Err(None),
    };
    let body = oauth::authorization_code_body(
        &code.0,
        &client.client_id,
        &client.client_secret.0,
        &redirect_uri,
        pkce.verifier(),
    );
    let reply = net::post_form(oauth::TOKEN_ENDPOINT, &body)
        .await
        .map_err(|detail| Some(GoogleError::Network { detail }))?;
    let grant = match oauth::read_token_response(reply.status, &reply.body) {
        Ok(grant) => grant,
        Err(TokenError::InvalidClient { .. }) => return Err(Some(GoogleError::ClientRejected {})),
        Err(error) => {
            return Err(Some(GoogleError::TokenRejected {
                detail: error.detail(),
            }));
        }
    };
    let Some(refresh_token) = grant.refresh_token.clone() else {
        revoke(&grant.access_token).await;
        return Err(Some(GoogleError::NoRefreshToken {}));
    };
    if !grant.grants(oauth::GMAIL_MODIFY_SCOPE) {
        revoke(&refresh_token).await;
        return Err(Some(GoogleError::ScopeNotGranted {}));
    }
    let profile = match net::gmail(&gmail_api::profile(), &grant.access_token).await {
        Ok(reply) => Profile::from_response(reply.status, &reply.body),
        Err(detail) => {
            revoke(&refresh_token).await;
            return Err(Some(GoogleError::Network { detail }));
        }
    };
    let profile = match profile {
        Ok(profile) => profile,
        Err(error) => {
            revoke(&refresh_token).await;
            return Err(Some(api_error(error)));
        }
    };
    if !profile.belongs_to(&address) {
        revoke(&refresh_token).await;
        return Err(Some(GoogleError::WrongAccount {
            expected: address,
            actual: profile.email_address,
        }));
    }
    let client_id = client.client_id.clone();
    let access_token = grant.access_token.clone();
    let expires = SystemTime::now() + grant.expires_in;
    queue
        .ask(move |runtime, reply| {
            let item = SecretItem::GoogleRefreshToken {
                address: address.clone(),
            };
            runtime.request(
                PortRequest::WriteSecret {
                    item,
                    value: refresh_token,
                },
                move |runtime, response| match response {
                    PortResponse::SecretChanged {} => {
                        runtime
                            .google
                            .tokens
                            .insert((address, client_id), (access_token, expires));
                        reply(Ok(()));
                    }
                    PortResponse::SecretStoreFailed { failure } => {
                        reply(Err(Some(GoogleError::SecretStore { failure })))
                    }
                    other => panic!("a secret write answered with {other:?}"),
                },
            );
        })
        .await
}

pub(crate) fn api_error(error: ApiError) -> GoogleError {
    match error {
        ApiError::Forbidden { reason } => GoogleError::GmailApiRefused { reason },
        ApiError::Unauthorized => GoogleError::TokenRejected {
            detail: "401".into(),
        },
        ApiError::RateLimited => GoogleError::GmailApiRefused {
            reason: "rateLimitExceeded".into(),
        },
        ApiError::NotFound => GoogleError::TokenRejected {
            detail: "404".into(),
        },
        ApiError::Server { status } => GoogleError::TokenRejected {
            detail: format!("HTTP {status}"),
        },
    }
}

async fn revoke(token: &SecretText) {
    let _ = net::post_form(
        oauth::REVOCATION_ENDPOINT,
        &oauth::revocation_body(&token.0),
    )
    .await;
}

/// Answers browser requests on the loopback listener until one carries the authorization code.
/// Each connection is answered on its own, as browsers open connections that never send anything.
async fn wait_for_callback(
    listener: &TcpListener,
    state: &str,
    pages: &SignInPages,
) -> Result<SecretText, Option<GoogleError>> {
    let (decided, mut decisions) = mpsc::unbounded_channel();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let (decided, state, pages) = (decided.clone(), state.to_owned(), pages.clone());
                tokio::spawn(async move {
                    if let Some(decision) = answer_callback(stream, &state, &pages).await {
                        let _ = decided.send(decision);
                    }
                });
            }
            Some(decision) = decisions.recv() => match decision {
                CallbackDecision::Authorized { code } => return Ok(code),
                CallbackDecision::Denied { error } if error == "access_denied" => {
                    return Err(Some(GoogleError::CancelledInBrowser {}));
                }
                CallbackDecision::Denied { error } => return Err(Some(GoogleError::Denied { code: error })),
                CallbackDecision::Rejected | CallbackDecision::NotFound => {}
            },
        }
    }
}

/// Reads one request head and answers it; `None` when the browser sent nothing usable in time.
async fn answer_callback(
    mut stream: TcpStream,
    state: &str,
    pages: &SignInPages,
) -> Option<CallbackDecision> {
    let decision = tokio::time::timeout(LOOPBACK_CONNECTION_TIMEOUT, async {
        let mut head = Vec::new();
        let mut buffer = [0u8; 2048];
        loop {
            let count = stream.read(&mut buffer).await.ok()?;
            if count == 0 {
                return None;
            }
            head.extend_from_slice(&buffer[..count]);
            match oauth::parse_request_head(&head) {
                HeadParse::Incomplete => {}
                HeadParse::Request(request) => {
                    return Some(CallbackDecision::decide(&request, state));
                }
                HeadParse::Malformed => return Some(CallbackDecision::Rejected),
            }
        }
    })
    .await
    .ok()??;
    let _ = stream.write_all(&decision.response(pages)).await;
    let _ = stream.shutdown().await;
    Some(decision)
}
