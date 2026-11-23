//! The connection state machine: what the user wants, where the connection is, and how each input
//! moves it.
//!
//! Timers, the activity hold and saving the ledger follow from the state after each input, so no
//! transition can forget to cancel a timer or save the ledger.

mod attempts {
    pub(super) mod action;
    pub(super) mod decision;
}
mod finish {
    pub(super) mod action;
    pub(super) mod decision;
}
mod helper {
    pub(super) mod action;
    pub(super) mod decision;
}
mod ledger {
    pub(super) mod action;
    pub(super) mod decision;
}
mod otp {
    pub(super) mod action;
    pub(super) mod decision;
}
mod outcomes {
    pub(super) mod action;
    pub(super) mod decision;
}
mod presence {
    pub(super) mod action;
    pub(super) mod decision;
}
mod readiness {
    pub(super) mod action;
    pub(super) mod decision;
}
mod requests {
    pub(super) mod action;
    pub(super) mod decision;
}
mod system {
    pub(super) mod action;
    pub(super) mod decision;
}
mod timers {
    pub(super) mod action;
    pub(super) mod decision;
}

use std::collections::BTreeMap;
use std::mem;
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

use fvm_contract::app_helper::{
    AttemptId, HelperEvent, HelperEventKind, Milestone, SessionId, StopReason,
};
use jiff::tz::TimeZone;

use super::classify::Outcome;
use super::ledger::{Failure, FailureLedger};
use super::readiness::{Readiness, SetupNeed};
use super::system::{PathSnapshot, SystemObservation, Underlay, WakeObservation};
use super::view::Shown;
use super::{Effect, Input, Note, OtpFailure, Settings, Startup, TimerKind, VpnAccount};

use attempts::decision::{attempt_abort, replacement_failure};
use presence::decision::next_lock_poll;
use readiness::decision::{follow_approval, registration};

const SECONDS_FROM_1970_TO_2026: u64 = 1_767_225_600;

pub struct ConnectionManager {
    pub(super) app_build: String,
    attempt_seed: [u8; 16],
    attempts_issued: u64,
    last_session: Option<SessionId>,
    pub(super) settings: Settings,
    pub(super) time_zone: TimeZone,
    /// The effects of the input being handled.
    pub(super) effects: Vec<Effect>,

