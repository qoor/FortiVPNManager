use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use fvm_contract::app_internal::port::{LogCategory, LogLevel};
use fvm_core::connection::OtpFailure;
use fvm_core::gmail_api::{self, ApiError, HistoryPage, MessageMetadata, MessageRef, Profile};
use fvm_core::otp_mail;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::{Control, Report};
use crate::google_account::TokenFailure;
use crate::net;
use crate::orchestration::Queue;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const MAX_POLL_FAILURES: u32 = 3;
const RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(2);
const MAX_HISTORY_PAGES: u32 = 10;

#[derive(Debug)]
pub(super) enum ApiFailure {
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
pub(super) async fn api_call<T>(
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
                runtime.access_token(&address_for_token, move |_, token| reply(token));
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

pub(super) async fn api_session(
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
    let mut trashing_stale = false;
    let mut seen: HashSet<String> = HashSet::new();
    let mut pending: Vec<MessageRef> = Vec::new();
    let mut messages: Vec<MessageMetadata> = Vec::new();
    let mut polls = 0u32;
    let mut failures = 0u32;
    // A control that came while waiting for the next poll.
    let mut received = None;
    let ending = loop {
        if trashing_stale && selection.is_some() {
            break Some(Control::Finish);
        }
        let control = match received.take() {
            Some(control) => Some(control),
            // Before the prompt, and after a match, nothing is to be done until the next control.
            None if prompted_at.is_none() || selection.is_some() => Some(controls.recv().await),
            None => match controls.try_recv() {
                Ok(control) => Some(Some(control)),
                Err(mpsc::error::TryRecvError::Empty) => None,
                Err(mpsc::error::TryRecvError::Disconnected) => Some(None),
            },
        };
        match control {
            Some(Some(Control::Search(at))) => {
                prompted_at.get_or_insert(at);
                continue;
            }
            Some(Some(Control::TrashStale { since })) => {
                prompted_at.get_or_insert(since);
                trashing_stale = true;
                continue;
            }
            Some(ending) => break ending,
            None => {}
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
                    runtime.system_log(LogLevel::Notice, LogCategory::Otp, detail);
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
            if !trashing_stale {
                let note = api_timing_note(prompted, found.arrived_at, polls);
                report(Report::CodeFound {
                    code: found.code.clone(),
                    note,
                });
            }
            selection = Some(found);
            continue;
        }
        let remaining = POLL_INTERVAL.saturating_sub(poll_started.elapsed());
        tokio::select! {
            control = controls.recv() => received = Some(control),
            () = tokio::time::sleep(remaining) => {}
        }
    };
    match ending {
        Some(Control::Finish) => {
            let mut not_trashed = None;
            for id in selection
                .map(|found| found.all_candidate_ids)
                .unwrap_or_default()
            {
                let trashed = match gmail_api::trash(&id) {
                    Some(request) => {
                        api_call(&queue, &address, &request, gmail_api::trash_from_response)
                            .await
                            .map_err(|failure| failure.describe())
                    }
                    None => Err("a message ID unusable in a request".to_owned()),
                };
                if let Err(reason) = trashed {
                    not_trashed.get_or_insert(reason);
                }
            }
            if trashing_stale {
                report(match not_trashed {
                    None => Report::StaleCodeTrashed,
                    Some(reason) => Report::StaleCodeNotTrashed { reason },
                });
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
