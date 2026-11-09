//! Keeping the primary network service's additional routes in line with the exception rules
//! (SPEC F-BR-02), and the Connection tab's list drafts (SPEC F-BR-03). Routes added elsewhere
//! through the router join the list before anything is recorded as applied, so a failed save leaves
//! them looking added elsewhere rather than removed from the list. A failed write is retried a few
//! times, then reported once as a notification.

use std::collections::HashSet;
use std::time::Duration;

use fvm_contract::app_helper::{
    AdditionalRoute, BypassRoutesReply, BypassRoutesRequest, BypassRoutesSnapshot, HelperErrorCode,
};
use fvm_contract::app_internal::command::BypassList;
use fvm_contract::app_internal::port::{
    HelperCall, LogCategory, LogLevel, PortAction, PortResponse, UserNotification,
};
use fvm_contract::app_internal::problem::ConfigFileError;
use fvm_contract::app_internal::section::{BypassFailure, BypassRulesSection, BypassStatus};
use fvm_core::bypass_rules::{RuleDrafts, plan_routes};

use crate::orchestration::Runtime;

/// Network settings often change several times in a row.
const SETTLE_DELAY: Duration = Duration::from_secs(1);
const WRITE_RETRY_DELAY: Duration = Duration::from_secs(10);
const MAX_WRITE_RETRIES: u32 = 3;

pub(crate) struct Bypass {
    drafts: Option<RuleDrafts>,
    status: BypassStatus,
    current: Option<BypassRoutesSnapshot>,
    apply_error: Option<ConfigFileError>,
    syncing: bool,
    sync_requested: bool,
    write_failures: u32,
    retry_generation: u64,
    settle_generation: u64,
    /// The Helper answered with this build; a Helper that becomes ready later starts a sync.
    pub(crate) helper_ready: bool,
}

impl Default for Bypass {
    fn default() -> Self {
        Self {
            drafts: None,
            status: BypassStatus::Checking {},
            current: None,
            apply_error: None,
            syncing: false,
            sync_requested: false,
            write_failures: 0,
            retry_generation: 0,
            settle_generation: 0,
            helper_ready: false,
        }
    }
}

/// Why reading or writing the routes failed, and whether it happened while writing.
struct SyncFailure {
    failure: BypassFailure,
    writing: bool,
}

impl Runtime {
    fn drafts(&mut self) -> &mut RuleDrafts {
        let routes = &self.config.settings().routes;
        let (networks, targets) = (routes.networks.clone(), routes.bypass_vpn.clone());
        self.bypass
            .drafts
            .get_or_insert_with(|| RuleDrafts::new(networks, targets))
    }

    pub(crate) fn bypass_section(&self) -> BypassRulesSection {
        let routes = &self.config.settings().routes;
        let fresh;
        let drafts = match &self.bypass.drafts {
            Some(drafts) => drafts,
            None => {
                fresh = RuleDrafts::new(routes.networks.clone(), routes.bypass_vpn.clone());
                &fresh
            }
        };
        drafts.section(
            self.bypass.current.as_ref(),
            self.bypass.status.clone(),
            self.bypass.apply_error.clone(),
        )
    }

    pub(crate) fn set_bypass_rows(&mut self, list: BypassList, rows: Vec<String>) {
        self.drafts().list_mut(list).set_rows(rows);
        self.bypass.apply_error = None;
    }

    pub(crate) fn add_current_network(&mut self) {
        if let Some(current) = self.bypass.current.clone() {
            self.drafts().add_current_network(&current);
            self.bypass.apply_error = None;
        }
    }

    pub(crate) fn discard_bypass_changes(&mut self) {
        self.drafts().discard_changes();
        self.bypass.apply_error = None;
    }

    pub(crate) fn apply_bypass_rules(&mut self) {
        let Some((networks, targets)) = self.drafts().entries() else {
            return;
        };
        match self.config.update(|settings| {
            settings.routes.networks.clone_from(&networks);
            settings.routes.bypass_vpn.clone_from(&targets);
        }) {
            Ok(_) => {
                self.bypass.apply_error = None;
                self.bypass.drafts = None;
                self.config_reloaded();
            }
            Err(error) => self.bypass.apply_error = Some(error),
        }
    }

    /// The settings may have changed: the drafts take in the saved lists, and the routes are
    /// synced again.
    pub(crate) fn sync_bypass_routes(&mut self) {
        let routes = self.config.settings().routes.clone();
        if let Some(drafts) = &mut self.bypass.drafts {
            if drafts.networks.saved() != routes.networks.as_slice() {
                drafts.networks.merge_saved(routes.networks.clone());
            }
            if drafts.targets.saved() != routes.bypass_vpn.as_slice() {
                drafts.targets.merge_saved(routes.bypass_vpn.clone());
            }
        }
        self.sync_routes();
    }

