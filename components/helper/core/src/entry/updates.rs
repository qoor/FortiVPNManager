use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use fvm_contract::app_helper::{
    HelperErrorCode, PreparedUpdateReply, UpdateDiscardReply, UpdateInstallReply,
    UpdateInstallRequest, UpdatePrepareReply, UpdatePrepareRequest,
};
use fvm_contract::helper_internal::{AppRequest, LogLevel};

use crate::auto_update::{Installer, Log, NotDone};

use super::{Event, error, to_json};

pub(super) fn serve_updates(
    mut installer: Installer,
    requests: &Receiver<(u64, AppRequest)>,
    replies: &Sender<Event>,
) {
    let logs = replies.clone();
    let log: Log = Arc::new(move |level, line| {
        logs.send(Event::UpdateLog(level, line))
            .expect("the entry runs as long as the process");
    });
    installer.recover(&log);
    for (id, request) in requests {
        let (line, json) = match request {
            AppRequest::PrepareUpdate { json } => prepare_update(&mut installer, &json),
            AppRequest::InstallUpdate { json } => install_update(&mut installer, &json, &log),
            AppRequest::ReadPreparedUpdate {} => read_prepared_update(&installer),
            AppRequest::DiscardPreparedUpdate {} => discard_prepared_update(&mut installer),
            other => unreachable!("{other:?} reached the update thread"),
        };
        replies
            .send(Event::Updated {
                id,
                log: line,
                json,
            })
            .expect("the entry runs as long as the process");
    }
}

fn prepare_update(installer: &mut Installer, json: &str) -> (Option<(LogLevel, String)>, String) {
    let Ok(request) = serde_json::from_str::<UpdatePrepareRequest>(json) else {
        let error = error(
            HelperErrorCode::InvalidRequest,
            "could not decode the update prepare request",
        );
        return (None, to_json(&UpdatePrepareReply::Failure { error }));
    };
    let relative = [
        &request.installer_path,
        &request.checksums_path,
        &request.signature_path,
    ]
    .into_iter()
    .find(|path| !Path::new(path).is_absolute());
    if let Some(relative) = relative {
        let message = format!("{relative} is not an absolute path");
        let line = format!("update prepare invalid: {message}");
        let error = error(HelperErrorCode::InvalidRequest, message);
        return (
            Some((LogLevel::Error, line)),
            to_json(&UpdatePrepareReply::Failure { error }),
        );
    }
    let (level, line, reply) = match installer.prepare(&request) {
        Ok(prepared) => (
            LogLevel::Notice,
            format!("update prepared: {} verified", prepared.installer),
            UpdatePrepareReply::Prepared {
                version: prepared.version,
            },
        ),
        Err(NotDone::Refused(reason, message)) => (
            LogLevel::Notice,
            format!("update prepare refused ({reason:?}): {message}"),
            UpdatePrepareReply::Refused { reason, message },
        ),
        Err(NotDone::Failed(message)) => (
            LogLevel::Error,
            format!("update prepare failed: {message}"),
            UpdatePrepareReply::Failure {
                error: error(HelperErrorCode::InternalError, message),
            },
        ),
    };
    (Some((level, line)), to_json(&reply))
}

fn install_update(
    installer: &mut Installer,
    json: &str,
    log: &Log,
) -> (Option<(LogLevel, String)>, String) {
    let Ok(request) = serde_json::from_str::<UpdateInstallRequest>(json) else {
        let error = error(
            HelperErrorCode::InvalidRequest,
            "could not decode the update install request",
        );
        return (None, to_json(&UpdateInstallReply::Failure { error }));
    };
    let id = request.install_id;
    let (level, line, reply) = match installer.install(id, log) {
        Ok(started) => (
            LogLevel::Notice,
            format!(
                "update install {id}: runner {} started for {}",
                started.runner, started.installer
            ),
            UpdateInstallReply::Started {
                installer_process_id: started.installer_process,
            },
        ),
        Err(NotDone::Refused(reason, message)) => (
            LogLevel::Notice,
            format!("update install {id} refused ({reason:?}): {message}"),
            UpdateInstallReply::Refused { reason, message },
        ),
        Err(NotDone::Failed(message)) => (
            LogLevel::Error,
            format!("update install {id} failed: {message}"),
            UpdateInstallReply::Failure {
                error: error(HelperErrorCode::InternalError, message),
            },
        ),
    };
    (Some((level, line)), to_json(&reply))
}

fn read_prepared_update(installer: &Installer) -> (Option<(LogLevel, String)>, String) {
    let (log, reply) = match installer.prepared() {
        Ok(Some(prepared)) => (
            None,
            PreparedUpdateReply::Prepared {
                version: prepared.version,
            },
        ),
        Ok(None) => (None, PreparedUpdateReply::NotPrepared {}),
        Err(NotDone::Refused(_, message)) => (
            Some((LogLevel::Notice, message)),
            PreparedUpdateReply::NotPrepared {},
        ),
        Err(NotDone::Failed(message)) => (
            Some((
                LogLevel::Error,
                format!("reading the prepared update failed: {message}"),
            )),
            PreparedUpdateReply::Failure {
                error: error(HelperErrorCode::InternalError, message),
            },
        ),
    };
    (log, to_json(&reply))
}

fn discard_prepared_update(installer: &mut Installer) -> (Option<(LogLevel, String)>, String) {
    let (level, line, reply) = match installer.discard() {
        Ok(()) => (
            LogLevel::Notice,
            "prepared update discarded".to_owned(),
            UpdateDiscardReply::Discarded {},
        ),
        Err(NotDone::Refused(_, message)) => (
            LogLevel::Notice,
            format!("discarding the prepared update refused: {message}"),
            UpdateDiscardReply::Failure {
                error: error(HelperErrorCode::Busy, message),
            },
        ),
        Err(NotDone::Failed(message)) => (
            LogLevel::Error,
            format!("discarding the prepared update failed: {message}"),
            UpdateDiscardReply::Failure {
                error: error(HelperErrorCode::InternalError, message),
            },
        ),
    };
    (Some((level, line)), to_json(&reply))
}
