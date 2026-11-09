//! The runtime's connections to Google: HTTPS for OAuth and the Gmail API, and IMAP over TLS. TLS is
//! rustls with the ring provider and the OS's trust store; HTTPS follows the OS's proxy settings.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use fvm_contract::app_internal::SecretText;
use fvm_contract::app_internal::section::{ImapFailure, ImapStep};
use fvm_core::gmail_api::{self, Method};
use fvm_core::imap::{Command, IDLE_DONE, Parser, Response, Status};
use rustls::ClientConfig;
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout_at};
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

/// Replies normally take milliseconds; this only guards against a hung server.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("ring supports the default protocol versions")
                .with_platform_verifier()
                .expect("the OS trust store is usable")
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .tls_backend_preconfigured((*tls_config()).clone())
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("building the HTTP client")
    })
}

/// A reply's status and body.
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

/// Why a request got no reply, as the HTTP library describes it.
pub(crate) type NetworkError = String;

fn describe(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

async fn finish(request: reqwest::RequestBuilder) -> Result<Reply, NetworkError> {
    let response = request.send().await.map_err(|error| describe(&error))?;
    let status = response.status().as_u16();
    let body = response
        .bytes()
        .await
        .map_err(|error| describe(&error))?
        .to_vec();
    Ok(Reply { status, body })
}

/// A form POST, for the token and revocation endpoints.
pub(crate) async fn post_form(url: &str, body: &SecretText) -> Result<Reply, NetworkError> {
    finish(
        http()
            .post(url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body.0.clone()),
    )
    .await
}

/// A Gmail API request with the access token.
pub(crate) async fn gmail(
    request: &gmail_api::Request,
    access_token: &SecretText,
) -> Result<Reply, NetworkError> {
    let builder = match request.method {
        Method::Get => http().get(&request.url),
        Method::Post => http().post(&request.url).header("Content-Length", "0"),
    };
    finish(builder.bearer_auth(&access_token.0)).await
}

// MARK: IMAP

const IMAP_HOST: &str = "imap.gmail.com";
const IMAP_PORT: u16 = 993;
/// TCP, TLS, the greeting and the login.
pub(crate) const IMAP_LOGIN_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const IMAP_COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

/// One IMAP connection with one command at a time. A failure ends the connection.
pub(crate) struct Imap {
    stream: TlsStream<TcpStream>,
    parser: Parser,
    tags: fvm_core::imap::TagGenerator,
    /// Responses read past the end of the last command.
    backlog: Vec<Response>,
}

impl Imap {
    /// Connects and reads the greeting, all by `deadline`.
    pub(crate) async fn connect(deadline: Instant) -> Result<Self, ImapFailure> {
        let timed_out = || ImapFailure::TimedOut {
            step: ImapStep::Connecting {},
        };
        let tcp = timeout_at(deadline, TcpStream::connect((IMAP_HOST, IMAP_PORT)))
            .await
            .map_err(|_| timed_out())?
            .map_err(|error| ImapFailure::Network {
                detail: error.to_string(),
            })?;
        let name = rustls::pki_types::ServerName::try_from(IMAP_HOST).expect("a valid host name");
        let stream = timeout_at(
            deadline,
            TlsConnector::from(tls_config()).connect(name, tcp),
        )
        .await
        .map_err(|_| timed_out())?
        .map_err(|error| ImapFailure::Network {
            detail: error.to_string(),
        })?;
        let mut imap = Self {
            stream,
            parser: Parser::default(),
            tags: Default::default(),
            backlog: Vec::new(),
        };
        loop {
            match imap
                .next_response(deadline, ImapStep::Connecting {})
                .await?
            {
                Response::UntaggedStatus {
                    status: Status::Ok, ..
                } => return Ok(imap),
                Response::Bye { text, .. } => {
                    return Err(ImapFailure::ServerClosed { text: Some(text) });
                }
                _ => continue,
            }
        }
    }

    /// Sends `command` and returns every response up to its tagged completion. A tagged NO or BAD
    /// is `Rejected`, with the server's code and text for the caller to judge.
    pub(crate) async fn execute(
        &mut self,
        command: &Command,
        deadline: Instant,
    ) -> Result<Vec<Response>, Rejection> {
        let tag = self.tags.next_tag();
        let line = command.encode(&tag).map_err(|error| {
            Rejection::Failed(ImapFailure::Protocol {
                detail: format!("{error:?}"),
            })
        })?;
        let step = ImapStep::Command {
            name: command.name().to_owned(),
        };
        self.write(&line, deadline, &step)
            .await
            .map_err(Rejection::Failed)?;
        let mut responses = Vec::new();
        loop {
            match self
                .next_response(deadline, step.clone())
                .await
                .map_err(Rejection::Failed)?
            {
                Response::Tagged {
                    tag: done,
                    status,
                    code,
                    text,
                } if done == tag => {
                    return match status {
                        Status::Ok => Ok(responses),
                        status => Err(Rejection::Refused {
                            command: command.name().to_owned(),
                            status,
                            code,
                            text,
                        }),
                    };
                }
                Response::Bye { text, .. } if *command != Command::Logout => {
                    return Err(Rejection::Failed(ImapFailure::ServerClosed {
                        text: Some(text),
                    }));
                }
                response => responses.push(response),
            }
        }
    }

    /// IDLE for up to `duration`, until new mail is reported, or until `interrupt` resolves;
    /// returns the updates the server sent meanwhile and the interruption's value.
    pub(crate) async fn idle<T>(
        &mut self,
        duration: Duration,
        interrupt: impl std::future::Future<Output = T>,
    ) -> Result<(Vec<Response>, Option<T>), ImapFailure> {
        let tag = self.tags.next_tag();
        let line = Command::Idle.encode(&tag).expect("IDLE encodes");
        let step = ImapStep::Command {
            name: "IDLE".into(),
        };
        let entered_by = Instant::now() + IMAP_COMMAND_TIMEOUT;
        self.write(&line, entered_by, &step).await?;
        let mut updates = Vec::new();
        loop {
            match self.next_response(entered_by, step.clone()).await? {
                Response::Continuation(_) => break,
                Response::Tagged { status, text, .. } if status != Status::Ok => {
                    return Err(ImapFailure::Rejected {
                        command: "IDLE".into(),
                        status: status.as_str().into(),
                        text,
                    });
                }
                response => updates.push(response),
            }
        }
        let until = Instant::now() + duration;
        let mut interrupted = None;
        tokio::pin!(interrupt);
        loop {
            tokio::select! {
                response = self.next_response(until + IMAP_COMMAND_TIMEOUT, step.clone()) => {
                    let response = response?;
                    if let Response::Bye { text, .. } = response {
                        return Err(ImapFailure::ServerClosed { text: Some(text) });
                    }
                    let exists = matches!(response, Response::Exists(_));
                    updates.push(response);
                    if exists {
                        break;
                    }
                }
                () = tokio::time::sleep_until(until) => break,
                value = &mut interrupt => {
                    interrupted = Some(value);
                    break;
                }
            }
        }
        let done_by = Instant::now() + IMAP_COMMAND_TIMEOUT;
        self.write(IDLE_DONE, done_by, &step).await?;
        loop {
            match self.next_response(done_by, step.clone()).await? {
                Response::Tagged { tag: done, .. } if done == tag => {
                    return Ok((updates, interrupted));
                }
                response => updates.push(response),
            }
        }
    }

    pub(crate) async fn logout(mut self) {
        let _ = self
            .execute(&Command::Logout, Instant::now() + IMAP_COMMAND_TIMEOUT)
            .await;
    }

    async fn write(
        &mut self,
        bytes: &[u8],
        deadline: Instant,
        step: &ImapStep,
    ) -> Result<(), ImapFailure> {
        timeout_at(deadline, self.stream.write_all(bytes))
            .await
            .map_err(|_| ImapFailure::TimedOut { step: step.clone() })?
            .map_err(|error| ImapFailure::Network {
                detail: error.to_string(),
            })
    }

    async fn next_response(
        &mut self,
        deadline: Instant,
        step: ImapStep,
    ) -> Result<Response, ImapFailure> {
        loop {
            if !self.backlog.is_empty() {
                return Ok(self.backlog.remove(0));
            }
            let mut buffer = [0u8; 16 * 1024];
            let count = timeout_at(deadline, self.stream.read(&mut buffer))
                .await
                .map_err(|_| ImapFailure::TimedOut { step: step.clone() })?
                .map_err(|error| ImapFailure::Network {
                    detail: error.to_string(),
                })?;
            if count == 0 {
                return Err(ImapFailure::ServerClosed { text: None });
            }
            let responses =
                self.parser
                    .feed(&buffer[..count])
                    .map_err(|error| ImapFailure::Protocol {
                        detail: format!("{error:?}"),
                    })?;
            self.backlog.extend(responses);
        }
    }
}

/// Why a command did not complete.
#[derive(Debug)]
pub(crate) enum Rejection {
    /// The server answered NO or BAD; the connection stays usable.
    Refused {
        command: String,
        status: Status,
        code: Option<fvm_core::imap::ResponseCode>,
        text: String,
    },
    Failed(ImapFailure),
}

impl Rejection {
    pub(crate) fn into_failure(self) -> ImapFailure {
        match self {
            Rejection::Refused {
                command,
                status,
                text,
                ..
            } => ImapFailure::Rejected {
                command,
                status: status.as_str().into(),
                text,
            },
            Rejection::Failed(failure) => failure,
        }
    }
}
