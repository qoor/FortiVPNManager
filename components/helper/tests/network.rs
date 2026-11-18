#![cfg(target_os = "macos")]

use std::collections::VecDeque;
use std::fs;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use std::{env, process, thread};

use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_contract::app_helper::{
    ExitFacts, HelperEventKind, Milestone, OutputSource, RunRequest, SessionId, StopInitiator,
    StopReason,
};
use fvm_contract::backend::{
    Backend, ClientCheckWait, ClientExecutable, ClientProblem, IpNetwork, LaunchSpec,
    NetworkSettings, OutputInterpreter, TunnelDevice, UntestedVersion,
};
use fvm_contract::helper_internal::{
    NetworkChange, NetworkOutcome, NetworkStep, OutsideChanges, OutsideRoute,
};
use fvm_helper_core::process::{NetworkJob, Report, Run, StageLimits, Timing, Watchdog};
use fvm_helper_core::recovery::RecordSlot;
use serde_json::Value;

const WAIT_LIMIT: Duration = Duration::from_secs(10);
const RETRY: Duration = Duration::from_millis(200);
const VALUES: &str = "VPNGATEWAY=203.0.113.10 TUNDEV=utun9 INTERNAL_IP4_ADDRESS=10.10.0.5 \
                      INTERNAL_IP4_DNS=10.20.0.53 CISCO_DEF_DOMAIN=corp.example.com \
                      CISCO_SPLIT_INC=1 CISCO_SPLIT_INC_0_ADDR=10.20.0.0 \
                      CISCO_SPLIT_INC_0_MASK=255.255.0.0 CISCO_SPLIT_EXC=1 \
                      CISCO_SPLIT_EXC_0_ADDR=10.20.161.16 CISCO_SPLIT_EXC_0_MASK=255.255.255.255";
const OTHER_VALUES: &str = "VPNGATEWAY=203.0.113.10 TUNDEV=utun9 INTERNAL_IP4_ADDRESS=10.10.0.5 \
                            INTERNAL_IP4_DNS=10.20.0.53 CISCO_SPLIT_INC=1 \
                            CISCO_SPLIT_INC_0_ADDR=10.20.0.0 CISCO_SPLIT_INC_0_MASK=255.255.0.0 \
                            CISCO_SPLIT_EXC=1 CISCO_SPLIT_EXC_0_ADDR=10.20.170.0 \
                            CISCO_SPLIT_EXC_0_MASK=255.255.255.0";
const CONFIGURED: &str = "echo 'Got Legacy IP address 10.10.0.5'\n\
                          echo 'Configured as 10.10.0.5, with SSL connected and DTLS in progress'";

struct Scripted {
    client: String,
}

impl Backend for Scripted {
    fn client_name(&self) -> &'static str {
        OpenConnect.client_name()
    }

    fn locate_client(&self, _: ClientCheckWait) -> Result<ClientExecutable, ClientProblem> {
        Ok(ClientExecutable {
            path: PathBuf::from("/bin/sh"),
            version: "9.21".to_owned(),
        })
    }

    fn describe_problem(&self, _: &ClientProblem) -> String {
        unreachable!("the client is always found")
    }

    fn untested_version(&self, _: &str) -> Option<UntestedVersion> {
        unreachable!("the tests do not ask for the client's version")
    }

    fn launch_spec(
        &self,
        client: &ClientExecutable,
        request: &RunRequest,
        config_path: &Path,
        values_path: &Path,
    ) -> Result<LaunchSpec, String> {
        let tunnel = OpenConnect
            .launch_spec(client, request, config_path, values_path)?
            .session
            .expect("openconnect opens the tunnel in a run of its own")
            .tunnel;
        let script = tunnel
            .args
            .iter()
            .find_map(|arg| arg.strip_prefix("--script="))
            .expect("the tunnel run gets the value script")
            .to_owned();
        Ok(LaunchSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec![
                "-c".to_owned(),
                self.client.clone(),
                "openconnect".to_owned(),
                script,
            ],
            ..tunnel
        })
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
        Duration::from_secs(2)
    }

    fn is_own_executable(&self, _: &Path) -> bool {
        false
    }
}

fn connect(values: &str) -> String {
    format!("reason=connect {values} /bin/sh -c \"$1\"")
}

