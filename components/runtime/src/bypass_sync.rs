//! Keeping the app's exception routes in line with the exception rules on the primary network, and
//! the Exception Rules pane's list drafts. The routes are in the routing table only while the app
//! wants the VPN up, so a change of that is synced at once: an attempt's routes are in place
//! before the tunnel's. The Helper removes them when the app quits or its connection goes away,
//! so the app writes them again on a new connection. A failed write, or a failed read after the
//! connection was lost, is retried a few times, then reported once as a notification.

use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::time::Duration;

use fvm_contract::app_helper::{
    BypassRoute, BypassRoutesReply, BypassRoutesRequest, BypassRoutesSnapshot, HelperErrorCode,
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
/// How long a quit waits for the Helper to remove the routes. One that does not answer removes
/// them anyway when the app's connection goes away, or a later Helper does.
const QUIT_REMOVAL_LIMIT: Duration = Duration::from_secs(5);

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
    /// The app quits: nothing is synced any more, and the quit waits for the routes' removal.
    quitting: bool,
    /// The Helper connection was lost and no sync has succeeded since.
    connection_lost: bool,
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
            quitting: false,
            connection_lost: false,
        }
    }
}

/// Why reading or writing the routes failed, and whether it happened while writing.
struct SyncFailure {
    failure: BypassFailure,
    writing: bool,
}

impl Runtime {
    /// The VPN server's address, when the settings name it by one.
    fn vpn_server_address(&self) -> Option<Ipv4Addr> {
        self.config.settings().vpn.host.parse().ok()
    }

    fn saved_drafts(&self) -> RuleDrafts {
        let routes = &self.config.settings().routes;
        RuleDrafts::new(
            routes.networks.clone(),
            routes.bypass_vpn.clone(),
            self.vpn_server_address(),
        )
    }

    fn drafts(&mut self) -> &mut RuleDrafts {
        if self.bypass.drafts.is_none() {
            self.bypass.drafts = Some(self.saved_drafts());
        }
        self.bypass.drafts.as_mut().expect("just set")
    }

    pub(crate) fn bypass_section(&self) -> BypassRulesSection {
        let fresh;
        let drafts = match &self.bypass.drafts {
            Some(drafts) => drafts,
            None => {
                fresh = self.saved_drafts();
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
        let vpn_server = self.vpn_server_address();
        if let Some(drafts) = &mut self.bypass.drafts {
            drafts.set_vpn_server(vpn_server);
            if drafts.networks.saved() != routes.networks.as_slice() {
                drafts.networks.merge_saved(routes.networks.clone());
            }
            if drafts.targets.saved() != routes.bypass_vpn.as_slice() {
                drafts.targets.merge_saved(routes.bypass_vpn.clone());
            }
        }
        self.sync_routes();
    }

    /// The Helper connection is gone, and the routes with it: the next one gets them again, and a
    /// failed read is retried until it does.
    pub(crate) fn bypass_connection_lost(&mut self) {
        self.bypass.connection_lost = true;
        self.sync_bypass_routes_soon();
    }

    /// The last sync failed and no retry is planned.
    pub(crate) fn bypass_sync_gave_up(&self) -> bool {
        matches!(
            self.bypass.status,
            BypassStatus::Failed {
                retry_in_seconds: None,
                ..
            }
        )
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
        if self.bypass.quitting {
            return;
        }
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
                Ok((snapshot, routes)) => runtime.plan_and_write(snapshot, &routes),
                Err(error) => runtime.sync_failed(SyncFailure {
                    failure: error,
                    writing: false,
                }),
            },
        );
    }

