//! The OTP session of an attempt (SPEC F-OT-01..04): signed in to Gmail before the client starts,
//! it looks only at mail that arrived after it opened. The Gmail API is used when it is the
//! method and the Google account is connected, IMAP otherwise. When the API is refused before the
//! session is ready and an app password exists, the same attempt goes on over IMAP. After the code
//! was submitted, the used mail go to the trash; a session closed after the OTP time limit
//! looks in the spam folder once. Also the Account tab's Gmail test (SPEC F-OT-05).

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use fvm_contract::app_helper::AttemptId;
use fvm_contract::app_internal::port::{
    LogCategory, LogLevel, PortRequest, PortResponse, SecretItem,
};
use fvm_contract::app_internal::problem::ConfigFileError;
use fvm_contract::app_internal::section::{
    GmailSection, GmailTest, GmailTestFailure, GoogleAccountStatus, ImapFailure,
};
use fvm_contract::app_internal::{ButtonState, OtpMethod, SecretText};
use fvm_core::connection::{self, OtpFailure};
use fvm_core::gmail_api::{self, ApiError, HistoryPage, MessageMetadata, MessageRef, Profile};
use fvm_core::imap::{
    self, Command, MailboxSnapshot, Response, SpecialMailboxes, UidSet, is_credential_rejection,
};
use fvm_core::otp_mail;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::google_account::{TokenFailure, normalized};
use crate::net::{self, IMAP_COMMAND_TIMEOUT, IMAP_LOGIN_TIMEOUT, Imap, Rejection};
use crate::orchestration::{Queue, Runtime};

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_POLL_FAILURES: u32 = 3;
const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(2);
const MAX_HISTORY_PAGES: u32 = 10;
/// The IMAP search period while looking for the code, in case an IDLE notification is missed.
const IMAP_SEARCH_INTERVAL: Duration = Duration::from_secs(1);
/// The IDLE refresh period while not searching; servers end IDLE after about 30 minutes.
const IMAP_IDLE_REFRESH: Duration = Duration::from_secs(300);

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
        // The app password is stored under the address as saved; earlier builds did the same.
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
            None => read(self, None),
            Some(item) => self.request(
                PortRequest::ReadSecret { item },
                move |runtime, response| {
                    let password = match response {
                        PortResponse::Secret { value } => {
                            value.filter(|password| !password.0.is_empty())
                        }
                        _ => None,
                    };
                    read(runtime, password);
                },
            ),
        }
    }

    fn start_session(
        &mut self,
        attempt: AttemptId,
        address: String,
        app_password: Option<SecretText>,
        api_available: bool,
    ) {
        let method = self.config.settings().otp.method;
        let backend = match (api_available, method, app_password.is_some()) {
            (true, _, _) => Backend::GmailApi,
            (false, OtpMethod::GmailApi, true) => {
                self.note("‹Gmail API selected but not signed in with Google; using IMAP›");
                Backend::Imap
            }
            (false, OtpMethod::Imap, true) => Backend::Imap,
            (false, OtpMethod::GmailApi, false) => {
                self.note("‹Gmail API selected but not signed in with Google, and no IMAP app password is stored›");
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
                    self.note(format!(
                        "‹Gmail API unavailable ({reason}); falling back to IMAP›"
                    ));
                    let session = self.gmail.session.as_mut().expect("checked above");
                    let _ = session.control.send(Control::Close { check_spam: false });
                    let (control, controls) = mpsc::unbounded_channel();
                    session.control = control;
                    session.backend = Backend::Imap;
                    let (address, password) =
                        (session.address.clone(), session.app_password.clone());
                    return self.spawn_session(attempt, Backend::Imap, address, password, controls);
                }
                self.note(format!("‹Gmail API session failed ({reason})›"));
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
                self.feed(connection::Input::OtpCodeFound(attempt, code));
                self.note(note);
            }
            Report::FoundInSpam => self.feed(connection::Input::OtpFoundInSpam(attempt)),
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

    // MARK: The Gmail test

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

// MARK: The Gmail API session

#[derive(Debug)]
enum ApiFailure {
    Token(TokenFailure),
    Api(ApiError),
    Network(String),
    OtherAccount,
}

impl ApiFailure {
    fn failure(&self) -> OtpFailure {
        match self {
            Self::Token(failure) if failure.is_authentication() => OtpFailure::Authentication,
            Self::Api(ApiError::Unauthorized | ApiError::Forbidden { .. }) | Self::OtherAccount => {
                OtpFailure::Authentication
            }
            _ => OtpFailure::Network,
        }
    }

    fn is_transient(&self) -> bool {
        match self {
            Self::Token(failure) => failure.is_transient(),
            Self::Network(_) | Self::Api(ApiError::Server { .. } | ApiError::RateLimited) => true,
            _ => false,
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Token(failure) => failure.describe(),
            Self::Api(ApiError::Unauthorized) => "Gmail API rejected the access token".into(),
            Self::Api(ApiError::Forbidden { reason }) => {
                format!("Gmail API access forbidden ({reason})")
            }
            Self::Api(ApiError::NotFound) => "Gmail API: not found".into(),
            Self::Api(ApiError::RateLimited) => "Gmail API rate limit".into(),
            Self::Api(ApiError::Server { status }) => format!("Gmail API error HTTP {status}"),
            Self::Network(detail) => format!("network error: {detail}"),
            Self::OtherAccount => "the Google authorization belongs to another account".into(),
        }
    }
}

/// Runs a Gmail API request with an access token; after a 401 the cached token is dropped and the
/// request is tried once more.
async fn api_call<T>(
    queue: &Queue,
    address: &str,
    request: &gmail_api::Request,
    read: impl Fn(u16, &[u8]) -> Result<T, ApiError>,
) -> Result<T, ApiFailure> {
    let mut retried = false;
    loop {
        let address_for_token = address.to_owned();
        let token = queue
            .ask(move |runtime, reply| {
                runtime.access_token(&address_for_token, move |_, token| reply(token))
            })
            .await
            .map_err(ApiFailure::Token)?;
        let reply = net::gmail(request, &token)
            .await
            .map_err(ApiFailure::Network)?;
        match read(reply.status, &reply.body) {
            Err(ApiError::Unauthorized) if !retried => {
                retried = true;
                let address = address.to_owned();
                queue.run(move |runtime| runtime.invalidate_access_token(&address));
            }
            result => return result.map_err(ApiFailure::Api),
        }
    }
}

async fn api_session(
    queue: Queue,
    address: String,
    mut controls: mpsc::UnboundedReceiver<Control>,
    report: impl Fn(Report) + Send + 'static,
) {
    let profile = match api_call(
        &queue,
        &address,
        &gmail_api::profile(),
        Profile::from_response,
    )
    .await
    {
        Ok(profile) if profile.belongs_to(&address) => profile,
        Ok(_) => return report(failed(&ApiFailure::OtherAccount)),
        Err(failure) => return report(failed(&failure)),
    };
    let started_at = SystemTime::now();
    let mut cursor = profile.usable_history_id().map(str::to_owned);
    report(Report::Ready);
    let mut prompted_at = None;
    let mut selection: Option<otp_mail::GmailSelection> = None;
    let mut seen: HashSet<String> = HashSet::new();
    let mut pending: Vec<MessageRef> = Vec::new();
    let mut messages: Vec<MessageMetadata> = Vec::new();
    let mut polls = 0u32;
    let mut failures = 0u32;
    let ending = loop {
        // Before the prompt, and after a match, nothing is to be done until the next control.
        if prompted_at.is_none() || selection.is_some() {
            match controls.recv().await {
                Some(Control::Search(at)) => {
                    prompted_at.get_or_insert(at);
                    continue;
                }
                Some(ending) => break Some(ending),
                None => break None,
            }
        }
        match controls.try_recv() {
            Ok(Control::Search(_)) | Err(mpsc::error::TryRecvError::Empty) => {}
            Ok(ending) => break Some(ending),
            Err(mpsc::error::TryRecvError::Disconnected) => break None,
        }
        let poll_started = Instant::now();
        polls += 1;
        let prompted = prompted_at.expect("searching");
        let result = api_poll(
            &queue,
            &address,
            &mut cursor,
            &mut seen,
            &mut pending,
            &mut messages,
        )
        .await;
        match result {
            Ok(()) => failures = 0,
            Err(failure) if failure.is_transient() => {
                failures += 1;
                let detail = format!(
                    "Gmail API poll failed ({failures}/{MAX_POLL_FAILURES}): {}",
                    failure.describe()
                );
                queue.run(move |runtime| {
                    runtime.system_log(LogLevel::Notice, LogCategory::Otp, detail)
                });
                if failures >= MAX_POLL_FAILURES {
                    return report(failed(&failure));
                }
                if matches!(failure, ApiFailure::Api(ApiError::RateLimited)) {
                    tokio::time::sleep(RATE_LIMIT_BACKOFF).await;
                    continue;
                }
            }
            Err(failure) => return report(failed(&failure)),
        }
        let not_before = cursor
            .is_none()
            .then(|| started_at - otp_mail::ARRIVAL_TOLERANCE);
        if let Some(found) = otp_mail::select_gmail(prompted, &messages, not_before) {
            let note = api_timing_note(prompted, found.arrived_at, polls);
            report(Report::CodeFound {
                code: found.code.clone(),
                note,
            });
            selection = Some(found);
            continue;
        }
        let remaining = POLL_INTERVAL.saturating_sub(poll_started.elapsed());
        tokio::select! {
            control = controls.recv() => match control {
                Some(Control::Search(_)) => {}
                Some(ending @ (Control::Finish | Control::Close { .. })) => break Some(ending),
                None => break None,
            },
            () = tokio::time::sleep(remaining) => {}
        }
    };
    match ending {
        Some(Control::Finish) => {
            for id in selection
                .map(|found| found.all_candidate_ids)
                .unwrap_or_default()
            {
                if let Some(request) = gmail_api::trash(&id) {
                    let _ =
                        api_call(&queue, &address, &request, gmail_api::trash_from_response).await;
                }
            }
        }
        Some(Control::Close { check_spam: true }) => {
            if let Some(prompted_at) = prompted_at
                && let Ok(refs) = api_call(
                    &queue,
                    &address,
                    &otp_mail::gmail_spam_search(),
                    gmail_api::messages_from_response,
                )
                .await
            {
                let mut spam = Vec::new();
                for reference in refs {
                    if let Some(request) = gmail_api::message_metadata(&reference.id)
                        && let Ok(message) =
                            api_call(&queue, &address, &request, MessageMetadata::from_response)
                                .await
                    {
                        spam.push(message);
                    }
                }
                if otp_mail::gmail_has_fresh_code(prompted_at, &spam) {
                    report(Report::FoundInSpam);
                }
            }
        }
        _ => {}
    }
}

fn failed(failure: &ApiFailure) -> Report {
    Report::Failed {
        failure: failure.failure(),
        reason: failure.describe(),
    }
}

/// One poll: the messages added since the cursor (or found by search once the history is gone),
/// then the metadata of the ones not fetched yet.
async fn api_poll(
    queue: &Queue,
    address: &str,
    cursor: &mut Option<String>,
    seen: &mut HashSet<String>,
    pending: &mut Vec<MessageRef>,
    messages: &mut Vec<MessageMetadata>,
) -> Result<(), ApiFailure> {
    let references = match cursor.clone() {
        Some(start) => match history(queue, address, &start).await {
            Ok((references, latest)) => {
                if let Some(latest) = latest {
                    *cursor = Some(latest);
                }
                references
            }
            Err(ApiFailure::Api(ApiError::NotFound)) => {
                queue.run(|runtime| {
                    runtime.system_log(
                        LogLevel::Notice,
                        LogCategory::Otp,
                        "Gmail history ID is no longer valid; searching instead",
                    );
                });
                *cursor = None;
                api_call(
                    queue,
                    address,
                    &otp_mail::gmail_search(),
                    gmail_api::messages_from_response,
                )
                .await?
            }
            Err(failure) => return Err(failure),
        },
        None => {
            api_call(
                queue,
                address,
                &otp_mail::gmail_search(),
                gmail_api::messages_from_response,
            )
            .await?
        }
    };
    for reference in references {
        if seen.contains(&reference.id) || pending.iter().any(|other| other.id == reference.id) {
            continue;
        }
        if otp_mail::is_excluded_gmail_message(&reference.label_ids) {
            seen.insert(reference.id);
        } else {
            pending.push(reference);
        }
    }
    let mut first_error = None;
    for reference in std::mem::take(pending) {
        let Some(request) = gmail_api::message_metadata(&reference.id) else {
            seen.insert(reference.id);
            continue;
        };
        match api_call(queue, address, &request, MessageMetadata::from_response).await {
            Ok(message) => {
                seen.insert(reference.id);
                messages.push(message);
            }
            Err(ApiFailure::Api(ApiError::NotFound)) => {
                seen.insert(reference.id);
            }
            Err(failure) => {
                pending.push(reference);
                first_error.get_or_insert(failure);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// The messages added since `start`, and the history ID to go on from when every page was read.
async fn history(
    queue: &Queue,
    address: &str,
    start: &str,
) -> Result<(Vec<MessageRef>, Option<String>), ApiFailure> {
    let mut references = Vec::new();
    let mut page_token: Option<String> = None;
    let mut latest = None;
    for _ in 0..MAX_HISTORY_PAGES {
        let request = gmail_api::history(start, page_token.as_deref());
        let page = api_call(queue, address, &request, HistoryPage::from_response).await?;
        references.extend(page.added_messages);
        latest = page.history_id.or(latest);
        page_token = page.next_page_token;
        if page_token.is_none() {
            return Ok((
                references,
                latest.filter(|id| gmail_api::is_valid_history_id(id)),
            ));
        }
    }
    Ok((references, None))
}

fn api_timing_note(prompted_at: SystemTime, arrived_at: SystemTime, polls: u32) -> String {
    let seconds = |from: SystemTime, to: SystemTime| {
        to.duration_since(from).unwrap_or_default().as_secs_f64()
    };
    let delivery = seconds(prompted_at, arrived_at);
    let detection = seconds(arrived_at, SystemTime::now());
    let noun = if polls == 1 { "poll" } else { "polls" };
    format!(
        "‹OTP mail (Gmail API): in Gmail {delivery:.1}s after the prompt · detected {detection:.1}s after arrival ({polls} {noun})›"
    )
}

// MARK: The IMAP session

#[derive(Debug)]
enum ImapError {
    LoginRejected(String),
    Failure(ImapFailure),
    AllMailMissing,
    SnapshotMissing,
    UidValidityChanged,
}

impl ImapError {
    fn describe(&self) -> String {
        match self {
            Self::LoginRejected(text) if text.is_empty() => "Gmail rejected the login".into(),
            Self::LoginRejected(text) => format!("Gmail rejected the login: {text}"),
            Self::Failure(failure) => format!("{failure:?}"),
            Self::AllMailMissing => "LIST returned no \\All mailbox".into(),
            Self::SnapshotMissing => "SELECT response has no UIDVALIDITY/UIDNEXT".into(),
            Self::UidValidityChanged => "UIDVALIDITY of the \\All mailbox changed".into(),
        }
    }
}

impl From<Rejection> for ImapError {
    fn from(rejection: Rejection) -> Self {
        Self::Failure(rejection.into_failure())
    }
}

impl From<ImapFailure> for ImapError {
    fn from(failure: ImapFailure) -> Self {
        Self::Failure(failure)
    }
}

fn deadline() -> Instant {
    Instant::now() + IMAP_COMMAND_TIMEOUT
}

/// Connects and logs in, then finds the special mailboxes.
async fn imap_log_in(
    address: &str,
    password: &SecretText,
) -> Result<(Imap, SpecialMailboxes), ImapError> {
    let by = Instant::now() + IMAP_LOGIN_TIMEOUT;
    let mut connection = Imap::connect(by).await?;
    let login = Command::AuthenticatePlain {
        username: address.to_owned(),
        password: password.clone(),
    };
    match connection.execute(&login, by).await {
        Ok(_) => {}
        Err(Rejection::Refused { code, text, .. })
            if is_credential_rejection(code.as_ref(), &text) =>
        {
            return Err(ImapError::LoginRejected(text));
        }
        Err(Rejection::Failed(ImapFailure::Protocol { .. })) if address.is_empty() => {
            return Err(ImapError::LoginRejected(String::new()));
        }
        Err(rejection) => return Err(rejection.into()),
    }
    let listed = connection.execute(&Command::List, deadline()).await?;
    Ok((connection, SpecialMailboxes::from_responses(&listed)))
}

async fn imap_session(
    address: String,
    password: SecretText,
    mut controls: mpsc::UnboundedReceiver<Control>,
    report: impl Fn(Report) + Send + 'static,
) {
    let mut state = ImapState {
        prompted_at: None,
        selection: None,
        ending: None,
        searches: 0,
        notifications: 0,
    };
    match imap_watch(&address, &password, &mut controls, &report, &mut state).await {
        Ok((connection, mailboxes)) => {
            let _ = imap_wrap_up(connection, &mailboxes, &state, &report).await;
        }
        Err(error) => {
            if state.ending.is_none() && state.selection.is_none() {
                let failure = if matches!(error, ImapError::LoginRejected(_)) {
                    OtpFailure::Authentication
                } else {
                    OtpFailure::Network
                };
                report(Report::Failed {
                    failure,
                    reason: error.describe(),
                });
            }
        }
    }
}

struct ImapState {
    prompted_at: Option<SystemTime>,
    selection: Option<otp_mail::ImapSelection>,
    ending: Option<Control>,
    searches: u32,
    notifications: u32,
}

async fn imap_watch(
    address: &str,
    password: &SecretText,
    controls: &mut mpsc::UnboundedReceiver<Control>,
    report: &(impl Fn(Report) + Send),
    state: &mut ImapState,
) -> Result<(Imap, SpecialMailboxes), ImapError> {
    let (mut connection, mailboxes) = imap_log_in(address, password).await?;
    let all = mailboxes.all.clone().ok_or(ImapError::AllMailMissing)?;
    let selected = connection
        .execute(
            &Command::Select {
                mailbox: all.name.clone(),
            },
            deadline(),
        )
        .await?;
    let snapshot =
        MailboxSnapshot::from_select_responses(&selected).ok_or(ImapError::SnapshotMissing)?;
    report(Report::Ready);
    let mut trigger = "search right after the prompt";
    loop {
        while let Ok(control) = controls.try_recv() {
            match control {
                Control::Search(at) => {
                    state.prompted_at.get_or_insert(at);
                }
                ending => {
                    state.ending = Some(ending);
                    return Ok((connection, mailboxes));
                }
            }
        }
        let searching = state.prompted_at.is_some() && state.selection.is_none();
        if searching {
            state.searches += 1;
            imap_search(&mut connection, &snapshot, state, trigger, report).await?;
        }
        let searching = state.prompted_at.is_some() && state.selection.is_none();
        let period = if searching {
            IMAP_SEARCH_INTERVAL
        } else {
            IMAP_IDLE_REFRESH
        };
        let (updates, interrupted) = connection.idle(period, controls.recv()).await?;
        match interrupted {
            Some(Some(Control::Search(at))) => {
                state.prompted_at.get_or_insert(at);
            }
            Some(Some(ending)) => {
                state.ending = Some(ending);
                return Ok((connection, mailboxes));
            }
            Some(None) => {
                state.ending = Some(Control::Close { check_spam: false });
                return Ok((connection, mailboxes));
            }
            None => {}
        }
        if !snapshot.uid_validity_kept(&updates) {
            return Err(ImapError::UidValidityChanged);
        }
        let notified = updates
            .iter()
            .any(|response| matches!(response, Response::Exists(_)));
        if searching && notified {
            state.notifications += 1;
        }
        trigger = if notified {
            "IDLE notification"
        } else {
            "periodic search"
        };
    }
}

async fn imap_search(
    connection: &mut Imap,
    snapshot: &MailboxSnapshot,
    state: &mut ImapState,
    trigger: &str,
    report: &(impl Fn(Report) + Send),
) -> Result<(), ImapError> {
    let prompted_at = state.prompted_at.expect("searching");
    let search = connection
        .execute(
            &Command::UidSearch(UidSet::starting_at(snapshot.uid_next)),
            deadline(),
        )
        .await?;
    if !snapshot.uid_validity_kept(&search) {
        return Err(ImapError::UidValidityChanged);
    }
    let Some(uids) = snapshot.new_uids(&search) else {
        return Ok(());
    };
    let fetched = connection
        .execute(&Command::UidFetchHeaders(uids), deadline())
        .await?;
    if !snapshot.uid_validity_kept(&fetched) {
        return Err(ImapError::UidValidityChanged);
    }
    let messages = otp_mail::imap_messages(&fetched);
    let Some(selection) = otp_mail::select_imap(prompted_at, &messages) else {
        return Ok(());
    };
    let rounded = |from: SystemTime, to: SystemTime| {
        to.duration_since(from)
            .unwrap_or_default()
            .as_secs_f64()
            .round() as u64
    };
    let note = format!(
        "‹OTP mail: in Gmail {}s after the prompt · detected {}s after arrival ({trigger} · {} searches · {} IDLE notifications)›",
        rounded(prompted_at, selection.arrived_at),
        rounded(selection.arrived_at, SystemTime::now()),
        state.searches,
        state.notifications,
    );
    report(Report::CodeFound {
        code: selection.code.clone(),
        note,
    });
    state.selection = Some(selection);
    Ok(())
}

/// The requested ending, then logout; errors only matter to the log, as the attempt moved on.
async fn imap_wrap_up(
    mut connection: Imap,
    mailboxes: &SpecialMailboxes,
    state: &ImapState,
    report: &(impl Fn(Report) + Send),
) -> Result<(), ImapError> {
    match &state.ending {
        Some(Control::Finish) => {
            if let (Some(selection), Some(trash)) = (&state.selection, &mailboxes.trash)
                && let Some(uids) = UidSet::new(selection.all_candidate_uids.clone())
            {
                let _ = connection
                    .execute(
                        &Command::UidMove {
                            uids,
                            mailbox: trash.name.clone(),
                        },
                        deadline(),
                    )
                    .await;
            }
        }
        Some(Control::Close { check_spam: true }) => {
            if let (Some(prompted_at), Some(junk)) = (state.prompted_at, &mailboxes.junk) {
                connection
                    .execute(
                        &Command::Examine {
                            mailbox: junk.name.clone(),
                        },
                        deadline(),
                    )
                    .await?;
                let found = connection
                    .execute(&otp_mail::imap_spam_search(), deadline())
                    .await?;
                if let Some(uids) = otp_mail::imap_spam_check_uids(&found) {
                    let fetched = connection
                        .execute(&Command::UidFetchHeaders(uids), deadline())
                        .await?;
                    if otp_mail::imap_has_fresh_code(
                        prompted_at,
                        &otp_mail::imap_messages(&fetched),
                    ) {
                        report(Report::FoundInSpam);
                    }
                }
            }
        }
        _ => {}
    }
    connection.logout().await;
    Ok(())
}

// MARK: Tests of the Account tab

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
