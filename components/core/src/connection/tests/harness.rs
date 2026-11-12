//! Drives the manager the way the runtime does: one input at a time with a controllable clock,
//! keeping the timers it asks for and answering what it asks.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fvm_contract::app_helper::{
    AttemptId, ExitFacts, HelperEvent, HelperEventKind, HelperStatus, Milestone, ProcessExitStatus,
    StopInitiator, StopReason,
};
use fvm_contract::app_internal::event::{InterfaceKind, PathInterface};
use fvm_contract::app_internal::port::InterfaceAddress;
use fvm_contract::app_internal::{OtpMethod, SecretText, TimeOfDay, Weekday};
use jiff::civil::date;
use jiff::tz::{self, TimeZone};

use super::super::machine::{Attempt, Phase, RetryReason, Session, Stop};
use super::super::system::Underlay;
use super::super::{
    ConnectionManager, ConnectionSettings, Effect, FailureLedger, HelperFacts, Input, Launch,
    ReadinessFacts, Schedule, Settings, Startup, TimerId, TimerKind, VpnAccount,
};

pub(super) const APP_BUILD: &str = "100";
pub(super) const TUNNEL_IP: Ipv4Addr = Ipv4Addr::new(10, 212, 134, 5);

pub(super) fn t0() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_900_000_000)
}

pub(super) fn secs(seconds: f64) -> Duration {
    Duration::from_secs_f64(seconds)
}

/// Seoul has no daylight saving time; the schedule scenarios are written in it.
pub(super) fn seoul() -> TimeZone {
    TimeZone::fixed(tz::offset(9))
}

/// Local time in Seoul in March 2030: the 18th is a Monday (and holds t0), the 22nd a Friday, the
/// 23rd a Saturday.
pub(super) fn at(day: i8, hour: i8, minute: i8, second: i8) -> SystemTime {
    in_zone(&seoul(), 2030, 3, day, hour, minute, second)
}

pub(super) fn in_zone(
    zone: &TimeZone,
    year: i16,
    month: i8,
    day: i8,
    hour: i8,
    minute: i8,
    second: i8,
) -> SystemTime {
    let zoned = date(year, month, day)
        .at(hour, minute, second, 0)
        .to_zoned(zone.clone())
        .unwrap();
    SystemTime::from(zoned.timestamp())
}

/// The `n`-th attempt ID of the all-zero seed.
pub(super) fn id(n: u64) -> AttemptId {
    let counter = format!("{n:016X}");
    format!("00000000-0000-0000-{}-{}", &counter[..4], &counter[4..])
        .parse()
        .unwrap()
}

/// An attempt the manager never issued.
pub(super) fn stranger() -> AttemptId {
    "DEAD0000-0000-0000-0000-0000000000FF".parse().unwrap()
}

pub(super) fn digest(fill: char) -> String {
    fill.to_string().repeat(64)
}

pub(super) fn pin_old() -> String {
    digest('0')
}

pub(super) fn wifi() -> Underlay {
    Underlay {
        interface: "en0".to_owned(),
        address: Some(Ipv4Addr::new(192, 168, 0, 10)),
    }
}

pub(super) fn other_wifi() -> Underlay {
    Underlay {
        interface: "en0".to_owned(),
        address: Some(Ipv4Addr::new(10, 1, 2, 3)),
    }
}

pub(super) fn ethernet() -> Underlay {
    Underlay {
        interface: "en7".to_owned(),
        address: Some(Ipv4Addr::new(192, 168, 1, 20)),
    }
}

pub(super) fn account() -> VpnAccount {
    VpnAccount {
        host: "vpn.example.com".to_owned(),
        port: 8443,
        username: "alice".to_owned(),
        realm: String::new(),
        trusted_certs: vec![pin_old()],
    }
}

pub(super) fn bob() -> VpnAccount {
    VpnAccount {
        username: "bob".to_owned(),
        ..account()
    }
}

pub(super) const LEDGER_KEY: &str = "alice@vpn.example.com";

pub(super) fn empty_ledger() -> FailureLedger {
    FailureLedger::new(LEDGER_KEY.to_owned())
}

/// The `connection.*` defaults, with the schedule off.
pub(super) fn connection_settings() -> ConnectionSettings {
    ConnectionSettings {
        session_max_minutes: 180,
        proactive_reconnect: true,
        proactive_margin_minutes: 5,
        reconnect_after_sleep: false,
        min_sleep_minutes: 30,
        schedule: None,
    }
}

