use fvm_contract::app_internal::port::{PortRequest, PortResponse, SecretItem};
use fvm_contract::app_internal::section::{
    GmailSection, GmailTest, GmailTestFailure, GoogleAccountStatus, ImapFailure,
};
use fvm_contract::app_internal::{ButtonState, OtpMethod, SecretText};
use fvm_core::gmail_api::{self, ApiError, Profile};
use fvm_core::imap;

use super::api_session::{ApiFailure, api_call};
use super::imap_session::{ImapError, imap_log_in};
use crate::google_account::{TokenFailure, normalized};
use crate::orchestration::{Queue, Runtime};

impl Runtime {
    pub(crate) fn reset_gmail_test(&mut self) {
        if !matches!(self.gmail.test, Some(GmailTest::Running {})) {
            self.gmail.test = None;
        }
    }

    /// "Test Gmail Connection" for the method in use, with the typed address and the typed (or
    /// else the stored) app password.
    pub(crate) fn test_gmail(&mut self) {
        if matches!(self.gmail.test, Some(GmailTest::Running {})) {
            return;
        }
        self.gmail.test_generation += 1;
        let generation = self.gmail.test_generation;
        let method = self.config.settings().otp.method;
        let typed_address = self.forms.typed_gmail_address();
        let address = normalized(&typed_address);
        let finish = move |runtime: &mut Runtime, test: GmailTest| {
            // A result for a method no longer selected would mislead.
            if runtime.gmail.test_generation == generation {
                runtime.gmail.test =
                    (runtime.config.settings().otp.method == method).then_some(test);
            }
        };
        if address.is_empty() {
            return finish(
                self,
                GmailTest::Failed {
                    failure: GmailTestFailure::EnterAddress {},
                },
            );
        }
        match method {
            OtpMethod::Imap => {
                let typed = self.forms.typed_app_password();
                let item = SecretItem::GmailAppPassword {
                    address: typed_address,
                };
                let test = move |runtime: &mut Runtime, password: String| {
                    runtime.gmail.test = Some(GmailTest::Running {});
                    let queue = runtime.queue();
                    runtime.tasks.spawn(async move {
                        let test = imap_test(&address, &SecretText(password)).await;
                        queue.run(move |runtime| finish(runtime, test));
                    });
                };
                match typed {
                    Some(password) => test(self, password),
                    None => {
                        self.request(
                            PortRequest::ReadSecret { item },
                            move |runtime, response| match response {
                                PortResponse::Secret {
                                    value: Some(password),
                                } if !password.0.is_empty() => test(runtime, password.0),
                                PortResponse::Secret { .. } => {
                                    finish(
                                        runtime,
                                        GmailTest::Failed {
                                            failure: GmailTestFailure::EnterAppPassword {},
                                        },
                                    );
                                }
                                PortResponse::SecretStoreFailed { failure } => {
                                    finish(
                                        runtime,
                                        GmailTest::Failed {
                                            failure: GmailTestFailure::AppPasswordUnreadable {
                                                failure,
                                            },
                                        },
                                    );
                                }
                                other => panic!("a secret read answered with {other:?}"),
                            },
                        );
                    }
                }
            }
            OtpMethod::GmailApi => {
                self.gmail.test = Some(GmailTest::Running {});
                let queue = self.queue();
                self.tasks.spawn(async move {
                    let test = api_test(&queue, &address).await;
                    queue.run(move |runtime| finish(runtime, test));
                });
            }
        }
    }