fn wait_for_go() -> &'static str {
    "read go < \"$GO\""
}

struct Session {
    run: Run,
    reports: Receiver<Report>,
    directory: PathBuf,
    lines: Vec<(OutputSource, String)>,
    milestones: Vec<Milestone>,
    exit: Option<ExitFacts>,
    jobs: VecDeque<NetworkJob>,
}

impl Session {
    fn start(client: &str) -> Self {
        Self::start_with_tunnel_stage(client, Duration::from_secs(5))
    }

    fn start_with_tunnel_stage(client: &str, auth_to_tunnel: Duration) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = env::temp_dir().join(format!(
            "fvm-network-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir(&directory).unwrap();
        let go = directory.join("go");
        let made = process::Command::new("/usr/bin/mkfifo")
            .arg(&go)
            .status()
            .unwrap();
        assert!(made.success());
        let client = format!("GO='{}'\n{client}", go.display());
        let stage = Duration::from_secs(5);
        let timing = Timing {
            stages: StageLimits {
                spawn_to_gateway: stage,
                gateway_to_prompt_or_auth: stage,
                otp_prompt_to_submit: stage,
                otp_submit_to_auth: stage,
                auth_to_tunnel,
            },
            output_batch: Duration::from_millis(20),
            line_idle_flush: Duration::from_millis(250),
            pipe_drain: Duration::from_secs(1),
            network_values: Duration::from_secs(1),
            network_retry: RETRY,
        };
        let mut watchdog =
            Watchdog::new(Arc::new(Scripted { client }), directory.join("run"), timing);
        let mut request = RunRequest {
            attempt_id: "E621E1F8-C36C-495A-93FC-0C247A3E6E5F".parse().unwrap(),
            replacing: None,
            session_id: SessionId(277_070_470),
            host: "203.0.113.10".to_owned(),
            port: 443,
            username: "alice".to_owned(),
            realm: None,
            trusted_certs: vec![
                "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=".to_owned(),
            ],
            dtls: false,
            password: "secret".to_owned(),
        };
        let (sender, reports) = mpsc::channel();
        let (run, _) = watchdog
            .start(&mut request, RecordSlot::First, None, move |report| {
                let _ = sender.send(report);
            })
            .unwrap();
        Self {
            run,
            reports,
            directory,
            lines: Vec::new(),
            milestones: Vec::new(),
            exit: None,
            jobs: VecDeque::new(),
        }
    }

    fn wait(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + WAIT_LIMIT;
        while !done(self) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "no {what}: {:#?}", self.lines);
            match self.reports.recv_timeout(left) {
                Ok(Report::Output(lines)) => self
                    .lines
                    .extend(lines.into_iter().map(|line| (line.source, line.text))),
                Ok(Report::Event(HelperEventKind::Milestone { milestone, .. })) => {
                    self.milestones.push(milestone);
                }
                Ok(Report::Event(HelperEventKind::Exited { facts })) => self.exit = Some(facts),
                Ok(Report::ChangeNetwork(job)) => self.jobs.push_back(job),
                Ok(Report::RepairRoutes(_)) => panic!("openconnect leaves no routes to repair"),
                Ok(Report::Log(..)) => {}
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => panic!("the run ended without an exit"),
            }
        }
    }

    fn next_job(&mut self) -> NetworkJob {
        self.wait("network change", |session| !session.jobs.is_empty());
        self.jobs.pop_front().unwrap()
    }

    fn wait_for(&mut self, milestone: Milestone) {
        self.wait(&format!("{milestone:?}"), |session| {
            session.milestones.contains(&milestone)
        });
    }

    fn wait_for_exit(&mut self) -> ExitFacts {
        self.wait("exit", |session| session.exit.is_some());
        self.exit.clone().unwrap()
    }

    fn go(&self) {
        fs::write(self.directory.join("go"), "go\n").unwrap();
    }

    fn run_file(&self, name: &str) -> PathBuf {
        self.directory.join("run").join(name)
    }

    fn recorded(&self) -> Value {
        let record = fs::read(self.run_file("run.json")).unwrap();
        serde_json::from_slice::<Value>(&record).unwrap()["outsideChanges"].clone()
    }

    fn count(&self, text: &str) -> usize {
        self.lines.iter().filter(|(_, shown)| shown == text).count()
    }

    fn stop_and_revert(&mut self, routes: Vec<OutsideRoute>) -> ExitFacts {
        self.run.stop(StopReason::UserDisconnect);
        let revert = self.next_job();
        assert_eq!(
            revert.change,
            NetworkChange::Revert {
                changes: OutsideChanges {
                    dns_service_id: "svc".to_owned(),
                    routes: routes.clone(),
                },
            }
        );
        assert!(self.exit.is_none());
        revert.finish(NetworkOutcome::Reverted { left: Vec::new() });
        self.wait_for_exit()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.run.stop(StopReason::AppQuit);
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn network(address: &str, prefix_length: u8) -> IpNetwork {
    IpNetwork {
        address: address.parse().unwrap(),
        prefix_length,
    }
}

fn outside(destination: IpNetwork, router: &str) -> OutsideRoute {
    OutsideRoute {
        destination,
        router: router.parse::<IpAddr>().unwrap(),
        service_id: "ethernet".to_owned(),
    }
}

fn gateway_route(router: &str) -> OutsideRoute {
    outside(network("203.0.113.10", 32), router)
}

fn excluded_route() -> OutsideRoute {
    outside(network("10.20.161.16", 32), "192.0.2.1")
}

fn looked_up(missing: Vec<OutsideRoute>) -> NetworkOutcome {
    NetworkOutcome::LookedUp {
        dns_service_id: "svc".to_owned(),
        missing,
    }
}

fn applied(added: Vec<OutsideRoute>) -> NetworkOutcome {
    NetworkOutcome::Applied {
        added,
        left: Vec::new(),
    }
}

fn apply_parts(
    job: &NetworkJob,
) -> (
    &NetworkSettings,
    Option<&NetworkSettings>,
    &[OutsideRoute],
    &[OutsideRoute],
) {
    match &job.change {
        NetworkChange::Apply {
            settings,
            previous,
            remove,
            add,
        } => (settings, previous.as_deref(), remove, add),
        other => panic!("not an apply: {other:?}"),
    }
}

fn to_value(routes: &[OutsideRoute]) -> Value {
    serde_json::to_value(routes).unwrap()
}

#[test]
fn the_settings_are_recorded_and_applied_before_the_tunnel_is_up_and_reverted_at_the_end() {
    let mut session = Session::start(&format!(
        "{CONFIGURED}\n{}\nexec /bin/sleep 30",
        connect(VALUES)
    ));
    let look_up = session.next_job();
    assert_eq!(
        look_up.change,
        NetworkChange::LookUp {
            tunnel: TunnelDevice {
                name: "utun9".to_owned(),
                index: None,
            },
            outside: vec![network("203.0.113.10", 32), network("10.20.161.16", 32)],
        }
    );
    assert!(!session.run_file("values/values.1").exists());
    look_up.finish(looked_up(vec![
        gateway_route("192.0.2.1"),
        excluded_route(),
    ]));

    let apply = session.next_job();
    let (settings, previous, remove, add) = apply_parts(&apply);
    assert_eq!(settings.tunnel.name, "utun9");
    assert_eq!(settings.default_domains, ["corp.example.com"]);
    assert_eq!(previous, None);
    assert!(remove.is_empty());
    assert_eq!(add, [gateway_route("192.0.2.1"), excluded_route()]);
    assert_eq!(session.recorded()["dnsServiceID"], "svc");
    assert_eq!(
        session.recorded()["routes"],
        to_value(&[gateway_route("192.0.2.1"), excluded_route()])
    );
    assert!(!session.milestones.contains(&Milestone::TunnelUp));
    apply.finish(applied(vec![gateway_route("192.0.2.1"), excluded_route()]));

    session.wait_for(Milestone::TunnelUp);
    assert_eq!(session.count("‹network settings applied›"), 1);
    let facts = session.stop_and_revert(vec![gateway_route("192.0.2.1"), excluded_route()]);
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(facts.furthest_milestone, Some(Milestone::TunnelUp));
    assert_eq!(session.count("‹network settings reverted›"), 1);
    assert!(!session.run_file("run.json").exists());
}

#[test]
fn a_failing_apply_is_tried_four_times_then_the_run_ends_and_reverts() {
    let mut session = Session::start(&format!(
        "{CONFIGURED}\n{}\nexec /bin/sleep 30",
        connect(VALUES)
    ));
    let mut failed_at = Vec::new();
    for attempt in 0..4 {
        session
            .next_job()
            .finish(looked_up(vec![gateway_route("192.0.2.1")]));
        let apply = session.next_job();
        let (.., remove, _) = apply_parts(&apply);
        if attempt > 0 {
            assert_eq!(remove, [gateway_route("192.0.2.1")]);
        }
        apply.finish(NetworkOutcome::Failed {
            step: NetworkStep::Dns,
            error: "no DNS".to_owned(),
            added: vec![gateway_route("192.0.2.1")],
            left: Vec::new(),
        });
        failed_at.push(Instant::now());
    }
    for pair in failed_at.windows(2) {
        assert!(pair[1] - pair[0] >= RETRY, "{failed_at:?}");
    }
    let revert = session.next_job();
    assert_eq!(
        revert.change,
        NetworkChange::Revert {
            changes: OutsideChanges {
                dns_service_id: "svc".to_owned(),
                routes: vec![gateway_route("192.0.2.1")],
            },
        }
    );
    revert.finish(NetworkOutcome::Reverted { left: Vec::new() });
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::NetworkSetupFailed
        }
    );
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
    assert_eq!(
        session.count("‹network settings not applied: Dns: no DNS›"),
        4
    );
    assert_eq!(
        session.count("‹network settings still not applied after 4 tries›"),
        1
    );
}

