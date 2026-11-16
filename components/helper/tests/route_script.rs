#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
use std::{env, process};

use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_contract::app_helper::{
    HelperEventKind, Milestone, OutputSource, RunRequest, StopReason, VpnClient,
};
use fvm_contract::backend::{
    Backend, ClientCheckWait, ClientExecutable, ClientProblem, LaunchSpec, OutputInterpreter,
    UntestedVersion,
};
use fvm_helper_core::process::{Report, Timing, Watchdog};

const ROUTE_SCRIPT: &str = "/Applications/FortiVPNManager.app/Contents/Resources/vpnc-script";
const MARKER: &str = "FortiVPNManager: route script finished connecting";

struct Scripted(LaunchSpec);

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
        unreachable!("the script is always found")
    }

    fn untested_version(&self, _: &str) -> Option<UntestedVersion> {
        unreachable!("the test does not ask for the client's version")
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

fn request() -> RunRequest {
    RunRequest {
        attempt_id: "E621E1F8-C36C-495A-93FC-0C247A3E6E5F".parse().unwrap(),
        client: VpnClient::Openconnect,
        host: "203.0.113.10".to_owned(),
        port: 443,
        username: "alice".to_owned(),
        realm: None,
        trusted_certs: vec!["pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=".to_owned()],
        password: "secret".to_owned(),
    }
}

fn script_command(script: &Path) -> String {
    let client = ClientExecutable {
        path: PathBuf::from("/Applications/FortiVPNManager.app/Contents/MacOS/openconnect"),
        version: "9.21".to_owned(),
    };
    let launch = OpenConnect
        .launch_spec(&client, &request(), Path::new("unused"))
        .unwrap();
    let tunnel = launch.session.unwrap().tunnel;
    let command = tunnel
        .args
        .iter()
        .find_map(|arg| arg.strip_prefix("--script="))
        .expect("the tunnel run gets a route script command");
    assert!(command.starts_with(ROUTE_SCRIPT), "{command}");
    command.replacen(ROUTE_SCRIPT, script.to_str().unwrap(), 1)
}

#[test]
fn the_route_script_line_reaches_the_output_and_shows_the_tunnel_up() {
    let directory = env::temp_dir().join(format!("fvm-route-script-{}", process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir(&directory).unwrap();
    let script = directory.join("vpnc-script");
    fs::write(&script, "#!/bin/sh\necho \"vpnc-script: $reason done\"\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let client = "echo 'Configured as 10.10.0.5, with SSL connected and DTLS in progress'\n\
                  reason=connect /bin/sh -c \"$1\"\n\
                  exec /bin/sleep 30\n"
        .to_owned();
    let launch = LaunchSpec {
        program: PathBuf::from("/bin/sh"),
        args: vec![
            "-c".to_owned(),
            client,
            "openconnect".to_owned(),
            script_command(&script),
        ],
        env: vec![(
            "PATH".to_owned(),
            "/usr/bin:/bin:/usr/sbin:/sbin".to_owned(),
        )],
        config_file: None,
        session: None,
    };
    let mut watchdog = Watchdog::new(
        vec![(VpnClient::Openconnect, Arc::new(Scripted(launch)))],
        directory.join("run"),
        Timing::default(),
    );
    let (sender, reports) = mpsc::channel();
    let (run, _) = watchdog
        .start(&mut request(), move |report| {
            let _ = sender.send(report);
        })
        .unwrap();

    let mut lines = Vec::new();
    let mut tunnel_up = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while tunnel_up.is_none() {
        let left = deadline.saturating_duration_since(Instant::now());
        match reports.recv_timeout(left) {
            Ok(Report::Output(batch)) => {
                lines.extend(batch.into_iter().map(|line| (line.source, line.text)))
            }
            Ok(Report::Event(HelperEventKind::Milestone {
                milestone: Milestone::TunnelUp,
                tunnel_ip,
            })) => tunnel_up = Some(tunnel_ip),
            Ok(Report::Event(HelperEventKind::Exited { facts })) => {
                panic!("exited before the tunnel: {facts:?} {lines:#?}")
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => panic!("no tunnel: {lines:#?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the run stopped reporting"),
        }
    }
    assert_eq!(tunnel_up, Some(Some("10.10.0.5".parse().unwrap())));
    let stdout: Vec<&str> = lines
        .iter()
        .filter(|(source, _)| *source == OutputSource::Stdout)
        .map(|(_, text)| text.as_str())
        .collect();
    assert_eq!(
        stdout,
        [
            "Configured as 10.10.0.5, with SSL connected and DTLS in progress",
            "vpnc-script: connect done",
            MARKER,
        ]
    );

    run.stop(StopReason::UserDisconnect);
    loop {
        match reports.recv_timeout(Duration::from_secs(10)) {
            Ok(Report::Event(HelperEventKind::Exited { .. })) => break,
            Ok(Report::RepairRoutes(repair)) => {
                repair.finish(fvm_contract::backend::RouteRepairOutcome::NotNeeded)
            }
            Ok(_) => {}
            Err(error) => panic!("no exit: {error:?}"),
        }
    }
    fs::remove_dir_all(&directory).unwrap();
}
