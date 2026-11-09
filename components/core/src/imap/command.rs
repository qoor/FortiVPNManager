use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use fvm_contract::app_internal::SecretText;

/// The line that ends IDLE; it has no tag.
pub const IDLE_DONE: &[u8] = b"DONE\r\n";

/// Command tags for one session: `A0001`, `A0002`, and so on, at least four digits.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct TagGenerator {
    counter: u64,
}

impl TagGenerator {
    pub fn next_tag(&mut self) -> String {
        self.counter += 1;
        format!("A{:04}", self.counter)
    }
}

/// A non-empty set of message UIDs as a command writes it, such as `1,3:5,9` or `42:*`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct UidSet(Kind);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Kind {
    /// Ascending, neither overlapping nor adjacent.
    Ranges(Vec<(u32, u32)>),
    From(u32),
}

impl UidSet {
    /// The given UIDs in compact form; `None` when there are none or one is 0, which no message
    /// has.
    pub fn new(uids: impl IntoIterator<Item = u32>) -> Option<Self> {
        let mut uids: Vec<u32> = uids.into_iter().collect();
        uids.sort_unstable();
        uids.dedup();
        if uids.first().is_none_or(|first| *first == 0) {
            return None;
        }
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for uid in uids {
            match ranges.last_mut() {
                Some((_, upper)) if upper.checked_add(1) == Some(uid) => *upper = uid,
                _ => ranges.push((uid, uid)),
            }
        }
        Some(Self(Kind::Ranges(ranges)))
    }

    /// `uid:*`; a `uid` of 0 counts as 1. The server answers `n:*` with the newest message even
    /// when its UID is below `n`, so the caller filters what comes back.
    pub fn starting_at(uid: u32) -> Self {
        Self(Kind::From(uid.max(1)))
    }
}

impl fmt::Display for UidSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Kind::From(uid) => write!(f, "{uid}:*"),
            Kind::Ranges(ranges) => {
                for (index, (lower, upper)) in ranges.iter().enumerate() {
                    if index > 0 {
                        f.write_str(",")?;
                    }
                    if lower == upper {
                        write!(f, "{lower}")?;
                    } else {
                        write!(f, "{lower}:{upper}")?;
                    }
                }
                Ok(())
            }
        }
    }
}

/// Why a command cannot be written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandError {
    /// The username is empty, or the username or password holds NUL, which SASL PLAIN forbids.
    InvalidCredentials,
    /// The mailbox name holds CR, LF, NUL or a non-ASCII character. LIST returns 7-bit names
    /// (modified UTF-7) unless UTF8=ACCEPT is enabled, which this client never does.
    InvalidMailboxName,
    /// The search text holds CR, LF, NUL or a non-ASCII character.
    InvalidSearchText,
}

/// The commands the OTP session sends.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Command {
    /// `AUTHENTICATE PLAIN` with the initial response (SASL-IR, RFC 4959).
    AuthenticatePlain {
        username: String,
        password: SecretText,
    },
    /// `LIST "" "*"`; the special-use attributes come with the replies.
    List,
    /// Takes `ListEntry::name` unchanged.
    Select {
        mailbox: String,
    },
    /// SELECT without changes, for the one spam folder check.
    Examine {
        mailbox: String,
    },
    /// `UID SEARCH UID <set>`.
    UidSearch(UidSet),
    /// `UID SEARCH SUBJECT "<text>"`.
    UidSearchSubject(String),
    /// The UID, INTERNALDATE and the Subject and Date header fields, without marking the
    /// messages read.
    UidFetchHeaders(UidSet),
    /// Left by sending `IDLE_DONE`.
    Idle,
    /// `UID MOVE` (RFC 6851).
    UidMove {
        uids: UidSet,
        mailbox: String,
    },
    Logout,
}

impl Command {
    /// The whole line with `tag`, ending in CRLF. It holds the password for `AuthenticatePlain`.
    pub fn encode(&self, tag: &str) -> Result<Vec<u8>, CommandError> {
        Ok(format!("{tag} {}\r\n", self.body()?).into_bytes())
    }

    /// The command's name, as the session's failures and logs name it.
    pub fn name(&self) -> &'static str {
        match self {
            Command::AuthenticatePlain { .. } => "AUTHENTICATE",
            Command::List => "LIST",
            Command::Select { .. } => "SELECT",
            Command::Examine { .. } => "EXAMINE",
            Command::UidSearch(_) | Command::UidSearchSubject(_) => "UID SEARCH",
            Command::UidFetchHeaders(_) => "UID FETCH",
            Command::Idle => "IDLE",
            Command::UidMove { .. } => "UID MOVE",
            Command::Logout => "LOGOUT",
        }
    }

    fn body(&self) -> Result<String, CommandError> {
        Ok(match self {
            Command::AuthenticatePlain { username, password } => {
                let password = &password.0;
                if username.is_empty() || username.contains('\0') || password.contains('\0') {
                    return Err(CommandError::InvalidCredentials);
                }
                let message = format!("\0{username}\0{password}");
                format!("AUTHENTICATE PLAIN {}", STANDARD.encode(message))
            }
            Command::List => r#"LIST "" "*""#.to_owned(),
            Command::Select { mailbox } => {
                format!(
                    "SELECT {}",
                    quoted(mailbox, CommandError::InvalidMailboxName)?
                )
            }
            Command::Examine { mailbox } => {
                format!(
                    "EXAMINE {}",
                    quoted(mailbox, CommandError::InvalidMailboxName)?
                )
            }
            Command::UidSearch(uids) => format!("UID SEARCH UID {uids}"),
            Command::UidSearchSubject(text) => format!(
                "UID SEARCH SUBJECT {}",
                quoted(text, CommandError::InvalidSearchText)?
            ),
            Command::UidFetchHeaders(uids) => {
                format!(
                    "UID FETCH {uids} (UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT DATE)])"
                )
            }
            Command::Idle => "IDLE".to_owned(),
            Command::UidMove { uids, mailbox } => format!(
                "UID MOVE {uids} {}",
                quoted(mailbox, CommandError::InvalidMailboxName)?
            ),
            Command::Logout => "LOGOUT".to_owned(),
        })
    }
}

/// An IMAP quoted string, with `"` and `\` escaped.
fn quoted(value: &str, error: CommandError) -> Result<String, CommandError> {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        if !c.is_ascii() || matches!(c, '\r' | '\n' | '\0') {
            return Err(error);
        }
        if matches!(c, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    Ok(quoted)
}
