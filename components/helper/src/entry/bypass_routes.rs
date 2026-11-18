//! The exception routes: each app connection's own, which this Helper adds to the routing table only
//! while that app wants them. The entry decides which routes to add and remove; the OS side carries
//! the work out, one change at a time, apart from the run. A route stays as long as any app still
//! wants it on the network it goes through, and only routes this Helper added are ever removed: one
//! that was in the table already is someone else's. A write first looks up which of its routes the
//! table has, so that the record a later Helper cleans up after lists only the routes this one is
//! about to add, before it adds them.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr};

use fvm_contract::app_helper::{
    BypassRoute, BypassRoutesReply, BypassRoutesRequest, BypassRoutesSnapshot, HelperError,
    HelperErrorCode,
};
use fvm_contract::helper_internal::{
    AddedRoute, BypassRoutesChange, BypassRoutesOutcome, FromEntry, IpNetwork, LogCategory,
    LogLevel, NetworkChange, NetworkCheck, NetworkOutcome, OutsideRoute,
};

use super::{Entry, error, to_json};
use crate::session::NetworkJob;

/// Whether `added` and `route` through `service` are the same route in the routing table. On
/// Windows a route belongs to an interface, so the same route through another one is another
/// route. On macOS it belongs to none: the route this Helper added through one network service is
/// the table's route through the same router when another service takes over.
fn same_route(added: &AddedRoute, route: &BypassRoute, service: &str) -> bool {
    added.route == *route && (added.service_id == service || cfg!(not(windows)))
}

pub(super) struct BypassRoutes {
    /// Each app connection's routes as it last wrote them.
    wanted: HashMap<u64, Wanted>,
    /// The routes this Helper added, or is adding, and has not removed, as recorded.
    added: Vec<AddedRoute>,
    /// Work for the OS side, the first one under way.
    jobs: VecDeque<Job>,
}

struct Wanted {
    /// The network service the routes go through.
    service_id: String,
    routes: Vec<BypassRoute>,
}

pub(super) enum Job {
    Network(NetworkJob),
    Status {
        id: u64,
    },
    Read {
        app: u64,
        id: u64,
    },
    Write {
        app: u64,
        id: u64,
        expected: BypassRoutesSnapshot,
        routes: Vec<BypassRoute>,
        step: WriteStep,
    },
    /// The app quits: answered once its routes are gone.
    Remove {
        app: u64,
        id: u64,
        unwanted: Vec<AddedRoute>,
    },
    /// The app's connection went away: its routes go, with no answer.
    Disconnected {
        app: u64,
        unwanted: Vec<AddedRoute>,
    },
}

pub(super) enum WriteStep {
    /// Which of the routes the table has.
    LookingUp,
    /// Removing what no app wants any more and adding what the table lacked.
    Changing { unwanted: Vec<AddedRoute> },
}

impl BypassRoutes {
    /// Starts with the routes an earlier Helper added that are still in place.
    pub(super) fn new(left: Vec<AddedRoute>) -> Self {
        Self {
            wanted: HashMap::new(),
            added: left,
            jobs: VecDeque::new(),
        }
    }

    fn unwanted(&self, app: u64, now: Option<(&str, &[BypassRoute])>) -> Vec<AddedRoute> {
        let wants = |service: &str, routes: &[BypassRoute], added: &AddedRoute| {
            routes.iter().any(|route| same_route(added, route, service))
        };
        self.added
            .iter()
            .filter(|added| {
                !now.is_some_and(|(service, routes)| wants(service, routes, added))
                    && !self.wanted.iter().any(|(other, wanted)| {
                        *other != app && wants(&wanted.service_id, &wanted.routes, added)
                    })
            })
            .cloned()
            .collect()
    }
}

/// Removing `routes` wherever the network is.
pub(super) fn removal(routes: &[AddedRoute]) -> BypassRoutesChange {
    BypassRoutesChange {
        network: NetworkCheck::Skip {},
        remove: routes.to_vec(),
        add: Vec::new(),
        look_up: Vec::new(),
    }
}

