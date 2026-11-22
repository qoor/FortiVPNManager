//! The connection state machine: what the user wants, where the connection is, and how each input
//! moves it.
//!
//! Timers, the activity hold and saving the ledger follow from the state after each input, so no
//! transition can forget to cancel a timer or save the ledger.

mod attempts;
mod helper;
mod otp;
mod outcomes;
mod presence;
mod wake;

use std::collections::BTreeMap;
use std::mem;
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime};

use fvm_contract::app_helper::{AttemptId, Milestone, SessionId, StopReason};
use fvm_contract::app_internal::port::RegistrationError;
use jiff::tz::TimeZone;

use super::ledger::{Failure, FailureLedger};
use super::policy;
use super::readiness::{FollowApproval, HelperFacts, SetupNeed};
use super::system::{PathSnapshot, RawPath, Underlay, WakeSettle};
use super::{
    ConnectionSettings, Effect, Input, Launch, Note, OtpFailure, Settings, Startup, TimerId,
    TimerKind, VpnAccount,
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
    pub(super) sleeping: bool,
    last_wake_at: Option<SystemTime>,
    /// A wake not judged yet, with how long the system slept.
    pending_wake: Option<Duration>,
    quit_requested_at: Option<SystemTime>,
    terminated: bool,
    pub(super) update_stop: Option<UpdateStop>,
    /// Since when the user has been away; `None` while present.
    pub(super) away_since: Option<SystemTime>,
    /// The away limit was reached: no automatic reconnect until Connect, Reconnect Now or the next
    /// scheduled start.
    pub(super) reconnect_suspended: bool,
    /// The latest schedule start or end acted on.
    schedule_boundary: Option<SystemTime>,
    network_failure_streak: u32,
    /// The immediate reconnect after a forced termination was used and no tunnel has come up since.
    forced_retry_used: bool,
    /// The attempt whose failure the phase shows, for a late spam-folder finding.
    last_failure_attempt: Option<AttemptId>,
    /// The code email the next login waits for.
    stale_otp: Option<StaleOtp>,
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
    otp_submitted: bool,
    /// The OTP session is open and this attempt's; it closes itself after the submission.
    otp_session_open: bool,
    otp_failure: Option<OtpFailure>,
    otp_in_spam: bool,
    /// The stop the app asked for; the exit is still classified.
    stop_requested: Option<StopReason>,
    /// The account when the attempt began: its readiness check was for it and its run uses it.
    pub(super) account: VpnAccount,
    pub(super) replacing: Option<Session>,
}

impl Attempt {
    /// Closes the session if it is open. Once the credentials went out and no code was used, the
    /// gateway may have sent a code email that arrives late, within the next prompt's arrival
    /// tolerance. With `search`, the session goes on looking for that email to trash it, from the
    /// prompt or else from when the credentials went out (user decision 2026-11-16). The callers
    /// turn it off on the app's quit and the Mac's sleep, at the OTP time limit and after a
    /// rejected login.
    fn close_otp_session(&mut self, check_spam: bool, search: bool) -> Option<Effect> {
        if !mem::take(&mut self.otp_session_open) {
            return None;
        }
        Some(match self.credentials_sent_at {
            Some(sent_at) if search && self.milestone <= Some(Milestone::OtpPrompted) => {
                Effect::TrashStaleOtp {
                    attempt: self.id.clone(),
                    since: self.otp_prompted_at.unwrap_or(sent_at),
                }
            }
            _ => Effect::CloseOtpSession {
                attempt: self.id.clone(),
                check_spam,
            },
        })
    }
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

/// What the attempt announcement and the schedule notes compare before and after an input.
struct NoteMarks {
    /// The attempt under way.
    attempt: Option<AttemptId>,
    scheduled_attempt: Option<AttemptId>,
    holding: bool,
    suspended: bool,
}

impl ConnectionManager {
    #[must_use]
    pub fn new(startup: Startup) -> Self {
        let ledger = FailureLedger::new(startup.settings.account.ledger_key());
        Self {
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
            sleeping: false,
            last_wake_at: None,
            pending_wake: None,
            quit_requested_at: None,
            terminated: false,
            update_stop: None,
            away_since: None,
            reconnect_suspended: false,
            schedule_boundary: None,
            network_failure_streak: 0,
            forced_retry_used: false,
            last_failure_attempt: None,
            stale_otp: None,
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
            Input::NetworkProbed(result) => self.network_probed(&result),
        }
    }

    fn finish(&mut self, marks: NoteMarks) -> Vec<Effect> {
        self.reconcile_timers();
        // A pending stale-code search keeps its polling and its time limit on time.
        let hold = !self.terminated && (self.phase.holds_activity() || self.stale_otp.is_some());
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
        let mut effects = match &self.phase {
            Phase::Connecting(attempt) if marks.attempt.as_ref() != Some(&attempt.id) => {
                vec![Effect::AttemptBegun {
                    attempt: attempt.id.clone(),
                    session: attempt.session,
                }]
            }
            _ => Vec::new(),
        };
        effects.append(&mut self.notes(&marks));
        effects.append(&mut self.effects);
        effects
    }