#[test]
fn without_values_in_time_the_run_ends_with_nothing_to_revert() {
    let mut session = Session::start(&format!("{CONFIGURED}\nexec /bin/sleep 30"));
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::NetworkSetupFailed
        }
    );
    assert_eq!(
        session.count("‹no network settings from openconnect within 1s›"),
        1
    );
    assert!(session.jobs.is_empty());
}

#[test]
fn a_network_setup_that_outlasts_the_tunnel_stage_ends_the_run_as_a_network_setup_failure() {
    let mut session = Session::start_with_tunnel_stage(
        &format!(
            "{CONFIGURED}\n{{ {}; /bin/sleep 3; }} &\nwait",
            connect(VALUES)
        ),
        Duration::from_secs(1),
    );
    session
        .next_job()
        .finish(looked_up(vec![gateway_route("192.0.2.1")]));
    session
        .next_job()
        .finish(applied(vec![gateway_route("192.0.2.1")]));
    let revert = session.next_job();
    assert!(matches!(revert.change, NetworkChange::Revert { .. }));
    revert.finish(NetworkOutcome::Reverted { left: Vec::new() });
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::NetworkSetupFailed
        }
    );
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
    assert_eq!(
        session.count("‹watchdog: authToTunnel did not finish within 1s›"),
        1
    );
}