/// Of the `removal` of `routes`, those still in the routing table; all of them when the OS side
/// could not read the table, with why.
pub(super) fn left_after_removal(
    routes: &[AddedRoute],
    outcome: BypassRoutesOutcome,
) -> Result<Vec<AddedRoute>, (Vec<AddedRoute>, HelperError)> {
    match outcome {
        BypassRoutesOutcome::Changed { left, .. } => Ok(left),
        BypassRoutesOutcome::Refused { error } => Err((routes.to_vec(), error)),
    }
}

impl Entry {
    pub(super) fn read_bypass_routes(&mut self, app: u64, id: u64) {
        self.queue_bypass_job(Job::Read { app, id });
    }

    pub(super) fn write_bypass_routes(&mut self, app: u64, id: u64, json: &str) {
        match serde_json::from_str::<BypassRoutesRequest>(json) {
            Ok(BypassRoutesRequest::Write { expected, routes }) => {
                self.queue_bypass_job(Job::Write {
                    app,
                    id,
                    expected,
                    routes,
                    step: WriteStep::LookingUp,
                });
            }
            Ok(BypassRoutesRequest::Remove {}) => self.queue_bypass_job(Job::Remove {
                app,
                id,
                unwanted: Vec::new(),
            }),
            Err(_) => self.reply_bypass(
                id,
                &failure(
                    HelperErrorCode::InvalidRequest,
                    "could not decode the route request",
                ),
            ),
        }
    }

    /// The app's connection went away: no app wants its routes any more.
    pub(super) fn app_gone_with_its_bypass_routes(&mut self, app: u64) {
        self.queue_bypass_job(Job::Disconnected {
            app,
            unwanted: Vec::new(),
        });
    }

    pub(super) fn queue_bypass_job(&mut self, job: Job) {
        self.bypass_routes.jobs.push_back(job);
        if self.bypass_routes.jobs.len() == 1 {
            self.start_bypass_job();
        }
    }

    /// Starts the first job that needs the OS side, answering those before it that need none.
    fn start_bypass_job(&mut self) {
        while let Some(job) = self.bypass_routes.jobs.pop_front() {
            if let Job::Network(job) = job {
                let change = job.change.clone();
                return self.send_network_job(Job::Network(job), change);
            }
            if let Job::Status { id } = job {
                match self.runs.values().find_map(|held| held.run.routing_check()) {
                    Some(change) => return self.send_network_job(job, change),
                    None => {
                        self.reply_status(id, None);
                        continue;
                    }
                }
            }
            if let Some((job, change)) = self.begin_bypass_job(job) {
                self.send_bypass_job(job, change);
                return;
            }
        }
    }

    /// Puts `job` first again while the OS side carries out `change` for it.
    fn send_bypass_job(&mut self, job: Job, change: BypassRoutesChange) {
        self.bypass_routes.jobs.push_front(job);
        self.os.send(&FromEntry::ChangeBypassRoutes { change });
    }