pub(super) fn settings() -> Settings {
    Settings {
        account: account(),
        connection: connection_settings(),
        otp_method: OtpMethod::Imap,
    }
}

/// Monday to Friday, 09:00 to 18:00; reconnects stop after 120 minutes away.
pub(super) fn weekday_hours() -> Schedule {
    Schedule {
        days: vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ],
        start: TimeOfDay { hour: 9, minute: 0 },
        end: TimeOfDay {
            hour: 18,
            minute: 0,
        },
        away_minutes: 120,
    }
}

pub(super) fn scheduled(schedule: Schedule) -> Settings {
    let mut settings = settings();
    settings.connection.schedule = Some(schedule);
    settings
}

pub(super) fn ready() -> ReadinessFacts {
    ReadinessFacts {
        config_valid: true,
        account_complete: true,
        vpn_password_stored: true,
        gmail_credential_stored: true,
        helper: HelperFacts::Answered {
            build: APP_BUILD.to_owned(),
            status: Some(HelperStatus {
                client_running: false,
                client_path: Some("/opt/homebrew/Cellar/vpn/1.24.1/bin/vpn".to_owned()),
                client_version: Some("1.24.1".to_owned()),
                client_problem: None,
                untested_version: None,
            }),
        },
    }
}

/// How a run ended, for the exit facts.
#[derive(Clone)]
pub(super) struct Exit {
    furthest: Option<Milestone>,
    initiator: StopInitiator,
    up: Option<Duration>,
    certs: Vec<String>,
    cert_at: Option<Milestone>,
    status: Option<ProcessExitStatus>,
    tunnel_mode_rejected: bool,
}

/// The client exited on its own after reaching `furthest`.
pub(super) fn ended(furthest: Milestone) -> Exit {
    Exit {
        furthest: Some(furthest),
        initiator: StopInitiator::Client {},
        up: None,
        certs: Vec::new(),
        cert_at: None,
        status: None,
        tunnel_mode_rejected: false,
    }
}

pub(super) fn spawn_failed() -> Exit {
    Exit {
        furthest: None,
        ..ended(Milestone::Spawned)
    }
}

/// Killed with SIGKILL by someone outside the app and the Helper.
pub(super) fn killed(furthest: Milestone) -> Exit {
    ended(furthest).status(ProcessExitStatus::Signaled { signal: 9 })
}

impl Exit {
    pub(super) fn by_helper(mut self, reason: StopReason) -> Self {
        self.initiator = StopInitiator::Helper { reason };
        self
    }

    pub(super) fn up(mut self, seconds: f64) -> Self {
        self.up = Some(secs(seconds));
        self
    }

    pub(super) fn certs(mut self, certs: &[String], at: Option<Milestone>) -> Self {
        self.certs = certs.to_vec();
        self.cert_at = at;
        self
    }

    pub(super) fn status(mut self, status: ProcessExitStatus) -> Self {
        self.status = Some(status);
        self
    }

    pub(super) fn tunnel_mode_rejected(mut self) -> Self {
        self.tunnel_mode_rejected = true;
        self
    }

    pub(super) fn facts(self) -> ExitFacts {
        ExitFacts {
            furthest_milestone: self.furthest,
            stop_initiator: self.initiator,
            cert_rejected_digests: self.certs,
            cert_rejected_at: self.cert_at,
            tunnel_mode_rejected: self.tunnel_mode_rejected,
            up_duration: self.up,
            exit_status: self.status.unwrap_or(match self.furthest {
                None => ProcessExitStatus::SpawnFailed { errno: 2 },
                Some(_) => ProcessExitStatus::Exited { code: 1 },
            }),
        }
    }
}

/// The network path the OS reports, and the route to the gateway it leads to.
#[derive(Clone)]
pub(super) struct Path {
    pub(super) satisfied: bool,
    pub(super) underlay: Option<Underlay>,
}

pub(super) fn online() -> Path {
    online_via(wifi())
}

pub(super) fn online_via(underlay: Underlay) -> Path {
    Path {
        satisfied: true,
        underlay: Some(underlay),
    }
}

pub(super) fn offline() -> Path {
    Path {
        satisfied: false,
        underlay: None,
    }
}

