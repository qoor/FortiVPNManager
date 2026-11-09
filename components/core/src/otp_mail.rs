//! Recognizing the OTP email and its code.
//!
//! FortiGate's OTP email has the subject `AuthCode: NNNNNN` and no fixed sender, so only the
//! subject counts. A message is the attempt's OTP when it arrived after the session opened (the
//! IMAP snapshot, or the Gmail history point) and no earlier than the OTP prompt, less a small
//! tolerance; of several, the latest wins. Every message whose subject carries a code is trashed
//! after the OTP is submitted, stale ones included, so the next attempt never picks an old code.

mod header;

use std::time::{Duration, SystemTime};

use fvm_contract::app_internal::SecretText;

use crate::gmail_api::{self, MessageMetadata, Request};
use crate::imap::{self, Command, FetchResponse, Response, UidSet};

/// The subject marker; the code is the 6 to 8 ASCII digits after it, past any whitespace.
pub const SUBJECT_MARKER: &str = "AuthCode:";
/// A message counts when it arrived no earlier than the OTP prompt less this: INTERNALDATE has
/// one-second resolution and the server's and this Mac's clocks differ slightly.
pub const ARRIVAL_TOLERANCE: Duration = Duration::from_secs(5);

/// The newest spam messages the IMAP spam check reads.
const IMAP_SPAM_CHECK_LIMIT: usize = 5;
/// Gmail labels whose messages never count: the IMAP `\All` mailbox leaves spam and trash out as
/// well, and a draft is not a delivered OTP. Only the spam check looks at spam.
const EXCLUDED_GMAIL_LABELS: [&str; 3] = ["SPAM", "TRASH", "DRAFT"];

/// The code in a decoded subject, such as `123456` in `AuthCode: 123456`.
pub fn code_in_subject(subject: &str) -> Option<&str> {
    let mut rest = subject;
    while let Some(position) = rest.find(SUBJECT_MARKER) {
        rest = &rest[position + SUBJECT_MARKER.len()..];
        let digits = rest.trim_start_matches(char::is_whitespace);
        let length = digits.bytes().take_while(u8::is_ascii_digit).count();
        if (6..=8).contains(&length) {
            return Some(&digits[..length]);
        }
    }
    None
}

fn earliest_arrival(prompted_at: SystemTime) -> SystemTime {
    prompted_at
        .checked_sub(ARRIVAL_TOLERANCE)
        .expect("the prompt time is well after 1970")
}

// MARK: IMAP

/// A message fetched with `Command::UidFetchHeaders`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ImapMessage {
    pub uid: u32,
    pub internal_date: SystemTime,
    pub header_block: Vec<u8>,
}

impl ImapMessage {
    /// `None` unless the reply carries the UID, INTERNALDATE and the header block.
    pub fn from_fetch(fetch: &FetchResponse) -> Option<Self> {
        Some(Self {
            uid: fetch.uid?,
            internal_date: fetch.internal_date?,
            header_block: fetch.header.clone()?,
        })
    }

    /// The Subject field, unfolded and decoded.
    pub fn subject(&self) -> Option<String> {
        header::field("Subject", &self.header_block)
    }
}

/// The FETCH replies among `responses` that carry what `ImapMessage` needs.
pub fn imap_messages(responses: &[Response]) -> Vec<ImapMessage> {
    responses
        .iter()
        .filter_map(|response| match response {
            Response::Fetch(fetch) => ImapMessage::from_fetch(fetch),
            _ => None,
        })
        .collect()
}

/// The code to submit, and every message to trash afterwards.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImapSelection {
    pub code: SecretText,
    /// The message the code came from, and when it arrived.
    pub uid: u32,
    pub arrived_at: SystemTime,
    /// Ascending: every message whose subject carries a code, stale ones included.
    pub all_candidate_uids: Vec<u32>,
}

/// Picks the OTP once the client asked for it at `prompted_at`: the highest UID among messages
/// whose subject carries a code and that arrived in time.
pub fn select_imap(prompted_at: SystemTime, messages: &[ImapMessage]) -> Option<ImapSelection> {
    let earliest = earliest_arrival(prompted_at);
    let mut candidates: Vec<u32> = Vec::new();
    let mut best: Option<(&ImapMessage, String)> = None;
    for message in messages {
        let Some(subject) = message.subject() else {
            continue;
        };
        let Some(code) = code_in_subject(&subject) else {
            continue;
        };
        candidates.push(message.uid);
        if message.internal_date >= earliest
            && message.uid > best.as_ref().map_or(0, |(best, _)| best.uid)
        {
            best = Some((message, code.to_owned()));
        }
    }
    let (message, code) = best?;
    candidates.sort_unstable();
    candidates.dedup();
    Some(ImapSelection {
        code: SecretText(code),
        uid: message.uid,
        arrived_at: message.internal_date,
        all_candidate_uids: candidates,
    })
}

