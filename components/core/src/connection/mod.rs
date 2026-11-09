//! The Connection Manager's rules: user requests, attempt progress, classifying how a run ended,
//! retries and the failure limit, proactive reconnects, sleep and wake, network changes, the
//! schedule and being away, trusting the gateway certificate, what the connection and schedule
//! sections show, and readiness.
//!
//! The runtime creates one [`ConnectionManager`] per launch and hands it every [`Input`] in order,
//! one at a time, with the wall-clock time it took the input. It carries out the returned
//! [`Effect`]s in order before the next input; their results come back as inputs. After each input
//! it reads the sections ([`ConnectionManager::connection_section`],
//! [`ConnectionManager::schedule_outlook`], [`ConnectionManager::readiness_section`]) and sends the
//! ones that changed. The manager reads no clock, opens no connection and runs no timer itself.

mod certs;
mod classify;
mod ledger;
mod machine;
mod policy;
mod readiness;
mod schedule;
mod system;
mod view;

#[cfg(test)]
mod tests;

use std::net::Ipv4Addr;
use std::time::SystemTime;

use fvm_contract::app_helper::{AttemptId, HelperErrorCode, HelperEvent, RunRequest, StopReason};
use fvm_contract::app_internal::event::PathInterface;
use fvm_contract::app_internal::port::{InterfaceAddress, RegistrationError};
use fvm_contract::app_internal::{OtpMethod, SecretText};
use jiff::tz::TimeZone;

pub use certs::{TRUSTED_CERT_LIMIT, gateway_change_drops_certs, merge_trusted_certs};
pub use ledger::{Failure, FailureLedger};
pub use machine::ConnectionManager;
pub use readiness::{HelperFacts, ReadinessFacts};
pub use schedule::Schedule;

/// What the manager starts from, once per launch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Startup {
    /// The build the Helper must also report.
    pub app_build: String,
    /// Random per launch; attempt IDs derive from it so the manager stays deterministic.
    pub attempt_seed: [u8; 16],
    pub settings: Settings,
    /// The system time zone, which the schedule's times are read in.
    pub time_zone: TimeZone,
}

/// The settings the manager acts on, as the settings file holds them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Settings {
    pub account: VpnAccount,
    pub connection: ConnectionSettings,
    /// How OTP emails are read; it names the Gmail credential in what the UI shows.
    pub otp_method: OtpMethod,
}

/// The VPN account and the gateway certificate values trusted for it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VpnAccount {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// Empty for none.
    pub realm: String,
    /// Newest first, at most [`TRUSTED_CERT_LIMIT`].
    pub trusted_certs: Vec<String>,
}

impl VpnAccount {
    /// The key the failure ledger is kept under.
    pub fn ledger_key(&self) -> String {
        format!("{}@{}", self.username, self.host)
    }

    /// The Helper's run request for `attempt`, with the password from the secret storage.
    pub fn run_request(&self, attempt: AttemptId, password: String) -> RunRequest {
        RunRequest {
            attempt_id: attempt,
            host: self.host.clone(),
            port: i64::from(self.port),
            username: self.username.clone(),
            realm: (!self.realm.is_empty()).then(|| self.realm.clone()),
            trusted_certs: self.trusted_certs.clone(),
            password,
        }
    }

    fn same_gateway(&self, other: &Self) -> bool {
        self.host == other.host && self.port == other.port
    }

    /// The account a password and the failure ledger belong to.
    fn same_identity(&self, other: &Self) -> bool {
        self.same_gateway(other) && self.username == other.username
    }

    /// Everything a run logs in with, apart from the trusted certificate values.
    fn same_login(&self, other: &Self) -> bool {
        self.same_identity(other) && self.realm == other.realm
    }
}

/// The `connection.*` options.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ConnectionSettings {
    pub session_max_minutes: u32,
    pub proactive_reconnect: bool,
    pub proactive_margin_minutes: u32,
    pub reconnect_after_sleep: bool,
    pub min_sleep_minutes: u32,
    /// `None` while the schedule is off.
    pub schedule: Option<Schedule>,
}

