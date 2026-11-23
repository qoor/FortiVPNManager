//! The connection state machine: what the user wants, where the connection is, and how each input
//! moves it.
//!
//! Timers, the activity hold and saving the ledger follow from the state after each input, so no
//! transition can forget to cancel a timer or save the ledger.

mod attempts;
mod finish {
    pub(super) mod action;
    pub(super) mod decision;
}
mod helper;
mod ledger {
    pub(super) mod action;
    pub(super) mod decision;
}
mod otp;
mod outcomes;
mod presence;
mod timers {
    pub(super) mod action;
    pub(in crate::connection::machine) mod decision;
}
mod wake;

use std::collections::BTreeMap;
use std::mem;
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

use fvm_contract::app_helper::{AttemptId, Milestone, SessionId, StopReason};
use jiff::tz::TimeZone;

use super::classify::Outcome;
use super::ledger::{Failure, FailureLedger};
use super::policy;
use super::readiness::{Readiness, SetupNeed};
use super::system::{PathSnapshot, SystemObservation, Underlay};
use super::view::Shown;
use super::{
    ConnectionSettings, Effect, Input, Launch, OtpFailure, Settings, Startup, TimerKind, VpnAccount,
};

const SECONDS_FROM_1970_TO_2026: u64 = 1_767_225_600;

pub struct ConnectionManager {
    pub(super) app_build: String,
    attempt_seed: [u8; 16],
    attempts_issued: u64,
    last_session: Option<SessionId>,
    pub(super) settings: Settings,
    pub(super) time_zone: TimeZone,
    /// The time of the input being handled, or of the last one.
    pub(super) now: SystemTime,
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
    PrepareOtp,
    CheckReadiness,
    StoppedForUpdate,
}

