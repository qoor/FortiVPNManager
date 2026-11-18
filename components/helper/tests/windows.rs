//! The session manager on Windows with openconnect's output reading: a client started in a new
//! console, its output read, its process tree killed, and an attempt's two runs and logout request.
//! The fake clients are cmd scripts, so the test executable starts nothing but cmd.exe and what the
//! scripts start.
#![cfg(windows)]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_contract::app_helper::{
    ExitFacts, HelperEventKind, Milestone, OutputSource, ProcessExitStatus, RunRequest, SessionId,
    StopInitiator, StopReason,
};
use fvm_contract::backend::{
    Backend, ClientCheckWait, ClientExecutable, ClientProblem, LaunchSpec, NetworkSettings,
    OutputInterpreter, RouteRepairOutcome, SessionLaunch, UntestedVersion,
};
use fvm_contract::helper_internal::{NetworkChange, NetworkOutcome};
use fvm_helper_core::recovery::RecordSlot;
use fvm_helper_core::session::{Report, Run, SessionManager, Timing};

const CMD: &str = r"C:\Windows\System32\cmd.exe";
const CSCRIPT: &str = r"C:\Windows\System32\cscript.exe";
const PASSWORD: &str = "Pa55 w#rd";
const OTP: &str = "042137";
const COOKIE: &str = "SVPNCOOKIE=Ab12Cd34Ef56";
const WAIT_LIMIT: Duration = Duration::from_secs(30);

/// Logs in the way openconnect's login run does: progress and prompts on stderr, the session lines
/// on stdout.
const LOGIN: &str = "@echo off\r
>&2 echo Connected to HTTPS on 203.0.113.10 with ciphersuite (TLS1.3)\r
>&2 <nul set /p \"=Password: \"\r
set /p PW=\r
>>\"%~1\" echo password=%PW%\r
>&2 echo.\r
>&2 <nul set /p \"=Code: \"\r
set /p OTP=\r
>>\"%~1\" echo otp=%OTP%\r
echo COOKIE='SVPNCOOKIE=Ab12Cd34Ef56'\r
echo HOST='203.0.113.10'\r
";

/// Opens the tunnel with the session from stdin, passes its network values through the value
/// script and stays up in a child process that holds its stderr.
const TUNNEL: &str = "@echo off\r
set /p SESSION=\r
>>\"%~1\" echo tunnel_cookie=%SESSION%\r
echo Got Legacy IP address 10.10.0.5\r
echo Configured as 10.10.0.5, with SSL disconnected and DTLS established\r
>&2 echo Using Wintun device 'vpn', index 5\r
set reason=connect\r
set VPNGATEWAY=203.0.113.10\r
set TUNDEV=vpn\r
set TUNIDX=5\r
set INTERNAL_IP4_ADDRESS=10.10.0.5\r
set CISCO_DEF_DOMAIN=corp.example.com\r
set CISCO_BANNER=사내 VPN\r
C:\\Windows\\System32\\cscript.exe //nologo /e:JScript \"%VALUE_SCRIPT%\"\r
>>\"%~1\" echo values_left=%ERRORLEVEL%\r
ping -n 120 127.0.0.1 >nul\r
ping -n 120 127.0.0.1 >nul 2>nul\r
";

/// Answers like the adapter's curl request. Printing without a line break leaves error level 1,
/// so it exits explicitly, as curl does after an answer.
const LOGOUT: &str = "@echo off\r
set /p HEADER=\r
>>\"%~1\" echo logout_input=%HEADER%\r
<nul set /p \"=HTTP 200\"\r
exit /b 0\r
";

/// openconnect as far as the session manager can tell, but the client is the test's cmd script.
struct Scripted(LaunchSpec);

fn value_script() -> String {
    env::current_dir()
        .unwrap()
        .join(r"..\..\packaging\windows\vpnc-values.js")
        .display()
        .to_string()
}