/// Facts the first input of a launch carries.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Launch {
    /// `connection.auto_connect_on_launch`.
    pub auto_connect: bool,
    /// The ledger state.json holds, if any.
    pub restored_ledger: Option<FailureLedger>,
    /// The first config section carries the report of the automatic first-launch import.
    pub import_report_shown: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Input {
    /// Always the first input.
    Launched(Launch),
    /// The settings file changed. Changes that only the readiness facts reflect, such as the file
    /// becoming invalid or the Gmail address changing, come as [`Input::RecheckReadiness`].
    SettingsChanged(Settings),
    TimeZoneChanged(TimeZone),

    Connect,
    Disconnect,
    ReconnectNow,
    /// The user or the system asked the app to quit.
    QuitRequested,
    SettingsOpened,
    AppActivated,
    /// Something readiness rests on may have changed outside the settings this manager holds:
    /// the settings file's validity, the Gmail settings, a secret, the Google authorization or the
    /// OAuth client.
    RecheckReadiness,
    /// The result of registering the Helper as a system service.
    HelperRegistered(Result<(), RegistrationError>),
    /// The system settings where the Helper is approved were opened.
    HelperApprovalSettingsOpened,

    /// Answers [`Effect::CheckReadiness`].
    ReadinessChecked(ReadinessFacts),

    /// The OTP session took its starting point and waits for the prompt.
    OtpSessionReady(AttemptId),
    OtpSessionFailed(AttemptId, OtpFailure),
    OtpCodeFound(AttemptId, SecretText),
    /// After [`Effect::CloseOtpSession`] with `check_spam`: the OTP email was in the spam folder.
    OtpFoundInSpam(AttemptId),

    /// A milestone or the exit of a run, from the connection that carries the runs.
    Helper(HelperEvent),
    /// The Helper did not take [`Effect::StartRun`], or the run request could not be made, such
    /// as without a VPN password; no exit follows. Channel failures come as
    /// [`HelperErrorCode::InternalError`].
    RunRejected(AttemptId, HelperErrorCode),
    /// The connection that carries the runs went away; no exit follows for its runs.
    HelperConnectionLost,

    /// A timer of [`Effect::SetTimer`] is due. A replaced or cancelled one is ignored.
    TimerFired(TimerId),

    /// The system is about to sleep; `at` is when the OS reported it.
    WillSleep {
        at: SystemTime,
    },
    DidWake,
    ScreenLocked {
        at: SystemTime,
    },
    ScreenUnlocked,
    /// Every display went to sleep.
    DisplaysSlept {
        at: SystemTime,
    },
    DisplaysWoke,
    /// Answers [`Effect::ReadScreenState`].
    ScreenState {
        locked: bool,
        displays_asleep: bool,
    },
    /// The system's network path, on every update the OS reports.
    NetworkPathChanged {
        satisfied: bool,
        interfaces: Vec<PathInterface>,
    },
    /// Answers [`Effect::ReadRouter`]: whether the primary network has an IPv4 router.
    RouterRead {
        present: bool,
    },
    /// Answers [`Effect::ProbeNetwork`].
    NetworkProbed(ProbeResult),
}

