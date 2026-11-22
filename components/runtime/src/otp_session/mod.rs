//! The OTP session of an attempt: signed in to Gmail before the client starts,
//! it looks only at mail that arrived after it opened. The Gmail API is used when it is the
//! method and the Google account is connected, IMAP otherwise. When the API is refused before the
//! session is ready and an app password exists, the same attempt goes on over IMAP. After the code
//! was submitted, the used mail go to the trash; a session closed after the OTP time limit
//! looks in the spam folder once. When the attempt ended after its credentials went out with no
//! code submitted, the session can go on looking for its code and trash its mail when it comes.
//! Also the Account pane's Gmail test.

mod api_session;
mod gmail_test;
mod imap_session;

use std::time::SystemTime;

use fvm_contract::app_helper::AttemptId;
use fvm_contract::app_internal::port::{LogCategory, LogLevel};
use fvm_contract::app_internal::problem::ConfigFileError;
use fvm_contract::app_internal::section::GmailTest;
use fvm_contract::app_internal::{OtpMethod, SecretText};
use fvm_core::connection::{self, OtpFailure};
use tokio::sync::mpsc;

use crate::google_account::normalized;
use crate::orchestration::Runtime;
use crate::os::{OsRequest, OsResponse, SecretItem};
use api_session::api_session;
use imap_session::imap_session;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backend {
    Imap,
    GmailApi,
}

/// What the attempt's runtime tells its session.
#[derive(Debug)]
enum Control {
    Search(SystemTime),
    /// The code was submitted: trash the used mail and end.
    Finish,
    /// The attempt ended with no code submitted: search as after a prompt at `since` unless one
    /// came, and once the code is found, finish without reporting it.
    TrashStale {
        since: SystemTime,
    },
    Close {
        check_spam: bool,
    },
}

/// What a session reports.
enum Report {
    Ready,
    Failed { failure: OtpFailure, reason: String },
    CodeFound { code: SecretText, note: String },
    FoundInSpam,
    StaleCodeTrashed,
    StaleCodeNotTrashed { reason: String },
    SpamCheckFailed { reason: String },
}

struct Session {
    attempt: AttemptId,
    backend: Backend,
    control: mpsc::UnboundedSender<Control>,
    address: String,
    app_password: Option<SecretText>,
    ready: bool,
}

#[derive(Default)]
pub(crate) struct Gmail {
    pub(crate) method_error: Option<ConfigFileError>,
    pub(crate) test: Option<GmailTest>,
    session: Option<Session>,
    /// Grows with each test, so a result for an older one is dropped.
    test_generation: u64,
}

impl Runtime {
    pub(crate) fn open_otp_session(&mut self, attempt: AttemptId) {
        if let Some(previous) = self.gmail.session.take() {
            let _ = previous.control.send(Control::Close { check_spam: false });
        }
        let saved = self.config.settings().otp.gmail_address.clone();
        let address = normalized(&saved);
        let method = self.config.settings().otp.method;
        // The app password is stored under the address as saved, not normalized.
        let item = (!saved.is_empty()).then_some(SecretItem::GmailAppPassword { address: saved });
        let read = move |runtime: &mut Runtime, app_password: Option<SecretText>| {
            if method != OtpMethod::GmailApi {
                return runtime.start_session(attempt, address, app_password, false);
            }
            let check = address.clone();
            runtime.gmail_api_authorized(&check, move |runtime, authorized| {
                runtime.start_session(attempt, address, app_password, authorized);
            });
        };
        match item {
            // On an input of its own, like the Keychain's answer: a failure feeds the core, which
            // is in the middle of this effect's input.
            None => self.queue().run(move |runtime| read(runtime, None)),
            Some(item) => self.ask_os(OsRequest::ReadSecret { item }, move |runtime, response| {
                let password = match response {
                    OsResponse::Secret { value } => value.filter(|password| !password.0.is_empty()),
                    _ => None,
                };
                read(runtime, password);
            }),
        }
    }

    fn start_session(
        &mut self,
        attempt: AttemptId,
        address: String,
        app_password: Option<SecretText>,
        api_available: bool,
    ) {
        if !self.cm.wants_otp_session(&attempt) {
            return;
        }
        let method = self.config.settings().otp.method;
        let backend = match (api_available, method, app_password.is_some()) {
            (true, _, _) => Backend::GmailApi,
            (false, OtpMethod::GmailApi, true) => {
                self.attempt_note(
                    &attempt,
                    "‹Gmail API selected but not signed in with Google; using IMAP›",
                );
                Backend::Imap
            }
            (false, OtpMethod::Imap, true) => Backend::Imap,
            (false, OtpMethod::GmailApi, false) => {
                self.attempt_note(&attempt, "‹Gmail API selected but not signed in with Google, and no IMAP app password is stored›");
                return self.feed(connection::Input::OtpSessionFailed(
                    attempt,
                    OtpFailure::Authentication,
                ));
            }
            (false, OtpMethod::Imap, false) => {
                return self.feed(connection::Input::OtpSessionFailed(
                    attempt,
                    OtpFailure::Authentication,
                ));
            }
        };
        let (control, controls) = mpsc::unbounded_channel();
        self.gmail.session = Some(Session {
            attempt: attempt.clone(),
            backend,
            control,
            address: address.clone(),
            app_password: app_password.clone(),
            ready: false,
        });
        self.spawn_session(attempt, backend, address, app_password, controls);
    }

