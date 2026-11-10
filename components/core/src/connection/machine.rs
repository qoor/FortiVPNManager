//! The connection state machine: what the user wants, where the connection is, and how each input
//! moves it.
//!
//! Timers, the activity hold and saving the ledger follow from the state after each input, so no
//! transition can forget to cancel a timer or save the ledger.

use std::collections::BTreeMap;
use std::mem;
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

use fvm_contract::app_helper::{
    AttemptId, ExitFacts, HelperErrorCode, HelperEvent, HelperEventKind, Milestone, StopReason,
};
use fvm_contract::app_internal::SecretText;
use fvm_contract::app_internal::port::RegistrationError;
use jiff::tz::TimeZone;

use super::certs::merge_trusted_certs;
use super::classify::{ExitContext, Outcome, classify};
use super::ledger::{Failure, FailureLedger};
use super::policy::{self, elapsed};
use super::readiness::{FollowApproval, HelperFacts, SetupNeed};
use super::system::{PathSnapshot, RawPath, Underlay, WakeObservation, WakeSettle};
use super::{
    ConnectionSettings, Effect, Input, Launch, Note, OtpFailure, Settings, Startup, TimerId,
    TimerKind, VpnAccount,
};

pub struct ConnectionManager {
    pub(super) app_build: String,
    attempt_seed: [u8; 16],
    attempts_issued: u64,
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
    pub(super) sleeping: bool,
    last_wake_at: Option<SystemTime>,
    /// A wake not judged yet, with how long the system slept.
    pending_wake: Option<Duration>,
    quit_requested_at: Option<SystemTime>,
    terminated: bool,
    /// Since when the user has been away; `None` while present.
    pub(super) away_since: Option<SystemTime>,
    /// The away limit was reached: no automatic reconnect until Connect, Reconnect Now or the next
    /// scheduled start.
    pub(super) reconnect_suspended: bool,
    /// The latest schedule start or end acted on.
    schedule_boundary: Option<SystemTime>,
    network_failure_streak: u32,
    /// The immediate reconnect after a forced termination was used and no session has been stable
    /// since.
    forced_retry_used: bool,
    /// The attempt whose failure the phase shows, for a late spam-folder finding.
    last_failure_attempt: Option<AttemptId>,
    /// The account of the last run started: the run whose events are taken now. It differs from
    /// the settings' account when that changed during the run.
    pub(super) run_account: Option<VpnAccount>,
    timers: BTreeMap<TimerKind, TimerSlot>,
    next_timer_generation: u64,
    activity_held: bool,

    // The system as observed (system.rs).
    pub(super) raw_path: Option<RawPath>,
    pub(super) path_settle_at: Option<SystemTime>,
    /// The probe a network path report waits for, with the host it looks up.
    pub(super) path_probe: Option<(u64, String)>,
    pub(super) wake_settle: Option<WakeSettle>,
    pub(super) next_probe: u64,
    /// The gateway address resolved for a host.
    pub(super) gateway_ip: Option<(String, Ipv4Addr)>,
    pub(super) screen_locked: bool,
    pub(super) displays_asleep: bool,
    pub(super) lock_poll_at: Option<SystemTime>,
    pub(super) sleep_reported_at: Option<SystemTime>,

    // Readiness (readiness.rs).
    /// The setup the last readiness check found missing; `None` before the first check.
    pub(super) setup_needed: Option<Vec<SetupNeed>>,
    pub(super) helper: Option<HelperFacts>,
    pub(super) register_error: Option<RegistrationError>,
    pub(super) follow_approval: Option<FollowApproval>,
    pub(super) settings_open_decided: bool,
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
    /// The OTP session takes its starting point.
    PreparingOtp,
    /// The run was asked for; its progress is the milestone.
    Run,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Attempt {
    pub(super) id: AttemptId,
    pub(super) reason: AttemptReason,
    cert_retry_used: bool,
    started_at: SystemTime,
    pub(super) stage: Stage,
    pub(super) milestone: Option<Milestone>,
    otp_prompted_at: Option<SystemTime>,
    otp_submitted: bool,
    /// The OTP session is open and this attempt's; it closes itself after the submission.
    otp_session_open: bool,
    otp_failure: Option<OtpFailure>,
    otp_in_spam: bool,
    path_changed: bool,
    /// The stop the app asked for; the exit is still classified.
    stop_requested: Option<StopReason>,
    /// The account when the attempt began: its readiness check was for it and its run uses it.
    pub(super) account: VpnAccount,
}

impl Attempt {
    /// Readiness holds: the OTP session opens before the VPN client starts.
    pub(super) fn prepare_otp(&mut self) -> Effect {
        self.stage = Stage::PreparingOtp;
        self.otp_session_open = true;
        Effect::OpenOtpSession {
            attempt: self.id.clone(),
        }
    }

    fn close_otp_session(&mut self, check_spam: bool) -> Option<Effect> {
        mem::take(&mut self.otp_session_open).then(|| Effect::CloseOtpSession {
            attempt: self.id.clone(),
            check_spam,
        })
    }
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
}

/// A stop the app asked for; it is complete at the run's exit event, not at the reply.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Stop {
    attempt: AttemptId,
    pub(super) after: AfterStop,
    /// Set when the run had a tunnel up.
    connected_since: Option<SystemTime>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum AfterStop {
    Idle,
    Reconnect(AttemptReason),
    ResumeAfterSleep,
    Quit,
}

/// Why the connection waits before the next attempt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RetryReason {
    Network,
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

/// What the schedule notes compare before and after an input.
struct NoteMarks {
    scheduled_attempt: Option<AttemptId>,
    holding: bool,
    suspended: bool,
}

impl ConnectionManager {
    pub fn new(startup: Startup) -> Self {
        let ledger = FailureLedger::new(startup.settings.account.ledger_key());
        Self {
            app_build: startup.app_build,
            attempt_seed: startup.attempt_seed,
            attempts_issued: 0,
            settings: startup.settings,
            time_zone: startup.time_zone,
            now: SystemTime::UNIX_EPOCH,
            effects: Vec::new(),
            wants_connection: false,
            phase: Phase::Idle,
            saved_ledger: ledger.clone(),
            ledger,
            path: None,
            sleeping: false,
            last_wake_at: None,
            pending_wake: None,
            quit_requested_at: None,
            terminated: false,
            away_since: None,
            reconnect_suspended: false,
            schedule_boundary: None,
            network_failure_streak: 0,
            forced_retry_used: false,
            last_failure_attempt: None,
            run_account: None,
            timers: BTreeMap::new(),
            next_timer_generation: 0,
            activity_held: false,
            raw_path: None,
            path_settle_at: None,
            path_probe: None,
            wake_settle: None,
            next_probe: 0,
            gateway_ip: None,
            screen_locked: false,
            displays_asleep: false,
            lock_poll_at: None,
            sleep_reported_at: None,
            setup_needed: None,
            helper: None,
            register_error: None,
            follow_approval: None,
            settings_open_decided: false,
        }
    }