/// How an OTP session failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OtpFailure {
    /// Gmail refused the credential.
    Authentication,
    Network,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Effect {
    /// Gather the readiness facts and answer with [`Input::ReadinessChecked`]. A request made
    /// while a check runs is served by one more check after it. On a Helper of another build, the
    /// check asks [`ConnectionManager::should_restart_helper`] whether to have it exit when idle and
    /// asks the build again.
    CheckReadiness,
    /// Start the OTP session of `attempt`, taking its starting point in the mailbox.
    OpenOtpSession {
        attempt: AttemptId,
    },
    /// The OTP prompt appeared at `prompted_at`; look for the code.
    SearchOtp {
        attempt: AttemptId,
        prompted_at: SystemTime,
    },
    /// Close the session of `attempt`. With `check_spam`, first look in the spam folder once and
    /// answer [`Input::OtpFoundInSpam`] if the OTP email is there.
    CloseOtpSession {
        attempt: AttemptId,
        check_spam: bool,
    },
    /// Read the VPN password of `account` and ask the Helper to start the run
    /// ([`VpnAccount::run_request`]).
    StartRun {
        attempt: AttemptId,
        account: VpnAccount,
    },
    /// Hand the code to the run, then move the session's OTP emails to the trash and close it.
    SubmitOtp {
        attempt: AttemptId,
        code: SecretText,
    },
    /// The run's end is its exit event, not the reply.
    StopRun {
        attempt: AttemptId,
        reason: StopReason,
    },
    /// Write `vpn.trusted_certs` with exactly these values.
    SaveTrustedCerts(Vec<String>),
    /// Write the ledger to state.json.
    SaveLedger(FailureLedger),
    /// Replaces any pending timer of the same kind.
    SetTimer {
        timer: TimerId,
        at: SystemTime,
    },
    CancelTimer(TimerKind),
    /// Keep the app from being suspended or throttled while a connection is wanted.
    HoldActivity(bool),
    /// Answer with [`Input::RouterRead`].
    ReadRouter,
    /// Answer with [`Input::NetworkProbed`].
    ProbeNetwork(NetworkProbe),
    /// Answer with [`Input::ScreenState`].
    ReadScreenState,
    /// A line for the Log tab.
    Note(Note),
    /// Show the settings window (`FromRuntime::OpenSettings`), after the sections of this input.
    OpenSettings,
    /// Finish writing the log file, then let the app quit. Always the last effect; the manager
    /// takes no input after it.
    ApproveQuit,
}

/// A timer the manager asked for; the runtime keeps at most one per kind.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TimerId {
    pub kind: TimerKind,
    pub generation: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum TimerKind {
    AttemptDeadline,
    OtpDeadline,
    Retry,
    ProactiveReconnect,
    QuitDeadline,
    /// The next start or end of the scheduled hours.
    Schedule,
    /// When the away user reaches the schedule's away time.
    AwayLimit,
    /// The network path stayed unchanged long enough to look at.
    PathSettle,
    /// The next look at the network after a wake.
    WakeSettle,
    /// The next look at the screen while it is locked, in case the unlock went unreported.
    LockPoll,
    /// The next readiness check while following the Helper's approval.
    ApprovalFollow,
}

impl TimerKind {
    const ALL: [Self; 11] = [
        Self::AttemptDeadline,
        Self::OtpDeadline,
        Self::Retry,
        Self::ProactiveReconnect,
        Self::QuitDeadline,
        Self::Schedule,
        Self::AwayLimit,
        Self::PathSettle,
        Self::WakeSettle,
        Self::LockPoll,
        Self::ApprovalFollow,
    ];
}

/// What to find out about the network.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NetworkProbe {
    pub id: u64,
    /// The gateway to find the route to: the interface the routing table uses for it and the
    /// interface addresses. `None` when no server address is set.
    pub gateway: Option<GatewayLookup>,
    /// Also ask the Helper whether the VPN client is running.
    pub client_running: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GatewayLookup {
    pub host: String,
    /// The address resolved earlier for `host`. Without it the runtime resolves `host`; inside the
    /// tunnel, DNS may answer with another address, so it is resolved once per host.
    pub known_ip: Option<Ipv4Addr>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProbeResult {
    pub id: u64,
    /// As known or resolved; `None` when it could not be resolved.
    pub gateway_ip: Option<Ipv4Addr>,
    pub route_interface: Option<String>,
    pub addresses: Vec<InterfaceAddress>,
    /// When asked: whether the VPN client runs, `false` when the Helper did not answer.
    pub client_running: Option<bool>,
}

/// Schedule decisions the Log tab shows, so that a quiet evening can be told from a failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Note {
    ScheduledHoursStarted,
    /// Outside the scheduled hours with the user away: the reconnect waits for the user.
    ReconnectHeldWhileAway,
    /// Away for `away_minutes` outside the scheduled hours: automatic reconnects stopped.
    ReconnectsSuspended {
        away_minutes: u32,
    },
}
