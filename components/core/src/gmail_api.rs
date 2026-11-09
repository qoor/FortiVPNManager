//! The Gmail API requests and responses the OTP session and the connection test use.
//!
//! Requests are a method and a URL without a body; the runtime sends them with the access token
//! as a bearer token. Responses are read from their HTTP status and body. Reading is lenient:
//! missing lists are empty and unknown fields are ignored.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::oauth::form_encoded;

const BASE_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me/";

/// 403 reasons that mean a rate limit rather than a refusal.
const RATE_LIMIT_REASONS: [&str; 5] = [
    "rateLimitExceeded",
    "userRateLimitExceeded",
    "dailyLimitExceeded",
    "quotaExceeded",
    "RATE_LIMIT_EXCEEDED",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Method {
    Get,
    Post,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Request {
    pub method: Method,
    pub url: String,
}

fn request(method: Method, path: &str, query: &[(&str, &str)]) -> Request {
    let mut url = format!("{BASE_URL}{path}");
    if !query.is_empty() {
        url.push('?');
        url.push_str(&form_encoded(query));
    }
    Request { method, url }
}

/// `users.getProfile`: the signed-in address and the mailbox's current history ID.
pub fn profile() -> Request {
    request(Method::Get, "profile", &[])
}

/// `users.history.list`: messages added after `start_history_id`, one page.
pub fn history(start_history_id: &str, page_token: Option<&str>) -> Request {
    let mut query = vec![
        ("startHistoryId", start_history_id),
        ("historyTypes", "messageAdded"),
        ("maxResults", "100"),
    ];
    query.extend(page_token.map(|token| ("pageToken", token)));
    request(Method::Get, "history", &query)
}

/// `users.messages.get` with only the labels, the arrival time and the Subject and Date headers.
/// `None` for an ID that cannot be a Gmail message ID; treat it as a message that is gone.
pub fn message_metadata(id: &str) -> Option<Request> {
    is_safe_identifier(id).then(|| {
        request(
            Method::Get,
            &format!("messages/{id}"),
            &[
                ("format", "metadata"),
                ("metadataHeaders", "Subject"),
                ("metadataHeaders", "Date"),
            ],
        )
    })
}

/// `users.messages.trash`: moves the message to the trash, which Gmail empties after 30 days.
/// `None` for an ID that cannot be a Gmail message ID.
pub fn trash(id: &str) -> Option<Request> {
    is_safe_identifier(id).then(|| request(Method::Post, &format!("messages/{id}/trash"), &[]))
}

/// `users.messages.list` for a Gmail search, newest first.
pub fn list_messages(search: &str, include_spam_trash: bool, max_results: u32) -> Request {
    let max_results = max_results.to_string();
    let include_spam_trash = if include_spam_trash { "true" } else { "false" };
    request(
        Method::Get,
        "messages",
        &[
            ("q", search),
            ("maxResults", &max_results),
            ("includeSpamTrash", include_spam_trash),
        ],
    )
}

/// Message IDs go into URL paths; Gmail's are URL-safe already.
fn is_safe_identifier(value: &str) -> bool {
    (1..=512).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// History IDs are unsigned 64-bit integers written in decimal.
pub fn is_valid_history_id(value: &str) -> bool {
    value.len() <= 20 && value.parse::<u64>().is_ok()
}

/// How a Gmail API call failed, from the HTTP status and Google's error body.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ApiError {
    /// 401: the access token is missing, expired or revoked.
    Unauthorized,
    /// 403 for a reason other than a rate limit, such as `insufficientPermissions`,
    /// `ACCESS_TOKEN_SCOPE_INSUFFICIENT` or `accessNotConfigured` (the Gmail API is off in the
    /// project). The reason holds only `[A-Za-z0-9_]`.
    Forbidden { reason: String },
    /// 404: an unknown message, or a history ID too old to list from.
    NotFound,
    /// 429, or 403 with a rate-limit reason.
    RateLimited,
    /// Any other status, or a success whose body could not be read.
    Server { status: u16 },
}

impl ApiError {
    /// `None` for a success.
    pub fn classify(status: u16, body: &[u8]) -> Option<Self> {
        Some(match status {
            200..=299 => return None,
            401 => ApiError::Unauthorized,
            404 => ApiError::NotFound,
            429 => ApiError::RateLimited,
            403 => {
                let reason = reason(body).unwrap_or_else(|| "forbidden".to_owned());
                if RATE_LIMIT_REASONS.contains(&reason.as_str()) {
                    ApiError::RateLimited
                } else {
                    ApiError::Forbidden { reason }
                }
            }
            _ => ApiError::Server { status },
        })
    }

    /// Worth another try before giving up.
    pub fn is_transient(&self) -> bool {
        matches!(self, ApiError::RateLimited | ApiError::Server { .. })
    }
}

/// The most specific reason in a Google error body: `errors[].reason`, then `details[].reason`,
/// then `status`, kept to `[A-Za-z0-9_]` so it is safe to log.
fn reason(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Body {
        error: Option<Inner>,
    }
    #[derive(Deserialize)]
    struct Inner {
        status: Option<String>,
        errors: Option<Vec<Item>>,
        details: Option<Vec<Item>>,
    }
    #[derive(Deserialize)]
    struct Item {
        reason: Option<String>,
    }

    let inner = serde_json::from_slice::<Body>(body).ok()?.error?;
    let items = inner
        .errors
        .into_iter()
        .flatten()
        .chain(inner.details.into_iter().flatten());
    items
        .filter_map(|item| item.reason)
        .chain(inner.status)
        .map(|candidate| {
            candidate
                .chars()
                .take(64)
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<String>()
        })
        .find(|reason| !reason.is_empty())
}

/// Classifies the response, then reads a successful body as `T`.
fn read<T: for<'de> Deserialize<'de>>(status: u16, body: &[u8]) -> Result<T, ApiError> {
    if let Some(error) = ApiError::classify(status, body) {
        return Err(error);
    }
    serde_json::from_slice(body).map_err(|_| ApiError::Server { status })
}

/// `users.getProfile`.
#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub email_address: String,
    /// Listing history from here reports only later changes.
    pub history_id: String,
}

impl Profile {
    pub fn from_response(status: u16, body: &[u8]) -> Result<Self, ApiError> {
        read(status, body)
    }

    /// The history ID to list from; `None` when it is not one, and the session searches instead.
    pub fn usable_history_id(&self) -> Option<&str> {
        is_valid_history_id(&self.history_id).then_some(self.history_id.as_str())
    }

    /// Whether this is the Gmail address `address`, without regard to case.
    pub fn belongs_to(&self, address: &str) -> bool {
        self.email_address.to_lowercase() == address.to_lowercase()
    }
}

/// A message as history and message lists name it.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct MessageRef {
    pub id: String,
    pub label_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireMessageRef {
    id: String,
    label_ids: Option<Vec<String>>,
}

impl From<WireMessageRef> for MessageRef {
    fn from(wire: WireMessageRef) -> Self {
        Self {
            id: wire.id,
            label_ids: wire.label_ids.unwrap_or_default(),
        }
    }
}

/// One page of `users.history.list` with only added messages.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HistoryPage {
    /// Every added message in history order, each once.
    pub added_messages: Vec<MessageRef>,
    /// The mailbox's current history ID, where the next listing starts once the last page is read.
    pub history_id: Option<String>,
    pub next_page_token: Option<String>,
}

