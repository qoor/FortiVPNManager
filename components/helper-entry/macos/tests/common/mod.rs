//! A test backend that runs the watchdog's fake openfortivpn scripts with the openfortivpn adapter's
//! checks and output reading, and a fake Swift side that drives the entry through the
//! Helper-internal protocol and records everything it sends.

#![allow(dead_code)] // Each test file uses a different part.

use std::io::{self, BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::{env, fs, process};

use fvm_backend_adapter::openfortivpn::{Interpreter, OpenFortiVpn};
use fvm_contract::app_helper::RunRequest;
use fvm_contract::backend::{Backend, ClientExecutable, ClientProblem, LaunchSpec, StopPolicy};
use fvm_contract::stream;
use serde_json::{Value, json};

pub const ATTEMPT: &str = "E621E1F8-C36C-495A-93FC-0C247A3E6E5F";
pub const OTHER_ATTEMPT: &str = "0B6C3D51-2F4A-4E8B-9C7D-1A2B3C4D5E6F";
/// An IPv4 literal, so the gateway lookup needs no DNS.
pub const GATEWAY: &str = "203.0.113.10";
/// Leading and trailing spaces and non-ASCII must reach the client byte for byte.
pub const PASSWORD: &str = " pässwörd 🔑 ";
pub const OTP: &str = "042137";
pub const REJECTED_DIGEST: &str =
    "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11";

pub const SPAWNED: u64 = 0;
pub const CONNECTED_TO_GATEWAY: u64 = 1;
pub const OTP_PROMPTED: u64 = 2;
pub const TUNNEL_UP: u64 = 5;

/// Generous, so a broken test fails instead of hanging.
const WAIT_LIMIT: Duration = Duration::from_secs(10);

/// openfortivpn as far as the entry can tell: the adapter checks the request, builds the config file
/// and reads the output, but the client is a fixture script run by /bin/sh.
pub struct TestBackend {
    client: Result<ClientExecutable, ClientProblem>,
    fixture: PathBuf,
    args: Vec<String>,
    record: PathBuf,
    grace_period: Duration,
}

impl Backend for TestBackend {
    type Interpreter = Interpreter;

    fn client_name(&self) -> &'static str {
        OpenFortiVpn.client_name()
    }

    fn locate_client(&self) -> Result<ClientExecutable, ClientProblem> {
        self.client.clone()
    }

    fn describe_problem(&self, problem: &ClientProblem) -> String {
        OpenFortiVpn.describe_problem(problem)
    }

    fn launch_spec(
        &self,
        client: &ClientExecutable,
        request: &RunRequest,
        config_path: &Path,
    ) -> Result<LaunchSpec, String> {
        let checked = OpenFortiVpn.launch_spec(client, request, config_path)?;
        let mut args = vec![
            self.fixture.display().to_string(),
            self.record.display().to_string(),
        ];
        args.extend(self.args.iter().cloned());
        Ok(LaunchSpec {
            program: PathBuf::from("/bin/sh"),
            args,
            ..checked
        })
    }

    fn check_otp(&self, code: &str) -> Result<(), String> {
        OpenFortiVpn.check_otp(code)
    }

    fn interpreter(&self) -> Interpreter {
        OpenFortiVpn.interpreter()
    }

    fn stop_policy(&self) -> StopPolicy {
        StopPolicy {
            grace_period: self.grace_period,
            ..OpenFortiVpn.stop_policy()
        }
    }

    /// Every path, so that only the process identity keeps a leftover alive.
    fn is_own_executable(&self, _: &Path) -> bool {
        true
    }
}

pub struct Setup {
    pub fixture: &'static str,
    pub args: Vec<&'static str>,
    pub client: Result<ClientExecutable, ClientProblem>,
    pub grace_period: Duration,
}

impl Setup {
    pub fn fixture(fixture: &'static str) -> Self {
        Self {
            fixture,
            args: Vec::new(),
            client: Ok(ClientExecutable {
                path: PathBuf::from("/bin/sh"),
                version: Some("1.24.1".to_owned()),
            }),
            grace_period: OpenFortiVpn.stop_policy().grace_period,
        }
    }

    pub fn args(self, args: &[&'static str]) -> Self {
        Self {
            args: args.to_vec(),
            ..self
        }
    }
}

static SCRATCH_NUMBER: AtomicU64 = AtomicU64::new(0);

/// A directory per test for the run directory and the fixture's record file.
pub struct Scratch {
    pub root: PathBuf,
}

impl Scratch {
    pub fn new() -> Self {
        let number = SCRATCH_NUMBER.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!("fvm-helper-entry-{}-{number}", process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    pub fn run_directory(&self) -> PathBuf {
        self.root.join("run")
    }

    /// The `key=value` lines the fixture writes: lengths and digests of what it read, never the
    /// secrets themselves.
    pub fn record_value(&self, key: &str) -> Option<String> {
        let text = fs::read_to_string(self.root.join("record.txt")).unwrap_or_default();
        text.lines().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name == key).then(|| value.to_owned())
        })
    }