/// The subject search of the spam check, which runs in the junk mailbox after the OTP time limit.
pub fn imap_spam_search() -> Command {
    Command::UidSearchSubject("AuthCode".to_owned())
}

/// The newest of the UIDs the spam search found, to fetch.
pub fn imap_spam_check_uids(search: &[Response]) -> Option<UidSet> {
    let mut uids = imap::searched_uids(search);
    uids.sort_unstable();
    let newest = uids.len().saturating_sub(IMAP_SPAM_CHECK_LIMIT);
    UidSet::new(uids.split_off(newest))
}

/// Whether a message whose subject carries a code arrived in time; for the spam check, to explain
/// why the OTP did not come.
pub fn imap_has_fresh_code(prompted_at: SystemTime, messages: &[ImapMessage]) -> bool {
    let earliest = earliest_arrival(prompted_at);
    messages.iter().any(|message| {
        message.internal_date >= earliest
            && message
                .subject()
                .is_some_and(|subject| code_in_subject(&subject).is_some())
    })
}

// MARK: Gmail API

/// The search when the history point is unusable.
pub fn gmail_search() -> Request {
    gmail_api::list_messages("subject:AuthCode newer_than:1d", false, 10)
}

/// The search of the spam check after the OTP time limit.
pub fn gmail_spam_search() -> Request {
    gmail_api::list_messages("in:spam subject:AuthCode newer_than:1h", true, 5)
}

/// Whether a message with these labels never counts, so it need not be fetched.
pub fn is_excluded_gmail_message(label_ids: &[String]) -> bool {
    label_ids
        .iter()
        .any(|label| EXCLUDED_GMAIL_LABELS.contains(&label.as_str()))
}

/// The Subject header, trimmed and decoded.
pub fn gmail_subject(message: &MessageMetadata) -> Option<String> {
    message.header("Subject").map(header::value)
}

/// The code to submit, and every message to trash afterwards.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GmailSelection {
    pub code: SecretText,
    /// When the code's message arrived.
    pub arrived_at: SystemTime,
    /// Oldest first: every message whose subject carries a code, stale ones included.
    pub all_candidate_ids: Vec<String>,
}

/// Picks the OTP once the client asked for it at `prompted_at`: the newest message, by arrival and
/// then by ID, whose subject carries a code, that arrived in time and has no excluded label.
///
/// When the session searched instead of following the history, `candidates_not_before` is when it
/// opened, less `ARRIVAL_TOLERANCE`: older messages are neither picked nor trashed, since nothing
/// shows they arrived after the session opened.
pub fn select_gmail(
    prompted_at: SystemTime,
    messages: &[MessageMetadata],
    candidates_not_before: Option<SystemTime>,
) -> Option<GmailSelection> {
    let earliest = earliest_arrival(prompted_at);
    // An ID with its arrival; `None` sorts first.
    let mut candidates: Vec<(Option<SystemTime>, &str)> = Vec::new();
    let mut best: Option<(&MessageMetadata, SystemTime, String)> = None;
    for message in messages {
        if is_excluded_gmail_message(&message.label_ids) {
            continue;
        }
        let Some(subject) = gmail_subject(message) else {
            continue;
        };
        let Some(code) = code_in_subject(&subject) else {
            continue;
        };
        if let Some(not_before) = candidates_not_before
            && message.internal_date.is_none_or(|date| date < not_before)
        {
            continue;
        }
        match candidates.iter_mut().find(|(_, id)| *id == message.id) {
            Some(candidate) => candidate.0 = message.internal_date,
            None => candidates.push((message.internal_date, &message.id)),
        }
        let Some(date) = message.internal_date.filter(|date| *date >= earliest) else {
            continue;
        };
        if best.as_ref().is_none_or(|(best, best_date, _)| {
            (date, id_order(&message.id)) > (*best_date, id_order(&best.id))
        }) {
            best = Some((message, date, code.to_owned()));
        }
    }
    let (_, date, code) = best?;
    candidates.sort_by(|(date, id), (other_date, other_id)| {
        (date, id_order(id)).cmp(&(other_date, id_order(other_id)))
    });
    Some(GmailSelection {
        code: SecretText(code),
        arrived_at: date,
        all_candidate_ids: candidates
            .into_iter()
            .map(|(_, id)| id.to_owned())
            .collect(),
    })
}

/// Gmail message IDs are hexadecimal and grow over time: shorter first, then in text order.
fn id_order(id: &str) -> (usize, &str) {
    (id.len(), id)
}

/// Whether a message whose subject carries a code arrived in time, whatever its labels; for the
/// spam check, to explain why the OTP did not come.
pub fn gmail_has_fresh_code(prompted_at: SystemTime, messages: &[MessageMetadata]) -> bool {
    let earliest = earliest_arrival(prompted_at);
    messages.iter().any(|message| {
        message.internal_date.is_some_and(|date| date >= earliest)
            && gmail_subject(message).is_some_and(|subject| code_in_subject(&subject).is_some())
    })
}
