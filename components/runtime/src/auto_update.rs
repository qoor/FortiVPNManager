use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use fvm_contract::app_helper::{
    CHECKSUMS_FILE, CHECKSUMS_SIGNATURE_FILE, PreparedUpdateReply, UpdateDiscardReply,
    UpdateInstallReply, UpdateInstallRequest, UpdatePrepareReply, UpdatePrepareRequest,
};
use fvm_contract::app_internal::ButtonState;
use fvm_contract::app_internal::port::{
    HelperCall, LogCategory, LogLevel, PortAction, PortRequest, PortResponse, UserNotification,
};
use fvm_contract::app_internal::section::{
    AutoUpdateSection, HelperState, InstallFailure, UpdateCheckFailure, UpdateProgress,
    UpdateStatus,
};
use fvm_core::auto_update::{self, Check, DownloadProgress, Platform, Release, Update, Version};
use fvm_core::connection;
use serde::de::DeserializeOwned;
use tokio::sync::oneshot;

use crate::orchestration::Runtime;
use crate::timers::Timer;
use crate::{files, net};

#[cfg(not(windows))]
pub(crate) const PLATFORM: Platform = Platform::MacOs;
#[cfg(windows)]
pub(crate) const PLATFORM: Platform = Platform::Windows;

pub(crate) const DOWNLOAD_FOLDER: &str = "FortiVPNManager-update";

pub(crate) struct Updates {
    url: String,
    version: Version,
    folder: PathBuf,
    checking: bool,
    pub(crate) last_check: Option<SystemTime>,
    pub(crate) found: Result<Option<Update>, UpdateCheckFailure>,
    failed_install_version: Option<Version>,
    phase: Phase,
    update_after_check: bool,
    prepared_question: PreparedQuestion,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreparedQuestion {
    NotAsked,
    Asking,
    Answered,
    Failed,
}

enum Phase {
    Idle,
    Restoring(Version),
    Downloading {
        version: Version,
        progress: DownloadProgress,
        cancel: Option<oneshot::Sender<()>>,
    },
    Preparing,
    Ready(Version),
    Installing {
        version: Version,
        reconnect: bool,
    },
    Failed(InstallFailure),
}

pub(crate) enum Downloaded {
    Files(UpdatePrepareRequest),
    Failed(String, io::Result<()>),
    Cancelled(io::Result<()>),
}

impl Updates {
    pub(crate) fn new(url: String, app_version: &str, folder: PathBuf) -> Self {
        Self {
            url,
            version: Version::parse(app_version).expect("the app's version is three numbers"),
            folder,
            checking: false,
            last_check: None,
            found: Ok(None),
            failed_install_version: None,
            phase: Phase::Idle,
            update_after_check: false,
            prepared_question: PreparedQuestion::NotAsked,
        }
    }

    fn busy(&self) -> bool {
        matches!(
            self.phase,
            Phase::Downloading { .. } | Phase::Preparing | Phase::Installing { .. }
        )
    }

    fn available(&self) -> Option<&Update> {
        self.found.as_ref().ok().and_then(Option::as_ref)
    }
}

impl Runtime {
    pub(crate) fn take_update_record(&mut self) -> bool {
        let state = self.state.state();
        let connected_before_update = state.connected_before_update;
        self.updates.failed_install_version = state
            .update_target
            .and_then(|target| auto_update::failed_install_version(target, self.updates.version));
        if let Err(problem) = self.state.update(|state| {
            state.update_target = None;
            state.connected_before_update = false;
        }) {
            self.system_log(LogLevel::Error, LogCategory::State, problem);
        }
        connected_before_update
    }

    pub(crate) fn check_for_updates(&mut self) {
        if self.updates.url.is_empty() || self.updates.checking || self.updates.busy() {
            return;
        }
        let now = SystemTime::now();
        self.updates.checking = true;
        self.updates.last_check = Some(now);
        self.timers
            .set(Timer::UpdateCheck, auto_update::next_check_at(now));
        let url = self.updates.url.clone();
        let user_agent = format!("FortiVPNManager/{}", self.facts.app_version);
        let queue = self.queue();
        self.tasks.spawn(async move {
            let reply = net::latest_release(&url, &user_agent).await;
            queue.run(move |runtime| runtime.update_checked(reply));
        });
    }