#[test]
fn a_stopping_run_neither_reports_missing_values_nor_looks_up_later_ones() {
    let mut session = Session::start(&format!(
        "trap '' TERM\n{CONFIGURED}\n{}\n{}\nexec /bin/sleep 30",
        wait_for_go(),
        connect(VALUES)
    ));
    session.wait("the configured line", |session| {
        session
            .lines
            .iter()
            .any(|(_, text)| text.starts_with("Configured as "))
    });
    session.run.stop(StopReason::UserDisconnect);
    thread::sleep(Duration::from_millis(1200));
    session.go();
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert_eq!(
        session.count("‹no network settings from openconnect within 1s›"),
        0
    );
    assert!(session.jobs.is_empty());
}

#[test]
fn invalid_values_end_the_run_and_their_file_is_deleted() {
    let mut session = Session::start(&format!(
        "{CONFIGURED}\n{}\nexec /bin/sleep 30",
        connect("VPNGATEWAY=203.0.113.10 TUNDEV=utun9")
    ));
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::NetworkSetupFailed
        }
    );
    assert_eq!(
        session.count("‹invalid network settings: INTERNAL_IP4_ADDRESS is missing›"),
        1
    );
    assert!(session.jobs.is_empty());
    assert!(!session.run_file("values/values.1").exists());
}