/// What the runtime finds after a wake.
#[derive(Clone)]
pub(super) struct Observe {
    pub(super) alive: bool,
    pub(super) tunnel: bool,
    pub(super) underlay: Option<Underlay>,
    pub(super) settled: bool,
}

pub(super) fn observe() -> Observe {
    Observe {
        alive: true,
        tunnel: true,
        underlay: Some(wifi()),
        settled: true,
    }
}

impl Observe {
    pub(super) fn dead(self) -> Self {
        Self {
            alive: false,
            tunnel: false,
            ..self
        }
    }

    pub(super) fn alive(self, alive: bool) -> Self {
        Self { alive, ..self }
    }

    pub(super) fn tunnel(self, tunnel: bool) -> Self {
        Self { tunnel, ..self }
    }

    pub(super) fn via(self, underlay: Option<Underlay>) -> Self {
        Self { underlay, ..self }
    }

    pub(super) fn unsettled(self) -> Self {
        Self {
            settled: false,
            ..self
        }
    }
}

pub(super) struct Harness {
    pub(super) cm: ConnectionManager,
    pub(super) now: SystemTime,
    /// The effects of the last input.
    pub(super) effects: Vec<Effect>,
    timers: HashMap<TimerKind, (TimerId, SystemTime)>,
    wake_at: Option<SystemTime>,
    /// The manager asked for the router and has no answer yet.
    router_asked: bool,
}

impl Harness {
    /// Online over Wi-Fi from t0, with the default settings.
    pub(super) fn new() -> Self {
        Self::with(settings(), Some(online()))
    }

    pub(super) fn with_settings(settings: Settings) -> Self {
        Self::with(settings, Some(online()))
    }

    /// The path, when given, is reported and judged before t0.
    pub(super) fn with(settings: Settings, path: Option<Path>) -> Self {
        let mut harness = Self {
            cm: ConnectionManager::new(Startup {
                app_build: APP_BUILD.to_owned(),
                attempt_seed: [0; 16],
                settings,
                time_zone: seoul(),
                kills_show_as_signals: true,
            }),
            now: t0() - Duration::from_secs(3),
            effects: Vec::new(),
            timers: HashMap::new(),
            wake_at: None,
            router_asked: false,
        };
        match path {
            Some(path) => {
                harness.set_path(path);
            }
            None => harness.now = t0(),
        }
        assert_eq!(harness.now, t0());
        harness
    }

    pub(super) fn send(&mut self, input: Input) -> Vec<Effect> {
        if matches!(input, Input::RouterRead { .. }) {
            self.router_asked = false;
        }
        self.effects = self.cm.handle(input, self.now);
        self.router_asked |= self.effects.contains(&Effect::ReadRouter);
        for effect in &self.effects {
            match effect {
                Effect::SetTimer { timer, at } => {
                    self.timers.insert(timer.kind, (*timer, *at));
                }
                Effect::CancelTimer(kind) => {
                    self.timers.remove(kind);
                }
                _ => {}
            }
        }
        self.effects.clone()
    }

    pub(super) fn advance(&mut self, seconds: f64) {
        self.now += secs(seconds);
    }

    /// Moves the clock to the timer's time, if later, and fires it.
    pub(super) fn fire(&mut self, kind: TimerKind) -> Vec<Effect> {
        let (timer, at) = self.timers.remove(&kind).unwrap_or_else(|| {
            panic!("{kind:?} is not set: {:?}", self.cm.phase);
        });
        self.now = self.now.max(at);
        self.send(Input::TimerFired(timer))
    }

    /// Fires the timer `seconds` before its time and leaves the clock there, like a timer slept on
    /// the monotonic clock while the wall clock was slewed.
    pub(super) fn fire_early(&mut self, kind: TimerKind, seconds: f64) -> Vec<Effect> {
        let (timer, at) = self.timers.remove(&kind).unwrap();
        self.now = at - secs(seconds);
        self.send(Input::TimerFired(timer))
    }

    pub(super) fn timer(&self, kind: TimerKind) -> Option<SystemTime> {
        self.timers.get(&kind).map(|(_, at)| *at)
    }

    pub(super) fn timer_id(&self, kind: TimerKind) -> TimerId {
        self.timers[&kind].0
    }