    /// Syncs once the network settings stopped changing.
    pub(crate) fn sync_bypass_routes_soon(&mut self) {
        self.bypass.settle_generation += 1;
        let generation = self.bypass.settle_generation;
        let queue = self.queue();
        self.tasks.spawn(async move {
            tokio::time::sleep(SETTLE_DELAY).await;
            queue.run(move |runtime| {
                if runtime.bypass.settle_generation == generation {
                    runtime.sync_routes();
                }
            });
        });
    }

    fn sync_routes(&mut self) {
        if self.bypass.syncing {
            self.bypass.sync_requested = true;
            return;
        }
        if !self.config.exists() {
            self.bypass.status = BypassStatus::NoConfigFile {};
            return;
        }
        if !self.config.is_valid() {
            self.bypass.status = BypassStatus::ConfigInvalid {};
            return;
        }
        // A result already shown stays until the new one is known.
        if !matches!(
            self.bypass.status,
            BypassStatus::Applied { .. } | BypassStatus::Failed { .. }
        ) {
            self.bypass.status = BypassStatus::Checking {};
        }
        self.bypass.syncing = true;
        self.call_helper(
            HelperCall::ReadBypassRoutes {},
            |runtime, response| match runtime.routes_reply(response) {
                Ok(snapshot) => runtime.plan_and_write(snapshot),
                Err(error) => runtime.sync_failed(SyncFailure {
                    failure: error,
                    writing: false,
                }),
            },
        );
    }

    /// The snapshot in a route call's answer, or why there is none. A Helper without a primary
    /// network service answers with the wrong-phase code.
    fn routes_reply(
        &mut self,
        response: PortResponse,
    ) -> Result<BypassRoutesSnapshot, BypassFailure> {
        match response {
            PortResponse::HelperReply { json: Some(json) } => {
                match serde_json::from_str::<BypassRoutesReply>(&json) {
                    Ok(BypassRoutesReply::Success { snapshot }) => Ok(snapshot),
                    Ok(BypassRoutesReply::Failure { error })
                        if error.code == HelperErrorCode::WrongPhase =>
                    {
                        self.system_log(
                            LogLevel::Notice,
                            LogCategory::BypassRoutes,
                            format!("exception rules not applied: {}", error.message),
                        );
                        Err(BypassFailure::NetworkChanged {})
                    }
                    Ok(BypassRoutesReply::Failure { error })
                        if error.code == HelperErrorCode::InvalidRequest =>
                    {
                        Err(BypassFailure::HelperRefused {
                            message: error.message,
                        })
                    }
                    Ok(BypassRoutesReply::Failure { error }) => {
                        Err(BypassFailure::SettingsNotChanged {
                            message: error.message,
                        })
                    }
                    Err(error) => Err(BypassFailure::SettingsNotChanged {
                        message: error.to_string(),
                    }),
                }
            }
            PortResponse::HelperReply { json: None } => Err(BypassFailure::SettingsNotChanged {
                message: "empty reply".into(),
            }),
            PortResponse::HelperUnreachable { failure } => {
                Err(BypassFailure::HelperUnreachable { failure })
            }
            PortResponse::HelperBuildMismatch { helper_build } => {
                Err(BypassFailure::HelperBuildMismatch {
                    helper_build,
                    app_build: self.facts.app_build.clone(),
                })
            }
            other => panic!("a route call answered with {other:?}"),
        }
    }