#[test]
fn a_second_connect_applies_the_difference() {
    let mut session = Session::start(&format!(
        "{CONFIGURED}\n{}\n{}\n{}\nexec /bin/sleep 30",
        connect(VALUES),
        wait_for_go(),
        connect(OTHER_VALUES)
    ));
    session.next_job().finish(looked_up(vec![
        gateway_route("192.0.2.1"),
        excluded_route(),
    ]));
    let first = session.next_job();
    let first_settings = apply_parts(&first).0.clone();
    first.finish(applied(vec![gateway_route("192.0.2.1"), excluded_route()]));
    session.wait_for(Milestone::TunnelUp);

    session.go();
    let look_up = session.next_job();
    assert_eq!(
        look_up.change,
        NetworkChange::LookUp {
            tunnel: first_settings.tunnel.clone(),
            outside: vec![network("203.0.113.10", 32), network("10.20.170.0", 24)],
        }
    );
    let new_exclude = outside(network("10.20.170.0", 24), "192.0.2.1");
    look_up.finish(looked_up(vec![new_exclude.clone()]));
    let second = session.next_job();
    let (settings, previous, remove, add) = apply_parts(&second);
    assert_eq!(previous, Some(&first_settings));
    assert_eq!(settings.split_exclude, [network("10.20.170.0", 24)]);
    assert_eq!(remove, [excluded_route()]);
    assert_eq!(add, std::slice::from_ref(&new_exclude));
    assert_eq!(
        session.recorded()["routes"],
        to_value(&[
            gateway_route("192.0.2.1"),
            excluded_route(),
            new_exclude.clone()
        ])
    );
    second.finish(applied(vec![new_exclude.clone()]));
    session.wait("the second apply line", |session| {
        session.count("‹network settings applied›") == 2
    });
    session.stop_and_revert(vec![gateway_route("192.0.2.1"), new_exclude]);
}

#[test]
fn a_gateway_route_request_moves_the_gateway_route_to_the_current_router() {
    let mut session = Session::start(&format!(
        "{CONFIGURED}\n{}\n{}\nreason=attempt-reconnect /bin/sh -c \"$1\"\nexec /bin/sleep 30",
        connect(VALUES),
        wait_for_go()
    ));
    session
        .next_job()
        .finish(looked_up(vec![gateway_route("192.0.2.1")]));
    let first = session.next_job();
    let first_settings = apply_parts(&first).0.clone();
    first.finish(applied(vec![gateway_route("192.0.2.1")]));
    session.wait_for(Milestone::TunnelUp);

    session.go();
    session
        .next_job()
        .finish(looked_up(vec![gateway_route("192.0.2.254")]));
    let moved = session.next_job();
    let (settings, previous, remove, add) = apply_parts(&moved);
    assert_eq!(settings, &first_settings);
    assert_eq!(previous, Some(&first_settings));
    assert_eq!(remove, [gateway_route("192.0.2.1")]);
    assert_eq!(add, [gateway_route("192.0.2.254")]);
    moved.finish(applied(vec![gateway_route("192.0.2.254")]));
    session.wait("the second apply line", |session| {
        session.count("‹network settings applied›") == 2
    });
    session.stop_and_revert(vec![gateway_route("192.0.2.254")]);
}

#[test]
fn the_tunnel_is_up_only_once_the_script_has_exited() {
    let mut session = Session::start(&format!(
        "{CONFIGURED}\n{{ {}; /bin/sleep 1; }} &\nwait\nexec /bin/sleep 30",
        connect(VALUES)
    ));
    session
        .next_job()
        .finish(looked_up(vec![gateway_route("192.0.2.1")]));
    session
        .next_job()
        .finish(applied(vec![gateway_route("192.0.2.1")]));
    let applied_at = Instant::now();
    session.wait_for(Milestone::TunnelUp);
    assert!(
        applied_at.elapsed() >= Duration::from_millis(500),
        "{:?}",
        applied_at.elapsed()
    );
    session.stop_and_revert(vec![gateway_route("192.0.2.1")]);
}

#[test]
fn an_adapter_already_using_the_tunnels_address_is_told_in_the_exit() {
    let mut session = Session::start(&format!(
        "{CONFIGURED}\n>&2 echo 'Adapter \"Ethernet 2\" / 23 is UP and using our IPv4 address. \
         Cannot resolve.'\nexit 1"
    ));
    let facts = session.wait_for_exit();
    assert!(facts.tunnel_address_in_use, "{facts:?}");
    assert_eq!(facts.stop_initiator, StopInitiator::Client {});
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
    assert!(session.jobs.is_empty());
}