    /// The timers the connection rules set, without the ones that observe the system.
    pub(super) fn connection_timers(&self) -> Vec<TimerKind> {
        let mut kinds: Vec<TimerKind> = self
            .timers
            .keys()
            .copied()
            .filter(|kind| {
                !matches!(
                    kind,
                    TimerKind::PathSettle
                        | TimerKind::WakeSettle
                        | TimerKind::LockPoll
                        | TimerKind::ApprovalFollow
                )
            })
            .collect();
        kinds.sort();
        kinds
    }

    /// The effect that sets `kind` to `at` now.
    pub(super) fn set(&self, kind: TimerKind, at: SystemTime) -> Effect {
        Effect::SetTimer {
            timer: self.timer_id(kind),
            at,
        }
    }

    pub(super) fn launch(&mut self) -> Vec<Effect> {
        self.launch_with(true, None)
    }

    pub(super) fn launch_with(
        &mut self,
        auto_connect: bool,
        restored_ledger: Option<FailureLedger>,
    ) -> Vec<Effect> {
        self.send(Input::Launched(Launch {
            auto_connect,
            restored_ledger,
            import_report_shown: false,
        }))
    }

    pub(super) fn attempt(&self) -> Option<&Attempt> {
        match &self.cm.phase {
            Phase::Connecting(attempt) => Some(attempt),
            _ => None,
        }
    }

    pub(super) fn connected(&self) -> Option<&Session> {
        match &self.cm.phase {
            Phase::Connected(session) => Some(session),
            _ => None,
        }
    }

    pub(super) fn stopping(&self) -> Option<&Stop> {
        match &self.cm.phase {
            Phase::Disconnecting(stop) => Some(stop),
            _ => None,
        }
    }

    pub(super) fn retry_wait(&self) -> Option<(SystemTime, RetryReason)> {
        match self.cm.phase {
            Phase::WaitingRetry { until, reason } => Some((until, reason)),
            _ => None,
        }
    }

    pub(super) fn network_wait(&self) -> Option<(RetryReason, Duration)> {
        match self.cm.phase {
            Phase::WaitingNetwork {
                reason,
                delay_after_path,
            } => Some((reason, delay_after_path)),
            _ => None,
        }
    }

    /// The attempt under way, or the one whose run the manager follows.
    pub(super) fn current_id(&self) -> AttemptId {
        if let Some(attempt) = self.attempt() {
            return attempt.id.clone();
        }
        if let Some(session) = self.connected() {
            return session.attempt.clone();
        }
        stranger()
    }

    pub(super) fn milestone(&mut self, milestone: Milestone) -> Vec<Effect> {
        let id = self.current_id();
        self.milestone_for(&id, milestone, None)
    }

    pub(super) fn milestone_for(
        &mut self,
        id: &AttemptId,
        milestone: Milestone,
        tunnel_ip: Option<Ipv4Addr>,
    ) -> Vec<Effect> {
        self.send(Input::Helper(HelperEvent {
            attempt_id: id.clone(),
            kind: HelperEventKind::Milestone {
                milestone,
                tunnel_ip,
            },
        }))
    }

    /// The exit of the run under way.
    pub(super) fn exit(&mut self, exit: Exit) -> Vec<Effect> {
        let id = self
            .stopping()
            .map_or_else(|| self.current_id(), stop_attempt);
        self.exit_for(&id, exit)
    }

    pub(super) fn exit_for(&mut self, id: &AttemptId, exit: Exit) -> Vec<Effect> {
        self.send(Input::Helper(HelperEvent {
            attempt_id: id.clone(),
            kind: HelperEventKind::Exited {
                facts: exit.facts(),
            },
        }))
    }

    /// Answers the readiness check if one is pending, then the OTP session's start.
    pub(super) fn start_tunnel(&mut self) -> AttemptId {
        let attempt = self
            .attempt()
            .unwrap_or_else(|| panic!("not connecting: {:?}", self.cm.phase))
            .clone();
        if attempt.stage == super::super::machine::Stage::CheckingReadiness {
            self.send(Input::ReadinessChecked(ready()));
        }
        let effects = self.send(Input::OtpSessionReady(attempt.id.clone()));
        assert_eq!(run_starts(&effects).len(), 1, "{effects:?}");
        attempt.id
    }