    /// Handles one input taken at `now` and returns what to carry out, in order.
    pub fn handle(&mut self, input: Input, now: SystemTime) -> Vec<Effect> {
        if self.terminated {
            return Vec::new();
        }
        self.now = match &input {
            // A current timer is never early here. The runtime may sleep on a monotonic clock and
            // stamp a slewed or stepped-back wall clock; judged a few milliseconds early, a retry
            // bound by the cooldown would enter the very wait whose timer already fired, and
            // nothing would move it on.
            Input::TimerFired(timer) => match self.timers.get(&timer.kind) {
                Some(slot) if slot.generation == timer.generation && !slot.fired => {
                    now.max(slot.at)
                }
                _ => now,
            },
            _ => now,
        };
        let marks = self.note_marks();
        self.dispatch(input);
        self.finish(marks)
    }

    fn dispatch(&mut self, input: Input) {
        match input {
            Input::Launched(launch) => self.launched(launch),
            Input::SettingsChanged(settings) => self.settings_changed(settings),
            Input::TimeZoneChanged(time_zone) => self.time_zone_changed(time_zone),
            Input::Connect => self.connect_requested(AttemptReason::User),
            Input::Disconnect => self.disconnect_requested(),
            Input::ReconnectNow => self.reconnect_now_requested(),
            Input::QuitRequested => self.quit_requested(),
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
            Input::Helper(event) => self.helper_event(event),
            Input::RunRejected(attempt, code) => self.run_rejected(&attempt, code),
            Input::HelperConnectionLost => self.helper_connection_lost(),
            Input::TimerFired(timer) => self.timer_fired(timer),
            Input::WillSleep { at } => self.will_sleep_reported(at),
            Input::DidWake => self.did_wake_reported(),
            Input::ScreenLocked { at } => {
                self.screen_locked = true;
                self.update_presence(at);
            }
            Input::ScreenUnlocked => {
                self.screen_locked = false;
                self.update_presence(self.now);
            }
            Input::DisplaysSlept { at } => {
                self.displays_asleep = true;
                self.update_presence(at);
            }
            Input::DisplaysWoke => {
                // Whether the screen is locked behind the displays decides; the answer judges.
                self.displays_asleep = false;
                self.effects.push(Effect::ReadScreenState);
            }
            Input::ScreenState {
                locked,
                displays_asleep,
            } => {
                self.screen_locked = locked;
                self.displays_asleep = displays_asleep;
                self.update_presence(self.now);
            }
            Input::NetworkPathChanged {
                satisfied,
                interfaces,
            } => self.network_path_changed(satisfied, interfaces),
            Input::RouterRead { present } => self.router_read(present),
            Input::NetworkProbed(result) => self.network_probed(result),
        }
    }

    fn finish(&mut self, marks: NoteMarks) -> Vec<Effect> {
        self.reconcile_timers();
        let hold = !self.terminated && self.phase.holds_activity();
        if hold != self.activity_held {
            self.activity_held = hold;
            self.effects.push(Effect::HoldActivity(hold));
        }
        if self.ledger != self.saved_ledger {
            self.saved_ledger = self.ledger.clone();
            self.effects.push(Effect::SaveLedger(self.ledger.clone()));
        }
        if self.terminated {
            self.effects.push(Effect::ApproveQuit);
        }
        let mut effects = self.notes(marks);
        effects.append(&mut self.effects);
        effects
    }

    fn note_marks(&self) -> NoteMarks {
        NoteMarks {
            scheduled_attempt: match &self.phase {
                Phase::Connecting(attempt) if attempt.reason == AttemptReason::Scheduled => {
                    Some(attempt.id.clone())
                }
                _ => None,
            },
            holding: self.phase.is_holding(),
            suspended: self.reconnect_suspended,
        }
    }

    fn notes(&self, before: NoteMarks) -> Vec<Effect> {
        let Some(schedule) = &self.settings.connection.schedule else {
            return Vec::new();
        };
        let after = self.note_marks();
        let mut notes = Vec::new();
        if after.scheduled_attempt.is_some() && after.scheduled_attempt != before.scheduled_attempt
        {
            notes.push(Effect::Note(Note::ScheduledHoursStarted));
        }
        if after.holding && !before.holding {
            notes.push(Effect::Note(Note::ReconnectHeldWhileAway));
        }
        if after.suspended && !before.suspended {
            notes.push(Effect::Note(Note::ReconnectsSuspended {
                away_minutes: schedule.away_minutes,
            }));
        }
        notes
    }