    /// The job with what it learned and the change it needs from the OS side, or `None` once it is
    /// finished without one.
    fn begin_bypass_job(&mut self, job: Job) -> Option<(Job, BypassRoutesChange)> {
        let routes = &mut self.bypass_routes;
        match job {
            Job::Network(_) | Job::Status { .. } => {
                unreachable!("network jobs are started apart")
            }
            Job::Read { app, id } | Job::Write { app, id, .. } | Job::Remove { app, id, .. }
                if !self.apps.contains_key(&app) =>
            {
                let closed = failure(
                    HelperErrorCode::NotAuthorized,
                    "the app connection is closed",
                );
                self.reply_bypass(id, &closed);
                None
            }
            Job::Read { app, .. } => {
                let look_up = routes
                    .wanted
                    .get(&app)
                    .map(|wanted| wanted.routes.clone())
                    .unwrap_or_default();
                let change = BypassRoutesChange {
                    network: NetworkCheck::Read {},
                    remove: Vec::new(),
                    add: Vec::new(),
                    look_up,
                };
                Some((job, change))
            }
            Job::Write {
                ref expected,
                routes: ref wanted,
                ..
            } => {
                let change = BypassRoutesChange {
                    network: NetworkCheck::Expect {
                        network: expected.clone(),
                    },
                    remove: Vec::new(),
                    add: Vec::new(),
                    look_up: wanted.clone(),
                };
                Some((job, change))
            }
            Job::Remove { app, id, .. } => {
                let unwanted = routes.unwanted(app, None);
                if unwanted.is_empty() {
                    routes.wanted.remove(&app);
                    self.reply_bypass(id, &BypassRoutesReply::Removed {});
                    return None;
                }
                let change = removal(&unwanted);
                Some((Job::Remove { app, id, unwanted }, change))
            }
            Job::Disconnected { app, .. } => {
                routes.wanted.remove(&app);
                let unwanted = routes.unwanted(app, None);
                if unwanted.is_empty() {
                    return None;
                }
                let change = removal(&unwanted);
                Some((Job::Disconnected { app, unwanted }, change))
            }
        }
    }

    /// The OS side finished the first job's change.
    pub(super) fn bypass_routes_changed(&mut self, outcome: BypassRoutesOutcome) {
        let job = self
            .bypass_routes
            .jobs
            .pop_front()
            .expect("the OS side answers only the change it was asked for");
        match (job, outcome) {
            (Job::Network(_) | Job::Status { .. }, _) => {
                panic!("the OS side answered a network change with an exception-route outcome")
            }
            (Job::Read { id, .. }, BypassRoutesOutcome::Changed { network, found, .. }) => {
                let snapshot = network.expect("a read reads the network");
                let reply = BypassRoutesReply::Success {
                    snapshot,
                    routes: found,
                };
                self.reply_bypass(id, &reply);
            }
            (
                Job::Read { id, .. }
                | Job::Write {
                    id,
                    step: WriteStep::LookingUp,
                    ..
                },
                BypassRoutesOutcome::Refused { error },
            ) => {
                self.reply_bypass(id, &BypassRoutesReply::Failure { error });
            }
            (
                Job::Write {
                    app,
                    id,
                    expected,
                    routes,
                    step: WriteStep::LookingUp,
                },
                BypassRoutesOutcome::Changed { network, found, .. },
            ) => {
                match self.write_changes(app, &expected, &routes, &found) {
                    Ok(Some(change)) => {
                        let unwanted = change.remove.clone();
                        let job = Job::Write {
                            app,
                            id,
                            expected,
                            routes,
                            step: WriteStep::Changing { unwanted },
                        };
                        return self.send_bypass_job(job, change);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        self.reply_bypass(id, &BypassRoutesReply::Failure { error });
                        return self.start_bypass_job();
                    }
                }
                let snapshot = network.expect("a write reads the network");
                let service_id = snapshot.service_id.clone();
                self.bypass_routes
                    .wanted
                    .insert(app, Wanted { service_id, routes });
                let reply = BypassRoutesReply::Success {
                    snapshot,
                    routes: found,
                };
                self.reply_bypass(id, &reply);
            }
            (
                Job::Write {
                    id,
                    step: WriteStep::Changing { .. },
                    ..
                },
                BypassRoutesOutcome::Refused { error },
            ) => {
                // Nothing was added, so the record goes back to what is in place.
                let added = self.bypass_routes.added.clone();
                self.record_added(&added);
                self.reply_bypass(id, &BypassRoutesReply::Failure { error });
            }
            (
                Job::Write {
                    app,
                    id,
                    routes,
                    step: WriteStep::Changing { unwanted },
                    ..
                },
                BypassRoutesOutcome::Changed {
                    network,
                    added,
                    found,
                    left,
                },
            ) => {
                let snapshot = network.expect("a write reads the network");
                let service_id = &snapshot.service_id;
                let bypass = &mut self.bypass_routes;
                let listed = |list: &[AddedRoute], added: &AddedRoute| {
                    list.iter()
                        .any(|other| same_route(other, &added.route, &added.service_id))
                };
                bypass
                    .added
                    .retain(|route| !listed(&unwanted, route) || listed(&left, route));
                for route in added {
                    if !bypass
                        .added
                        .iter()
                        .any(|kept| same_route(kept, &route, service_id))
                    {
                        bypass.added.push(AddedRoute {
                            route,
                            service_id: service_id.clone(),
                        });
                    }
                }
                bypass.wanted.insert(
                    app,
                    Wanted {
                        service_id: snapshot.service_id.clone(),
                        routes,
                    },
                );
                let added = bypass.added.clone();
                self.record_added(&added);
                let reply = BypassRoutesReply::Success {
                    snapshot,
                    routes: found,
                };
                self.reply_bypass(id, &reply);
            }
            (
                Job::Remove {
                    app, id, unwanted, ..
                },
                outcome,
            ) => {
                self.bypass_routes.wanted.remove(&app);
                self.removed(&unwanted, outcome);
                self.reply_bypass(id, &BypassRoutesReply::Removed {});
            }
            (Job::Disconnected { unwanted, .. }, outcome) => self.removed(&unwanted, outcome),
        }
        self.start_bypass_job();
    }