impl HistoryPage {
    pub fn from_response(status: u16, body: &[u8]) -> Result<Self, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            history: Option<Vec<Record>>,
            history_id: Option<String>,
            next_page_token: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Record {
            messages_added: Option<Vec<Added>>,
        }
        #[derive(Deserialize)]
        struct Added {
            message: WireMessageRef,
        }

        let wire: Wire = read(status, body)?;
        let mut added_messages: Vec<MessageRef> = Vec::new();
        let added = wire
            .history
            .into_iter()
            .flatten()
            .flat_map(|record| record.messages_added.into_iter().flatten());
        for Added { message } in added {
            if !added_messages.iter().any(|seen| seen.id == message.id) {
                added_messages.push(message.into());
            }
        }
        Ok(Self {
            added_messages,
            history_id: wire.history_id,
            next_page_token: wire.next_page_token,
        })
    }
}

/// `users.messages.list`.
pub fn messages_from_response(status: u16, body: &[u8]) -> Result<Vec<MessageRef>, ApiError> {
    #[derive(Deserialize)]
    struct Wire {
        messages: Option<Vec<WireMessageRef>>,
    }

    let wire: Wire = read(status, body)?;
    Ok(wire
        .messages
        .into_iter()
        .flatten()
        .map(Into::into)
        .collect())
}

/// The answer to `trash`.
pub fn trash_from_response(status: u16, body: &[u8]) -> Result<(), ApiError> {
    ApiError::classify(status, body).map_or(Ok(()), Err)
}

/// `users.messages.get` with the metadata format: no body.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MessageMetadata {
    pub id: String,
    pub label_ids: Vec<String>,
    /// When Gmail received the message, to the millisecond; `None` when absent or unreadable.
    pub internal_date: Option<SystemTime>,
    /// As Gmail hands them over, encoded-words included.
    pub headers: Vec<Header>,
}

#[derive(Clone, PartialEq, Eq, Debug, Deserialize)]
pub struct Header {
    pub name: String,
    pub value: String,
}

impl MessageMetadata {
    pub fn from_response(status: u16, body: &[u8]) -> Result<Self, ApiError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Wire {
            id: String,
            label_ids: Option<Vec<String>>,
            internal_date: Option<serde_json::Value>,
            payload: Option<Payload>,
        }
        #[derive(Deserialize)]
        struct Payload {
            headers: Option<Vec<Header>>,
        }

        let wire: Wire = read(status, body)?;
        Ok(Self {
            id: wire.id,
            label_ids: wire.label_ids.unwrap_or_default(),
            internal_date: wire.internal_date.as_ref().and_then(internal_date),
            headers: wire
                .payload
                .and_then(|payload| payload.headers)
                .unwrap_or_default(),
        })
    }

    /// The raw value of the first header named `name`, without regard to case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_str())
    }
}

/// Gmail writes milliseconds since 1970 as decimal text; a JSON number is read too.
fn internal_date(value: &serde_json::Value) -> Option<SystemTime> {
    let milliseconds = match value {
        serde_json::Value::String(text) => {
            if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            text.parse::<i64>().ok()?
        }
        serde_json::Value::Number(number) => number.as_i64()?,
        _ => return None,
    };
    let magnitude = Duration::from_millis(milliseconds.unsigned_abs());
    if milliseconds >= 0 {
        UNIX_EPOCH.checked_add(magnitude)
    } else {
        UNIX_EPOCH.checked_sub(magnitude)
    }
}
