use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

use fvm_contract::app_internal::event::InterfaceKind;

use crate::connection::machine::attempts::decision::{Restart, restart_for};
use crate::connection::machine::otp::decision::OtpClose;
use crate::connection::machine::presence::decision::AwayLimit;
use crate::connection::machine::{
    AfterStop, AttemptReason, ConnectionManager, Phase, RetryReason, Session, Stage,
};
use crate::connection::policy::{self, elapsed};
use crate::connection::system::{PathSnapshot, SettleStep, Underlay, WakeObservation, WakeSettle};
use crate::connection::{GatewayLookup, InterfaceAddress, ProbeResult};

/// What the coming sleep does to the connection.
pub(in crate::connection::machine) enum Sleep {
    /// A tunnel that is up is kept across the sleep, its proactive timer waiting, as is a wait
    /// for an away user.
    Keep,
    /// The replacement under way is dropped and tried again at `next_try`.
    PauseReplacement {
        stop_run: bool,
        otp_close: Option<OtpClose>,
        next_try: SystemTime,
    },
    AbortAttempt,
    WaitForWake,
    /// The reconnect after the stop waits for the wake.
    ResumeAfterWake,
}

/// The wake, before the network is looked at.
pub(in crate::connection::machine) struct Wake {
    pub(super) at: SystemTime,
    pub(super) slept_for: Duration,
    /// The away time may have run out during the sleep; acted on before the user unlocks.
    pub(super) limit: AwayLimit,
    pub(super) settle: WakeSettle,
}

/// The next look at the network after a wake: until it is back or the time is up, every
/// second; after that, every two seconds until it is back, to report the path.
pub(in crate::connection::machine) enum WakeLook {
    Ignore,
    ReadRouter,
    LookAgain {
        at: SystemTime,
    },
    /// Judged without a network; what survived does not matter then.
    JudgeWithoutNetwork,
    /// Back after the wake was judged: the path is reported.
    ReportPath,
    /// Back: the probe observes what survived.
    Observe(ProbePlan),
}

/// A look at the route to the gateway, and at the session when asked.
pub(in crate::connection::machine) struct ProbePlan {
    pub(super) host: String,
    pub(super) gateway: Option<GatewayLookup>,
    pub(super) session_running: bool,
}

/// A report of the network path.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) struct PathReport {
    pub(super) settle_at: SystemTime,
    /// Back while a wake is waiting for it: look now rather than at the next poll.
    pub(super) look_now: bool,
}

/// The path once it stayed unchanged long enough.
pub(in crate::connection::machine) enum PathLook {
    Probe(ProbePlan),
    Unsatisfied,
}

/// What a probe's answer was for.
pub(in crate::connection::machine) enum ProbeAnswer {
    Ignore,
    Path {
        gateway_ip: Option<(String, Ipv4Addr)>,
        snapshot: PathSnapshot,
        routing_incomplete: bool,
    },
    Wake {
        gateway_ip: Option<(String, Ipv4Addr)>,
        observation: WakeObservation,
    },
}

/// What the observation after a wake leads to.
pub(in crate::connection::machine) struct WakeSettled {
    pub(super) snapshot: PathSnapshot,
    pub(super) then: AfterWake,
}

pub(in crate::connection::machine) enum AfterWake {
    Nothing,
    Session(WakeEvaluation),
    Resume(Resume),
}

/// The rules for a tunnel that was up across the sleep.
pub(in crate::connection::machine) enum WakeEvaluation {
    /// Kept as it is, or with its planned reconnect or route updated; its proactive timer is set
    /// again.
    Keep(Session),
    Restart {
        session: Session,
        restart: Restart,
    },
    Renew(Session),
}

/// What follows the end of a wait for the network.
#[derive(Clone, Copy)]
pub(in crate::connection::machine) enum Resume {
    Wait {
        until: SystemTime,
        reason: RetryReason,
    },
    Attempt(AttemptReason),
}

/// What a judged change of the network path does to the connection.
pub(in crate::connection::machine) enum AfterPath {
    Nothing,
    Resume(Resume),
    /// The connected session's route changed.
    RestartSession(Restart),
    /// The route of the session an attempt is to replace changed.
    AbortReplacement,
    /// The route is the session's: it is taken when the session had none, and a connected
    /// session whose routing is incomplete restarts.
    SameRoute {
        adopt: Option<Underlay>,
        restart: Option<Restart>,
    },
}