    pub fn run_directory_names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.run_directory())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub struct Swift {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    /// Every message from the entry, in order.
    pub received: Vec<Value>,
    last_request: u64,
    pub scratch: Scratch,
}

impl Swift {
    /// Starts an entry and waits for it to be ready.
    pub fn start(setup: Setup) -> Self {
        let mut swift = Self::launch(Scratch::new(), setup);
        swift.wait_until("ready", |swift| swift.position(is("ready")).is_some());
        swift
    }

    /// Starts an entry in `scratch`, which may hold what an earlier Helper left.
    pub fn launch(scratch: Scratch, setup: Setup) -> Self {
        let backend = TestBackend {
            client: setup.client,
            fixture: fixture_path(setup.fixture),
            args: setup.args.iter().map(|&arg| arg.to_owned()).collect(),
            record: scratch.root.join("record.txt"),
            grace_period: setup.grace_period,
        };
        let stream = fvm_helper_entry_macos::start(backend, scratch.run_directory());
        let reader = BufReader::new(stream.try_clone().unwrap());
        Self {
            stream,
            reader,
            received: Vec::new(),
            last_request: 0,
            scratch,
        }
    }

    pub fn send(&self, message: Value) {
        stream::send(&mut &self.stream, &message).unwrap();
    }

    /// The next message, if one starts arriving within `timeout`. Only the wait for its first bytes
    /// has the timeout, so a timeout never cuts a message in two.
    pub fn receive_within(&mut self, timeout: Duration) -> Option<Value> {
        let socket = self.reader.get_ref();
        socket
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))
            .unwrap();
        match self.reader.fill_buf() {
            Ok([]) => panic!("the entry closed its end"),
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                return None;
            }
            Err(error) => panic!("reading from the entry: {error}"),
        }
        self.reader.get_ref().set_read_timeout(None).unwrap();
        let message: Value = stream::receive(&mut self.reader).unwrap();
        self.received.push(message.clone());
        Some(message)
    }

    /// Receives until `condition` holds for everything received so far.
    pub fn wait_until(&mut self, what: &str, condition: impl Fn(&Self) -> bool) {
        self.wait_until_within(WAIT_LIMIT, what, condition);
    }

    pub fn wait_until_within(
        &mut self,
        timeout: Duration,
        what: &str,
        condition: impl Fn(&Self) -> bool,
    ) {
        let deadline = Instant::now() + timeout;
        while !condition(self) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "no {what} within {timeout:?}: {:#?}",
                self.received
            );
            self.receive_within(remaining);
        }
    }

    /// What arrives within `duration`.
    pub fn received_within(&mut self, duration: Duration) -> Vec<Value> {
        let deadline = Instant::now() + duration;
        let mut messages = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return messages;
            }
            messages.extend(self.receive_within(remaining));
        }
    }

    pub fn position(&self, matches: impl Fn(&Value) -> bool) -> Option<usize> {
        self.received.iter().position(matches)
    }

    /// Announces app connection `app` and returns whether the entry admitted it.
    pub fn connect_app(&mut self, app: u64, uid: u32) -> bool {
        self.send(json!({"appConnected": {"app": app, "uid": uid}}));
        self.wait_until("the admission", |swift| {
            swift
                .position(|message| message["admitted"]["app"] == app)
                .is_some()
        });
        let index = self
            .position(|message| message["admitted"]["app"] == app)
            .unwrap();
        self.received[index]["admitted"]["accept"]
            .as_bool()
            .unwrap()
    }

    pub fn disconnect_app(&self, app: u64) {
        self.send(json!({"appDisconnected": {"app": app}}));
    }

    /// Sends `request` from `app` and returns its id; does not wait for the reply.
    pub fn request(&mut self, app: u64, request: Value) -> u64 {
        self.last_request += 1;
        let id = self.last_request;
        self.send(json!({"request": {"id": id, "app": app, "request": request}}));
        id
    }

    /// Every reply to request `id`.
    pub fn replies(&self, id: u64) -> Vec<&Value> {
        self.received
            .iter()
            .filter(|message| message["reply"]["id"] == id || message["replyFlag"]["id"] == id)
            .collect()
    }

    /// The error a connect, submitOtp or disconnect reply carries, `None` for success.
    pub fn reply(&mut self, id: u64) -> Option<Value> {
        self.wait_until("the reply", |swift| !swift.replies(id).is_empty());
        let reply = &self.replies(id)[0]["reply"];
        assert!(!reply.is_null(), "request {id} got a flag: {reply}");
        reply["json"]
            .as_str()
            .map(|json| serde_json::from_str(json).unwrap())
    }

    /// The JSON of a status reply.
    pub fn reply_json(&mut self, id: u64) -> Value {
        self.reply(id).expect("a status reply carries JSON")
    }

    pub fn reply_flag(&mut self, id: u64) -> bool {
        self.wait_until("the reply", |swift| !swift.replies(id).is_empty());
        self.replies(id)[0]["replyFlag"]["value"]
            .as_bool()
            .expect("an exitWhenIdle reply is a flag")
    }

    /// Sends a connect request for `attempt` and returns its reply.
    pub fn connect(&mut self, app: u64, attempt: &str) -> Option<Value> {
        let id = self.request(app, connect_request(attempt));
        self.reply(id)
    }

    pub fn status(&mut self, app: u64) -> Value {
        let id = self.request(app, json!({"status": {}}));
        self.reply_json(id)
    }

    /// The `HelperEvent`s sent to `app`.
    pub fn events(&self, app: u64) -> Vec<Value> {
        self.received
            .iter()
            .filter(|message| message["sendEvent"]["app"] == app)
            .map(|message| embedded(&message["sendEvent"]["json"]))
            .collect()
    }

    pub fn milestones(&self, app: u64) -> Vec<u64> {
        self.events(app)
            .iter()
            .filter_map(|event| event["kind"]["milestone"]["_0"].as_u64())
            .collect()
    }

    pub fn wait_for_milestone(&mut self, app: u64, milestone: u64) {
        self.wait_until("the milestone", |swift| {
            swift.milestones(app).contains(&milestone)
        });
    }

    pub fn exit_facts(&self, app: u64) -> Option<Value> {
        self.events(app)
            .into_iter()
            .find_map(|event| Some(event["kind"].get("exited")?["_0"].clone()))
    }

    pub fn wait_for_exit(&mut self, app: u64) -> Value {
        self.wait_until("the exit", |swift| swift.exit_facts(app).is_some());
        self.exit_facts(app).unwrap()
    }

    /// The Log tab lines sent to `app`.
    pub fn output(&self, app: u64) -> Vec<String> {
        self.received
            .iter()
            .filter(|message| message["sendOutput"]["app"] == app)
            .flat_map(|message| {
                embedded(&message["sendOutput"]["json"])
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|line| line["text"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Unified-log lines as `level category: message`.
    pub fn logs(&self) -> Vec<String> {
        self.received
            .iter()
            .filter_map(|message| {
                let log = message.get("log")?;
                Some(format!(
                    "{} {}: {}",
                    log["level"].as_str().unwrap(),
                    log["category"].as_str().unwrap(),
                    log["message"].as_str().unwrap()
                ))
            })
            .collect()
    }

    pub fn has_log(&self, line: &str) -> bool {
        self.logs().iter().any(|logged| logged == line)
    }

    pub fn count(&self, matches: impl Fn(&Value) -> bool) -> usize {
        self.received
            .iter()
            .filter(|&message| matches(message))
            .count()
    }

    /// Nothing the entry sent holds the password or the OTP.
    pub fn assert_no_secret(&self) {
        for message in &self.received {
            let text = message.to_string();
            assert!(
                !text.contains(PASSWORD) && !text.contains(OTP),
                "secret in {text}"
            );
        }
    }
}

/// Whether a message is the case `name` of `FromEntry`.
pub fn is(name: &'static str) -> impl Fn(&Value) -> bool {
    move |message| message.get(name).is_some()
}

/// The JSON a message carries as text.
fn embedded(json: &Value) -> Value {
    serde_json::from_str(json.as_str().unwrap()).unwrap()
}

pub fn connect_request(attempt: &str) -> Value {
    connect_request_with(json!({
        "attemptID": attempt,
        "host": GATEWAY,
        "port": 443,
        "username": "alice",
        "trustedCerts": [],
        "password": PASSWORD,
    }))
}

pub fn connect_request_with(run_request: Value) -> Value {
    json!({"connect": {"json": run_request.to_string()}})
}

pub fn submit_otp(attempt: &str, code: &str) -> Value {
    json!({"submitOtp": {"attempt": attempt, "code": code}})
}

pub fn disconnect(attempt: &str, reason: &str) -> Value {
    json!({"disconnect": {"attempt": attempt, "reason": reason}})
}

pub fn error(code: &str, message: &str) -> Option<Value> {
    Some(json!({"code": code, "message": message}))
}

pub fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../watchdog/tests/fixtures/{name}.sh"))
}

pub fn sha256(text: &str) -> String {
    let output = Command::new("/bin/sh")
        .args(["-c", "printf '%s' \"$1\" | /sbin/sha256 -q", "sh", text])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