    fn spawn_session(
        &mut self,
        attempt: AttemptId,
        backend: Backend,
        address: String,
        app_password: Option<SecretText>,
        controls: mpsc::UnboundedReceiver<Control>,
    ) {
        let queue = self.queue();
        let report = {
            let queue = queue.clone();
            move |report: Report| {
                let attempt = attempt.clone();
                queue.run(move |runtime| runtime.session_reported(attempt, backend, report));
            }
        };
        match backend {
            Backend::GmailApi => {
                self.tasks
                    .spawn(api_session(queue, address, controls, report));
            }
            Backend::Imap => {
                let password = app_password.expect("IMAP needs the app password");
                self.tasks
                    .spawn(imap_session(address, password, controls, report));
            }
        }
    }

    fn session_reported(&mut self, attempt: AttemptId, backend: Backend, report: Report) {
        let Some(session) = self
            .gmail
            .session
            .as_mut()
            .filter(|session| session.attempt == attempt && session.backend == backend)
        else {
            return;
        };
        match report {
            Report::Ready => {
                session.ready = true;
                self.feed(connection::Input::OtpSessionReady(attempt));
            }
            Report::Failed { failure, reason } if backend == Backend::GmailApi => {
                if failure == OtpFailure::Authentication
                    && !session.ready
                    && session.app_password.is_some()
                {
                    self.system_log(
                        LogLevel::Notice,
                        LogCategory::Otp,
                        format!("Gmail API unavailable ({reason}); falling back to IMAP"),
                    );
                    self.attempt_note(
                        &attempt,
                        format!("‹Gmail API unavailable ({reason}); falling back to IMAP›"),
                    );
                    let session = self.gmail.session.as_mut().expect("checked above");
                    let _ = session.control.send(Control::Close { check_spam: false });
                    let (control, controls) = mpsc::unbounded_channel();
                    session.control = control;
                    session.backend = Backend::Imap;
                    let (address, password) =
                        (session.address.clone(), session.app_password.clone());
                    return self.spawn_session(attempt, Backend::Imap, address, password, controls);
                }
                self.attempt_note(&attempt, format!("‹Gmail API session failed ({reason})›"));
                self.feed(connection::Input::OtpSessionFailed(attempt, failure));
            }
            Report::Failed { failure, reason } => {
                self.system_log(
                    LogLevel::Error,
                    LogCategory::Otp,
                    format!("Gmail session failed: {reason}"),
                );
                self.feed(connection::Input::OtpSessionFailed(attempt, failure));
            }
            Report::CodeFound { code, note } => {
                self.feed(connection::Input::OtpCodeFound(attempt.clone(), code));
                self.attempt_note(&attempt, note);
            }
            Report::FoundInSpam => self.feed(connection::Input::OtpFoundInSpam(attempt)),
            Report::StaleCodeTrashed => self.feed(connection::Input::StaleOtpTrashed(attempt)),
            Report::StaleCodeNotTrashed { reason } => {
                self.attempt_note(
                    &attempt,
                    format!("‹the stopped attempt's OTP email was not trashed: {reason}›"),
                );
            }
            Report::SpamCheckFailed { reason } => self.system_log(
                LogLevel::Notice,
                LogCategory::Otp,
                format!("Gmail spam folder check failed: {reason}"),
            ),
        }
    }

    fn session_for(&self, attempt: &AttemptId) -> Option<&Session> {
        self.gmail
            .session
            .as_ref()
            .filter(|session| session.attempt == *attempt)
    }

    pub(crate) fn search_otp(&mut self, attempt: &AttemptId, prompted_at: SystemTime) {
        if let Some(session) = self.session_for(attempt) {
            let _ = session.control.send(Control::Search(prompted_at));
        }
    }

    /// After the code was handed to the Helper.
    pub(crate) fn finish_otp_session(&mut self, attempt: &AttemptId) {
        if let Some(session) = self.session_for(attempt) {
            let _ = session.control.send(Control::Finish);
        }
    }

    pub(crate) fn close_otp_session(&mut self, attempt: &AttemptId, check_spam: bool) {
        if let Some(session) = self.session_for(attempt) {
            let _ = session.control.send(Control::Close { check_spam });
        }
    }

    /// After the attempt ended with its credentials sent and no code submitted.
    pub(crate) fn trash_stale_otp(&mut self, attempt: &AttemptId, since: SystemTime) {
        if let Some(session) = self.session_for(attempt) {
            let _ = session.control.send(Control::TrashStale { since });
        }
    }
}
