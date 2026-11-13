//! The watchdog on Windows with openconnect's output reading: a client started without a window,
//! its output read, its process tree killed, and an attempt's two runs and logout request. The fake
//! clients are cmd scripts, so the test executable starts nothing but cmd.exe and what the scripts
//! start.
#![cfg(windows)]

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use fvm_backend_adapter::openconnect::{Interpreter, OpenConnect};
use fvm_contract::app_helper::{
    ExitFacts, HelperEventKind, Milestone, OutputSource, RunRequest, StopInitiator, StopReason,
};
use fvm_contract::backend::{
    Backend, ClientExecutable, ClientProblem, LaunchSpec, RouteRepairOutcome, SessionLaunch,
    UntestedVersion,
};
use fvm_watchdog::{Report, Run, Timing, Watchdog};

const CMD: &str = r"C:\Windows\System32\cmd.exe";
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

/// Opens the tunnel with the session from stdin and stays up in a child process that holds its
/// stderr, as a route script would.
const TUNNEL: &str = "@echo off\r
set /p SESSION=\r
>>\"%~1\" echo tunnel_cookie=%SESSION%\r
echo Got Legacy IP address 10.10.0.5\r
echo Configured as 10.10.0.5, with SSL disconnected and DTLS established\r
>&2 echo Using Wintun device 'vpn', index 5\r
ping -n 120 127.0.0.1 >nul\r
";

/// Answers like the adapter's curl request. Printing without a line break leaves error level 1,
/// so it exits explicitly, as curl does after an answer.
const LOGOUT: &str = "@echo off\r
set /p HEADER=\r
>>\"%~1\" echo logout_input=%HEADER%\r
<nul set /p \"=HTTP 200\"\r
exit /b 0\r
";

/// openconnect as far as the watchdog can tell, but the client is the test's cmd script.
struct Scripted(LaunchSpec);

impl Backend for Scripted {
    type Interpreter = Interpreter;

    fn client_name(&self) -> &'static str {
        OpenConnect.client_name()
    }

    fn locate_client(&self) -> Result<ClientExecutable, ClientProblem> {
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
    ) -> Result<LaunchSpec, String> {
        Ok(self.0.clone())
    }

    fn check_otp(&self, code: &str) -> Result<(), String> {
        OpenConnect.check_otp(code)
    }

    fn interpreter(&self) -> Interpreter {
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
            "fvm-watchdog-windows-{}-{}",
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
    run: Run<Scripted>,
    reports: Receiver<Report>,
    lines: Vec<(OutputSource, String)>,
    milestones: Vec<Milestone>,
    exit: Option<ExitFacts>,
}

impl Session {
    fn start(scratch: &Scratch, launch: LaunchSpec) -> Self {
        let mut watchdog =
            Watchdog::new(Scripted(launch), scratch.run_directory(), Timing::default());
        let mut request = RunRequest {
            attempt_id: "E621E1F8-C36C-495A-93FC-0C247A3E6E5F".parse().unwrap(),
            host: "203.0.113.10".to_owned(),
            port: 443,
            username: "alice".to_owned(),
            realm: None,
            trusted_certs: Vec::new(),
            password: PASSWORD.to_owned(),
        };
        let (sender, reports) = mpsc::channel();
        let started = watchdog.start(&mut request, move |report| {
            let _ = sender.send(report);
        });
        let (run, _) = started.unwrap();
        Self {
            run,
            reports,
            lines: Vec::new(),
            milestones: Vec::new(),
            exit: None,
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
                }
                Ok(Report::Event(HelperEventKind::Exited { facts })) => self.exit = Some(facts),
                Ok(Report::RepairRoutes(repair)) => repair.finish(RouteRepairOutcome::NotNeeded),
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
    let values = scratch.values();
    assert_eq!(values["password"], PASSWORD);
    assert_eq!(values["otp"], OTP);
    assert_eq!(values["tunnel_cookie"], COOKIE);

    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(facts.furthest_milestone, Some(Milestone::TunnelUp));
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