    fn wanted_timers(&self) -> BTreeMap<TimerKind, SystemTime> {
        let mut wanted = BTreeMap::new();
        if self.terminated {
            return wanted;
        }
        match &self.phase {
            Phase::Connecting(attempt) if attempt.stop_requested.is_none() => {
                wanted.insert(
                    TimerKind::AttemptDeadline,
                    attempt.started_at + policy::ATTEMPT_TIMEOUT,
                );
                if attempt.stage == Stage::Run
                    && !attempt.otp_submitted
                    && let Some(prompted) = attempt.otp_prompted_at
                {
                    wanted.insert(TimerKind::OtpDeadline, prompted + policy::OTP_DEADLINE);
                }
            }
            // Suspended while asleep and until the wake is judged; outside the scheduled hours
            // also while the away user is away.
            Phase::Connected(session) if !self.sleeping && self.pending_wake.is_none() => {
                if let Some(planned) = session.planned_reconnect
                    && self.automatic_reconnect_allowed()
                {
                    wanted.insert(TimerKind::ProactiveReconnect, planned);
                }
            }
            Phase::WaitingRetry { until, .. } => {
                wanted.insert(TimerKind::Retry, *until);
            }
            _ => {}
        }
        if let Some(requested) = self.quit_requested_at {
            wanted.insert(TimerKind::QuitDeadline, requested + policy::QUIT_TIMEOUT);
        }
        if let Some(schedule) = &self.settings.connection.schedule
            && self.quit_requested_at.is_none()
        {
            if let Some(handled) = self.schedule_boundary
                && let Some(next) = schedule.next_boundary(handled, &self.time_zone)
            {
                wanted.insert(TimerKind::Schedule, next.at);
            }
            if let Some(away_since) = self.away_since
                && !self.reconnect_suspended
            {
                wanted.insert(TimerKind::AwayLimit, away_since + schedule.away_limit());
            }
        }
        let observation = [
            (TimerKind::PathSettle, self.path_settle_at),
            (
                TimerKind::WakeSettle,
                self.wake_settle.as_ref().and_then(WakeSettle::next_look),
            ),
            (TimerKind::LockPoll, self.lock_poll_at),
            (
                TimerKind::ApprovalFollow,
                self.follow_approval.as_ref().map(|follow| follow.next),
            ),
        ];
        for (kind, at) in observation {
            if let Some(at) = at {
                wanted.insert(kind, at);
            }
        }
        wanted
    }

    fn reconcile_timers(&mut self) {
        let wanted = self.wanted_timers();
        for kind in TimerKind::ALL {
            match (self.timers.get(&kind).copied(), wanted.get(&kind).copied()) {
                (None, None) => {}
                (Some(slot), None) => {
                    self.timers.remove(&kind);
                    if !slot.fired {
                        self.effects.push(Effect::CancelTimer(kind));
                    }
                }
                // A fired timer still wanted at the same time stays fired: no firing loop.
                (Some(slot), Some(at)) if slot.at == at => {}
                (_, Some(at)) => {
                    self.next_timer_generation += 1;
                    let timer = TimerId {
                        kind,
                        generation: self.next_timer_generation,
                    };
                    self.timers.insert(
                        kind,
                        TimerSlot {
                            at,
                            generation: timer.generation,
                            fired: false,
                        },
                    );
                    self.effects.push(Effect::SetTimer { timer, at });
                }
            }
        }
    }

    /// Asks the runtime for a fresh timer even at an unchanged time: a timer may not survive the
    /// sleep reliably.
    fn rearm_proactive_timer(&mut self) {
        if let Some(slot) = self.timers.get_mut(&TimerKind::ProactiveReconnect) {
            slot.at = SystemTime::UNIX_EPOCH;
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
        self.ledger.prune(self.now);
        self.reset_schedule_boundary();
        self.effects.push(Effect::ReadScreenState);
        if launch.import_report_shown {
            self.settings_open_decided = true;
            self.effects.push(Effect::OpenSettings);
        }

        if !launch.auto_connect || !self.within_schedule() {
            self.wants_connection = false;
            self.check_readiness();
            return;
        }
        self.wants_connection = true;
        if self.ledger.consecutive_auth_failures >= policy::MAX_CONSECUTIVE_AUTH_FAILURES {
            // The retries had stopped before the relaunch; only the user clears that.
            let failure = self
                .ledger
                .last_auth_failure
                .unwrap_or(Failure::AuthenticationRejected);
            self.fail(failure);
            return;
        }
        self.begin_attempt(AttemptReason::Launch, false, false);
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
        if !account.trusted_certs.is_empty()
            && super::certs::gateway_change_drops_certs(old, account)
        {
            // The next connection learns the new gateway's certificate.
            account.trusted_certs.clear();
            self.effects.push(Effect::SaveTrustedCerts(Vec::new()));
        }
        if account.host != old.host {
            self.gateway_ip = None;
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
                self.reconnect_suspended = false;
                self.begin_attempt(reason, false, false);
            }
            phase => {
                self.reconnect_suspended = false;
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
                self.stop_session(session, StopReason::UserDisconnect, AfterStop::Idle);
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
                self.stop_session(
                    session,
                    StopReason::Reconnect,
                    AfterStop::Reconnect(AttemptReason::ReconnectNow),
                );
            }
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } | Phase::Failed(_) => {
                self.wants_connection = true;
                self.reset_for_user_action();
                self.begin_attempt(AttemptReason::ReconnectNow, false, false);
            }
            // Restarting an attempt under way could waste an OTP or a login.
            phase => self.phase = phase,
        }
    }

    fn quit_requested(&mut self) {
        self.wants_connection = false;
        self.quit_requested_at.get_or_insert(self.now);
        match self.take_phase() {
            Phase::Connecting(attempt) => {
                self.abort_attempt(*attempt, StopReason::AppQuit, AfterStop::Quit);
            }
            Phase::Connected(session) => {
                self.stop_session(session, StopReason::AppQuit, AfterStop::Quit);
            }
            Phase::Disconnecting(mut stop) => {
                stop.after = AfterStop::Quit;
                self.phase = Phase::Disconnecting(stop);
            }
            phase => {
                self.phase = phase;
                self.terminated = true;
            }
        }
    }

    fn reset_for_user_action(&mut self) {
        self.ledger.reset_for_user_action();
        self.network_failure_streak = 0;
        self.forced_retry_used = false;
        self.reconnect_suspended = false;
    }

    // MARK: OTP session