impl Backend for Scripted {
    fn client_name(&self) -> &'static str {
        OpenConnect.client_name()
    }

    fn locate_client(&self, _: ClientCheckWait) -> Result<ClientExecutable, ClientProblem> {
        Ok(ClientExecutable {
            path: PathBuf::from(CMD),
            version: "9.21".to_owned(),
        })
    }

    fn describe_problem(&self, _: &ClientProblem) -> String {
        unreachable!("the script is always found")
    }

    fn untested_version(&self, _: &str) -> Option<UntestedVersion> {
        unreachable!("the tests do not ask for the client's version")
    }

    fn launch_spec(
        &self,
        _: &ClientExecutable,
        _: &RunRequest,
        _: &Path,
        values_path: &Path,
    ) -> Result<LaunchSpec, String> {
        let mut launch = self.0.clone();
        if let Some(session) = launch.session.as_mut() {
            let values = values_path.display().to_string();
            session.tunnel.env.extend([
                ("FVM_VALUES_FILE".to_owned(), values),
                ("VALUE_SCRIPT".to_owned(), value_script()),
            ]);
            session.tunnel.script_runner = Some(PathBuf::from(CSCRIPT));
        }
        Ok(launch)
    }

    fn network_settings(&self, values: &[u8]) -> Result<NetworkSettings, String> {
        OpenConnect.network_settings(values)
    }

    fn check_otp(&self, code: &str) -> Result<(), String> {
        OpenConnect.check_otp(code)
    }

    fn interpreter(&self) -> Box<dyn OutputInterpreter + Send> {
        OpenConnect.interpreter()
    }

    fn stop_grace_period(&self) -> Duration {
        OpenConnect.stop_grace_period()
    }

    fn is_own_executable(&self, path: &Path) -> bool {
        OpenConnect.is_own_executable(path)
    }
}

/// A directory of its own for each test's scripts, record file and run directory.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = env::temp_dir().join(format!(
            "fvm-process-windows-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn record(&self) -> PathBuf {
        self.0.join("record.txt")
    }

    fn run_directory(&self) -> PathBuf {
        self.0.join("run")
    }

    /// `cmd /d /c <script> <record file>`, with only the variables cmd and ping need.
    fn launch(&self, name: &str, script: &str) -> LaunchSpec {
        let path = self.0.join(format!("{name}.cmd"));
        fs::write(&path, script).unwrap();
        LaunchSpec {
            program: PathBuf::from(CMD),
            args: vec![
                "/d".to_owned(),
                "/c".to_owned(),
                path.display().to_string(),
                self.record().display().to_string(),
            ],
            env: ["SystemRoot", "PATH", "PATHEXT", "ComSpec"]
                .iter()
                .filter_map(|&name| Some((name.to_owned(), env::var(name).ok()?)))
                .collect(),
            config_file: None,
            session: None,
            script_runner: None,
        }
    }

    fn values(&self) -> HashMap<String, String> {
        fs::read_to_string(self.record())
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once('=')?;
                Some((key.to_owned(), value.trim_end().to_owned()))
            })
            .collect()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Session {
    run: Run,
    reports: Receiver<Report>,
    lines: Vec<(OutputSource, String)>,
    milestones: Vec<Milestone>,
    exit: Option<ExitFacts>,
    applied: Vec<NetworkSettings>,
    applied_at: Vec<Instant>,
    tunnel_up_at: Option<Instant>,
    reverts: usize,
}

impl Session {
    fn start(scratch: &Scratch, launch: LaunchSpec) -> Self {
        let mut sessions = SessionManager::new(
            Arc::new(Scripted(launch)),
            scratch.run_directory(),
            Timing::default(),
        );
        let mut request = RunRequest {
            attempt_id: "E621E1F8-C36C-495A-93FC-0C247A3E6E5F".parse().unwrap(),
            replacing: None,
            session_id: SessionId(277_070_470),
            host: "203.0.113.10".to_owned(),
            port: 443,
            username: "alice".to_owned(),
            realm: None,
            trusted_certs: Vec::new(),
            dtls: false,
            password: PASSWORD.to_owned(),
        };
        let (sender, reports) = mpsc::channel();
        let started = sessions.start(&mut request, RecordSlot::First, None, move |report| {
            let _ = sender.send(report);
        });
        let (run, _) = started.unwrap();
        Self {
            run,
            reports,
            lines: Vec::new(),
            milestones: Vec::new(),
            exit: None,
            applied: Vec::new(),
            applied_at: Vec::new(),
            tunnel_up_at: None,
            reverts: 0,
        }
    }

    /// The client processes started: the login run and, if it came to that, the tunnel run.
    fn client_starts(&self) -> usize {
        self.lines
            .iter()
            .filter(|(_, text)| {
                text.starts_with("‹attempt started: pid ")
                    || text.starts_with("‹tunnel run started: pid ")
            })
            .count()
    }