    pub(crate) fn check_for_updates_now(&mut self) {
        if self.updates.prepared_question != PreparedQuestion::Failed
            || self.updates.url.is_empty()
            || self.updates.checking
            || self.updates.busy()
        {
            self.check_for_updates();
            return;
        }
        self.ask_for_prepared_update(Runtime::check_for_updates);
    }

    pub(crate) fn check_for_updates_if_due(&mut self) {
        if self
            .updates
            .last_check
            .is_some_and(|last| auto_update::check_due(last, SystemTime::now()))
        {
            self.check_for_updates();
        }
    }

    pub(crate) fn check_for_updates_after_network_failure(&mut self) {
        if matches!(self.updates.found, Err(UpdateCheckFailure::Network { .. })) {
            self.check_for_updates();
        }
    }

    pub(crate) fn update_page_opened(&mut self) {
        if self
            .updates
            .last_check
            .is_none_or(|last| auto_update::page_check_due(last, SystemTime::now()))
        {
            self.check_for_updates();
        }
    }

    pub(crate) fn update_now_from_notification(&mut self) {
        if self.updates.busy() {
            return;
        }
        self.check_for_updates();
        self.updates.update_after_check = self.updates.checking;
    }

    pub(crate) fn update_checked(&mut self, reply: Result<net::Reply, net::NetworkError>) {
        self.updates.checking = false;
        let found = match reply {
            Err(detail) => Err(UpdateCheckFailure::Network { detail }),
            Ok(reply) if !(200..300).contains(&reply.status) => {
                Err(UpdateCheckFailure::HttpStatus { code: reply.status })
            }
            Ok(reply) => match Release::from_json(&reply.body) {
                Err(_) => Err(UpdateCheckFailure::InvalidReply {}),
                Ok(release) => match auto_update::check(self.updates.version, &release, PLATFORM) {
                    Check::UpToDate => Ok(None),
                    Check::Available(update) => Ok(Some(update)),
                    Check::Failed => Err(UpdateCheckFailure::InstallerMissing {}),
                },
            },
        };
        match &found {
            Err(reason) => self.system_log(
                LogLevel::Error,
                LogCategory::Update,
                format!("update check failed: {reason:?}"),
            ),
            Ok(Some(update)) => {
                let version = update.version;
                self.system_log(
                    LogLevel::Notice,
                    LogCategory::Update,
                    format!("update available: {version}"),
                );
                if self.state.state().last_notified_update != Some(version) {
                    self.action(PortAction::PostNotification {
                        notification: UserNotification::UpdateAvailable {
                            version: version.to_string(),
                        },
                    });
                    if let Err(problem) = self
                        .state
                        .update(|state| state.last_notified_update = Some(version))
                    {
                        self.system_log(LogLevel::Error, LogCategory::State, problem);
                    }
                }
            }
            Ok(None) => {}
        }
        self.updates.found = found;
        if let Phase::Restoring(version) = self.updates.phase {
            self.updates.phase = Phase::Ready(version);
        }
        if let Phase::Ready(prepared) = self.updates.phase {
            let latest = match &self.updates.found {
                Ok(found) => Some(found.as_ref().map(|update| update.version)),
                Err(_) => None,
            };
            if latest.is_some_and(|latest| latest != Some(prepared)) {
                self.discard_prepared_update(prepared);
            }
        }
        if std::mem::take(&mut self.updates.update_after_check) {
            self.update_now();
        }
    }

    pub(crate) fn read_prepared_update_once(&mut self) {
        if self.updates.prepared_question == PreparedQuestion::NotAsked {
            self.ask_for_prepared_update(|_| {});
        }
    }