    pub(crate) fn gmail_section(&self) -> GmailSection {
        let settings = self.config.settings();
        let saved_address = settings.otp.gmail_address.clone();
        let google = &self.google;
        let blocker = self.google_connect_blocker();
        let google_account = if google.signing_in() {
            GoogleAccountStatus::SigningIn {}
        } else if google.disconnecting {
            GoogleAccountStatus::Disconnecting {}
        } else if google.authorized {
            GoogleAccountStatus::Connected {
                address: saved_address,
            }
        } else {
            GoogleAccountStatus::NotConnected {}
        };
        let connected = matches!(
            google_account,
            GoogleAccountStatus::Connected { .. } | GoogleAccountStatus::Disconnecting {}
        );
        let imported = !settings.otp.google_client_id.is_empty();
        let running = matches!(self.gmail.test, Some(GmailTest::Running {}));
        GmailSection {
            method: settings.otp.method,
            method_error: self.gmail.method_error.clone(),
            connect_google: if google.signing_in() || connected {
                ButtonState::Hidden
            } else if blocker.is_some() {
                ButtonState::Disabled
            } else {
                ButtonState::Enabled
            },
            disconnect_google: match google_account {
                GoogleAccountStatus::Connected { .. } => ButtonState::Enabled,
                GoogleAccountStatus::Disconnecting {} => ButtonState::Disabled,
                _ => ButtonState::Hidden,
            },
            cancel_sign_in: if google.signing_in() {
                ButtonState::Enabled
            } else {
                ButtonState::Hidden
            },
            google_account,
            connect_blocker: blocker,
            google_error: google.error.clone(),
            oauth_client: self.oauth_client_source(),
            imported_client_secret_missing: imported && !google.imported_secret_stored,
            use_built_in_client: if imported {
                ButtonState::Enabled
            } else {
                ButtonState::Hidden
            },
            import_client: ButtonState::Enabled,
            test: self.gmail.test.clone().unwrap_or(GmailTest::Idle {}),
            test_button: if running {
                ButtonState::Disabled
            } else {
                ButtonState::Enabled
            },
        }
    }
}

async fn imap_test(address: &str, password: &SecretText) -> GmailTest {
    let failed = |failure| GmailTest::Failed { failure };
    match imap_log_in(address, password).await {
        Ok((connection, mailboxes)) => {
            connection.logout().await;
            match (&mailboxes.all, &mailboxes.trash) {
                (Some(all), Some(trash)) => GmailTest::ImapSucceeded {
                    archive: all.display_name(),
                    trash: trash.display_name(),
                    spam: mailboxes.junk.as_ref().map(imap::ListEntry::display_name),
                },
                (all, trash) => failed(GmailTestFailure::MailboxesMissing {
                    all_mail: all.is_none(),
                    trash: trash.is_none(),
                }),
            }
        }
        Err(ImapError::LoginRejected(text)) => failed(GmailTestFailure::ImapLoginRejected {
            server_text: (!text.is_empty()).then_some(text),
        }),
        Err(ImapError::Failure(failure)) => failed(GmailTestFailure::ImapUnreachable { failure }),
        Err(other) => failed(GmailTestFailure::ImapUnreachable {
            failure: ImapFailure::Protocol {
                detail: other.describe(),
            },
        }),
    }
}

async fn api_test(queue: &Queue, address: &str) -> GmailTest {
    let failed = |failure| GmailTest::Failed { failure };
    match api_call(
        queue,
        address,
        &gmail_api::profile(),
        Profile::from_response,
    )
    .await
    {
        Ok(profile) if profile.belongs_to(address) => GmailTest::ApiSucceeded {
            address: profile.email_address,
        },
        Ok(profile) => failed(GmailTestFailure::OtherAccount {
            authorized: profile.email_address,
            expected: address.to_owned(),
        }),
        Err(ApiFailure::Token(failure)) => failed(match failure {
            TokenFailure::NoClient => GmailTestFailure::NoOAuthClient {},
            TokenFailure::NotSignedIn => GmailTestFailure::NotSignedIn {},
            TokenFailure::Revoked => GmailTestFailure::AccessRevoked {},
            TokenFailure::ClientRejected => GmailTestFailure::ClientRejected {},
            TokenFailure::SecretStore(failure) => GmailTestFailure::SecretStore { failure },
            TokenFailure::Network(detail) => GmailTestFailure::GoogleUnreachable { detail },
            TokenFailure::Server(detail) => GmailTestFailure::GoogleReturnedError { detail },
        }),
        Err(ApiFailure::Api(error)) => failed(match error {
            ApiError::Unauthorized => GmailTestFailure::AccessTokenRejected {},
            ApiError::Forbidden { reason } => GmailTestFailure::GmailApiRefused { reason },
            ApiError::RateLimited => GmailTestFailure::RateLimited {},
            ApiError::NotFound => GmailTestFailure::GoogleReturnedError {
                detail: "HTTP 404".into(),
            },
            ApiError::Server { status } => GmailTestFailure::GoogleReturnedError {
                detail: format!("HTTP {status}"),
            },
        }),
        Err(ApiFailure::Network(detail)) => failed(GmailTestFailure::GoogleUnreachable { detail }),
        Err(ApiFailure::OtherAccount) => unreachable!("only the session checks the account"),
    }
}