    /// Records reports, submitting the OTP when asked, until `done` holds.
    fn wait(&mut self, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + WAIT_LIMIT;
        while !done(self) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "timed out: {:#?}", self.lines);
            match self.reports.recv_timeout(left) {
                Ok(Report::Output(lines)) => self
                    .lines
                    .extend(lines.into_iter().map(|line| (line.source, line.text))),
                Ok(Report::Event(HelperEventKind::Milestone { milestone, .. })) => {
                    self.milestones.push(milestone);
                    if milestone == Milestone::OtpPrompted {
                        self.run.submit_otp(OTP.to_owned()).unwrap();
                    }
                    if milestone == Milestone::TunnelUp {
                        self.tunnel_up_at = Some(Instant::now());
                    }
                }
                Ok(Report::Event(HelperEventKind::Exited { facts })) => self.exit = Some(facts),
                Ok(Report::RepairRoutes(repair)) => repair.finish(RouteRepairOutcome::NotNeeded),
                Ok(Report::ChangeNetwork(job)) => {
                    let outcome = match &job.change {
                        NetworkChange::LookUp { .. } => NetworkOutcome::LookedUp {
                            dns_service_id: "svc".to_owned(),
                            missing: Vec::new(),
                        },
                        NetworkChange::Apply { settings, .. } => {
                            self.applied.push((**settings).clone());
                            self.applied_at.push(Instant::now());
                            NetworkOutcome::Applied {
                                added: Vec::new(),
                                left: Vec::new(),
                            }
                        }
                        NetworkChange::Revert { .. } => {
                            self.reverts += 1;
                            NetworkOutcome::Reverted { left: Vec::new() }
                        }
                        NetworkChange::Prepare { .. } | NetworkChange::Switch { .. } => {
                            panic!("a run that replaces none neither prepares nor switches")
                        }
                    };
                    job.finish(outcome);
                }
                Ok(Report::Log(..)) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => panic!("the run ended without an exit"),
            }
        }
    }

    fn wait_for_exit(&mut self) -> ExitFacts {
        self.wait(|session| session.exit.is_some());
        self.exit.clone().unwrap()
    }

    fn has_text(&self, text: &str) -> bool {
        self.lines.iter().any(|(_, shown)| shown == text)
    }
}

#[test]
fn an_attempt_runs_both_processes_and_logs_out_after_the_forced_stop() {
    let scratch = Scratch::new();
    let mut launch = scratch.launch("login", LOGIN);
    launch.session = Some(Box::new(SessionLaunch {
        tunnel: scratch.launch("tunnel", TUNNEL),
        logout: scratch.launch("logout", LOGOUT),
    }));
    let mut session = Session::start(&scratch, launch);
    session.wait(|session| session.milestones.contains(&Milestone::TunnelUp));
    assert_eq!(
        session.milestones,
        [
            Milestone::Spawned,
            Milestone::ConnectedToGateway,
            Milestone::OtpPrompted,
            Milestone::OtpSubmitted,
            Milestone::Authenticated,
            Milestone::TunnelUp,
        ]
    );
    assert_eq!(session.client_starts(), 2);
    assert_eq!(session.applied.len(), 1);
    let applied = &session.applied[0];
    assert_eq!(
        (applied.tunnel.name.as_str(), applied.tunnel.index),
        ("vpn", Some(5))
    );
    assert_eq!(applied.default_domains, ["corp.example.com"]);
    assert!(
        session.has_text("‹network settings applied›"),
        "{:#?}",
        session.lines
    );
    // One attempt: the client is named once, before the login run.
    let named: Vec<&str> = session
        .lines
        .iter()
        .map(|(_, text)| text.as_str())
        .filter(|text| text.starts_with("‹VPN client: "))
        .collect();
    assert_eq!(named, ["‹VPN client: openconnect 9.21›"]);
    assert_eq!(session.lines[0].1, named[0]);
    let deadline = Instant::now() + WAIT_LIMIT;
    while !scratch.values().contains_key("values_left") {
        assert!(
            Instant::now() < deadline,
            "the tunnel script never recorded values_left"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let values = scratch.values();
    assert_eq!(values["password"], PASSWORD);
    assert_eq!(values["otp"], OTP);
    assert_eq!(values["tunnel_cookie"], COOKIE);
    assert_eq!(values["values_left"], "0");
    assert_eq!(
        fs::read_dir(scratch.run_directory().join("values"))
            .unwrap()
            .count(),
        0
    );

    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(facts.furthest_milestone, Some(Milestone::TunnelUp));
    assert_eq!(session.reverts, 1);
    assert!(
        session.has_text("‹killed the process tree (reason: userDisconnect)›"),
        "{:#?}",
        session.lines
    );
    assert!(
        session.has_text("‹logout request sent: HTTP 200›"),
        "{:#?}",
        session.lines
    );
    assert_eq!(
        scratch.values()["logout_input"],
        format!("Cookie: {COOKIE}")
    );
    // The ping the tunnel script started held stderr; had it survived, the output would not end.
    assert!(
        !session
            .lines
            .iter()
            .any(|(_, text)| text.starts_with("‹output pipes still open")),
        "{:#?}",
        session.lines
    );
    assert!(
        !session
            .lines
            .iter()
            .any(|(_, text)| text.contains("Ab12Cd34Ef56")),
        "{:#?}",
        session.lines
    );
}

fn tunnel_with_a_sleeping_script(
    scratch: &Scratch,
    milliseconds: u32,
    after_values: &str,
) -> LaunchSpec {
    fs::write(
        scratch.0.join("sleep.js"),
        format!("WScript.Sleep({milliseconds});\r\n"),
    )
    .unwrap();
    let tunnel = TUNNEL
        .replace(
            "set reason=connect\r\n",
            "start \"\" /b C:\\Windows\\System32\\cscript.exe //nologo /e:JScript \"%~dp0sleep.js\"\r\n\
             set reason=connect\r\n",
        )
        .replace(
            ">>\"%~1\" echo values_left=%ERRORLEVEL%\r\n",
            &format!(">&2 echo {after_values}\r\n"),
        );
    let mut launch = scratch.launch("login", LOGIN);
    launch.session = Some(Box::new(SessionLaunch {
        tunnel: scratch.launch("tunnel", &tunnel),
        logout: scratch.launch("logout", LOGOUT),
    }));
    launch
}

#[test]
fn the_tunnel_is_up_only_once_every_script_runner_has_exited() {
    let scratch = Scratch::new();
    let launch = tunnel_with_a_sleeping_script(&scratch, 2500, "values written");
    let mut session = Session::start(&scratch, launch);
    session.wait(|session| session.milestones.contains(&Milestone::TunnelUp));
    let waited = session.tunnel_up_at.unwrap() - session.applied_at[0];
    assert!(waited >= Duration::from_secs(1), "{waited:?}");
    session.run.stop(StopReason::UserDisconnect);
    session.wait_for_exit();
}

#[test]
fn a_script_the_client_left_running_does_not_hold_the_tunnel_back() {
    let scratch = Scratch::new();
    let launch = tunnel_with_a_sleeping_script(
        &scratch,
        60_000,
        "Script did not complete within 10 seconds.",
    );
    let mut session = Session::start(&scratch, launch);
    session.wait(|session| session.milestones.contains(&Milestone::TunnelUp));
    let waited = session.tunnel_up_at.unwrap() - session.applied_at[0];
    assert!(waited < Duration::from_secs(5), "{waited:?}");
    session.run.stop(StopReason::UserDisconnect);
    session.wait_for_exit();
}

#[test]
fn an_adapter_already_using_the_tunnels_address_is_told_in_the_exit() {
    let scratch = Scratch::new();
    let tunnel = "@echo off\r
set /p SESSION=\r
echo Got Legacy IP address 10.10.0.5\r
echo Configured as 10.10.0.5, with SSL disconnected and DTLS established\r
>&2 echo Adapter \"OpenConnect 2\" / 23 is UP and using our IPv4 address. Cannot resolve.\r
exit /b 1\r
";
    let mut launch = scratch.launch("login", LOGIN);
    launch.session = Some(Box::new(SessionLaunch {
        tunnel: scratch.launch("tunnel", tunnel),
        logout: scratch.launch("logout", LOGOUT),
    }));
    let mut session = Session::start(&scratch, launch);
    let facts = session.wait_for_exit();
    assert!(facts.tunnel_address_in_use, "{:#?}", session.lines);
    assert_eq!(facts.stop_initiator, StopInitiator::Client {});
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 1 });
}