    fn ask_for_prepared_update(&mut self, then: fn(&mut Runtime)) {
        self.updates.prepared_question = PreparedQuestion::Asking;
        self.call_helper(
            HelperCall::ReadPreparedUpdate {},
            move |runtime, response| {
                let problem = match runtime.update_reply::<PreparedUpdateReply>(response) {
                    Ok(PreparedUpdateReply::Prepared { version }) => {
                        runtime.updates.prepared_question = PreparedQuestion::Answered;
                        runtime.prepared_update_found(version);
                        None
                    }
                    Ok(PreparedUpdateReply::NotPrepared {}) => {
                        runtime.updates.prepared_question = PreparedQuestion::Answered;
                        None
                    }
                    Ok(PreparedUpdateReply::Failure { error }) => Some(error.message),
                    Err(failure) => Some(format!("{failure:?}")),
                };
                if let Some(problem) = problem {
                    runtime.updates.prepared_question = PreparedQuestion::Failed;
                    runtime.system_log(
                        LogLevel::Error,
                        LogCategory::Update,
                        format!("asking for a prepared update failed: {problem}"),
                    );
                }
                then(runtime);
            },
        );
    }

    fn prepared_update_found(&mut self, version: Version) {
        if !matches!(self.updates.phase, Phase::Idle) {
            return;
        }
        self.system_log(
            LogLevel::Notice,
            LogCategory::Update,
            format!("update {version} was prepared before this launch"),
        );
        self.updates.phase = Phase::Restoring(version);
        self.check_for_updates();
    }

    fn discard_prepared_update(&mut self, prepared: Version) {
        self.updates.phase = Phase::Idle;
        self.call_helper(
            HelperCall::DiscardPreparedUpdate {},
            move |runtime, response| {
                let problem = match runtime.update_reply::<UpdateDiscardReply>(response) {
                    Ok(UpdateDiscardReply::Discarded {}) => {
                        runtime.system_log(
                            LogLevel::Notice,
                            LogCategory::Update,
                            format!("discarded the prepared update {prepared}"),
                        );
                        return;
                    }
                    Ok(UpdateDiscardReply::Failure { error }) => error.message,
                    Err(failure) => format!("{failure:?}"),
                };
                runtime.system_log(
                    LogLevel::Error,
                    LogCategory::Update,
                    format!("discarding the prepared update {prepared} failed: {problem}"),
                );
            },
        );
    }

    fn helper_running(&self) -> bool {
        matches!(
            self.cm.readiness_section().helper,
            HelperState::Running { .. }
        )
    }

    fn startable_update(&self) -> Option<&Update> {
        let startable = !self.updates.checking
            && matches!(self.updates.phase, Phase::Idle | Phase::Failed(_))
            && self.helper_running();
        self.updates.available().filter(|_| startable)
    }

    pub(crate) fn update_now(&mut self) {
        let Some(update) = self.startable_update().cloned() else {
            return;
        };
        self.updates.failed_install_version = None;
        let (cancel, cancelled) = oneshot::channel();
        self.updates.phase = Phase::Downloading {
            version: update.version,
            progress: DownloadProgress::new(update.installer_size),
            cancel: Some(cancel),
        };
        let folder = self.updates.folder.clone();
        let queue = self.queue();
        self.tasks.spawn(async move {
            let received = AtomicU64::new(0);
            let started = Instant::now();
            let ticks = async {
                loop {
                    tokio::time::sleep(auto_update::PROGRESS_INTERVAL).await;
                    let (elapsed, bytes) = (started.elapsed(), received.load(Ordering::Relaxed));
                    queue.run(move |runtime| runtime.download_progressed(elapsed, bytes));
                }
            };
            let downloaded = tokio::select! {
                downloaded = download(&update, &folder, &received) => Some(downloaded),
                _ = ticks => unreachable!("the ticks go on until the download ends"),
                _ = cancelled => None,
            };
            let downloaded = match downloaded {
                Some(Ok(request)) => Downloaded::Files(request),
                Some(Err((detail, removed))) => Downloaded::Failed(detail, removed),
                None => Downloaded::Cancelled(remove_folder(&folder)),
            };
            queue.run(move |runtime| runtime.update_downloaded(downloaded));
        });
    }

    pub(crate) fn download_progressed(&mut self, elapsed: Duration, received: u64) {
        let Phase::Downloading { progress, .. } = &mut self.updates.phase else {
            panic!("download progress arrives only while downloading")
        };
        progress.record(elapsed, received);
    }