    /// The user, or the schedule acting for them, wants a connection.
    pub(super) wants_connection: bool,
    pub(super) phase: Phase,
    pub(super) ledger: FailureLedger,
    saved_ledger: FailureLedger,
    /// The network path as last judged.
    pub(super) path: Option<PathSnapshot>,
    last_wake_at: Option<SystemTime>,
    /// A wake not judged yet, with how long the system slept.
    pending_wake: Option<Duration>,
    quit: Quit,
    pub(super) update_stop: Option<UpdateStop>,
    /// Since when the user has been away; `None` while present.
    pub(super) away_since: Option<SystemTime>,
    pub(super) reconnect_limits: ReconnectLimits,
    /// The latest schedule start or end acted on.
    schedule_boundary: Option<SystemTime>,
    /// The attempt whose failure the phase shows, for a late spam-folder finding.
    last_failure_attempt: Option<AttemptId>,
    /// The code email the next login waits for.
    stale_otp: Option<StaleOtp>,
    /// The account of the last run started: the run whose events are taken now. It differs from
    /// the settings' account when that changed during the run.
    pub(super) run_account: Option<VpnAccount>,
    timers: BTreeMap<TimerKind, TimerSlot>,
    next_timer_generation: u64,
    /// The proactive timer is set again at the end of the input even at an unchanged time.
    proactive_timer_rearm: bool,
    activity_held: bool,
    pub(super) system: SystemObservation,
    pub(super) readiness: Readiness,
    /// What the sections show of time-bound rules, as judged at the end of the last input.
    pub(super) shown: Shown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Quit {
    NotRequested,
    Requested { at: SystemTime },
    Terminated,
}

#[derive(Default)]
pub(super) struct ReconnectLimits {
    /// The away limit was reached: no automatic reconnect until Connect, Reconnect Now or the next
    /// scheduled start.
    pub(super) reconnect_suspended: bool,
    network_failure_streak: u32,
    /// The immediate reconnect after a forced termination was used and no tunnel has come up since.
    forced_retry_used: bool,
}

/// Where the connection is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Phase {
    Idle,
    SetupRequired(Vec<SetupNeed>),
    Connecting(Box<Attempt>),
    Connected(Session),
    WaitingRetry {
        until: SystemTime,
        reason: RetryReason,
    },
    /// Waiting for the network path, or for the wake, before the next attempt.
    WaitingNetwork {
        reason: RetryReason,
        /// The wait once the path is back; zero attempts at once.
        delay_after_path: Duration,
    },
    Disconnecting(Stop),
    Failed(Failure),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Stage {
    CheckingReadiness,
    /// Readiness holds since `since`; an unused code email of an earlier attempt is not in the
    /// trash yet.
    WaitingForStaleOtp {
        since: SystemTime,
    },
    /// The OTP session takes its starting point.
    PreparingOtp,
    /// The run was asked for; its progress is the milestone.
    Run,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Attempt {
    pub(super) id: AttemptId,
    session: SessionId,
    pub(super) reason: AttemptReason,
    cert_retry_used: bool,
    /// The attempt's time limit; the wait for an earlier attempt's code email does not count.
    deadline: SystemTime,
    pub(super) stage: Stage,
    pub(super) milestone: Option<Milestone>,
    /// When the credentials went out; the gateway sends the code email once it accepts them.
    credentials_sent_at: Option<SystemTime>,
    otp_prompted_at: Option<SystemTime>,
    /// Open while the OTP session is this attempt's; it closes itself after the submission.
    otp_session: OtpSession,
    otp_failure: Option<OtpFailure>,
    otp_in_spam: bool,
    /// The stop the app asked for; the exit is still classified.
    stop_requested: Option<StopReason>,
    /// The account when the attempt began: its readiness check was for it and its run uses it.
    pub(super) account: VpnAccount,
    pub(super) replacing: Option<Session>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OtpSession {
    Closed,
    Open,
    Submitted,
}

/// The code email the gateway may have sent for an attempt that ended with no code submitted.
#[derive(Clone, PartialEq, Eq, Debug)]
struct StaleOtp {
    attempt: AttemptId,
    /// When the next login stops waiting for it.
    until: SystemTime,
}

/// A tunnel that is up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Session {
    pub(super) attempt: AttemptId,
    pub(super) since: SystemTime,
    pub(super) tunnel_ip: Option<Ipv4Addr>,
    /// `None` while proactive reconnects are off.
    pub(super) planned_reconnect: Option<SystemTime>,
    /// The route to the gateway when the tunnel came up, or the first one known after.
    underlay: Option<Underlay>,
    pub(super) renewal: Option<Renewal>,
    pub(super) replaced: Option<AttemptId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct Renewal {
    pub(super) reason: AttemptReason,
    pub(super) tries: u32,
    pub(super) later_failures: u32,
    pub(super) next_try: Option<SystemTime>,
}

/// A stop the app asked for; it is complete at the run's exit event, not at the reply.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Stop {
    attempt: AttemptId,
    pub(super) after: AfterStop,
    replacement: Option<AttemptId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum AfterStop {
    Idle,
    Reconnect(AttemptReason),
    ResumeAfterSleep,
    Quit,
    Update,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct UpdateStop {
    requested_at: SystemTime,
    stopped: bool,
}

/// Why the connection waits before the next attempt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RetryReason {
    Network,
    NetworkSetupFailed,
    ServerSessionEnded,
    ForcedTermination,
    /// One authentication failure was counted; one more try follows the cooldown.
    AuthFailure(Failure),
    /// An attempt waits out the cooldown after the last authentication failure.
    AuthCooldown,
    /// An attempt the sleep stopped resumes after the wake.
    Sleep,
    /// Outside the scheduled hours, the reconnect waits for the away user.
    UserAway,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum AttemptReason {
    Launch,
    User,
    ReconnectNow,
    SetupCompleted,
    CertificateRetry,
    Retry(RetryReason),
    Proactive,
    NetworkChanged,
    TunnelLost,
    AfterSleep,
    Scheduled,
}

impl AttemptReason {
    /// The user's own attempts are not held to the attempt limit.
    fn is_user_action(self) -> bool {
        matches!(self, Self::User | Self::ReconnectNow)
    }

    /// Reconnects the app starts on its own; outside the scheduled hours they wait for, or give
    /// up on, an away user. The launch and the scheduled start are bound by the schedule itself,
    /// and a certificate retry continues an attempt already running.
    fn is_automatic_reconnect(self) -> bool {
        match self {
            Self::Retry(_)
            | Self::Proactive
            | Self::NetworkChanged
            | Self::TunnelLost
            | Self::AfterSleep
            | Self::SetupCompleted => true,
            Self::Launch
            | Self::User
            | Self::ReconnectNow
            | Self::CertificateRetry
            | Self::Scheduled => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct TimerSlot {
    at: SystemTime,
    generation: u64,
    fired: bool,
}

/// One transition of an input: judged, then carried out, which may lead to further steps.
pub(super) enum Step {
    Input(Input),
    TimerDue(TimerKind),
    BeginAttempt {
        reason: AttemptReason,
        cert_retry_used: bool,
        readiness_verified: bool,
    },
    PrepareReplacement {
        session: Session,
        cert_retry_used: bool,
    },
    ReplacementEnded {
        session: Session,
        outcome: Outcome,
    },
    ReplacementFailed(Session),
    AbortAttempt {
        attempt: Box<Attempt>,
        stop: StopReason,
        after: AfterStop,
    },
    ApplyOutcome(Outcome),
    ForcedTermination,
    NetworkFailure(RetryReason),
    AuthFailure(Failure),
    ScheduleRetry {
        reason: RetryReason,
        delay: Duration,
    },
    Fail(Failure),
    EnterConnected {
        tunnel_ip: Option<Ipv4Addr>,
    },
    StaleOtpWaitOver(Note),
    PrepareOtp,
    CheckReadiness,
    ConnectRequested(AttemptReason),
    StoppedForUpdate,
    /// A report of the screen, the displays or the sleep, as of `at` or else the input's time.
    UpdatePresence(Option<SystemTime>),
    PresenceChanged {
        away: bool,
        at: SystemTime,
    },
    RejudgeHeldReconnect,
    ScheduleChanged,
    ResetScheduleBoundary,
    WillSleep,
    WakeLookDue,
    WakeSettled(WakeObservation),
    ReportPath,
    PathChanged {
        snapshot: PathSnapshot,
        routing_incomplete: bool,
    },
}

impl ConnectionManager {
    #[must_use]
    pub fn new(startup: Startup) -> Self {
        let ledger = FailureLedger::new(startup.settings.account.ledger_key());
        let mut manager = Self {
            app_build: startup.app_build,
            attempt_seed: startup.attempt_seed,
            attempts_issued: 0,
            last_session: None,
            settings: startup.settings,
            time_zone: startup.time_zone,
            effects: Vec::new(),
            wants_connection: false,
            phase: Phase::Idle,
            saved_ledger: ledger.clone(),
            ledger,
            path: None,
            last_wake_at: None,
            pending_wake: None,
            quit: Quit::NotRequested,
            update_stop: None,
            away_since: None,
            reconnect_limits: ReconnectLimits::default(),
            schedule_boundary: None,
            last_failure_attempt: None,
            stale_otp: None,
            run_account: None,
            timers: BTreeMap::new(),
            next_timer_generation: 0,
            proactive_timer_rearm: false,
            activity_held: false,
            system: SystemObservation::default(),
            readiness: Readiness::default(),
            shown: Shown::default(),
        };
        manager.shown = manager.judge_shown(SystemTime::UNIX_EPOCH);
        manager
    }

    /// Handles one input at the time it is taken and returns what to carry out, in order.
    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        self.handle_at(input, SystemTime::now())
    }

    /// Handles one input taken at `now`; every judgement of the input uses that time.
    pub(super) fn handle_at(&mut self, input: Input, now: SystemTime) -> Vec<Effect> {
        if !self.takes_inputs() {
            return Vec::new();
        }
        let now = self.input_time(&input, now);
        let marks = self.note_marks();
        let mut steps = vec![Step::Input(input)];
        while let Some(step) = steps.pop() {
            let next = self.run_step(step, now);
            steps.extend(next.into_iter().rev());
        }
        let finish = self.finish_plan(&marks, now);
        self.finish(finish)
    }

    /// Judges one step at `now`, then carries it out; returns the steps that follow, in order.
    fn run_step(&mut self, step: Step, now: SystemTime) -> Vec<Step> {
        match step {
            Step::Input(input) => self.run_input(input, now),
            Step::TimerDue(kind) => self.run_timer(kind, now),
            Step::BeginAttempt {
                reason,
                cert_retry_used,
                readiness_verified,
            } => self.start_attempt(self.attempt_start(
                reason,
                cert_retry_used,
                readiness_verified,
                now,
            )),
            Step::PrepareReplacement {
                session,
                cert_retry_used,
            } => self.start_replacement(self.replacement_start(session, cert_retry_used, now)),
            Step::ReplacementEnded { session, outcome } => {
                self.end_replacement(self.replacement_end(session, outcome, now))
            }
            Step::ReplacementFailed(session) => {
                self.fail_replacement(replacement_failure(session, now))
            }
            Step::AbortAttempt {
                attempt,
                stop,
                after,
            } => self.abort(attempt_abort(*attempt, stop, after, now)),
            Step::ApplyOutcome(outcome) => self.outcome_steps(outcome),
            Step::ForcedTermination => self.forced_termination(self.forced_retry_left()),
            Step::NetworkFailure(reason) => {
                self.count_network_failure(self.network_failure(reason, now))
            }
            Step::AuthFailure(failure) => self.count_auth_failure(self.auth_failure(failure, now)),
            Step::ScheduleRetry { reason, delay } => {
                self.wait_to_retry(self.retry_wait(reason, delay, now));
                Vec::new()
            }
            Step::Fail(failure) => {
                self.fail(failure);
                Vec::new()
            }
            Step::EnterConnected { tunnel_ip } => {
                self.enter_connected(self.connected(tunnel_ip, now))
            }
            Step::StaleOtpWaitOver(note) => {
                self.go_on_after_stale_otp(self.stale_otp_wait_over(now), note)
            }
            Step::PrepareOtp => self.prepare_otp(self.otp_preparation(now)),
            Step::CheckReadiness => self.ask_readiness(self.readiness_check_asked()),
            Step::ConnectRequested(reason) => self.connect(self.connect_request(), reason),
            Step::StoppedForUpdate => self.report_stopped_for_update(self.update_stop_complete()),
            Step::UpdatePresence(at) => self.update_presence(self.presence_update(at, now)),
            Step::PresenceChanged { away, at } => {
                self.change_presence(self.presence_change(away, at, now))
            }
            Step::RejudgeHeldReconnect => self.held_reconnect(),
            Step::ScheduleChanged => self.change_schedule(self.schedule_change(now)),
            Step::ResetScheduleBoundary => {
                self.reset_schedule_boundary(self.schedule_boundary_at(now))
            }
            Step::WillSleep => self.go_to_sleep(self.sleep(now)),
            Step::WakeLookDue => self.look_after_wake(self.wake_look(now)),
            Step::WakeSettled(observation) => {
                self.settle_wake(self.wake_settled(&observation, now))
            }
            Step::ReportPath => self.look_at_path(self.path_look()),
            Step::PathChanged {
                snapshot,
                routing_incomplete,
            } => {
                let after = self.after_path(&snapshot, routing_incomplete, now);
                self.change_path(snapshot, after)
            }
        }
    }

    fn run_input(&mut self, input: Input, now: SystemTime) -> Vec<Step> {
        match input {
            Input::Launched(launch) => self.launch(self.launched(launch, now)),
            Input::SettingsChanged(settings) => {
                self.change_settings(self.settings_change(settings, now))
            }
            Input::TimeZoneChanged(time_zone) => self.change_time_zone(time_zone),
            Input::Connect => self.user_connect(),
            Input::Disconnect => self.disconnect(self.disconnect_request()),
            Input::ReconnectNow => self.reconnect_now(self.reconnect_now_request()),
            Input::QuitRequested => self.quit(self.quit_request(now)),
            Input::StopForUpdate => self.stop_for_update(self.update_stop_request(now)),
            Input::UpdateFailed { reconnect } => self.fail_update(self.update_failure(), reconnect),
            Input::SettingsOpened | Input::RecheckReadiness => vec![Step::CheckReadiness],
            Input::AppActivated => self.app_activated(),
            Input::HelperRegistered(result) => self.register(registration(result, now)),
            Input::HelperApprovalSettingsOpened => self.follow_approval(follow_approval(now)),
            Input::ReadinessChecked(facts) => self.take_readiness(self.readiness_checked(facts)),
            Input::OtpSessionReady(attempt) => self.start_run(self.otp_session_ready(&attempt)),
            Input::OtpSessionFailed(attempt, failure) => {
                self.end_otp_session(self.otp_session_failure(&attempt, failure))
            }
            Input::OtpCodeFound(attempt, code) => {
                self.submit_otp(self.otp_code_usable(&attempt), code)
            }
            Input::OtpFoundInSpam(attempt) => self.note_spam(self.spam_finding(&attempt)),
            Input::StaleOtpTrashed(attempt) => {
                self.end_stale_otp_wait(self.stale_otp_session(&attempt))
            }
            Input::Helper(HelperEvent { attempt_id, kind }) => match kind {
                HelperEventKind::Milestone {
                    milestone,
                    tunnel_ip,
                } => self.reach_milestone(self.milestone_reached(
                    &attempt_id,
                    milestone,
                    tunnel_ip,
                    now,
                )),
                HelperEventKind::Exited { facts } => {
                    self.end_run(self.run_exit(&attempt_id, &facts, now))
                }
            },
            Input::RunRejected(attempt, code) => {
                self.reject_run(self.run_rejection(&attempt, code, now))
            }
            Input::HelperConnectionLost => self.lose_connection(self.connection_loss(now)),
            Input::TimerFired(timer) => self.fire_timer(self.due_timer(timer)),
            Input::WillSleep { at } => self.sleep_reported(at),
            Input::DidWake => self.wake_up(self.wake(now)),
            Input::ScreenLocked { at } => self.screen_reported(Some(true), None, Some(at)),
            Input::ScreenUnlocked => self.screen_reported(Some(false), None, None),
            Input::DisplaysSlept { at } => self.screen_reported(None, Some(true), Some(at)),
            Input::DisplaysWoke => self.displays_woke(),
            Input::ScreenState {
                locked,
                displays_asleep,
            } => self.screen_reported(Some(locked), Some(displays_asleep), None),
            Input::NetworkPathChanged {
                satisfied,
                interfaces,
            } => self.record_path(satisfied, interfaces, self.path_report(satisfied, now)),
            Input::RouterRead { present } => self.look_after_wake(self.router_answer(present, now)),
            Input::NetworkProbed(result) => self.take_probe_answer(self.probe_answer(&result)),
        }
    }

    fn run_timer(&mut self, kind: TimerKind, now: SystemTime) -> Vec<Step> {
        match kind {
            TimerKind::AttemptDeadline => self.end_attempt_at_deadline(self.attempt_deadline(now)),
            TimerKind::OtpDeadline => self.stop_at_otp_deadline(self.otp_deadline(now)),
            TimerKind::StaleOtp => self.stop_stale_otp_search(),
            TimerKind::Retry => self.start_due_retry(self.retry_due()),
            TimerKind::ProactiveReconnect => self.renew_when_due(self.proactive_reconnect_due(now)),
            TimerKind::QuitDeadline => self.end_at_quit_deadline(self.quit_deadline_ends_quit()),
            TimerKind::Schedule => self.pass_boundary(self.boundary_due(now)),
            TimerKind::AwayLimit => {
                self.apply_away_limit(self.away_limit(now));
                Vec::new()
            }
            TimerKind::PathSettle => self.path_settled(),
            TimerKind::WakeSettle => vec![Step::WakeLookDue],
            TimerKind::LockPoll => self.poll_lock(next_lock_poll(now)),
            TimerKind::ApprovalFollow => self.approval_follow_due(self.approval_follow(now)),
        }
    }

    fn take_phase(&mut self) -> Phase {
        mem::replace(&mut self.phase, Phase::Idle)
    }
}

impl Phase {
    /// The app keeps itself from being suspended while work is under way or a timer must fire
    /// on time.
    fn holds_activity(&self) -> bool {
        match self {
            Self::Connecting(_)
            | Self::Connected(_)
            | Self::WaitingRetry { .. }
            | Self::WaitingNetwork { .. }
            | Self::Disconnecting(_) => true,
            Self::Idle | Self::SetupRequired(_) | Self::Failed(_) => false,
        }
    }

    /// Waiting for an away user outside the scheduled hours.
    pub(super) fn is_holding(&self) -> bool {
        matches!(
            self,
            Self::WaitingRetry {
                reason: RetryReason::UserAway,
                ..
            }
        )
    }
}
