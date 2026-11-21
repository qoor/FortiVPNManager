use std::time::{Duration, SystemTime};

use fvm_contract::app_internal::SecretText;
use fvm_contract::app_internal::section::ImapFailure;
use fvm_core::connection::OtpFailure;
use fvm_core::imap::{
    Command, MailboxSnapshot, Response, SpecialMailboxes, UidSet, is_credential_rejection,
};
use fvm_core::otp_mail;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::{Control, Report};
use crate::net::{IMAP_COMMAND_TIMEOUT, IMAP_LOGIN_TIMEOUT, Imap, Rejection};

/// The IMAP search period while looking for the code, in case an IDLE notification is missed.
const IMAP_SEARCH_INTERVAL: Duration = Duration::from_secs(1);
/// The IDLE refresh period while not searching; servers end IDLE after about 30 minutes.
const IMAP_IDLE_REFRESH: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub(super) enum ImapError {
    LoginRejected(String),
    Failure(ImapFailure),
    AllMailMissing,
    SnapshotMissing,
    UidValidityChanged,
}

impl ImapError {
    pub(super) fn describe(&self) -> String {
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
pub(super) async fn imap_log_in(
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

pub(super) async fn imap_session(
    address: String,
    password: SecretText,
    mut controls: mpsc::UnboundedReceiver<Control>,
    report: impl Fn(Report) + Send + 'static,
) {
    let mut state = ImapState {
        prompted_at: None,
        selection: None,
        trashing_stale: false,
        ending: None,
        searches: 0,
        notifications: 0,
    };
    match imap_watch(&address, &password, &mut controls, &report, &mut state).await {
        Ok((connection, mailboxes)) => {
            if let Err(error) = imap_wrap_up(connection, &mailboxes, &state, &report).await {
                report(Report::SpamCheckFailed {
                    reason: error.describe(),
                });
            }
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
    trashing_stale: bool,
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
                Control::TrashStale { since } => {
                    state.prompted_at.get_or_insert(since);
                    state.trashing_stale = true;
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
        if state.trashing_stale && state.selection.is_some() {
            state.ending = Some(Control::Finish);
            return Ok((connection, mailboxes));
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
            Some(Some(Control::TrashStale { since })) => {
                state.prompted_at.get_or_insert(since);
                state.trashing_stale = true;
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
    if !state.trashing_stale {
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
    }
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
            let uids = state
                .selection
                .as_ref()
                .and_then(|selection| UidSet::new(selection.all_candidate_uids.clone()));
            let moved = match (uids, &mailboxes.trash) {
                (None, _) => Ok(Vec::new()),
                (Some(uids), Some(trash)) => connection
                    .execute(
                        &Command::UidMove {
                            uids,
                            mailbox: trash.name.clone(),
                        },
                        deadline(),
                    )
                    .await
                    .map_err(|rejection| ImapError::from(rejection).describe()),
                (Some(_), None) => Err("Gmail listed no Trash mailbox".to_owned()),
            };
            if state.trashing_stale {
                report(match moved {
                    Ok(_) => Report::StaleCodeTrashed,
                    Err(reason) => Report::StaleCodeNotTrashed { reason },
                });
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