    pub(crate) fn cancel_update_download(&mut self) {
        if let Phase::Downloading { cancel, .. } = &mut self.updates.phase
            && let Some(cancel) = cancel.take()
        {
            let _ = cancel.send(());
        }
    }

    pub(crate) fn update_downloaded(&mut self, downloaded: Downloaded) {
        let Phase::Downloading { version, .. } = self.updates.phase else {
            panic!("a download ends only while downloading")
        };
        match downloaded {
            Downloaded::Files(request) => {
                self.updates.phase = Phase::Preparing;
                self.prepare_update(version, request);
            }
            Downloaded::Failed(detail, removed) => {
                self.system_log(
                    LogLevel::Error,
                    LogCategory::Update,
                    format!("downloading update {version} failed: {detail}"),
                );
                if let Err(error) = removed {
                    self.system_log(
                        LogLevel::Error,
                        LogCategory::Update,
                        format!("{}: {error}", self.updates.folder.display()),
                    );
                }
                self.end_busy(Phase::Failed(InstallFailure::DownloadFailed { detail }));
            }
            Downloaded::Cancelled(removed) => {
                self.system_log(
                    LogLevel::Notice,
                    LogCategory::Update,
                    format!("downloading update {version} cancelled"),
                );
                if let Err(error) = removed {
                    self.system_log(
                        LogLevel::Error,
                        LogCategory::Update,
                        format!("{}: {error}", self.updates.folder.display()),
                    );
                }
                self.end_busy(Phase::Idle);
            }
        }
    }

    fn prepare_update(&mut self, version: Version, request: UpdatePrepareRequest) {
        let json = serde_json::to_string(&request).expect("a prepare request encodes");
        self.call_helper(
            HelperCall::PrepareUpdate { json },
            move |runtime, response| {
                let folder = runtime.updates.folder.clone();
                let queue = runtime.queue();
                runtime.tasks.spawn(async move {
                    let removed = remove_folder(&folder);
                    queue.run(move |runtime| runtime.update_prepared(version, response, removed));
                });
            },
        );
    }

    fn update_prepared(
        &mut self,
        version: Version,
        response: PortResponse,
        removed: io::Result<()>,
    ) {
        if let Err(error) = removed {
            self.system_log(
                LogLevel::Error,
                LogCategory::Update,
                format!("{}: {error}", self.updates.folder.display()),
            );
        }
        let failure = match self.update_reply::<UpdatePrepareReply>(response) {
            Ok(UpdatePrepareReply::Prepared { version: prepared }) => {
                self.system_log(
                    LogLevel::Notice,
                    LogCategory::Update,
                    format!("update {prepared} prepared"),
                );
                self.end_busy(Phase::Ready(prepared));
                return;
            }
            Ok(UpdatePrepareReply::Refused { reason, message }) => {
                InstallFailure::HelperRefused { reason, message }
            }
            Ok(UpdatePrepareReply::Failure { error }) => InstallFailure::HelperFailed {
                message: error.message,
            },
            Err(failure) => failure,
        };
        self.system_log(
            LogLevel::Error,
            LogCategory::Update,
            format!("preparing update {version} failed: {failure:?}"),
        );
        self.end_busy(Phase::Failed(failure));
    }

    pub(crate) fn restart_to_update(&mut self) {
        let Phase::Ready(version) = self.updates.phase else {
            return;
        };
        if !self.helper_running() {
            return;
        }
        self.updates.phase = Phase::Installing {
            version,
            reconnect: self.cm.wants_vpn(),
        };
        self.feed(connection::Input::StopForUpdate);
    }

    pub(crate) fn update_vpn_stopped(&mut self) {
        self.remove_bypass_routes_then(Runtime::start_update_install);
    }