#[test]
fn a_repeated_password_prompt_ends_the_run_without_a_logout() {
    let scratch = Scratch::new();
    let login = "@echo off\r
>&2 <nul set /p \"=Password: \"\r
set /p PW=\r
>&2 echo Invalid credentials; try again.\r
>&2 <nul set /p \"=Password: \"\r
set /p PW=\r
";
    let mut launch = scratch.launch("login", login);
    launch.session = Some(Box::new(SessionLaunch {
        tunnel: scratch.launch("tunnel", TUNNEL),
        logout: scratch.launch("logout", LOGOUT),
    }));
    let mut session = Session::start(&scratch, launch);
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::PromptRepeated
        }
    );
    assert_eq!(session.client_starts(), 1);
    assert!(!scratch.values().contains_key("logout_input"));
}

#[test]
fn the_client_gets_only_the_launch_environment() {
    let scratch = Scratch::new();
    let launch = scratch.launch("environment", "@echo off\r\nset\r\n");
    let mut session = Session::start(&scratch, launch);
    session.wait_for_exit();
    let names: Vec<&str> = session
        .lines
        .iter()
        .filter(|(source, _)| *source == OutputSource::Stdout)
        .filter_map(|(_, text)| text.split_once('=').map(|(name, _)| name))
        .collect();
    assert!(
        names
            .iter()
            .any(|name| name.eq_ignore_ascii_case("SystemRoot"))
    );
    for inherited in ["USERNAME", "USERPROFILE", "APPDATA", "HTTPS_PROXY"] {
        assert!(
            !names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(inherited)),
            "{inherited} in {names:?}"
        );
    }
}
