//! The IMAP commands and responses the OTP session uses (SPEC F-OT-03).
//!
//! Commands become the exact bytes to send; the bytes the server sends become responses through a
//! resumable parser. Connections, TLS, time limits and matching responses to commands are the
//! runtime's.

mod command;
mod date;
mod mailbox_name;
mod parser;

use std::time::SystemTime;

pub use command::{Command, CommandError, IDLE_DONE, TagGenerator, UidSet};
pub use parser::{ParseError, Parser};

/// How the server completed a command, or the status of an untagged `OK`, `NO` or `BAD`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Status {
    Ok,
    No,
    Bad,
}

impl Status {
    /// As the server writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::No => "NO",
            Status::Bad => "BAD",
        }
    }
}

/// The bracketed code of a status response, such as `[UIDVALIDITY 3]` or `[ALERT]`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ResponseCode {
    Alert,
    Unavailable,
    Limit,
    UidValidity(u32),
    UidNext(u32),
    /// Any other code, its name in uppercase, with the raw text after the name.
    Other {
        name: String,
        argument: Option<String>,
    },
}

/// One complete server response. Anything the grammar below does not model, or that is malformed
/// but well framed, is `Other`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Response {
    /// Completion of the command sent with `tag`.
    Tagged {
        tag: String,
        status: Status,
        code: Option<ResponseCode>,
        text: String,
    },
    /// `* OK|NO|BAD [code] text`, such as `[UIDVALIDITY n]` and `[UIDNEXT n]` after SELECT.
    UntaggedStatus {
        status: Status,
        code: Option<ResponseCode>,
        text: String,
    },
    /// The server is closing the connection.
    Bye {
        code: Option<ResponseCode>,
        text: String,
    },
    List(ListEntry),
    Exists(u32),
    /// The UIDs found; empty when nothing matched.
    Search(Vec<u32>),
    Fetch(FetchResponse),
    /// A continuation request, such as the server's answer to IDLE.
    Continuation(String),
    /// The raw response without its final line break.
    Other(String),
}

/// One `* LIST` reply.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ListEntry {
    /// As sent, such as `\HasNoChildren` and `\All`.
    pub attributes: Vec<String>,
    /// The name as sent (modified UTF-7); commands take it unchanged.
    pub name: String,
}

impl ListEntry {
    /// The name for people, decoded from modified UTF-7, such as Gmail's Korean trash folder
    /// `[Gmail]/&1zTJwNG1-`; the name as sent when it does not decode.
    pub fn display_name(&self) -> String {
        mailbox_name::decode(&self.name).unwrap_or_else(|| self.name.clone())
    }

    /// Compares without regard to ASCII case.
    pub fn has_attribute(&self, attribute: &str) -> bool {
        self.attributes
            .iter()
            .any(|own| own.eq_ignore_ascii_case(attribute))
    }

    pub fn is_selectable(&self) -> bool {
        !self.has_attribute("\\Noselect") && !self.has_attribute("\\NonExistent")
    }
}

/// The mailboxes the OTP session needs, found by their special-use attribute (RFC 6154): the first
/// selectable LIST entry with each.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SpecialMailboxes {
    /// `\All`, where the session looks for the OTP email.
    pub all: Option<ListEntry>,
    /// `\Trash`, where used OTP emails go.
    pub trash: Option<ListEntry>,
    /// `\Junk`, checked when the OTP did not arrive; `\All` leaves spam out.
    pub junk: Option<ListEntry>,
}

impl SpecialMailboxes {
    /// From the replies to LIST; other responses are ignored.
    pub fn from_responses(responses: &[Response]) -> Self {
        let first = |attribute: &str| {
            responses.iter().find_map(|response| match response {
                Response::List(entry)
                    if entry.is_selectable() && entry.has_attribute(attribute) =>
                {
                    Some(entry.clone())
                }
                _ => None,
            })
        };
        Self {
            all: first("\\All"),
            trash: first("\\Trash"),
            junk: first("\\Junk"),
        }
    }
}