impl Step {
    fn begin_attempt(reason: AttemptReason) -> Self {
        Self::BeginAttempt {
            reason,
            cert_retry_used: false,
            readiness_verified: false,
        }
    }
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
            now: SystemTime::UNIX_EPOCH,
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
        // What the sections show before the first input, as of the start of the clock.
        manager.shown = manager.judge_shown(SystemTime::UNIX_EPOCH);
        manager
    }

    /// Handles one input taken at `now` and returns what to carry out, in order.
    pub fn handle(&mut self, input: Input, now: SystemTime) -> Vec<Effect> {
        if self.quit == Quit::Terminated {
            return Vec::new();
        }
        let now = self.input_time(&input, now);
        self.now = now;
        let marks = self.note_marks();
        let mut steps = vec![Step::Input(input)];
        while let Some(step) = steps.pop() {
            let next = self.run_step(step, now);
            steps.extend(next.into_iter().rev());
        }
        let finish = self.finish_plan(&marks, now);
        self.finish(finish)
    }

    /// Judges one step, then carries it out.
    fn run_step(&mut self, step: Step, now: SystemTime) -> Vec<Step> {
        match step {
            Step::Input(Input::TimerFired(timer)) => {
                let due = self.due_timer(timer);
                self.fire_timer(due)
            }
            Step::Input(input) => {
                self.dispatch(input);
                Vec::new()
            }
            Step::TimerDue(TimerKind::AttemptDeadline) => {
                let deadline = self.attempt_deadline(now);
                self.end_attempt_at_deadline(deadline)
            }
            Step::TimerDue(TimerKind::Retry) => {
                let due = self.retry_due();
                self.start_due_retry(due)
            }
            Step::TimerDue(TimerKind::ProactiveReconnect) => {
                let due = self.proactive_reconnect_due(now);
                self.renew_when_due(due)
            }
            Step::TimerDue(kind) => {
                self.timer_due(kind);
                Vec::new()
            }
            Step::BeginAttempt {
                reason,
                cert_retry_used,
                readiness_verified,
            } => {
                let start = self.attempt_start(reason, cert_retry_used, readiness_verified, now);
                self.start_attempt(start)
            }
            Step::PrepareReplacement {
                session,
                cert_retry_used,
            } => {
                let start = self.replacement_start(session, cert_retry_used, now);
                self.start_replacement(start)
            }
            Step::ReplacementEnded { session, outcome } => {
                let end = self.replacement_end(session, outcome, now);
                self.end_replacement(end)
            }
            Step::ReplacementFailed(session) => {
                let failure = attempts::decision::replacement_failure(session, now);
                self.fail_replacement(failure)
            }
            Step::AbortAttempt {
                attempt,
                stop,
                after,
            } => {
                let abort = attempts::decision::attempt_abort(*attempt, stop, after, now);
                self.abort(abort)
            }
            Step::ApplyOutcome(outcome) => self.outcome_steps(outcome),
            Step::ForcedTermination => {
                let retry_left = self.forced_retry_left();
                self.forced_termination(retry_left)
            }
            Step::NetworkFailure(reason) => {
                let failure = self.network_failure(reason, now);
                self.count_network_failure(failure);
                Vec::new()
            }
            Step::AuthFailure(failure) => {
                let failure = self.auth_failure(failure, now);
                self.count_auth_failure(failure);
                Vec::new()
            }
            Step::ScheduleRetry { reason, delay } => {
                let wait = self.retry_wait(reason, delay, now);
                self.wait_to_retry(wait);
                Vec::new()
            }
            Step::Fail(failure) => {
                self.fail(failure);
                Vec::new()
            }
            Step::PrepareOtp => {
                self.prepare_otp();
                Vec::new()
            }
            Step::CheckReadiness => {
                self.check_readiness();
                Vec::new()
            }
            Step::StoppedForUpdate => {
                self.stopped_for_update();
                Vec::new()
            }
        }
    }

    /// Runs `step` and what follows from it at once, for transitions not split yet.
    fn run_now(&mut self, step: Step) {
        self.run_all(vec![step]);
    }

    fn run_all(&mut self, steps: Vec<Step>) {
        let mut steps: Vec<Step> = steps.into_iter().rev().collect();
        while let Some(step) = steps.pop() {
            let next = self.run_step(step, self.now);
            steps.extend(next.into_iter().rev());
        }
    }

    fn dispatch(&mut self, input: Input) {
        match input {
            Input::Launched(launch) => self.launched(launch),
            Input::SettingsChanged(settings) => self.settings_changed(settings),
            Input::TimeZoneChanged(time_zone) => self.time_zone_changed(time_zone),
            Input::Connect | Input::Disconnect | Input::ReconnectNow
                if self.update_stop.is_some() => {}
            Input::Connect => self.connect_requested(AttemptReason::User),
            Input::Disconnect => self.disconnect_requested(),
            Input::ReconnectNow => self.reconnect_now_requested(),
            Input::QuitRequested => self.quit_requested(),
            Input::StopForUpdate => self.stop_for_update(),
            Input::UpdateFailed { reconnect } => self.update_failed(reconnect),
            Input::SettingsOpened | Input::RecheckReadiness => self.check_readiness(),
            Input::AppActivated => {
                self.effects.push(Effect::ReadScreenState);
                self.check_readiness();
            }
            Input::HelperRegistered(result) => self.helper_registered(result),
            Input::HelperApprovalSettingsOpened => self.follow_helper_approval(),
            Input::ReadinessChecked(facts) => self.readiness_checked(facts),
            Input::OtpSessionReady(attempt) => self.otp_session_ready(&attempt),
            Input::OtpSessionFailed(attempt, failure) => self.otp_session_failed(&attempt, failure),
            Input::OtpCodeFound(attempt, code) => self.otp_code_found(&attempt, code),
            Input::OtpFoundInSpam(attempt) => self.otp_found_in_spam(&attempt),
            Input::StaleOtpTrashed(attempt) => self.stale_otp_trashed(&attempt),
            Input::Helper(event) => self.helper_event(event),
            Input::RunRejected(attempt, code) => self.run_rejected(&attempt, code),
            Input::HelperConnectionLost => self.helper_connection_lost(),
            Input::TimerFired(_) => unreachable!("a timer input runs as its own step"),
            Input::WillSleep { at } => self.will_sleep_reported(at),
            Input::DidWake => self.did_wake_reported(),
            Input::ScreenLocked { at } => {
                self.system.screen_locked = true;
                self.update_presence(at);
            }
            Input::ScreenUnlocked => {
                self.system.screen_locked = false;
                self.update_presence(self.now);
            }
            Input::DisplaysSlept { at } => {
                self.system.displays_asleep = true;
                self.update_presence(at);
            }
            Input::DisplaysWoke => {
                // Whether the screen is locked behind the displays decides; the answer judges.
                self.system.displays_asleep = false;
                self.effects.push(Effect::ReadScreenState);
            }
            Input::ScreenState {
                locked,
                displays_asleep,
            } => {
                self.system.screen_locked = locked;
                self.system.displays_asleep = displays_asleep;
                self.update_presence(self.now);
            }
            Input::NetworkPathChanged {
                satisfied,
                interfaces,
            } => self.network_path_changed(satisfied, interfaces),
            Input::RouterRead { present } => self.router_read(present),
            Input::NetworkProbed(result) => self.network_probed(&result),
        }
    }

    fn take_phase(&mut self) -> Phase {
        mem::replace(&mut self.phase, Phase::Idle)
    }

    // MARK: Launch and settings

    fn launched(&mut self, launch: Launch) {
        if let Some(restored) = launch.restored_ledger
            && restored.account_key == self.ledger.account_key
        {
            self.saved_ledger = restored.clone();
            self.ledger = restored;
        }
        self.ledger.recent_auth_attempts = self.ledger.kept_attempts(self.now);
        self.reset_schedule_boundary();
        self.effects.push(Effect::ReadScreenState);
        if launch.import_report_shown {
            self.readiness.settings_open_decided = true;
            self.effects.push(Effect::OpenSettings);
        }

        if !launch.auto_connect || !self.within_schedule(self.now) {
            self.wants_connection = false;
            self.check_readiness();
        } else {
            self.wants_connection = true;
            if self.ledger.consecutive_auth_failures >= policy::MAX_CONSECUTIVE_AUTH_FAILURES {
                // The retries had stopped before the relaunch; only the user clears that.
                let failure = self
                    .ledger
                    .last_auth_failure
                    .unwrap_or(Failure::AuthenticationRejected);
                self.fail(failure);
            } else {
                self.begin_attempt(AttemptReason::Launch, false, false);
            }
        }
        if launch.connected_before_update {
            self.connect_requested(AttemptReason::User);
        }
    }

    fn settings_changed(&mut self, settings: Settings) {
        let old = mem::replace(&mut self.settings, settings);
        if self.settings.account != old.account {
            self.account_changed(&old.account);
        }
        if self.settings.connection != old.connection {
            self.connection_settings_changed(&old.connection);
        }
        if self.settings.otp_method != old.otp_method {
            self.check_readiness();
        }
    }

    fn account_changed(&mut self, old: &VpnAccount) {
        let account = &mut self.settings.account;
        if super::certs::gateway_change_drops_certs(old, account) {
            // The next connection learns the new gateway's certificate.
            account.trusted_certs.clear();
            self.effects.push(Effect::ClearTrustedCerts);
        }
        if account.host != old.host {
            self.system.gateway_ip = None;
        }
        let key = account.ledger_key();
        if key != self.ledger.account_key {
            self.ledger = FailureLedger::new(key);
        }
        if !self.settings.account.same_identity(old) {
            self.check_readiness();
        }
    }

    fn connection_settings_changed(&mut self, old: &ConnectionSettings) {
        let new = &self.settings.connection;
        let replans = new.session_max_minutes != old.session_max_minutes
            || new.proactive_reconnect != old.proactive_reconnect
            || new.proactive_margin_minutes != old.proactive_margin_minutes;
        if new.schedule != old.schedule {
            self.schedule_changed();
        }
        if replans && let Phase::Connected(session) = &mut self.phase {
            session.planned_reconnect =
                policy::planned_reconnect(session.since, &self.settings.connection, self.now);
            if session
                .renewal
                .is_some_and(|renewal| renewal.next_try.is_none())
            {
                session.renewal = None;
            }
        }
    }

    fn time_zone_changed(&mut self, time_zone: TimeZone) {
        self.time_zone = time_zone;
        self.reset_schedule_boundary();
        self.rejudge_held_reconnect();
    }

    // MARK: User requests

    /// Connect, or the start of the scheduled hours, which acts the same.
    fn connect_requested(&mut self, reason: AttemptReason) {
        match self.take_phase() {
            Phase::Idle | Phase::SetupRequired(_) | Phase::Failed(_) => {
                self.wants_connection = true;
                self.reset_for_user_action();
                self.begin_attempt(reason, false, false);
            }
            Phase::Disconnecting(mut stop) => {
                self.wants_connection = true;
                if stop.after == AfterStop::Idle {
                    self.reset_for_user_action();
                    stop.after = AfterStop::Reconnect(reason);
                }
                self.phase = Phase::Disconnecting(stop);
            }
            Phase::WaitingRetry {
                reason: RetryReason::UserAway,
                ..
            } => {
                self.reconnect_limits.reconnect_suspended = false;
                self.begin_attempt(reason, false, false);
            }
            phase => {
                self.reconnect_limits.reconnect_suspended = false;
                self.phase = phase;
            }
        }
    }

    fn disconnect_requested(&mut self) {
        self.wants_connection = false;
        match self.take_phase() {
            Phase::Failed(_) | Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } => {}
            Phase::Connecting(attempt) => {
                self.abort_attempt(*attempt, StopReason::UserDisconnect, AfterStop::Idle);
            }
            Phase::Connected(session) => {
                self.stop_session(session, StopReason::UserDisconnect, AfterStop::Idle, None);
            }
            Phase::Disconnecting(mut stop) => {
                if stop.after != AfterStop::Quit {
                    stop.after = AfterStop::Idle;
                }
                self.phase = Phase::Disconnecting(stop);
            }
            phase @ (Phase::Idle | Phase::SetupRequired(_)) => self.phase = phase,
        }
    }

    fn reconnect_now_requested(&mut self) {
        match self.take_phase() {
            Phase::Connected(session) => {
                self.reset_for_user_action();
                self.renew_now(session, AttemptReason::ReconnectNow);
            }
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } | Phase::Failed(_) => {
                self.wants_connection = true;
                self.reset_for_user_action();
                self.begin_attempt(AttemptReason::ReconnectNow, false, false);
            }
            Phase::Connecting(mut attempt)
                if attempt.replacing.is_some() && attempt.reason != AttemptReason::ReconnectNow =>
            {
                attempt.reason = AttemptReason::ReconnectNow;
                let renewal = attempt
                    .replacing
                    .as_mut()
                    .and_then(|session| session.renewal.as_mut())
                    .expect("a replacement is prepared for a renewal");
                renewal.reason = AttemptReason::ReconnectNow;
                renewal.tries = 0;
                renewal.later_failures = 0;
                self.reset_for_user_action();
                self.phase = Phase::Connecting(attempt);
            }
            // Restarting an attempt under way could waste an OTP or a login.
            phase => self.phase = phase,
        }
    }

    fn quit_requested(&mut self) {
        self.wants_connection = false;
        if self.quit == Quit::NotRequested {
            self.quit = Quit::Requested { at: self.now };
        }
        match self.take_phase() {
            Phase::Connecting(attempt) => {
                self.abort_attempt(*attempt, StopReason::AppQuit, AfterStop::Quit);
            }
            Phase::Connected(session) => {
                self.stop_session(session, StopReason::AppQuit, AfterStop::Quit, None);
            }
            Phase::Disconnecting(mut stop) => {
                stop.after = AfterStop::Quit;
                self.phase = Phase::Disconnecting(stop);
            }
            phase => {
                self.phase = phase;
                self.quit = Quit::Terminated;
            }
        }
    }

    fn stop_for_update(&mut self) {
        if self.quit != Quit::NotRequested {
            return;
        }
        self.wants_connection = false;
        self.update_stop = Some(UpdateStop {
            requested_at: self.now,
            stopped: false,
        });
        match self.take_phase() {
            Phase::Connecting(attempt) => {
                self.abort_attempt(*attempt, StopReason::AppQuit, AfterStop::Update);
            }
            Phase::Connected(session) => {
                self.stop_session(session, StopReason::AppQuit, AfterStop::Update, None);
            }
            Phase::Disconnecting(mut stop) => {
                stop.after = AfterStop::Update;
                self.phase = Phase::Disconnecting(stop);
            }
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } => {
                self.settle_now(AfterStop::Update);
            }
            phase @ (Phase::Idle | Phase::SetupRequired(_) | Phase::Failed(_)) => {
                self.phase = phase;
                self.stopped_for_update();
            }
        }
    }

    fn stopped_for_update(&mut self) {
        if let Some(stop) = &mut self.update_stop
            && !stop.stopped
        {
            stop.stopped = true;
            self.effects.push(Effect::StoppedForUpdate);
        }
    }

    fn update_failed(&mut self, reconnect: bool) {
        if self.quit != Quit::NotRequested {
            return;
        }
        self.update_stop = None;
        if let Phase::Disconnecting(stop) = &mut self.phase
            && stop.after == AfterStop::Update
        {
            stop.after = AfterStop::Idle;
        }
        if reconnect {
            self.connect_requested(AttemptReason::User);
        }
    }

    fn reset_for_user_action(&mut self) {
        self.ledger.reset_for_user_action();
        self.reconnect_limits.network_failure_streak = 0;
        self.reconnect_limits.forced_retry_used = false;
        self.reconnect_limits.reconnect_suspended = false;
    }

    // MARK: Timers

    fn timer_due(&mut self, kind: TimerKind) {
        match kind {
            TimerKind::AttemptDeadline | TimerKind::Retry | TimerKind::ProactiveReconnect => {
                unreachable!("run as their own steps")
            }
            TimerKind::OtpDeadline => self.otp_deadline_passed(),
            TimerKind::StaleOtp => self.stale_otp_wait_ran_out(),
            TimerKind::QuitDeadline if self.quit != Quit::NotRequested => {
                self.quit = Quit::Terminated;
            }
            TimerKind::QuitDeadline => self.stopped_for_update(),
            TimerKind::Schedule => self.schedule_boundary_due(),
            TimerKind::AwayLimit => self.check_away_limit(),
            TimerKind::PathSettle => self.path_settled(),
            TimerKind::WakeSettle => self.wake_look_due(),
            TimerKind::LockPoll => self.lock_poll_due(),
            TimerKind::ApprovalFollow => self.approval_follow_due(),
        }
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