    fn start_update_install(&mut self) {
        let Phase::Installing { version, reconnect } = self.updates.phase else {
            panic!("the update's install starts only while installing")
        };
        self.flush_log_lines();
        self.log.flush();
        let install_id = getrandom::u64().expect("random bytes from the OS");
        self.request(
            PortRequest::StartUpdateWaiter { install_id },
            move |runtime, response| match response {
                PortResponse::UpdateWaiterStarted {} => {
                    runtime.call_update_install(version, reconnect, install_id)
                }
                PortResponse::UpdateWaiterFailed { description } => {
                    runtime.system_log(
                        LogLevel::Error,
                        LogCategory::Update,
                        format!("starting the update waiter failed: {description}"),
                    );
                    runtime.update_install_failed(
                        reconnect,
                        InstallFailure::WaiterFailed { description },
                    );
                }
                other => panic!("an update waiter start answered with {other:?}"),
            },
        );
    }

    fn call_update_install(&mut self, version: Version, reconnect: bool, install_id: u64) {
        if let Err(problem) = self.state.update(|state| {
            state.update_target = Some(version);
            state.connected_before_update = reconnect;
        }) {
            self.system_log(LogLevel::Error, LogCategory::State, problem);
        }
        let json = serde_json::to_string(&UpdateInstallRequest { install_id })
            .expect("an install request encodes");
        self.call_helper(
            HelperCall::InstallUpdate { json },
            move |runtime, response| {
                let failure = match runtime.update_reply::<UpdateInstallReply>(response) {
                    Ok(UpdateInstallReply::Started {
                        installer_process_id,
                    }) => {
                        if let Some(installer_process_id) = installer_process_id {
                            runtime.action(PortAction::ReleaseUpdateWaiter {
                                installer_process_id,
                            });
                        }
                        runtime.system_log(
                            LogLevel::Notice,
                            LogCategory::Update,
                            format!("installing update {version}; the app quits"),
                        );
                        runtime.feed(connection::Input::QuitRequested);
                        return;
                    }
                    Ok(UpdateInstallReply::Refused { reason, message }) => {
                        InstallFailure::HelperRefused { reason, message }
                    }
                    Ok(UpdateInstallReply::Failure { error }) => InstallFailure::HelperFailed {
                        message: error.message,
                    },
                    Err(failure) => failure,
                };
                runtime.system_log(
                    LogLevel::Error,
                    LogCategory::Update,
                    format!("installing update {version} failed: {failure:?}"),
                );
                runtime.action(PortAction::StopUpdateWaiter {});
                if let Err(problem) = runtime.state.update(|state| {
                    state.update_target = None;
                    state.connected_before_update = false;
                }) {
                    runtime.system_log(LogLevel::Error, LogCategory::State, problem);
                }
                runtime.update_install_failed(reconnect, failure);
            },
        );
    }

    fn update_install_failed(&mut self, reconnect: bool, failure: InstallFailure) {
        self.resume_bypass_routes();
        self.feed(connection::Input::UpdateFailed { reconnect });
        self.end_busy(Phase::Failed(failure));
    }

    fn update_reply<T: DeserializeOwned>(
        &self,
        response: PortResponse,
    ) -> Result<T, InstallFailure> {
        match response {
            PortResponse::HelperReply { json } => json
                .ok_or_else(|| "an empty reply".to_owned())
                .and_then(|json| {
                    serde_json::from_str(&json)
                        .map_err(|error| format!("an unreadable reply: {error}"))
                })
                .map_err(|problem| InstallFailure::HelperFailed {
                    message: format!("the Helper sent {problem}"),
                }),
            PortResponse::HelperUnreachable { failure } => {
                Err(InstallFailure::HelperUnreachable { failure })
            }
            PortResponse::HelperBuildMismatch { helper_build } => {
                Err(InstallFailure::HelperBuildMismatch {
                    helper_build,
                    app_build: self.facts.app_build.clone(),
                })
            }
            other => panic!("an update call answered with {other:?}"),
        }
    }

    fn end_busy(&mut self, phase: Phase) {
        self.updates.phase = phase;
        if let Some(last) = self.updates.last_check {
            self.timers
                .set(Timer::UpdateCheck, auto_update::next_check_at(last));
        }
        self.check_for_updates_if_due();
    }