/// The parts of a `* n FETCH (...)` reply the OTP session uses.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct FetchResponse {
    pub uid: Option<u32>,
    /// When the server received the message (`INTERNALDATE`); `None` when absent or unreadable.
    pub internal_date: Option<SystemTime>,
    /// The raw header block (`BODY[HEADER...]` or `RFC822.HEADER`).
    pub header: Option<Vec<u8>>,
}

/// `\All`'s UIDVALIDITY and UIDNEXT, taken when the session opens: the OTP email is a message
/// that arrives after this point.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MailboxSnapshot {
    pub uid_validity: u32,
    pub uid_next: u32,
}

impl MailboxSnapshot {
    /// From the replies to SELECT or EXAMINE; `None` unless both codes are among them.
    pub fn from_select_responses(responses: &[Response]) -> Option<Self> {
        let mut uid_validity = None;
        let mut uid_next = None;
        for response in responses {
            let (Response::UntaggedStatus { code, .. } | Response::Tagged { code, .. }) = response
            else {
                continue;
            };
            match code {
                Some(ResponseCode::UidValidity(value)) => uid_validity = Some(*value),
                Some(ResponseCode::UidNext(value)) => uid_next = Some(*value),
                _ => {}
            }
        }
        Some(Self {
            uid_validity: uid_validity?,
            uid_next: uid_next?,
        })
    }

    /// The UIDs to fetch from the replies to `UID SEARCH UID <uid_next>:*`: only those at or after
    /// the snapshot, since the server also returns the newest older message for `n:*`
    /// (RFC 3501 §6.4.8). `None` when there are none.
    pub fn new_uids(&self, responses: &[Response]) -> Option<UidSet> {
        UidSet::new(
            searched_uids(responses)
                .into_iter()
                .filter(|uid| *uid >= self.uid_next),
        )
    }

    /// Whether every untagged UIDVALIDITY among `responses` is the snapshot's. After a change the
    /// UIDs mean nothing, so the session fails at once instead of waiting for the OTP time limit.
    pub fn uid_validity_kept(&self, responses: &[Response]) -> bool {
        responses.iter().all(|response| match response {
            Response::UntaggedStatus {
                code: Some(ResponseCode::UidValidity(value)),
                ..
            } => *value == self.uid_validity,
            _ => true,
        })
    }
}

/// The UIDs of every `* SEARCH` reply among `responses`.
pub fn searched_uids(responses: &[Response]) -> Vec<u32> {
    responses
        .iter()
        .flat_map(|response| match response {
            Response::Search(uids) => uids.as_slice(),
            _ => &[],
        })
        .copied()
        .collect()
}

/// Whether a tagged `NO` to the login means the account cannot log in this way, so retrying with
/// the same credentials cannot help: `[AUTHENTICATIONFAILED]` (a wrong or revoked app password), an
/// `[ALERT]` or Gmail's `[WEBALERT url]` asking for a sign-in in the browser, IMAP turned off for the
/// account or domain, or any other refusal. Only refusals that clearly mean "later" are not:
/// `[UNAVAILABLE]`, `[LIMIT]`, `[INUSE]`, `[SERVERBUG]`, or an `[ALERT]` about simultaneous
/// connections, bandwidth limits or a temporary problem.
pub fn is_credential_rejection(code: Option<&ResponseCode>, text: &str) -> bool {
    match code {
        Some(ResponseCode::Unavailable | ResponseCode::Limit) => false,
        Some(ResponseCode::Other { name, .. }) if name == "INUSE" || name == "SERVERBUG" => false,
        Some(ResponseCode::Alert) => {
            let text = text.to_lowercase();
            ![
                "simultaneous",
                "bandwidth",
                "limit",
                "temporar",
                "try again",
            ]
            .iter()
            .any(|keyword| text.contains(keyword))
        }
        _ => true,
    }
}