    /// Reports the milestones up to and including `last`, submitting a code at the prompt; up to
    /// the tunnel it takes 12 seconds.
    pub(super) fn run_milestones(&mut self, id: &AttemptId, last: Milestone) {
        let all = [
            Milestone::Spawned,
            Milestone::ConnectedToGateway,
            Milestone::OtpPrompted,
            Milestone::OtpSubmitted,
            Milestone::Authenticated,
            Milestone::TunnelUp,
        ];
        for milestone in all.into_iter().filter(|milestone| *milestone <= last) {
            match milestone {
                Milestone::Spawned | Milestone::ConnectedToGateway | Milestone::OtpPrompted => {
                    self.advance(1.0);
                    self.milestone_for(id, milestone, None);
                }
                Milestone::OtpSubmitted => {
                    self.advance(5.0);
                    let effects = self.send(Input::OtpCodeFound(
                        id.clone(),
                        SecretText("123456".to_owned()),
                    ));
                    assert!(submits_otp(&effects), "{effects:?}");
                    self.advance(1.0);
                    self.milestone_for(id, milestone, None);
                }
                Milestone::Authenticated => {
                    self.advance(1.0);
                    self.milestone_for(id, milestone, None);
                }
                Milestone::TunnelUp => {
                    self.advance(2.0);
                    self.milestone_for(id, milestone, Some(TUNNEL_IP));
                }
            }
        }
    }

    pub(super) fn run_to_connected(&mut self) {
        let id = self.start_tunnel();
        self.run_milestones(&id, Milestone::TunnelUp);
        assert!(
            self.connected().is_some(),
            "not connected: {:?}",
            self.cm.phase
        );
    }

    pub(super) fn launch_connected(&mut self) {
        self.launch();
        self.run_to_connected();
    }

    /// Ends the session `up` seconds after it came up, the clock moved there first.
    pub(super) fn forcibly_end(&mut self, end: ForcedEnd, up: f64) -> Vec<Effect> {
        let since = self.connected().expect("connected").since;
        self.now = since + secs(up);
        match end {
            ForcedEnd::ClientKilled => self.exit(killed(Milestone::TunnelUp).up(up)),
            ForcedEnd::HelperLost => self.send(Input::HelperConnectionLost),
        }
    }

    /// The OS reports `path`; after it stayed unchanged long enough, the manager looks at it.
    /// Takes 3 seconds.
    pub(super) fn set_path(&mut self, path: Path) -> Vec<Effect> {
        self.send(Input::NetworkPathChanged {
            satisfied: path.satisfied,
            interfaces: path_interfaces(),
        });
        let effects = self.fire(TimerKind::PathSettle);
        if !path.satisfied {
            return effects;
        }
        let probe = probe_id(&effects);
        self.answer_probe(probe, path.underlay.as_ref(), true, None)
    }

    pub(super) fn sleep(&mut self) -> Vec<Effect> {
        self.send(Input::WillSleep { at: self.now })
    }

    pub(super) fn wake_time(&self) -> SystemTime {
        self.wake_at.expect("woke")
    }

    pub(super) fn wake(&mut self) -> Vec<Effect> {
        self.wake_at = Some(self.now);
        self.send(Input::DidWake)
    }

    /// Answers the looks at the network after the wake until the manager judges it; returns the
    /// effects of that judgment. Unsettled, it takes the 30 seconds the manager waits.
    pub(super) fn settle(&mut self, observation: Observe) -> Vec<Effect> {
        let deadline = self.wake_at.expect("woke") + Duration::from_secs(30);
        if observation.settled {
            loop {
                if self.router_asked {
                    let effects = self.send(Input::RouterRead { present: true });
                    let probe = probe_id(&effects);
                    return self.answer_probe(
                        probe,
                        observation.underlay.as_ref(),
                        observation.tunnel,
                        Some(observation.alive),
                    );
                }
                self.fire(TimerKind::WakeSettle);
            }
        }
        loop {
            if self.router_asked {
                let effects = self.send(Input::RouterRead { present: false });
                if self.now >= deadline {
                    return effects;
                }
            } else if self.now >= deadline {
                return self.effects.clone();
            } else {
                self.fire(TimerKind::WakeSettle);
            }
        }
    }