    pub(crate) fn auto_update_section(&self) -> AutoUpdateSection {
        let updates = &self.updates;
        let status = if updates.checking {
            UpdateStatus::Checking {}
        } else {
            match &updates.found {
                Ok(None) => UpdateStatus::UpToDate {},
                Ok(Some(update)) => UpdateStatus::Available {
                    version: update.version.to_string(),
                    release_page_url: update.release_page_url.clone(),
                    installer_bytes: update.installer_size,
                    release_note_titles: update.release_note_titles.clone(),
                },
                Err(reason) => UpdateStatus::CheckFailed {
                    reason: reason.clone(),
                },
            }
        };
        let available = updates.available().is_some();
        let progress = match &updates.phase {
            Phase::Idle | Phase::Restoring(_) => UpdateProgress::Idle {},
            Phase::Downloading { progress, .. } => UpdateProgress::Downloading {
                received_bytes: progress.received(),
                total_bytes: progress.total(),
                percent: progress.percent(),
                remaining: progress.remaining(),
            },
            Phase::Preparing => UpdateProgress::Preparing {
                percent: auto_update::DOWNLOADED_PERCENT,
            },
            Phase::Ready(version) => UpdateProgress::ReadyToRestart {
                version: version.to_string(),
            },
            Phase::Installing { version, .. } => UpdateProgress::Installing {
                version: version.to_string(),
            },
            Phase::Failed(reason) => UpdateProgress::Failed {
                reason: reason.clone(),
            },
        };
        let update_now = if self.startable_update().is_some() {
            ButtonState::Enabled
        } else if matches!(updates.phase, Phase::Idle | Phase::Failed(_))
            && matches!(status, UpdateStatus::Available { .. })
        {
            ButtonState::Disabled
        } else {
            ButtonState::Hidden
        };
        let restart = match updates.phase {
            Phase::Ready(_) if self.helper_running() => ButtonState::Enabled,
            Phase::Ready(_) | Phase::Installing { .. } => ButtonState::Disabled,
            _ => ButtonState::Hidden,
        };
        let (card_dot, menu_badge) = match updates.phase {
            Phase::Idle | Phase::Restoring(_) | Phase::Failed(_) => (available, available),
            Phase::Downloading { .. } | Phase::Preparing => (false, true),
            Phase::Ready(_) => (true, true),
            Phase::Installing { .. } => (false, false),
        };
        AutoUpdateSection {
            current_version: self.facts.app_version.clone(),
            status,
            failed_install_version: updates
                .failed_install_version
                .map(|version| version.to_string()),
            progress,
            check: if updates.checking || updates.busy() {
                ButtonState::Disabled
            } else {
                ButtonState::Enabled
            },
            update_now,
            cancel: if matches!(updates.phase, Phase::Downloading { .. }) {
                ButtonState::Enabled
            } else {
                ButtonState::Hidden
            },
            restart,
            card_dot,
            menu_badge,
        }
    }
}

async fn download(
    update: &Update,
    folder: &Path,
    received: &AtomicU64,
) -> Result<UpdatePrepareRequest, (String, io::Result<()>)> {
    let folder_failed = |error: io::Error| (format!("{}: {error}", folder.display()), Ok(()));
    remove_folder(folder).map_err(folder_failed)?;
    files::make_directory(folder).map_err(folder_failed)?;
    let transfer_failed = |detail: String| (detail, remove_folder(folder));
    let installer = folder.join(&update.installer_file_name);
    net::download(&update.installer_url, &installer, |bytes| {
        received.fetch_add(bytes as u64, Ordering::Relaxed);
    })
    .await
    .map_err(|detail| transfer_failed(format!("{}: {detail}", update.installer_file_name)))?;
    for (url, name) in [
        (&update.checksums_url, CHECKSUMS_FILE),
        (&update.checksums_signature_url, CHECKSUMS_SIGNATURE_FILE),
    ] {
        net::download(url, &folder.join(name), |_| {})
            .await
            .map_err(|detail| transfer_failed(format!("{name}: {detail}")))?;
    }
    let path = |name: &str| folder.join(name).display().to_string();
    Ok(UpdatePrepareRequest {
        installer_path: path(&update.installer_file_name),
        checksums_path: path(CHECKSUMS_FILE),
        signature_path: path(CHECKSUMS_SIGNATURE_FILE),
    })
}

fn remove_folder(folder: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(folder) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}