    fn plan_and_write(&mut self, snapshot: BypassRoutesSnapshot) {
        self.bypass.current = Some(snapshot.clone());
        // The latest file, after the round trip.
        let routes = self.config.settings().routes.clone();
        let applied = self
            .state
            .state()
            .applied_routes
            .get(&snapshot.service_id)
            .cloned()
            .unwrap_or_default();
        let plan = plan_routes(&routes.networks, &routes.bypass_vpn, &applied, &snapshot);
        if !plan.imported.is_empty() {
            let imported = plan.imported.clone();
            let saved = self.config.update(|settings| {
                for network in &imported {
                    if !settings.routes.bypass_vpn.contains(network) {
                        settings.routes.bypass_vpn.push(*network);
                    }
                }
            });
            if let Err(error) = saved {
                return self.sync_failed(SyncFailure {
                    failure: BypassFailure::ImportNotSaved { error },
                    writing: true,
                });
            }
            self.system_log(
                LogLevel::Notice,
                LogCategory::BypassRoutes,
                format!(
                    "added {} route(s) found on {} to routes.bypass_vpn",
                    plan.imported.len(),
                    snapshot.service_name
                ),
            );
            self.config_reloaded();
        }
        if !plan.needs_write {
            return self.sync_succeeded(&snapshot, &plan.applied, plan.status(&snapshot));
        }
        let request = BypassRoutesRequest {
            expected: snapshot.clone(),
            routes: plan.routes.clone(),
        };
        let json = serde_json::to_string(&request).expect("a route request encodes");
        self.call_helper(
            HelperCall::WriteBypassRoutes { json },
            move |runtime, response| {
                let current = match runtime.routes_reply(response) {
                    Ok(current) => current,
                    Err(failure) => {
                        return runtime.sync_failed(SyncFailure {
                            failure,
                            writing: true,
                        });
                    }
                };
                let kept: HashSet<&AdditionalRoute> = current.routes.iter().collect();
                let wanted: HashSet<&AdditionalRoute> = plan.routes.iter().collect();
                if current.routes.len() != plan.routes.len() || kept != wanted {
                    runtime.system_log(
                        LogLevel::Error,
                        LogCategory::BypassRoutes,
                        format!(
                            "{} did not keep the routes that were written",
                            snapshot.service_name
                        ),
                    );
                    return runtime.sync_failed(SyncFailure {
                        failure: BypassFailure::RoutesNotKept {},
                        writing: true,
                    });
                }
                runtime.system_log(
                    LogLevel::Notice,
                    LogCategory::BypassRoutes,
                    format!(
                        "wrote {} exception route(s) to {}",
                        plan.applied.len(),
                        snapshot.service_name
                    ),
                );
                let status = plan.status(&current);
                runtime.sync_succeeded(&current, &plan.applied, status);
            },
        );
    }

    fn sync_succeeded(
        &mut self,
        current: &BypassRoutesSnapshot,
        applied: &[AdditionalRoute],
        status: BypassStatus,
    ) {
        let service = current.service_id.clone();
        let applied = applied.to_vec();
        if let Err(problem) = self.state.update(|state| {
            if applied.is_empty() {
                state.applied_routes.remove(&service);
            } else {
                state.applied_routes.insert(service, applied);
            }
        }) {
            self.system_log(LogLevel::Error, LogCategory::State, problem);
        }
        self.bypass.current = Some(current.clone());
        self.bypass.write_failures = 0;
        self.bypass.retry_generation += 1;
        self.bypass.status = status;
        self.sync_done();
    }

    fn sync_failed(&mut self, failure: SyncFailure) {
        if !failure.writing {
            // Reading failed: the network may be gone; nothing is retried.
            if matches!(failure.failure, BypassFailure::NetworkChanged {}) {
                self.bypass.current = None;
                self.bypass.status = BypassStatus::NoNetwork {};
            } else {
                self.bypass.current = None;
                self.bypass.status = BypassStatus::Failed {
                    failure: failure.failure,
                    retry_in_seconds: None,
                    stopped_after_retries: None,
                };
            }
            return self.sync_done();
        }
        self.bypass.write_failures += 1;
        let retrying = self.bypass.write_failures <= MAX_WRITE_RETRIES;
        self.bypass.status = BypassStatus::Failed {
            failure: failure.failure.clone(),
            retry_in_seconds: retrying.then_some(WRITE_RETRY_DELAY.as_secs()),
            stopped_after_retries: (!retrying).then_some(MAX_WRITE_RETRIES),
        };
        self.bypass.retry_generation += 1;
        if retrying {
            let generation = self.bypass.retry_generation;
            let queue = self.queue();
            self.tasks.spawn(async move {
                tokio::time::sleep(WRITE_RETRY_DELAY).await;
                queue.run(move |runtime| {
                    if runtime.bypass.retry_generation == generation {
                        runtime.sync_routes();
                    }
                });
            });
        } else if self.bypass.write_failures == MAX_WRITE_RETRIES + 1 {
            self.system_log(
                LogLevel::Error,
                LogCategory::BypassRoutes,
                format!("exception routes still failed after {MAX_WRITE_RETRIES} retries; retrying stopped"),
            );
            self.action(PortAction::PostNotification {
                notification: UserNotification::ExceptionRulesNotApplied {
                    retries: MAX_WRITE_RETRIES,
                    failure: failure.failure,
                },
            });
        }
        self.sync_done();
    }

    fn sync_done(&mut self) {
        self.bypass.syncing = false;
        if std::mem::take(&mut self.bypass.sync_requested) {
            self.sync_routes();
        }
    }
}