    fn answer_probe(
        &mut self,
        probe: u64,
        underlay: Option<&Underlay>,
        tunnel_up: bool,
        client_running: Option<bool>,
    ) -> Vec<Effect> {
        let mut addresses = vec![
            address("lo0", true, Some(Ipv4Addr::LOCALHOST)),
            address("en0", true, Some(Ipv4Addr::new(192, 168, 0, 10))),
            address("ppp0", tunnel_up, tunnel_up.then_some(TUNNEL_IP)),
        ];
        if let Some(underlay) = underlay {
            addresses.retain(|entry| entry.name != underlay.interface);
            addresses.push(address(&underlay.interface, true, underlay.address));
        }
        self.send(Input::NetworkProbed(super::super::ProbeResult {
            id: probe,
            gateway_ip: Some(Ipv4Addr::new(203, 0, 113, 7)),
            route_interface: underlay.map(|underlay| underlay.interface.clone()),
            addresses,
            client_running,
        }))
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ForcedEnd {
    /// The VPN client killed with a signal.
    ClientKilled,
    /// The Helper process ended, killed or crashed.
    HelperLost,
}

pub(super) const FORCED_ENDS: [ForcedEnd; 2] = [ForcedEnd::ClientKilled, ForcedEnd::HelperLost];

fn stop_attempt(stop: &Stop) -> AttemptId {
    // The stop keeps its attempt private to the machine; its Debug output names it.
    let text = format!("{stop:?}");
    let start = text.find("AttemptId(\"").expect("an attempt") + "AttemptId(\"".len();
    text[start..start + 36].parse().unwrap()
}

pub(super) fn address(name: &str, up: bool, ipv4: Option<Ipv4Addr>) -> InterfaceAddress {
    InterfaceAddress {
        name: name.to_owned(),
        up,
        ipv4,
    }
}

pub(super) fn path_interfaces() -> Vec<PathInterface> {
    vec![
        PathInterface {
            name: "en0".to_owned(),
            kind: InterfaceKind::Wifi,
        },
        PathInterface {
            name: "en7".to_owned(),
            kind: InterfaceKind::WiredEthernet,
        },
        PathInterface {
            name: "utun3".to_owned(),
            kind: InterfaceKind::Other,
        },
    ]
}

pub(super) fn probe_id(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::ProbeNetwork(probe) => Some(probe.id),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no probe: {effects:?}"))
}

pub(super) fn run_starts(effects: &[Effect]) -> Vec<VpnAccount> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::StartRun { account, .. } => Some(account.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn stop_reasons(effects: &[Effect]) -> Vec<StopReason> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::StopRun { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect()
}

pub(super) fn submits_otp(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::SubmitOtp { .. }))
}

pub(super) fn opens_otp_session(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::OpenOtpSession { .. }))
}

pub(super) fn checks_readiness(effects: &[Effect]) -> bool {
    effects.contains(&Effect::CheckReadiness)
}

pub(super) fn saved_ledger(effects: &[Effect]) -> Option<FailureLedger> {
    effects.iter().rev().find_map(|effect| match effect {
        Effect::SaveLedger(ledger) => Some(ledger.clone()),
        _ => None,
    })
}

pub(super) fn saved_certs(effects: &[Effect]) -> Option<Vec<String>> {
    effects.iter().rev().find_map(|effect| match effect {
        Effect::SaveTrustedCerts(certs) => Some(certs.clone()),
        _ => None,
    })
}

pub(super) fn timer_set(effects: &[Effect], kind: TimerKind) -> Option<SystemTime> {
    effects.iter().find_map(|effect| match effect {
        Effect::SetTimer { timer, at } if timer.kind == kind => Some(*at),
        _ => None,
    })
}

pub(super) fn cancels(effects: &[Effect], kind: TimerKind) -> bool {
    effects.contains(&Effect::CancelTimer(kind))
}

/// The connection effects, without the ones that read the system.
pub(super) fn connection_effects(effects: &[Effect]) -> Vec<Effect> {
    effects
        .iter()
        .filter(|effect| {
            !matches!(
                effect,
                Effect::ReadScreenState | Effect::ReadRouter | Effect::ProbeNetwork(_)
            ) && !matches!(
                effect,
                Effect::SetTimer { timer, .. } if matches!(
                    timer.kind,
                    TimerKind::PathSettle | TimerKind::WakeSettle | TimerKind::LockPoll
                )
            ) && !matches!(
                effect,
                Effect::CancelTimer(
                    TimerKind::PathSettle | TimerKind::WakeSettle | TimerKind::LockPoll
                )
            )
        })
        .cloned()
        .collect()
}

pub(super) fn retry(reason: RetryReason) -> super::super::machine::AttemptReason {
    super::super::machine::AttemptReason::Retry(reason)
}