    fn otp_session_ready(&mut self, id: &AttemptId) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.id != *id || attempt.stage != Stage::PreparingOtp {
            return;
        }
        attempt.stage = Stage::Run;
        self.run_account = Some(attempt.account.clone());
        self.effects.push(Effect::StartRun {
            attempt: attempt.id.clone(),
            account: attempt.account.clone(),
        });
    }

    fn otp_session_failed(&mut self, id: &AttemptId, failure: OtpFailure) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.id != *id || !attempt.otp_session_open {
            return;
        }
        attempt.otp_session_open = false;
        match attempt.stage {
            // The VPN client was never started: a Gmail credential problem costs no VPN login.
            Stage::PreparingOtp => match failure {
                OtpFailure::Authentication => self.fail(Failure::GmailRejected),
                OtpFailure::Network => self.network_failure(),
            },
            // No OTP can arrive any more: stop now instead of at the deadline; the exit classifies.
            Stage::Run => {
                attempt.otp_failure = Some(failure);
                if attempt.stop_requested.is_none() {
                    attempt.stop_requested = Some(StopReason::OtpUnavailable);
                    self.effects.push(Effect::StopRun {
                        attempt: attempt.id.clone(),
                        reason: StopReason::OtpUnavailable,
                    });
                }
            }
            Stage::CheckingReadiness => unreachable!("no OTP session before the readiness check"),
        }
    }

    fn otp_code_found(&mut self, id: &AttemptId, code: SecretText) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.id != *id
            || attempt.stage != Stage::Run
            || attempt.otp_prompted_at.is_none()
            || attempt.otp_submitted
            || attempt.stop_requested.is_some()
        {
            return;
        }
        attempt.otp_submitted = true;
        // The session moves the used emails to the trash and closes itself after the submission.
        attempt.otp_session_open = false;
        self.effects.push(Effect::SubmitOtp {
            attempt: attempt.id.clone(),
            code,
        });
    }

    fn otp_found_in_spam(&mut self, id: &AttemptId) {
        if let Phase::Connecting(attempt) = &mut self.phase
            && attempt.id == *id
        {
            attempt.otp_in_spam = true;
            return;
        }
        if self.last_failure_attempt.as_ref() != Some(id) {
            return;
        }
        let plain = Failure::OtpNotReceived { in_spam: false };
        let spam = Failure::OtpNotReceived { in_spam: true };
        match &mut self.phase {
            Phase::Failed(failure) if *failure == plain => *failure = spam,
            Phase::WaitingRetry { reason, .. } | Phase::WaitingNetwork { reason, .. }
                if *reason == RetryReason::AuthFailure(plain) =>
            {
                *reason = RetryReason::AuthFailure(spam);
            }
            _ => {}
        }
        if self.ledger.last_auth_failure == Some(plain) {
            self.ledger.last_auth_failure = Some(spam);
        }
    }

    // MARK: Helper

    fn helper_event(&mut self, event: HelperEvent) {
        match event.kind {
            HelperEventKind::Milestone {
                milestone,
                tunnel_ip,
            } => self.milestone_reached(&event.attempt_id, milestone, tunnel_ip),
            HelperEventKind::Exited { facts } => self.exited(&event.attempt_id, &facts),
        }
    }

    fn milestone_reached(
        &mut self,
        id: &AttemptId,
        milestone: Milestone,
        tunnel_ip: Option<Ipv4Addr>,
    ) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.stage != Stage::Run || attempt.id != *id || attempt.stop_requested.is_some() {
            return;
        }
        let previous = attempt.milestone;
        if previous.is_some_and(|previous| milestone <= previous) {
            return;
        }
        attempt.milestone = Some(milestone);
        if previous.unwrap_or(Milestone::Spawned) < Milestone::ConnectedToGateway
            && milestone >= Milestone::ConnectedToGateway
        {
            // The certificate check passed and the credentials go out: this counts toward the
            // attempt limit.
            self.ledger.record_auth_attempt(self.now);
        }
        match milestone {
            Milestone::TunnelUp => {
                let Phase::Connecting(attempt) = self.take_phase() else {
                    unreachable!("matched above");
                };
                self.enter_connected(*attempt, tunnel_ip);
            }
            Milestone::OtpPrompted => {
                attempt.otp_prompted_at = Some(self.now);
                self.effects.push(Effect::SearchOtp {
                    attempt: attempt.id.clone(),
                    prompted_at: self.now,
                });
            }
            Milestone::Spawned
            | Milestone::ConnectedToGateway
            | Milestone::OtpSubmitted
            | Milestone::Authenticated => {}
        }
    }

    fn enter_connected(&mut self, mut attempt: Attempt, tunnel_ip: Option<Ipv4Addr>) {
        // Still open only when the gateway asked for no OTP.
        self.effects.extend(attempt.close_otp_session(false));
        self.network_failure_streak = 0;
        self.phase = Phase::Connected(Session {
            attempt: attempt.id,
            since: self.now,
            tunnel_ip,
            planned_reconnect: policy::planned_reconnect(
                self.now,
                &self.settings.connection,
                self.now,
            ),
            underlay: self.path.as_ref().and_then(|path| path.underlay.clone()),
        });
    }

    fn exited(&mut self, id: &AttemptId, facts: &ExitFacts) {
        match self.take_phase() {
            Phase::Connecting(mut attempt) if attempt.stage == Stage::Run && attempt.id == *id => {
                self.learn_certificates(facts);
                self.effects.extend(attempt.close_otp_session(false));
                self.last_failure_attempt = Some(id.clone());
                let outcome = classify(&ExitContext {
                    facts,
                    seen_milestone: attempt.milestone,
                    cert_retry_used: attempt.cert_retry_used,
                    app_stop: attempt.stop_requested,
                    otp_failure: attempt.otp_failure,
                    otp_in_spam: attempt.otp_in_spam,
                    path_changed: attempt.path_changed,
                    within_wake_grace: self.started_within_wake_grace(&attempt),
                    observed_up: None,
                });
                self.apply(outcome);
            }
            Phase::Connected(session) if session.attempt == *id => {
                self.learn_certificates(facts);
                let up = facts
                    .up_duration
                    .unwrap_or_else(|| elapsed(session.since, self.now));
                if up >= policy::STABLE_SESSION {
                    self.ledger.reset_consecutive_failures();
                    self.forced_retry_used = false;
                }
                self.last_failure_attempt = Some(id.clone());
                let outcome = classify(&ExitContext {
                    facts,
                    seen_milestone: Some(Milestone::TunnelUp),
                    cert_retry_used: true,
                    app_stop: None,
                    otp_failure: None,
                    otp_in_spam: false,
                    path_changed: false,
                    within_wake_grace: false,
                    observed_up: Some(up),
                });
                self.apply(outcome);
            }
            Phase::Disconnecting(stop) if stop.attempt == *id => {
                self.learn_certificates(facts);
                self.finish_stop(stop, facts.up_duration);
            }
            // An attempt already finished with.
            phase => self.phase = phase,
        }
    }

    fn run_rejected(&mut self, id: &AttemptId, code: HelperErrorCode) {
        match self.take_phase() {
            Phase::Connecting(mut attempt) if attempt.stage == Stage::Run && attempt.id == *id => {
                self.effects.extend(attempt.close_otp_session(false));
                match (attempt.otp_failure, code) {
                    (Some(OtpFailure::Authentication), _) => self.fail(Failure::GmailRejected),
                    (Some(OtpFailure::Network), _) => self.network_failure(),
                    (
                        None,
                        HelperErrorCode::ClientUnavailable
                        | HelperErrorCode::InvalidRequest
                        | HelperErrorCode::NotAuthorized,
                    ) => self.fail(Failure::Helper(code)),
                    (
                        None,
                        HelperErrorCode::Busy
                        | HelperErrorCode::WrongPhase
                        | HelperErrorCode::UnknownAttempt
                        | HelperErrorCode::InternalError,
                    ) => self.network_failure(),
                }
            }
            Phase::Disconnecting(stop) if stop.attempt == *id => self.finish_stop(stop, None),
            phase => self.phase = phase,
        }
    }

    /// The connection stays inside the Mac: it breaks only when the Helper process ends, killed
    /// or crashed, never because of the network.
    fn helper_connection_lost(&mut self) {
        match self.take_phase() {
            Phase::Connecting(mut attempt) if attempt.stage == Stage::Run => {
                self.effects.extend(attempt.close_otp_session(false));
                if attempt.otp_failure == Some(OtpFailure::Authentication) {
                    self.fail(Failure::GmailRejected);
                } else {
                    self.forced_termination();
                }
            }
            Phase::Connected(session) => {
                if elapsed(session.since, self.now) >= policy::STABLE_SESSION {
                    self.ledger.reset_consecutive_failures();
                    self.forced_retry_used = false;
                }
                self.forced_termination();
            }
            Phase::Disconnecting(stop) => self.finish_stop(stop, None),
            phase => self.phase = phase,
        }
    }

    /// Whether the run of attempt `id` is still to be started: no stop was decided since it was
    /// asked for. A stop decided before the run's Connect left would reach the Helper first and
    /// find nothing to stop, so such a run is dropped instead.
    pub fn wants_run(&self, id: &AttemptId) -> bool {
        matches!(&self.phase, Phase::Connecting(attempt)
            if attempt.id == *id && attempt.stage == Stage::Run && attempt.stop_requested.is_none())
    }

    /// Whether attempt `id`'s OTP session is still to be opened: it was not closed while the
    /// session's secrets were read.
    pub fn wants_otp_session(&self, id: &AttemptId) -> bool {
        matches!(&self.phase, Phase::Connecting(attempt)
            if attempt.id == *id && attempt.otp_session_open)
    }

    /// Whether a readiness check that found the Helper at `helper_build` should have it exit when
    /// idle, so the system starts this build's Helper: only another build, and only while no run
    /// is going on.
    pub fn should_restart_helper(&self, helper_build: &str) -> bool {
        helper_build != self.app_build && !self.run_active()
    }

    /// A run's events are taken now.
    pub(super) fn run_active(&self) -> bool {
        match &self.phase {
            Phase::Connecting(attempt) => attempt.stage == Stage::Run,
            Phase::Connected(_) | Phase::Disconnecting(_) => true,
            _ => false,
        }
    }

    // MARK: Timers

    fn timer_fired(&mut self, timer: TimerId) {
        let Some(slot) = self.timers.get_mut(&timer.kind) else {
            return;
        };
        if slot.generation != timer.generation || slot.fired {
            return;
        }
        slot.fired = true;
        match timer.kind {
            TimerKind::AttemptDeadline => self.attempt_deadline_passed(),
            TimerKind::OtpDeadline => self.otp_deadline_passed(),
            TimerKind::Retry => {
                if let Phase::WaitingRetry { reason, .. } = self.phase {
                    self.begin_attempt(AttemptReason::Retry(reason), false, false);
                }
            }
            TimerKind::ProactiveReconnect => self.proactive_reconnect_due(),
            TimerKind::QuitDeadline => self.terminated = true,
            TimerKind::Schedule => self.schedule_boundary_due(),
            TimerKind::AwayLimit => self.check_away_limit(),
            TimerKind::PathSettle => self.path_settled(),
            TimerKind::WakeSettle => self.wake_look_due(),
            TimerKind::LockPoll => self.lock_poll_due(),
            TimerKind::ApprovalFollow => self.approval_follow_due(),
        }
    }

    fn attempt_deadline_passed(&mut self) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.stop_requested.is_some() {
            return;
        }
        if attempt.stage != Stage::Run {
            // The readiness check or the Gmail login hung; nothing runs yet.
            self.effects.extend(attempt.close_otp_session(false));
            self.network_failure();
            return;
        }
        attempt.stop_requested = Some(StopReason::AttemptTimeout);
        self.effects.push(Effect::StopRun {
            attempt: attempt.id.clone(),
            reason: StopReason::AttemptTimeout,
        });
        self.effects.extend(attempt.close_otp_session(false));
    }

    fn otp_deadline_passed(&mut self) {
        let Phase::Connecting(attempt) = &mut self.phase else {
            return;
        };
        if attempt.stage != Stage::Run
            || attempt.otp_prompted_at.is_none()
            || attempt.otp_submitted
            || attempt.stop_requested.is_some()
        {
            return;
        }
        attempt.stop_requested = Some(StopReason::OtpTimeout);
        self.effects.push(Effect::StopRun {
            attempt: attempt.id.clone(),
            reason: StopReason::OtpTimeout,
        });
        self.effects.extend(attempt.close_otp_session(true));
    }

    fn proactive_reconnect_due(&mut self) {
        if !matches!(self.phase, Phase::Connected(_)) || !self.automatic_reconnect_allowed() {
            return;
        }
        let Phase::Connected(session) = self.take_phase() else {
            unreachable!("matched above");
        };
        self.stop_session(
            session,
            StopReason::ProactiveReconnect,
            AfterStop::Reconnect(AttemptReason::Proactive),
        );
    }

    // MARK: Sleep, wake and the network path

    pub(super) fn will_sleep(&mut self) {
        match self.take_phase() {
            Phase::Connecting(attempt) => {
                self.abort_attempt(*attempt, StopReason::Sleep, AfterStop::ResumeAfterSleep);
            }
            // Already waiting for the user, who is away while the Mac sleeps.
            phase @ Phase::WaitingRetry {
                reason: RetryReason::UserAway,
                ..
            } => self.phase = phase,
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. } => {
                self.wait_for_network(RetryReason::Sleep, Duration::ZERO);
            }
            Phase::Disconnecting(mut stop) => {
                if matches!(stop.after, AfterStop::Reconnect(_)) {
                    stop.after = AfterStop::ResumeAfterSleep;
                }
                self.phase = Phase::Disconnecting(stop);
            }
            // A tunnel that is up is kept across the sleep; its proactive timer waits.
            phase => self.phase = phase,
        }
    }

    pub(super) fn did_wake(&mut self, slept_for: Duration) {
        self.sleeping = false;
        self.last_wake_at = Some(self.now);
        self.pending_wake = Some(slept_for);
        // The away time may have run out during the sleep; act before the user unlocks.
        self.check_away_limit();
    }

    pub(super) fn wake_settled(&mut self, observation: WakeObservation) {
        let slept_for = self.pending_wake.take().unwrap_or(Duration::ZERO);
        self.record_path(PathSnapshot {
            satisfied: observation.path_satisfied,
            underlay: observation.underlay.clone(),
        });
        match &self.phase {
            // The wake rules judge a tunnel that was up across the sleep. One that came up after
            // the wake, when the path returned before the wake was judged, is left alone: the
            // observation may predate it. Ending the pending wake re-arms its proactive timer.
            Phase::Connected(session) => {
                if self.last_wake_at.is_none_or(|wake| session.since < wake) {
                    let session = session.clone();
                    self.evaluate_wake(session, &observation, slept_for);
                }
            }
            Phase::WaitingNetwork {
                reason,
                delay_after_path,
            } if observation.path_satisfied && !self.sleeping => {
                self.resume_from_network_wait(*reason, *delay_after_path);
            }
            _ => {}
        }
    }

    /// The rules for a tunnel that was up across the sleep, in order. While automatic reconnects
    /// are held by the schedule, a working tunnel is kept rather than replaced; a dead or
    /// stranded one is still stopped, and the reconnect after it waits or is given up.
    fn evaluate_wake(
        &mut self,
        mut session: Session,
        observation: &WakeObservation,
        slept_for: Duration,
    ) {
        let may_reconnect = self.automatic_reconnect_allowed();
        if !observation.path_satisfied {
            // The network did not come back: keep the tunnel and judge it later.
            let earliest = self.now + policy::MINIMUM_PLANNED_LEAD;
            session.planned_reconnect = session.planned_reconnect.map(|at| at.max(earliest));
            self.phase = Phase::Connected(session);
            self.rearm_proactive_timer();
            return;
        }
        let min_sleep =
            Duration::from_secs(u64::from(self.settings.connection.min_sleep_minutes) * 60);
        let reason = if may_reconnect
            && session
                .planned_reconnect
                .is_some_and(|planned| planned <= self.now)
        {
            Some(AttemptReason::Proactive)
        } else if !observation.client_running || !observation.tunnel_up {
            Some(AttemptReason::TunnelLost)
        } else if let (Some(old), Some(new)) = (&session.underlay, &observation.underlay)
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
        match reason {
            Some(reason) => self.restart_session(session, reason),
            None => {
                if session.underlay.is_none() {
                    session.underlay = observation.underlay.clone();
                }
                self.phase = Phase::Connected(session);
                self.rearm_proactive_timer();
            }
        }
    }

    pub(super) fn path_changed(&mut self, snapshot: PathSnapshot) {
        self.record_path(snapshot.clone());
        if self.sleeping {
            return;
        }
        match &mut self.phase {
            Phase::WaitingNetwork {
                reason,
                delay_after_path,
            } if snapshot.satisfied => {
                let (reason, delay) = (*reason, *delay_after_path);
                self.resume_from_network_wait(reason, delay);
            }
            // While a wake is being judged, its observation decides instead.
            Phase::Connected(session) if self.pending_wake.is_none() => {
                let Some(underlay) = snapshot.underlay.filter(|_| snapshot.satisfied) else {
                    return;
                };
                match &session.underlay {
                    Some(old) if *old != underlay => {
                        let Phase::Connected(session) = self.take_phase() else {
                            unreachable!("matched above");
                        };
                        self.restart_session(session, AttemptReason::NetworkChanged);
                    }
                    Some(_) => {}
                    None => session.underlay = Some(underlay),
                }
            }
            // An attempt under way or a stop ignores the path; the other phases act on the next
            // attempt.
            _ => {}
        }
    }

    /// Keeps the path and marks an attempt under way whose network changed beneath it.
    fn record_path(&mut self, snapshot: PathSnapshot) {
        if self
            .path
            .as_ref()
            .is_some_and(|previous| *previous != snapshot)
            && let Phase::Connecting(attempt) = &mut self.phase
        {
            attempt.path_changed = true;
        }
        self.path = Some(snapshot);
    }

    fn resume_from_network_wait(&mut self, reason: RetryReason, delay: Duration) {
        if delay > Duration::ZERO {
            self.phase = Phase::WaitingRetry {
                until: self.now + delay,
                reason,
            };
        } else if reason == RetryReason::Sleep {
            self.begin_attempt(AttemptReason::AfterSleep, false, false);
        } else {
            self.begin_attempt(AttemptReason::Retry(reason), false, false);
        }
    }

    // MARK: Schedule and presence

    pub(super) fn presence_changed(&mut self, away: bool, at: SystemTime) {
        if away {
            self.away_since.get_or_insert(at);
            return;
        }
        // The away time may have run out before the app could act on it, such as right after a
        // wake.
        self.check_away_limit();
        self.away_since = None;
        self.rejudge_held_reconnect();
    }

    /// A new, changed or removed schedule applies from its next start or end; nothing connects at
    /// once. A held reconnect is judged again under the new rules.
    fn schedule_changed(&mut self) {
        self.reset_schedule_boundary();
        if self.settings.connection.schedule.is_none() {
            self.reconnect_suspended = false;
        } else {
            self.check_away_limit();
        }
        self.rejudge_held_reconnect();
    }

    /// A reconnect held for an away user passes the gate again: it may go ahead, wait with a new
    /// limit, or be given up.
    fn rejudge_held_reconnect(&mut self) {
        if self.phase.is_holding() {
            self.begin_attempt(AttemptReason::Retry(RetryReason::UserAway), false, false);
        }
    }

    /// Marks the latest start or end at or before now as handled, so only later ones act.
    fn reset_schedule_boundary(&mut self) {
        self.schedule_boundary = self
            .settings
            .connection
            .schedule
            .as_ref()
            .and_then(|schedule| schedule.latest_boundary(self.now, &self.time_zone))
            .map(|boundary| boundary.at);
    }

    /// Acts on the latest boundary only; earlier ones were missed in a sleep.
    fn schedule_boundary_due(&mut self) {
        let Some(latest) = self
            .settings
            .connection
            .schedule
            .as_ref()
            .and_then(|schedule| schedule.latest_boundary(self.now, &self.time_zone))
        else {
            return;
        };
        if self
            .schedule_boundary
            .is_some_and(|handled| latest.at <= handled)
        {
            return;
        }
        self.schedule_boundary = Some(latest.at);
        match latest.edge {
            super::schedule::Edge::Start => {
                self.reconnect_suspended = false;
                self.connect_requested(AttemptReason::Scheduled);
            }
            super::schedule::Edge::End => self.check_away_limit(),
        }
    }

    /// Stops automatic reconnects once the away limit is reached; a waiting one is dropped.
    fn check_away_limit(&mut self) {
        if self.reconnect_suspended || !self.away_limit_reached() {
            return;
        }
        self.reconnect_suspended = true;
        if matches!(
            self.phase,
            Phase::WaitingRetry { .. } | Phase::WaitingNetwork { .. }
        ) {
            self.give_up_reconnecting();
        }
    }

    /// Like Disconnect without stopping anything: the next connection waits for the user or the
    /// schedule.
    fn give_up_reconnecting(&mut self) {
        self.reconnect_suspended = true;
        self.wants_connection = false;
        self.phase = Phase::Idle;
    }

    pub(super) fn within_schedule(&self) -> bool {
        self.settings
            .connection
            .schedule
            .as_ref()
            .is_none_or(|schedule| schedule.is_active(self.now, &self.time_zone))
    }

    /// Always inside the scheduled hours; outside them only while the user is present and
    /// reconnects were not suspended.
    pub(super) fn automatic_reconnect_allowed(&self) -> bool {
        self.within_schedule() || (!self.reconnect_suspended && self.away_since.is_none())
    }

    /// Outside the scheduled hours, the user has been away for the schedule's away time.
    fn away_limit_reached(&self) -> bool {
        let (Some(schedule), Some(away_since)) =
            (&self.settings.connection.schedule, self.away_since)
        else {
            return false;
        };
        !schedule.is_active(self.now, &self.time_zone)
            && self.now >= away_since + schedule.away_limit()
    }

    // MARK: Attempts

    /// Starts an attempt unless a gate holds it: the wish to connect, the schedule, the sleep,
    /// the attempt limit, the cooldown after an authentication failure, or a missing network path.
    pub(super) fn begin_attempt(
        &mut self,
        reason: AttemptReason,
        cert_retry_used: bool,
        readiness_verified: bool,
    ) {
        if !self.wants_connection {
            self.phase = Phase::Idle;
            return;
        }
        if reason.is_automatic_reconnect()
            && let Some(away_limit) = self
                .settings
                .connection
                .schedule
                .as_ref()
                .filter(|schedule| !schedule.is_active(self.now, &self.time_zone))
                .map(super::Schedule::away_limit)
        {
            if self.reconnect_suspended || self.away_limit_reached() {
                self.give_up_reconnecting();
                return;
            }
            if let Some(away_since) = self.away_since {
                self.phase = Phase::WaitingRetry {
                    until: away_since + away_limit,
                    reason: RetryReason::UserAway,
                };
                return;
            }
        }
        if self.sleeping {
            self.wait_for_network(RetryReason::Sleep, Duration::ZERO);
            return;
        }
        self.ledger.prune(self.now);
        if !reason.is_user_action()
            && self.ledger.recent_auth_attempts.len() >= policy::MAX_AUTH_ATTEMPTS_PER_WINDOW
        {
            self.fail(Failure::TooManyReconnects);
            return;
        }
        if let Some(cooldown_end) = self.ledger.cooldown_end()
            && self.now < cooldown_end
        {
            let reason = match reason {
                AttemptReason::Retry(RetryReason::AuthFailure(failure)) => {
                    RetryReason::AuthFailure(failure)
                }
                _ => RetryReason::AuthCooldown,
            };
            self.phase = Phase::WaitingRetry {
                until: cooldown_end,
                reason,
            };
            return;
        }
        if self.path.as_ref().is_some_and(|path| !path.satisfied) {
            let reason = match reason {
                AttemptReason::Retry(reason) => reason,
                AttemptReason::AfterSleep => RetryReason::Sleep,
                _ => RetryReason::Network,
            };
            self.wait_for_network(reason, Duration::ZERO);
            return;
        }

        let id = self.next_attempt_id();
        let mut attempt = Attempt {
            id,
            reason,
            cert_retry_used,
            started_at: self.now,
            stage: Stage::CheckingReadiness,
            milestone: None,
            otp_prompted_at: None,
            otp_submitted: false,
            otp_session_open: false,
            otp_failure: None,
            otp_in_spam: false,
            path_changed: false,
            stop_requested: None,
            account: self.settings.account.clone(),
        };
        if readiness_verified {
            let open = attempt.prepare_otp();
            self.effects.push(open);
        } else {
            self.check_readiness();
        }
        self.phase = Phase::Connecting(Box::new(attempt));
    }

    /// The seed with the attempt count in its last eight bytes.
    fn next_attempt_id(&mut self) -> AttemptId {
        self.attempts_issued += 1;
        let mut bytes = self.attempt_seed;
        for (byte, count) in bytes[8..]
            .iter_mut()
            .zip(self.attempts_issued.to_be_bytes())
        {
            *byte ^= count;
        }
        let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
        format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
        .parse()
        .expect("a UUID")
    }

    /// Ends an attempt the app no longer wants: the user's, the sleep's or the quit's.
    fn abort_attempt(&mut self, mut attempt: Attempt, stop: StopReason, after: AfterStop) {
        if attempt.stage != Stage::Run {
            self.effects.extend(attempt.close_otp_session(false));
            self.settle(after);
            return;
        }
        if attempt.stop_requested.is_none() {
            self.effects.push(Effect::StopRun {
                attempt: attempt.id.clone(),
                reason: stop,
            });
        }
        self.effects.extend(attempt.close_otp_session(false));
        self.phase = Phase::Disconnecting(Stop {
            attempt: attempt.id,
            after,
            connected_since: None,
        });
    }

    /// Break before make: stop the tunnel; `after` follows its exit.
    fn stop_session(&mut self, session: Session, reason: StopReason, after: AfterStop) {
        self.effects.push(Effect::StopRun {
            attempt: session.attempt.clone(),
            reason,
        });
        self.phase = Phase::Disconnecting(Stop {
            attempt: session.attempt,
            after,
            connected_since: Some(session.since),
        });
    }

    fn restart_session(&mut self, session: Session, reason: AttemptReason) {
        let stop = if reason == AttemptReason::Proactive {
            StopReason::ProactiveReconnect
        } else {
            StopReason::Reconnect
        };
        self.stop_session(session, stop, AfterStop::Reconnect(reason));
    }

    fn finish_stop(&mut self, stop: Stop, up: Option<Duration>) {
        let up = up.or_else(|| stop.connected_since.map(|since| elapsed(since, self.now)));
        if up.is_some_and(|up| up >= policy::STABLE_SESSION) {
            self.ledger.reset_consecutive_failures();
            self.forced_retry_used = false;
        }
        self.settle(stop.after);
    }

    fn settle(&mut self, after: AfterStop) {
        match after {
            AfterStop::Idle => self.phase = Phase::Idle,
            AfterStop::Reconnect(reason) => self.begin_attempt(reason, false, false),
            AfterStop::ResumeAfterSleep => {
                self.begin_attempt(AttemptReason::AfterSleep, false, false);
            }
            AfterStop::Quit => {
                self.phase = Phase::Idle;
                self.terminated = true;
            }
        }
    }

    fn started_within_wake_grace(&self, attempt: &Attempt) -> bool {
        self.last_wake_at
            .is_some_and(|wake| attempt.started_at <= wake + policy::WAKE_GRACE)
    }

    /// The run that just ended talked to the gateway the settings name now.
    fn run_is_for_current_gateway(&self) -> bool {
        self.run_account
            .as_ref()
            .is_none_or(|run| run.same_gateway(&self.settings.account))
    }

    fn learn_certificates(&mut self, facts: &ExitFacts) {
        // A run started before a server address or port change saw the old gateway's certificate.
        if facts.cert_rejected_digests.is_empty() || !self.run_is_for_current_gateway() {
            return;
        }
        let merged = merge_trusted_certs(
            &facts.cert_rejected_digests,
            &self.settings.account.trusted_certs,
        );
        if merged == self.settings.account.trusted_certs {
            return;
        }
        self.settings.account.trusted_certs = merged.clone();
        self.effects.push(Effect::SaveTrustedCerts(merged));
    }

    // MARK: Outcomes

    fn apply(&mut self, outcome: Outcome) {
        match outcome {
            // The chain's one immediate retry belongs to a gateway; a new one starts its own.
            Outcome::CertificateRetry => {
                let used = self.run_is_for_current_gateway();
                self.begin_attempt(AttemptReason::CertificateRetry, used, false);
            }
            Outcome::ServerSessionEnded => self.schedule_retry(
                RetryReason::ServerSessionEnded,
                policy::SERVER_SESSION_END_DELAY,
            ),
            Outcome::ForcedTermination => self.forced_termination(),
            Outcome::NetworkError => self.network_failure(),
            Outcome::AuthFailure(failure) => self.auth_failure(failure),
            Outcome::Fatal(failure) => self.fail(failure),
        }
    }

    /// Reconnects at once through the gates of any retry, but once only until a session is
    /// stable again: the Helper aborts on internal faults, and one that crashed on every run
    /// would otherwise repeat VPN logins and OTP emails without end.
    fn forced_termination(&mut self) {
        if self.forced_retry_used {
            self.network_failure();
            return;
        }
        self.forced_retry_used = true;
        self.begin_attempt(
            AttemptReason::Retry(RetryReason::ForcedTermination),
            false,
            false,
        );
    }

    fn network_failure(&mut self) {
        self.network_failure_streak += 1;
        self.schedule_retry(
            RetryReason::Network,
            policy::network_backoff(self.network_failure_streak),
        );
    }

    fn auth_failure(&mut self, failure: Failure) {
        self.ledger.record_auth_failure(failure, self.now);
        if self.ledger.consecutive_auth_failures >= policy::MAX_CONSECUTIVE_AUTH_FAILURES {
            self.fail(failure);
        } else {
            self.schedule_retry(
                RetryReason::AuthFailure(failure),
                policy::AUTH_FAILURE_RETRY_DELAY,
            );
        }
    }

    fn schedule_retry(&mut self, reason: RetryReason, delay: Duration) {
        if !self.wants_connection {
            self.phase = Phase::Idle;
        } else if self.sleeping {
            self.wait_for_network(RetryReason::Sleep, Duration::ZERO);
        } else if self.path.as_ref().is_some_and(|path| !path.satisfied) {
            // An authentication retry is bound by the ledger's cooldown instead of a new wait.
            let delay = if matches!(reason, RetryReason::AuthFailure(_)) {
                Duration::ZERO
            } else {
                delay
            };
            self.wait_for_network(reason, delay);
        } else {
            self.phase = Phase::WaitingRetry {
                until: self.now + delay,
                reason,
            };
        }
    }

    fn wait_for_network(&mut self, reason: RetryReason, delay_after_path: Duration) {
        self.phase = Phase::WaitingNetwork {
            reason,
            delay_after_path,
        };
    }

    fn fail(&mut self, failure: Failure) {
        self.phase = Phase::Failed(failure);
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