    /// The primary network and the app's routes in place, in a route call's answer, or why there
    /// are none. A Helper without a primary network answers with the wrong-phase code.
    fn routes_reply(
        &mut self,
        response: PortResponse,
    ) -> Result<(BypassRoutesSnapshot, Vec<BypassRoute>), BypassFailure> {
        match response {
            PortResponse::HelperReply { json: Some(json) } => {
                match serde_json::from_str::<BypassRoutesReply>(&json) {
                    Ok(BypassRoutesReply::Success { snapshot, routes }) => Ok((snapshot, routes)),
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
                    Ok(BypassRoutesReply::Removed {}) => {
                        panic!("a read or write of the routes answered as a removal")
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

    /// Writes the planned routes unless they are the ones in place already, or the app quits.
    fn plan_and_write(&mut self, snapshot: BypassRoutesSnapshot, in_place: &[BypassRoute]) {
        if self.bypass.quitting {
            return self.sync_done();
        }
        self.bypass.current = Some(snapshot.clone());
        // The latest file, after the round trip.
        let routes = self.config.settings().routes.clone();
        let plan = plan_routes(
            &routes.networks,
            &routes.bypass_vpn,
            &snapshot,
            self.cm.wants_vpn(),
        );
        if same_routes(in_place, &plan.routes) {
            let status = plan.status(&snapshot);
            return self.sync_succeeded(&snapshot, status);
        }
        let request = BypassRoutesRequest::Write {
            expected: snapshot.clone(),
            routes: plan.routes.clone(),
        };
        let json = serde_json::to_string(&request).expect("a route request encodes");
        self.call_helper(
            HelperCall::WriteBypassRoutes { json },
            move |runtime, response| {
                let (current, found) = match runtime.routes_reply(response) {
                    Ok(answer) => answer,
                    Err(failure) => {
                        return runtime.sync_failed(SyncFailure {
                            failure,
                            writing: true,
                        });
                    }
                };
                if !same_routes(&found, &plan.routes) {
                    runtime.system_log(
                        LogLevel::Error,
                        LogCategory::BypassRoutes,
                        format!(
                            "{} of {} exception route(s) through {} are not in the routing table",
                            plan.routes.len() - found.len(),
                            plan.routes.len(),
                            current.service_name
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
                        "wrote {} exception route(s) through {}",
                        plan.routes.len(),
                        current.service_name
                    ),
                );
                let status = plan.status(&current);
                runtime.sync_succeeded(&current, status);
            },
        );
    }

    fn sync_succeeded(&mut self, current: &BypassRoutesSnapshot, status: BypassStatus) {
        self.bypass.current = Some(current.clone());
        self.bypass.write_failures = 0;
        self.bypass.retry_generation += 1;
        self.bypass.connection_lost = false;
        self.bypass.status = status;
        self.sync_done();
    }

    fn sync_failed(&mut self, failure: SyncFailure) {
        let no_network =
            !failure.writing && matches!(failure.failure, BypassFailure::NetworkChanged {});
        // A read that failed is retried only after the Helper connection was lost, which took the
        // routes with it; otherwise the Helper may simply not be set up yet.
        if no_network || (!failure.writing && !self.bypass.connection_lost) {
            self.bypass.current = None;
            self.bypass.status = if no_network {
                BypassStatus::NoNetwork {}
            } else {
                BypassStatus::Failed {
                    failure: failure.failure,
                    retry_in_seconds: None,
                    stopped_after_retries: None,
                }
            };
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
        if self.bypass.quitting {
            self.remove_routes_for_quit();
        } else if std::mem::take(&mut self.bypass.sync_requested) {
            self.sync_routes();
        }
    }

    /// The core approved the quit: the Helper removes the app's routes first, unless it is not the
    /// app's build, which never got any. A sync under way finishes first, since the Helper takes
    /// one route request of a connection at a time.
    pub(crate) fn quit_after_removing_bypass_routes(&mut self) {
        self.bypass.quitting = true;
        if !self.bypass.helper_ready {
            return self.approve_quit();
        }
        let queue = self.queue();
        self.tasks.spawn(async move {
            tokio::time::sleep(QUIT_REMOVAL_LIMIT).await;
            queue.run(Runtime::approve_quit);
        });
        if !self.bypass.syncing {
            self.remove_routes_for_quit();
        }
    }

    fn remove_routes_for_quit(&mut self) {
        let json = serde_json::to_string(&BypassRoutesRequest::Remove {})
            .expect("a route request encodes");
        self.call_helper(
            HelperCall::WriteBypassRoutes { json },
            |runtime, response| {
                let removed = matches!(
                    &response,
                    PortResponse::HelperReply { json: Some(json) }
                        if serde_json::from_str::<BypassRoutesReply>(json)
                            .is_ok_and(|reply| reply == BypassRoutesReply::Removed {})
                );
                if !removed {
                    runtime.system_log(
                        LogLevel::Error,
                        LogCategory::BypassRoutes,
                        format!("removing the exception routes on quit failed: {response:?}"),
                    );
                }
                runtime.approve_quit();
            },
        );
    }
}

/// The same routes, whatever their order.
fn same_routes(found: &[BypassRoute], planned: &[BypassRoute]) -> bool {
    found.len() == planned.len()
        && found.iter().collect::<HashSet<_>>() == planned.iter().collect::<HashSet<_>>()
}