    /// The change that makes `routes` the app's, now that the table is known to have `found` of
    /// them, or `None` when nothing needs changing. The routes it adds are recorded first; the
    /// error tells why they could not be.
    fn write_changes(
        &mut self,
        app: u64,
        expected: &BypassRoutesSnapshot,
        routes: &[BypassRoute],
        found: &[BypassRoute],
    ) -> Result<Option<BypassRoutesChange>, HelperError> {
        let service_id = expected.service_id.as_str();
        let unwanted = self.bypass_routes.unwanted(app, Some((service_id, routes)));
        let missing: Vec<BypassRoute> = routes
            .iter()
            .filter(|route| !found.contains(route))
            .cloned()
            .collect();
        if missing.is_empty() && unwanted.is_empty() {
            return Ok(None);
        }
        let mut recorded = self.bypass_routes.added.clone();
        for route in &missing {
            if !recorded
                .iter()
                .any(|added| same_route(added, route, service_id))
            {
                recorded.push(AddedRoute {
                    route: route.clone(),
                    service_id: service_id.to_owned(),
                });
            }
        }
        if let Err(problem) = self.sessions.record_bypass_routes(&recorded) {
            let message = format!("cannot record the exception routes: {problem}");
            self.os
                .log(LogLevel::Error, LogCategory::Routes, message.clone());
            return Err(error(HelperErrorCode::InternalError, message));
        }
        Ok(Some(BypassRoutesChange {
            network: NetworkCheck::Expect {
                network: expected.clone(),
            },
            remove: unwanted,
            add: missing,
            look_up: routes.to_vec(),
        }))
    }

    /// The routes no app wanted were removed, but for those still in the table, which stay this
    /// Helper's to remove later.
    fn removed(&mut self, unwanted: &[AddedRoute], outcome: BypassRoutesOutcome) {
        let left = match left_after_removal(unwanted, outcome) {
            Ok(left) => left,
            Err((left, error)) => {
                let message = format!("removing exception routes failed: {}", error.message);
                self.os.log(LogLevel::Error, LogCategory::Routes, message);
                left
            }
        };
        if !left.is_empty() {
            self.os.log(
                LogLevel::Error,
                LogCategory::Routes,
                format!(
                    "{} exception route(s) are still in the routing table after removing them",
                    left.len()
                ),
            );
        }
        let listed = |list: &[AddedRoute], added: &AddedRoute| {
            list.iter()
                .any(|other| same_route(other, &added.route, &added.service_id))
        };
        self.bypass_routes
            .added
            .retain(|route| !listed(unwanted, route) || listed(&left, route));
        let added = self.bypass_routes.added.clone();
        self.record_added(&added);
    }