impl ConnectionManager {
    pub(in crate::connection::machine) fn sleep(&self, now: SystemTime) -> Sleep {
        match &self.phase {
            Phase::Connecting(attempt) if attempt.replacing.is_some() => Sleep::PauseReplacement {
                stop_run: attempt.stage == Stage::Run && attempt.stop_requested.is_none(),
                otp_close: attempt.otp_close(false, false, now),
                next_try: now,
            },
            Phase::Connecting(_) => Sleep::AbortAttempt,
            // Already waiting for the user, who is away while the Mac sleeps.
            Phase::WaitingRetry {
                reason: RetryReason::UserAway,
                ..
            } => Sleep::Keep,
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } => Sleep::WaitForWake,
            Phase::Disconnecting(stop) if matches!(stop.after, AfterStop::Reconnect(_)) => {
                Sleep::ResumeAfterWake
            }
            _ => Sleep::Keep,
        }
    }

    pub(in crate::connection::machine) fn wake(&self, now: SystemTime) -> Wake {
        Wake {
            at: now,
            slept_for: self
                .system
                .sleep_reported_at
                .map_or(Duration::ZERO, |at| elapsed(at, now)),
            limit: self.away_limit(now),
            settle: WakeSettle {
                deadline: now + policy::WAKE_SETTLE_TIMEOUT,
                late: false,
                step: SettleStep::Waiting(now),
            },
        }
    }

    /// The network is back when the path is satisfied and the primary network has a router.
    pub(in crate::connection::machine) fn wake_look(&self, now: SystemTime) -> WakeLook {
        if self.system.wake_settle.is_none() {
            return WakeLook::Ignore;
        }
        if self.path_satisfied() {
            WakeLook::ReadRouter
        } else {
            self.wake_network_missing(now)
        }
    }

    pub(in crate::connection::machine) fn router_answer(
        &self,
        present: bool,
        now: SystemTime,
    ) -> WakeLook {
        let Some(settle) = &self.system.wake_settle else {
            return WakeLook::Ignore;
        };
        if settle.step != SettleStep::ReadingRouter {
            return WakeLook::Ignore;
        }
        if !present || !self.path_satisfied() {
            return self.wake_network_missing(now);
        }
        if settle.late {
            WakeLook::ReportPath
        } else {
            WakeLook::Observe(self.probe_plan(self.run_active()))
        }
    }

    fn wake_network_missing(&self, now: SystemTime) -> WakeLook {
        let settle = self
            .system
            .wake_settle
            .as_ref()
            .expect("a wake waits for the network");
        if settle.late {
            WakeLook::LookAgain {
                at: now + policy::LATE_SETTLE_POLL,
            }
        } else if now < settle.deadline {
            WakeLook::LookAgain {
                at: now + policy::WAKE_SETTLE_POLL,
            }
        } else {
            WakeLook::JudgeWithoutNetwork
        }
    }

    fn path_satisfied(&self) -> bool {
        self.system
            .raw_path
            .as_ref()
            .is_some_and(|path| path.satisfied)
    }

    pub(in crate::connection::machine) fn path_report(
        &self,
        satisfied: bool,
        now: SystemTime,
    ) -> PathReport {
        PathReport {
            settle_at: now + policy::PATH_SETTLE,
            look_now: satisfied
                && self.system.wake_settle.as_ref().is_some_and(|settle| {
                    !settle.late && matches!(settle.step, SettleStep::Waiting(_))
                }),
        }
    }

    pub(in crate::connection::machine) fn path_look(&self) -> PathLook {
        if self.path_satisfied() {
            PathLook::Probe(self.probe_plan(matches!(self.phase, Phase::Connected(_))))
        } else {
            PathLook::Unsatisfied
        }
    }

    /// Asks for the route to the gateway of the server address set now.
    fn probe_plan(&self, session_running: bool) -> ProbePlan {
        let host = self.settings.account.host.clone();
        let gateway = (!host.is_empty()).then(|| GatewayLookup {
            host: host.clone(),
            known_ip: self
                .system
                .gateway_ip
                .as_ref()
                .filter(|(resolved, _)| *resolved == host)
                .map(|(_, ip)| *ip),
        });
        ProbePlan {
            host,
            gateway,
            session_running,
        }
    }

    pub(in crate::connection::machine) fn probe_answer(&self, result: &ProbeResult) -> ProbeAnswer {
        let for_path = self
            .system
            .path_probe
            .as_ref()
            .filter(|(id, _)| *id == result.id)
            .map(|(_, host)| host.clone());
        let for_wake = match &self.system.wake_settle {
            Some(WakeSettle {
                step: SettleStep::Observing(id, host),
                ..
            }) if *id == result.id => Some(host.clone()),
            _ => None,
        };
        let Some(host) = for_path.clone().or(for_wake) else {
            return ProbeAnswer::Ignore;
        };
        let gateway_ip = result
            .gateway_ip
            .filter(|_| host == self.settings.account.host)
            .map(|ip| (host, ip));
        let underlay = self.underlay(result);
        let routing_incomplete = result.routing_complete == Some(false);
        if for_path.is_some() {
            ProbeAnswer::Path {
                gateway_ip,
                snapshot: PathSnapshot {
                    satisfied: true,
                    underlay,
                },
                routing_incomplete,
            }
        } else {
            ProbeAnswer::Wake {
                gateway_ip,
                observation: WakeObservation::NetworkBack {
                    underlay,
                    session_running: result.session_running.unwrap_or(false),
                    tunnel_up: self.tunnel_interface_up(result.addresses.as_deref()),
                    routing_incomplete,
                },
            }
        }
    }

    /// The route interface, unless it is the tunnel's own or one the path reports as a tunnel or
    /// other virtual interface. The tunnel interface is the one holding the tunnel IP the Helper
    /// reported.
    fn underlay(&self, result: &ProbeResult) -> Option<Underlay> {
        let name = result.route_interface.as_ref()?;
        let addresses = result.addresses.as_ref()?;
        let tunnel_ip = self.tunnel_ip();
        let carries_tunnel = addresses
            .iter()
            .any(|address| address.name == *name && Some(address.ipv4) == tunnel_ip);
        let virtual_interface = self.system.raw_path.as_ref().is_some_and(|path| {
            path.interfaces
                .iter()
                .any(|interface| interface.name == *name && interface.kind == InterfaceKind::Other)
        });
        if carries_tunnel || virtual_interface {
            return None;
        }
        Some(Underlay {
            interface: name.clone(),
            address: addresses
                .iter()
                .find(|address| address.name == *name)
                .map(|address| address.ipv4),
        })
    }

    /// Without a reported tunnel IP the interface cannot be told, so it counts as there: a wrong
    /// "gone" would replace a working tunnel after every wake.
    fn tunnel_interface_up(&self, addresses: Option<&[InterfaceAddress]>) -> bool {
        self.tunnel_ip().is_none_or(|ip| {
            addresses.is_none_or(|addresses| {
                addresses
                    .iter()
                    .any(|address| address.ipv4 == ip && address.up)
            })
        })
    }

    fn tunnel_ip(&self) -> Option<Ipv4Addr> {
        match &self.phase {
            Phase::Connected(session) => session.tunnel_ip,
            Phase::Connecting(attempt) => attempt
                .replacing
                .as_ref()
                .and_then(|session| session.tunnel_ip),
            _ => None,
        }
    }

    pub(in crate::connection::machine) fn wake_settled(
        &self,
        observation: &WakeObservation,
        now: SystemTime,
    ) -> WakeSettled {
        let slept_for = self.pending_wake.unwrap_or(Duration::ZERO);
        let snapshot = match observation {
            WakeObservation::NoNetwork => PathSnapshot {
                satisfied: false,
                underlay: None,
            },
            WakeObservation::NetworkBack { underlay, .. } => PathSnapshot {
                satisfied: true,
                underlay: underlay.clone(),
            },
        };
        let then = match &self.phase {
            // The wake rules judge a tunnel that was up across the sleep. One that came up after
            // the wake, when the path returned before the wake was judged, is left alone: the
            // observation may predate it. Ending the pending wake re-arms its proactive timer.
            Phase::Connected(session)
                if self.last_wake_at.is_none_or(|wake| session.since < wake) =>
            {
                AfterWake::Session(self.wake_evaluation(
                    session.clone(),
                    observation,
                    slept_for,
                    now,
                ))
            }
            Phase::WaitingNetwork {
                reason,
                delay_after_path,
            } if matches!(observation, WakeObservation::NetworkBack { .. })
                && !self.system.sleeping =>
            {
                AfterWake::Resume(resume(*reason, *delay_after_path, now))
            }
            _ => AfterWake::Nothing,
        };
        WakeSettled { snapshot, then }
    }

    /// The rules for a tunnel that was up across the sleep, in order. While automatic reconnects
    /// are held by the schedule, a working tunnel is kept rather than replaced; a dead or
    /// stranded one is still stopped, and the reconnect after it waits or is given up.
    fn wake_evaluation(
        &self,
        mut session: Session,
        observation: &WakeObservation,
        slept_for: Duration,
        now: SystemTime,
    ) -> WakeEvaluation {
        let may_reconnect = self.automatic_reconnect_allowed(now);
        let WakeObservation::NetworkBack {
            underlay,
            session_running,
            tunnel_up,
            routing_incomplete,
        } = observation
        else {
            // The network did not come back: keep the tunnel and judge it later.
            let earliest = now + policy::MINIMUM_PLANNED_LEAD;
            session.planned_reconnect = session.planned_reconnect.map(|at| at.max(earliest));
            return WakeEvaluation::Keep(session);
        };
        let min_sleep =
            Duration::from_secs(u64::from(self.settings.connection.min_sleep_minutes) * 60);
        let planned_passed = may_reconnect
            && session
                .planned_reconnect
                .is_some_and(|planned| planned <= now);
        let reason = if !session_running || !tunnel_up || *routing_incomplete {
            Some(AttemptReason::TunnelLost)
        } else if let (Some(old), Some(new)) = (&session.underlay, underlay)
            && old != new
        {
            Some(AttemptReason::NetworkChanged)
        } else if may_reconnect
            && self.settings.connection.reconnect_after_sleep
            && slept_for >= min_sleep
        {
            Some(AttemptReason::AfterSleep)
        } else {
            None
        };
        match (planned_passed, reason) {
            (true, Some(_)) => WakeEvaluation::Restart {
                session,
                restart: restart_for(AttemptReason::Proactive),
            },
            (false, Some(reason)) => WakeEvaluation::Restart {
                session,
                restart: restart_for(reason),
            },
            (true, None) if session.renewal.is_none() => WakeEvaluation::Renew(session),
            (_, None) => {
                if session.underlay.is_none() {
                    session.underlay.clone_from(underlay);
                }
                WakeEvaluation::Keep(session)
            }
        }
    }

    pub(in crate::connection::machine) fn after_path(
        &self,
        snapshot: &PathSnapshot,
        routing_incomplete: bool,
        now: SystemTime,
    ) -> AfterPath {
        if self.system.sleeping {
            return AfterPath::Nothing;
        }
        let (session, connected) = match &self.phase {
            Phase::WaitingNetwork {
                reason,
                delay_after_path,
            } if snapshot.satisfied => {
                return AfterPath::Resume(resume(*reason, *delay_after_path, now));
            }
            // While a wake is being judged, its observation decides instead.
            Phase::Connected(session) if self.pending_wake.is_none() => (session, true),
            Phase::Connecting(attempt) => match &attempt.replacing {
                Some(session) => (session, false),
                None => return AfterPath::Nothing,
            },
            // An attempt under way or a stop ignores the path; the other phases act on the next
            // attempt.
            _ => return AfterPath::Nothing,
        };
        let Some(underlay) = snapshot.underlay.clone().filter(|_| snapshot.satisfied) else {
            return AfterPath::Nothing;
        };
        match &session.underlay {
            Some(old) if *old != underlay => {
                if connected {
                    AfterPath::RestartSession(restart_for(AttemptReason::NetworkChanged))
                } else {
                    AfterPath::AbortReplacement
                }
            }
            known => AfterPath::SameRoute {
                adopt: known.is_none().then_some(underlay),
                restart: (routing_incomplete && connected)
                    .then(|| restart_for(AttemptReason::TunnelLost)),
            },
        }
    }
}

/// What follows the end of a wait for the network: the wait after the path is back, or the
/// attempt.
pub(in crate::connection::machine) fn resume(
    reason: RetryReason,
    delay: Duration,
    now: SystemTime,
) -> Resume {
    if delay > Duration::ZERO {
        Resume::Wait {
            until: now + delay,
            reason,
        }
    } else if reason == RetryReason::Sleep {
        Resume::Attempt(AttemptReason::AfterSleep)
    } else {
        Resume::Attempt(AttemptReason::Retry(reason))
    }
}