    fn note_marks(&self) -> NoteMarks {
        NoteMarks {
            attempt: match &self.phase {
                Phase::Connecting(attempt) => Some(attempt.id.clone()),
                _ => None,
            },
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

    fn notes(&self, before: &NoteMarks) -> Vec<Effect> {
        let Some(schedule) = &self.settings.connection.schedule else {
            return Vec::new();
        };
        let after = self.note_marks();
        let mut notes = Vec::new();
        if after.scheduled_attempt.is_some() && after.scheduled_attempt != before.scheduled_attempt
        {
            notes.push(Effect::Note {
                attempt: after.scheduled_attempt,
                note: Note::ScheduledHoursStarted,
            });
        }
        if after.holding && !before.holding {
            notes.push(Effect::Note {
                attempt: None,
                note: Note::ReconnectHeldWhileAway,
            });
        }
        if after.suspended && !before.suspended {
            notes.push(Effect::Note {
                attempt: None,
                note: Note::ReconnectsSuspended {
                    away_minutes: schedule.away_minutes,
                },
            });
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
                wanted.insert(TimerKind::AttemptDeadline, attempt.deadline);
                if attempt.stage == Stage::Run
                    && !attempt.otp_submitted
                    && let Some(prompted) = attempt.otp_prompted_at
                {
                    wanted.insert(TimerKind::OtpDeadline, prompted + policy::OTP_DEADLINE);
                }
            }
            Phase::Connected(session) if !self.sleeping && self.pending_wake.is_none() => {
                let next = match &session.renewal {
                    None if self.automatic_reconnect_allowed() => session
                        .planned_reconnect
                        .map(|planned| (TimerKind::ProactiveReconnect, planned)),
                    Some(renewal)
                        if !renewal.reason.is_automatic_reconnect()
                            || self.automatic_reconnect_allowed() =>
                    {
                        renewal.next_try.map(|at| (TimerKind::Retry, at))
                    }
                    _ => None,
                };
                wanted.extend(next);
            }
            Phase::WaitingRetry { until, .. } => {
                wanted.insert(TimerKind::Retry, *until);
            }
            _ => {}
        }
        if let Some(stale) = &self.stale_otp {
            wanted.insert(TimerKind::StaleOtp, stale.until);
        }
        let stopping_for_update = self
            .update_stop
            .filter(|stop| !stop.stopped)
            .map(|stop| stop.requested_at);
        if let Some(requested) = self.quit_requested_at.or(stopping_for_update) {
            wanted.insert(TimerKind::QuitDeadline, requested + policy::QUIT_TIMEOUT);
        }
        if let Some(schedule) = &self.settings.connection.schedule
            && self.quit_requested_at.is_none()
            && self.update_stop.is_none()
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
                self.renew(session, AttemptReason::ReconnectNow);
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
        self.quit_requested_at.get_or_insert(self.now);
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
                self.terminated = true;
            }
        }
    }

    fn stop_for_update(&mut self) {
        if self.quit_requested_at.is_some() {
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
                self.settle(AfterStop::Update);
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
        if self.quit_requested_at.is_some() {
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
        self.network_failure_streak = 0;
        self.forced_retry_used = false;
        self.reconnect_suspended = false;
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
            TimerKind::StaleOtp => self.stale_otp_wait_ran_out(),
            TimerKind::Retry => match self.take_phase() {
                Phase::WaitingRetry { reason, .. } => {
                    self.begin_attempt(AttemptReason::Retry(reason), false, false);
                }
                Phase::Connected(session) => self.prepare_replacement(session, false),
                phase => self.phase = phase,
            },
            TimerKind::ProactiveReconnect => self.proactive_reconnect_due(),
            TimerKind::QuitDeadline if self.quit_requested_at.is_some() => self.terminated = true,
            TimerKind::QuitDeadline => self.stopped_for_update(),
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
            let close = attempt.close_otp_session(false, true);
            let replacing = attempt.replacing.take();
            self.push_otp_close(close);
            match replacing {
                Some(session) => self.replacement_failed(session),
                None => self.network_failure(RetryReason::Network),
            }
            return;
        }
        attempt.stop_requested = Some(StopReason::AttemptTimeout);
        self.effects.push(Effect::StopRun {
            attempt: attempt.id.clone(),
            reason: StopReason::AttemptTimeout,
        });
        let close = attempt.close_otp_session(false, true);
        self.push_otp_close(close);
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
        // The spam check tells at once whether the code went there; no search for it.
        let close = attempt.close_otp_session(true, false);
        self.push_otp_close(close);
    }

    fn proactive_reconnect_due(&mut self) {
        if !matches!(self.phase, Phase::Connected(_)) || !self.automatic_reconnect_allowed() {
            return;
        }
        let Phase::Connected(session) = self.take_phase() else {
            unreachable!("matched above");
        };
        self.renew(session, AttemptReason::Proactive);
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