    fn record_added(&self, added: &[AddedRoute]) {
        if let Err(failure) = self.sessions.record_bypass_routes(added) {
            self.os.log(
                LogLevel::Error,
                LogCategory::Routes,
                format!("cannot record the exception routes: {failure}"),
            );
        }
    }

    fn send_network_job(&mut self, job: Job, mut change: NetworkChange) {
        match &mut change {
            NetworkChange::Apply {
                remove, outside, ..
            } => {
                self.keep_exception_routes(remove);
                self.add_exception_destinations(outside);
            }
            NetworkChange::Switch { outside, .. } | NetworkChange::Check { outside, .. } => {
                self.add_exception_destinations(outside);
            }
            NetworkChange::Revert { changes } => self.keep_exception_routes(&mut changes.routes),
            NetworkChange::LookUp { .. } | NetworkChange::Prepare { .. } => {}
        }
        self.bypass_routes.jobs.push_front(job);
        self.os.send(&FromEntry::ChangeNetwork { change });
    }

    pub(super) fn network_changed(&mut self, outcome: NetworkOutcome) {
        match self.bypass_routes.jobs.pop_front() {
            Some(Job::Network(job)) => job.finish(outcome),
            Some(Job::Status { id }) => {
                let routing_complete = match outcome {
                    NetworkOutcome::Checked { complete } => Some(complete),
                    NetworkOutcome::Failed { .. } => None,
                    other => panic!("the OS side answered a routing check with {other:?}"),
                };
                self.reply_status(id, routing_complete);
            }
            _ => panic!("the OS side answered a network change it was not asked for"),
        }
        self.start_bypass_job();
    }

    fn add_exception_destinations(&self, outside: &mut Vec<IpNetwork>) {
        let exceptions = self
            .bypass_routes
            .wanted
            .values()
            .flat_map(|wanted| &wanted.routes)
            .filter_map(exception_destination);
        for destination in exceptions {
            if !outside.contains(&destination) {
                outside.push(destination);
            }
        }
    }

    fn keep_exception_routes(&mut self, routes: &mut Vec<OutsideRoute>) {
        let bypass = &mut self.bypass_routes;
        let mut kept = Vec::new();
        routes.retain(|route| {
            let wanted = bypass.wanted.values().find_map(|wanted| {
                if cfg!(windows) && wanted.service_id != route.service_id {
                    return None;
                }
                let found = wanted.routes.iter().find(|exception| {
                    exception_destination(exception) == Some(route.destination)
                        && exception.gateway.parse::<IpAddr>() == Ok(route.router)
                })?;
                Some(AddedRoute {
                    route: found.clone(),
                    service_id: route.service_id.clone(),
                })
            });
            let Some(added) = wanted else {
                return true;
            };
            kept.push(added);
            false
        });
        if kept.is_empty() {
            return;
        }
        for added in kept {
            if !bypass
                .added
                .iter()
                .any(|known| same_route(known, &added.route, &added.service_id))
            {
                bypass.added.push(added);
            }
        }
        let added = bypass.added.clone();
        self.record_added(&added);
    }

    fn reply_bypass(&self, id: u64, reply: &BypassRoutesReply) {
        self.os.send(&FromEntry::Reply {
            id,
            json: Some(to_json(reply)),
        });
    }
}

fn exception_destination(exception: &BypassRoute) -> Option<IpNetwork> {
    let bits = u32::from(exception.subnet_mask.parse::<Ipv4Addr>().ok()?);
    let length = bits.leading_ones();
    if bits.checked_shl(length).unwrap_or(0) != 0 {
        return None;
    }
    Some(IpNetwork {
        address: exception.destination.parse().ok()?,
        prefix_length: u8::try_from(length).expect("a mask has at most 32 bits"),
    })
}

fn failure(code: HelperErrorCode, message: impl Into<String>) -> BypassRoutesReply {
    BypassRoutesReply::Failure {
        error: error(code, message),
    }
}
