mod common;

use std::fs;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use common::*;
use fvm_contract::app_helper::{
    BypassRoutesSnapshot, EndFacts, Milestone, RunRequest, StopInitiator, StopReason,
};
use fvm_contract::helper_internal::{
    AppRequest, BypassRoutesOutcome, FromEntry, LogCategory, LogLevel, NetworkChange,
    NetworkOutcome, NetworkSettings, NetworkStep, OutsideChanges, OutsideRoute, ToEntry,
    TunnelDevice,
};
use fvm_helper_core::entry::{self, Inbox};
use fvm_helper_core::session::backend::{Link, SessionEvent};
use fvm_helper_core::session::{NetworkJob, SessionManager};
use serde_json::{Value, json};

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
            ..
        } => (settings, previous.as_deref(), remove, add),
        other => panic!("not an apply: {other:?}"),
    }
}

fn recorded(session: &Session, name: &str) -> Value {
    let record = fs::read(session.directory.file(name)).unwrap();
    serde_json::from_slice::<Value>(&record).unwrap()
}

fn to_value(routes: &[OutsideRoute]) -> Value {
    serde_json::to_value(routes).unwrap()
}

fn revert(session: &mut Session, routes: Vec<OutsideRoute>) {
    let job = session.next_job();
    assert_eq!(
        job.change,
        NetworkChange::Revert {
            changes: OutsideChanges {
                dns_service_id: "svc".to_owned(),
                routes,
            },
        }
    );
    assert!(session.end_facts().is_none());
    job.finish(NetworkOutcome::Reverted { left: Vec::new() });
}

fn reconnecting(settings: NetworkSettings, reconnect: mpsc::Receiver<()>) -> Script {
    script(move |_, link: &Link| {
        trust_gateway(link);
        open(link);
        tunnel_ready(link, settings);
        while reconnect.recv().is_ok() {
            line(link, "Reconnected");
            link.report(SessionEvent::Reconnected);
        }
        ended(-EINTR)
    })
}

#[test]
fn the_settings_are_recorded_and_applied_before_the_tunnel_is_up_and_reverted_at_the_end() {
    let mut session = Session::start(
        up_without_otp(settings("utun9", "10.10.0.5")),
        Setup::default(),
    );
    let look_up = session.next_job();
    assert_eq!(
        look_up.change,
        NetworkChange::LookUp {
            tunnel: TunnelDevice {
                name: "utun9".to_owned(),
                index: None,
            },
            outside: vec![network(GATEWAY, 32), network("10.20.161.16", 32)],
        }
    );
    assert!(!session.directory.file("run.json").exists());
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
    let record = recorded(&session, "run.json");
    assert_eq!(record["outsideChanges"]["dnsServiceID"], "svc");
    assert_eq!(
        record["outsideChanges"]["routes"],
        to_value(&[gateway_route("192.0.2.1"), excluded_route()])
    );
    assert!(record.get("processes").is_none() && record.get("gatewayIP").is_none());
    assert_eq!(session.directory.names(), ["run.json"]);
    assert!(!session.reached(Milestone::TunnelUp));
    apply.finish(applied(vec![gateway_route("192.0.2.1"), excluded_route()]));

    session.wait_for(Milestone::TunnelUp);
    assert_eq!(session.count_text("‹network settings applied›"), 1);
    session.run.stop(StopReason::UserDisconnect);
    revert(
        &mut session,
        vec![gateway_route("192.0.2.1"), excluded_route()],
    );
    let facts = session.wait_for_exit();
    assert_eq!(facts.furthest_milestone, Milestone::TunnelUp);
    assert_eq!(session.count_text("‹network settings reverted›"), 1);
    assert!(session.directory.names().is_empty());
}

#[test]
fn the_apply_names_the_destinations_kept_outside_the_tunnel() {
    let mut session = Session::start(
        up_without_otp(settings("utun9", "10.10.0.5")),
        Setup::default(),
    );
    session
        .next_job()
        .finish(looked_up(vec![gateway_route("192.0.2.1")]));
    let apply = session.next_job();
    let NetworkChange::Apply { outside, .. } = &apply.change else {
        panic!("not an apply: {:?}", apply.change)
    };
    assert_eq!(
        outside,
        &[network(GATEWAY, 32), network("10.20.161.16", 32)]
    );
    apply.finish(applied(vec![gateway_route("192.0.2.1")]));
    session.wait_for(Milestone::TunnelUp);
    session.run.stop(StopReason::UserDisconnect);
    revert(&mut session, vec![gateway_route("192.0.2.1")]);
    session.wait_for_exit();
}

#[test]
fn only_an_up_run_whose_settings_are_in_place_offers_a_routing_check() {
    let (reconnect, reconnects) = mpsc::channel();
    let mut session = Session::start(
        reconnecting(settings("utun9", "10.10.0.5"), reconnects),
        Setup::default(),
    );
    let look_up = session.next_job();
    assert_eq!(session.run.routing_check(), None);
    look_up.finish(looked_up(vec![gateway_route("192.0.2.1")]));
    let apply = session.next_job();
    let applied_settings = apply_parts(&apply).0.clone();
    apply.finish(applied(vec![gateway_route("192.0.2.1")]));
    session.wait_for(Milestone::TunnelUp);
    let check = NetworkChange::Check {
        settings: Box::new(applied_settings),
        outside: vec![network(GATEWAY, 32), network("10.20.161.16", 32)],
    };
    assert_eq!(session.run.routing_check(), Some(check.clone()));

    reconnect.send(()).unwrap();
    let look_up = session.next_job();
    assert_eq!(session.run.routing_check(), None);
    look_up.finish(looked_up(Vec::new()));
    session.next_job().finish(applied(Vec::new()));
    assert!(session.wait(std::time::Duration::from_secs(5), |session| {
        session.count_text("‹network settings applied›") == 2
    }));
    assert_eq!(session.run.routing_check(), Some(check));

    session.run.stop(StopReason::UserDisconnect);
    assert_eq!(session.run.routing_check(), None);
    drop(reconnect);
    revert(&mut session, vec![gateway_route("192.0.2.1")]);
    session.wait_for_exit();
}

#[test]
fn a_failing_apply_is_tried_four_times_then_the_run_ends_and_reverts() {
    let mut session = Session::start(
        up_without_otp(settings("utun9", "10.10.0.5")),
        Setup::default(),
    );
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
    revert(&mut session, vec![gateway_route("192.0.2.1")]);
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::NetworkSetupFailed
        }
    );
    assert_eq!(facts.furthest_milestone, Milestone::Authenticated);
    assert_eq!(
        session.count_text("‹network settings not applied: Dns: no DNS›"),
        4
    );
    assert_eq!(
        session.count_text("‹network settings still not applied after 4 tries›"),
        1
    );
}

#[test]
fn settings_the_backend_could_not_read_end_the_run_with_nothing_to_revert() {
    let script = script(|_, link| {
        trust_gateway(link);
        open(link);
        link.report(SessionEvent::TunnelReady(Err(
            "the IPv4 address is missing".to_owned(),
        )));
        wait_for_cancel(link);
        ended(-EINTR)
    });
    let mut session = Session::start(script, Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::NetworkSetupFailed
        }
    );
    assert_eq!(
        session.count_text("‹invalid network settings: the IPv4 address is missing›"),
        1
    );
    assert!(session.jobs.is_empty());
}

#[test]
fn a_stopping_run_does_not_look_up_the_settings_that_arrive() {
    let (ready, wait_ready) = mpsc::channel::<()>();
    let script = script(move |_, link| {
        trust_gateway(link);
        open(link);
        let _ = wait_ready.recv();
        tunnel_ready(link, settings("utun9", "10.10.0.5"));
        ended(-EINTR)
    });
    let mut session = Session::start(script, Setup::default());
    session.wait_for(Milestone::Authenticated);
    session.run.stop(StopReason::UserDisconnect);
    session.settle(std::time::Duration::from_millis(100));
    ready.send(()).unwrap();
    let facts = session.wait_for_exit();
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
    assert!(session.jobs.is_empty());
}

#[test]
fn a_reconnect_applies_the_settings_again_through_the_current_router() {
    let (reconnect, reconnects) = mpsc::channel();
    let mut session = Session::start(
        reconnecting(settings("utun9", "10.10.0.5"), reconnects),
        Setup::default(),
    );
    session
        .next_job()
        .finish(looked_up(vec![gateway_route("192.0.2.1")]));
    let first = session.next_job();
    let first_settings = apply_parts(&first).0.clone();
    first.finish(applied(vec![gateway_route("192.0.2.1")]));
    session.wait_for(Milestone::TunnelUp);

    reconnect.send(()).unwrap();
    let look_up = session.next_job();
    assert!(matches!(look_up.change, NetworkChange::LookUp { .. }));
    look_up.finish(looked_up(vec![gateway_route("192.0.2.254")]));
    let moved = session.next_job();
    let (settings, previous, remove, add) = apply_parts(&moved);
    assert_eq!(settings, &first_settings);
    assert_eq!(previous, Some(&first_settings));
    assert_eq!(remove, [gateway_route("192.0.2.1")]);
    assert_eq!(add, [gateway_route("192.0.2.254")]);
    moved.finish(applied(vec![gateway_route("192.0.2.254")]));
    assert!(session.wait(std::time::Duration::from_secs(5), |session| {
        session.count_text("‹network settings applied›") == 2
    }));
    assert_eq!(session.milestones().last(), Some(&Milestone::TunnelUp));
    drop(reconnect);
    revert(&mut session, vec![gateway_route("192.0.2.254")]);
    let facts = session.wait_for_exit();
    assert_eq!(facts.stop_initiator, StopInitiator::Session {});
}

#[test]
fn a_look_up_answered_after_the_session_returned_is_not_recorded_or_applied() {
    let secrets = Arc::new(FakeSecrets::default());
    let release = secrets.hold_deletes();
    let setup = Setup {
        secrets,
        ..Setup::default()
    };
    let (end, ends) = mpsc::channel::<()>();
    let script = script(move |_, link| {
        trust_gateway(link);
        open(link);
        tunnel_ready(link, settings("utun9", "10.10.0.5"));
        let _ = ends.recv();
        ended(-EIO)
    });
    let mut session = Session::start(script, setup);
    let look_up = session.next_job();
    end.send(()).unwrap();
    assert!(session.wait(std::time::Duration::from_secs(5), |session| {
        !session.run.session_running()
    }));
    look_up.finish(looked_up(vec![gateway_route("192.0.2.1")]));
    session.settle(std::time::Duration::from_millis(100));
    assert!(session.jobs.is_empty());
    assert!(!session.directory.file("run.json").exists());
    release.send(()).unwrap();
    let facts = session.wait_for_exit();
    assert!(session.jobs.is_empty());
    assert_eq!(facts.stop_initiator, StopInitiator::Session {});
    assert_eq!(facts.furthest_milestone, Milestone::Authenticated);
    session.assert_finished_cleanly();
}

#[test]
fn an_apply_answered_after_the_session_returned_does_not_bring_the_tunnel_up() {
    let secrets = Arc::new(FakeSecrets::default());
    let release = secrets.hold_deletes();
    let setup = Setup {
        secrets,
        ..Setup::default()
    };
    let (end, ends) = mpsc::channel::<()>();
    let script = script(move |_, link| {
        trust_gateway(link);
        open(link);
        tunnel_ready(link, settings("utun9", "10.10.0.5"));
        let _ = ends.recv();
        ended(-EIO)
    });
    let mut session = Session::start(script, setup);
    session
        .next_job()
        .finish(looked_up(vec![gateway_route("192.0.2.1")]));
    let apply = session.next_job();
    end.send(()).unwrap();
    assert!(session.wait(std::time::Duration::from_secs(5), |session| {
        !session.run.session_running()
    }));
    apply.finish(applied(vec![gateway_route("192.0.2.1")]));
    session.settle(std::time::Duration::from_millis(100));
    assert!(session.jobs.is_empty());
    assert!(!session.reached(Milestone::TunnelUp));
    assert!(!session.has_text("‹network settings applied›"));
    release.send(()).unwrap();
    revert(&mut session, vec![gateway_route("192.0.2.1")]);
    let facts = session.wait_for_exit();
    assert_eq!(facts.stop_initiator, StopInitiator::Session {});
    assert_eq!(facts.furthest_milestone, Milestone::Authenticated);
    session.assert_finished_cleanly();
}

fn replacement(old: &mut Session) -> fvm_helper_core::session::Replaced {
    old.next_job()
        .finish(looked_up(vec![gateway_route("192.0.2.1")]));
    old.next_job()
        .finish(applied(vec![gateway_route("192.0.2.1")]));
    old.wait_for(Milestone::TunnelUp);
    old.run.replaceable().unwrap()
}

fn replacing_setup() -> Setup {
    let mut request = request(OTHER_ATTEMPT);
    request.replacing = Some(ATTEMPT.parse().unwrap());
    Setup {
        request,
        ..Setup::default()
    }
}

fn address_in_use_end(new: &mut Session) -> EndFacts {
    let facts = new.wait_for_exit();
    assert!(facts.tunnel_address_in_use, "{facts:?}");
    assert!(new.has_text("‹the connection to replace has the same tunnel address›"));
    facts
}

#[test]
fn a_tunnel_device_refused_for_the_replaced_runs_address_is_told_in_the_exit() {
    let mut old = Session::start(
        up_without_otp(settings("utun9", "10.10.0.5")),
        Setup::default(),
    );
    let replaced = replacement(&mut old);
    let script = script(|_, link| {
        trust_gateway(link);
        open(link);
        error_line(
            link,
            "Adapter \"Ethernet 2\" is UP and using our IPv4 address",
        );
        link.report(SessionEvent::TunnelFailed(
            Some("10.10.0.5".parse().unwrap()),
            None,
        ));
        ended(-EIO)
    });
    let mut new = Session::start_replacing(script, replacing_setup(), Some(replaced));
    let facts = address_in_use_end(&mut new);
    assert_eq!(facts.stop_initiator, StopInitiator::Session {});
    assert_eq!(facts.furthest_milestone, Milestone::Authenticated);
    assert!(new.jobs.is_empty());
    assert!(old.end_facts().is_none());
    old.run.stop(StopReason::UserDisconnect);
    revert(&mut old, vec![gateway_route("192.0.2.1")]);
    old.wait_for_exit();
}

#[test]
fn a_tunnel_device_refused_for_another_address_is_not_an_address_in_use() {
    let mut old = Session::start(
        up_without_otp(settings("utun9", "10.10.0.5")),
        Setup::default(),
    );
    let replaced = replacement(&mut old);
    let script = script(|_, link| {
        trust_gateway(link);
        open(link);
        link.report(SessionEvent::TunnelFailed(
            Some("10.10.0.6".parse().unwrap()),
            None,
        ));
        ended(-EIO)
    });
    let mut new = Session::start_replacing(script, replacing_setup(), Some(replaced));
    let facts = new.wait_for_exit();
    assert!(!facts.tunnel_address_in_use);
    assert!(new.has_text("‹the tunnel device could not be set up›"));
    old.run.stop(StopReason::UserDisconnect);
    revert(&mut old, vec![gateway_route("192.0.2.1")]);
    old.wait_for_exit();
}

#[test]
fn settings_with_the_replaced_runs_address_end_the_new_run_after_the_look_up() {
    let mut old = Session::start(
        up_without_otp(settings("utun9", "10.10.0.5")),
        Setup::default(),
    );
    let replaced = replacement(&mut old);
    let mut new = Session::start_replacing(
        up_without_otp(settings("utun10", "10.10.0.5")),
        replacing_setup(),
        Some(replaced),
    );
    new.next_job().finish(looked_up(Vec::new()));
    let facts = address_in_use_end(&mut new);
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::NetworkSetupFailed
        }
    );
    old.run.stop(StopReason::UserDisconnect);
    revert(&mut old, vec![gateway_route("192.0.2.1")]);
    old.wait_for_exit();
}

struct OsSide {
    inbox: Inbox,
    messages: mpsc::Receiver<FromEntry>,
    received: Vec<FromEntry>,
    directory: RunDirectory,
}

impl OsSide {
    fn start(scripts: Vec<Script>) -> Self {
        let mut os = Self::launch(RunDirectory::new(), scripts);
        os.wait_for(|message| *message == FromEntry::Ready {});
        os.admit();
        os
    }

    fn launch(directory: RunDirectory, scripts: Vec<Script>) -> Self {
        let sessions = SessionManager::new(
            Arc::new(FakeBackend::new(scripts)),
            Arc::new(FakeSecrets::default()),
            directory.path.clone(),
            timing(),
        );
        let (sender, messages) = mpsc::channel();
        let inbox = entry::start(sessions, move |message| {
            let _ = sender.send(message.clone());
        });
        Self {
            inbox,
            messages,
            received: Vec::new(),
            directory,
        }
    }

    fn admit(&mut self) {
        self.inbox.deliver(ToEntry::AppConnected { app: 1, uid: 1 });
        self.wait_for(|message| {
            *message
                == FromEntry::Admitted {
                    app: 1,
                    accept: true,
                }
        });
    }

    fn wait_for(&mut self, wanted: impl Fn(&FromEntry) -> bool) -> FromEntry {
        loop {
            let message = self
                .messages
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|error| panic!("{error}: {:#?}", self.received));
            self.received.push(message.clone());
            if wanted(&message) {
                return message;
            }
        }
    }

    fn request(&self, id: u64, request: AppRequest) {
        self.inbox.deliver(ToEntry::Request {
            id,
            app: 1,
            request,
        });
    }

    fn connect(&mut self, id: u64, request: &RunRequest) {
        let json = serde_json::to_string(request).unwrap();
        self.request(id, AppRequest::Connect { json });
        self.wait_for(|message| *message == FromEntry::Reply { id, json: None });
    }

    fn change_network(&mut self) -> NetworkChange {
        match self.wait_for(|message| matches!(message, FromEntry::ChangeNetwork { .. })) {
            FromEntry::ChangeNetwork { change } => change,
            _ => unreachable!("waited for a network change"),
        }
    }

    fn answer_network(&self, outcome: NetworkOutcome) {
        self.inbox.deliver(ToEntry::NetworkChanged { outcome });
    }

    fn wait_for_output(&mut self, text: &str) {
        self.wait_for(|message| {
            matches!(message, FromEntry::SendOutput { app: 1, json } if json.contains(text))
        });
    }

    fn wait_for_log(&mut self, ending: &str) {
        self.wait_for(|message| {
            matches!(message, FromEntry::Log { message, .. } if message.ends_with(ending))
        });
    }

    fn has_log(&self, level: LogLevel, line: &str) -> bool {
        self.received.iter().any(|message| {
            *message
                == FromEntry::Log {
                    level,
                    category: LogCategory::Helper,
                    message: line.to_owned(),
                }
        })
    }

    fn bring_up(&mut self, id: u64, attempt: &str, routes: Vec<OutsideRoute>) {
        self.connect(id, &request(attempt));
        assert!(matches!(
            self.change_network(),
            NetworkChange::LookUp { .. }
        ));
        self.answer_network(looked_up(routes.clone()));
        assert!(matches!(self.change_network(), NetworkChange::Apply { .. }));
        self.answer_network(applied(routes));
        self.wait_for_log(&format!("run {attempt}: tunnel up"));
    }

    fn disconnect(&mut self, id: u64, attempt: &str) {
        self.request(
            id,
            AppRequest::Disconnect {
                attempt: attempt.to_owned(),
                reason: "userDisconnect".to_owned(),
            },
        );
        self.wait_for(|message| *message == FromEntry::Reply { id, json: None });
    }

    fn revert(&mut self, outcome: NetworkOutcome) -> OutsideChanges {
        let NetworkChange::Revert { changes } = self.change_network() else {
            panic!("not a revert: {:#?}", self.received);
        };
        self.answer_network(outcome);
        changes
    }

    fn wait_for_exit(&mut self, attempt: &str) {
        let exited = format!("run {attempt} exited: ");
        self.wait_for(|message| {
            matches!(message, FromEntry::Log { message, .. } if message.contains(&exited))
        });
    }

    fn kept(&self) -> Option<Value> {
        let record = fs::read(self.directory.file("unreverted-changes.json")).ok()?;
        Some(serde_json::from_slice(&record).unwrap())
    }

    fn changes_sent(&self) -> Vec<&'static str> {
        self.received
            .iter()
            .filter_map(|message| match message {
                FromEntry::ChangeNetwork { change } => Some(match change {
                    NetworkChange::LookUp { .. } => "lookUp",
                    NetworkChange::Apply { .. } => "apply",
                    NetworkChange::Prepare { .. } => "prepare",
                    NetworkChange::Switch { .. } => "switch",
                    NetworkChange::Revert { .. } => "revert",
                    NetworkChange::Check { .. } => "check",
                }),
                _ => None,
            })
            .collect()
    }
}

fn a_queued_change_of_an_ended_session_is_not_sent(replacing: bool) {
    let (end, ends) = mpsc::channel::<()>();
    let ending = script(move |_, link| {
        trust_gateway(link);
        open(link);
        tunnel_ready(link, settings("utun9", "10.10.0.5"));
        let _ = ends.recv();
        ended(-EIO)
    });
    let mut scripts = Vec::new();
    if replacing {
        scripts.push(up_without_otp(settings("utun8", "10.10.0.4")));
    }
    scripts.push(ending);
    let mut os = OsSide::start(scripts);
    let mut ending_request = request(ATTEMPT);
    if replacing {
        os.connect(1, &request(OTHER_ATTEMPT));
        os.change_network();
        os.answer_network(looked_up(vec![gateway_route("192.0.2.1")]));
        os.change_network();
        os.answer_network(applied(vec![gateway_route("192.0.2.1")]));
        os.wait_for_log(": tunnel up");
        ending_request.replacing = Some(OTHER_ATTEMPT.parse().unwrap());
    }
    os.connect(2, &ending_request);
    assert!(matches!(os.change_network(), NetworkChange::LookUp { .. }));
    os.request(3, AppRequest::ReadBypassRoutes {});
    os.answer_network(looked_up(vec![gateway_route("192.0.2.1")]));
    os.wait_for(|message| matches!(message, FromEntry::ChangeBypassRoutes { .. }));
    end.send(()).unwrap();
    os.wait_for_output("‹session ended: code -5›");
    let snapshot = BypassRoutesSnapshot {
        service_id: "ethernet".to_owned(),
        service_name: "Ethernet".to_owned(),
        router: "192.0.2.1".to_owned(),
        address: "192.0.2.20".to_owned(),
        subnet_mask: "255.255.255.0".to_owned(),
    };
    os.inbox.deliver(ToEntry::BypassRoutesChanged {
        outcome: BypassRoutesOutcome::Changed {
            network: Some(snapshot),
            added: Vec::new(),
            found: Vec::new(),
            left: Vec::new(),
        },
    });
    os.wait_for(|message| {
        matches!(
            message,
            FromEntry::Reply {
                id: 3,
                json: Some(_)
            }
        )
    });
    let dns_service_id = if replacing { "" } else { "svc" };
    assert_eq!(
        os.change_network(),
        NetworkChange::Revert {
            changes: OutsideChanges {
                dns_service_id: dns_service_id.to_owned(),
                routes: vec![gateway_route("192.0.2.1")],
            },
        }
    );
    let sent = if replacing {
        vec!["lookUp", "apply", "lookUp", "revert"]
    } else {
        vec!["lookUp", "revert"]
    };
    assert_eq!(os.changes_sent(), sent);
    os.answer_network(NetworkOutcome::Reverted { left: Vec::new() });
    os.wait_for_log("exited: code -5, initiator Client, furthest Authenticated");
    let failed = "‹network settings not applied: Network: the session ended›";
    assert!(
        os.received.iter().any(|message| {
            matches!(message, FromEntry::SendOutput { app: 1, json } if json.contains(failed))
        }),
        "{:#?}",
        os.received
    );
}

#[test]
fn an_apply_queued_when_its_session_ends_is_answered_as_failed_and_the_revert_still_sent() {
    a_queued_change_of_an_ended_session_is_not_sent(false);
}

#[test]
fn a_prepare_queued_when_its_session_ends_is_answered_as_failed_and_the_revert_still_sent() {
    a_queued_change_of_an_ended_session_is_not_sent(true);
}

fn starting(started: mpsc::Sender<()>, then: Script) -> Script {
    script(move |request, link| {
        started.send(()).unwrap();
        then(request, link)
    })
}

fn failed(step: NetworkStep, error: &str, left: Vec<OutsideRoute>) -> NetworkOutcome {
    NetworkOutcome::Failed {
        step,
        error: error.to_owned(),
        added: Vec::new(),
        left,
    }
}

fn routes_only(routes: Vec<OutsideRoute>) -> OutsideChanges {
    OutsideChanges {
        dns_service_id: String::new(),
        routes,
    }
}

fn after_a_revert_that_left_the_gateway_route(mut scripts: Vec<Script>) -> OsSide {
    scripts.insert(0, up_without_otp(settings("utun9", "10.10.0.5")));
    let mut os = OsSide::start(scripts);
    os.bring_up(1, ATTEMPT, vec![gateway_route("192.0.2.1")]);
    os.disconnect(2, ATTEMPT);
    os.revert(NetworkOutcome::Reverted {
        left: vec![gateway_route("192.0.2.1")],
    });
    os.wait_for_exit(ATTEMPT);
    os
}

#[test]
fn what_a_failed_revert_leaves_is_recorded_and_the_run_exits_as_before() {
    let mut os = OsSide::start(vec![up_without_otp(settings("utun9", "10.10.0.5"))]);
    os.bring_up(1, ATTEMPT, vec![gateway_route("192.0.2.1")]);
    os.disconnect(2, ATTEMPT);
    let reverted = os.revert(failed(
        NetworkStep::Dns,
        "the DNS settings under svc could not be removed",
        vec![gateway_route("192.0.2.1")],
    ));
    assert_eq!(
        reverted,
        OutsideChanges {
            dns_service_id: "svc".to_owned(),
            routes: vec![gateway_route("192.0.2.1")],
        }
    );
    os.wait_for(|message| {
        matches!(message, FromEntry::SendEvent { app: 1, json } if json.contains("exited"))
    });
    os.wait_for_exit(ATTEMPT);
    assert!(os.has_log(
        LogLevel::Notice,
        "kept what a revert left to revert it again: 1 route(s) and the DNS settings"
    ));
    assert_eq!(os.kept(), Some(json!({"changes": [reverted]})));
    assert_eq!(os.directory.names(), ["unreverted-changes.json"]);

    os.request(3, AppRequest::Status {});
    let status = os.wait_for(|message| matches!(message, FromEntry::Reply { id: 3, .. }));
    let FromEntry::Reply {
        json: Some(status), ..
    } = status
    else {
        panic!("a status reply carries JSON");
    };
    assert_eq!(
        serde_json::from_str::<Value>(&status).unwrap()["sessionRunning"],
        false
    );
}

#[test]
fn the_next_connect_reverts_what_was_kept_before_its_session_starts() {
    let (started, starts) = mpsc::channel();
    let next = starting(started, up_without_otp(settings("utun10", "10.10.0.6")));
    let mut os = after_a_revert_that_left_the_gateway_route(vec![next]);
    assert_eq!(
        os.kept(),
        Some(json!({"changes": [routes_only(vec![gateway_route("192.0.2.1")])]}))
    );

    os.connect(3, &request(OTHER_ATTEMPT));
    let NetworkChange::Revert { changes } = os.change_network() else {
        panic!("not a revert: {:#?}", os.received);
    };
    assert_eq!(changes, routes_only(vec![gateway_route("192.0.2.1")]));
    assert!(starts.recv_timeout(Duration::from_millis(300)).is_err());
    os.answer_network(NetworkOutcome::Reverted { left: Vec::new() });
    starts.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(os.has_log(
        LogLevel::Notice,
        "reverted again the network settings a revert left (1 route(s))"
    ));
    assert_eq!(os.kept(), None);

    assert!(matches!(os.change_network(), NetworkChange::LookUp { .. }));
    os.answer_network(looked_up(vec![gateway_route("192.0.2.1")]));
    assert!(matches!(os.change_network(), NetworkChange::Apply { .. }));
    os.answer_network(applied(vec![gateway_route("192.0.2.1")]));
    os.wait_for_log(&format!("run {OTHER_ATTEMPT}: tunnel up"));
}

#[test]
fn a_revert_again_that_fails_keeps_what_is_left_logs_an_error_and_the_session_still_starts() {
    let (started, starts) = mpsc::channel();
    let next = starting(started, up_without_otp(settings("utun10", "10.10.0.6")));
    let mut os = after_a_revert_that_left_the_gateway_route(vec![next]);

    os.connect(3, &request(OTHER_ATTEMPT));
    os.revert(failed(
        NetworkStep::OutsideRoutes,
        "the routing table could not be read",
        vec![gateway_route("192.0.2.1")],
    ));
    starts.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(matches!(os.change_network(), NetworkChange::LookUp { .. }));
    assert!(os.has_log(
        LogLevel::Error,
        "reverting again the network settings a revert left failed (OutsideRoutes): the routing \
         table could not be read; 1 of 1 route(s) still in place"
    ));
    assert_eq!(
        os.kept(),
        Some(json!({"changes": [routes_only(vec![gateway_route("192.0.2.1")])]}))
    );
}

#[test]
fn both_runs_of_a_replacement_keep_what_their_failed_reverts_left() {
    let mut new_settings = settings("utun10", "10.10.0.6");
    new_settings.split_exclude = Vec::new();
    let mut os = OsSide::start(vec![
        up_without_otp(settings("utun9", "10.10.0.5")),
        up_without_otp(new_settings),
    ]);
    os.bring_up(
        1,
        ATTEMPT,
        vec![gateway_route("192.0.2.1"), excluded_route()],
    );
    let mut replacing = request(OTHER_ATTEMPT);
    replacing.replacing = Some(ATTEMPT.parse().unwrap());
    os.connect(2, &replacing);
    assert!(matches!(os.change_network(), NetworkChange::LookUp { .. }));
    os.answer_network(looked_up(Vec::new()));
    assert!(matches!(os.change_network(), NetworkChange::Prepare { .. }));
    os.answer_network(applied(Vec::new()));
    assert!(matches!(os.change_network(), NetworkChange::Switch { .. }));
    os.answer_network(applied(Vec::new()));
    let old = os.revert(NetworkOutcome::Reverted {
        left: vec![excluded_route()],
    });
    assert_eq!(old, routes_only(vec![excluded_route()]));
    os.wait_for_exit(ATTEMPT);

    os.disconnect(3, OTHER_ATTEMPT);
    let new = os.revert(failed(
        NetworkStep::Dns,
        "the DNS settings under svc could not be removed",
        vec![gateway_route("192.0.2.1")],
    ));
    os.wait_for_exit(OTHER_ATTEMPT);
    assert_eq!(os.kept(), Some(json!({"changes": [old, new]})));
}

#[test]
fn without_kept_settings_a_connect_starts_its_session_at_once() {
    let mut os = OsSide::start(vec![up_without_otp(settings("utun9", "10.10.0.5"))]);
    os.bring_up(1, ATTEMPT, vec![gateway_route("192.0.2.1")]);
    assert_eq!(os.changes_sent(), ["lookUp", "apply"]);
    assert_eq!(os.kept(), None);
}

#[test]
fn the_network_settings_a_revert_left_are_reverted_again_before_ready() {
    let directory = RunDirectory::new();
    fs::create_dir(&directory.path).unwrap();
    let kept = OutsideChanges {
        dns_service_id: "svc".to_owned(),
        routes: vec![gateway_route("192.0.2.1")],
    };
    fs::write(
        directory.file("unreverted-changes.json"),
        json!({"changes": [kept]}).to_string(),
    )
    .unwrap();
    let mut os = OsSide::launch(directory, Vec::new());
    let NetworkChange::Revert { changes } = os.change_network() else {
        panic!("not a revert: {:#?}", os.received);
    };
    assert_eq!(changes, kept);
    assert!(!os.received.contains(&FromEntry::Ready {}));
    os.answer_network(NetworkOutcome::Reverted { left: Vec::new() });
    os.wait_for(|message| *message == FromEntry::Ready {});
    assert!(os.has_log(
        LogLevel::Notice,
        "reverted again the network settings a revert left (1 route(s))"
    ));
    assert!(os.directory.names().is_empty());
}

#[test]
fn what_is_still_left_at_the_start_is_reverted_again_before_the_next_session() {
    let directory = RunDirectory::new();
    fs::create_dir(&directory.path).unwrap();
    let kept = routes_only(vec![gateway_route("192.0.2.1")]);
    fs::write(
        directory.file("unreverted-changes.json"),
        json!({"changes": [kept]}).to_string(),
    )
    .unwrap();
    let mut os = OsSide::launch(
        directory,
        vec![up_without_otp(settings("utun9", "10.10.0.5"))],
    );
    os.revert(NetworkOutcome::Reverted {
        left: vec![gateway_route("192.0.2.1")],
    });
    os.wait_for(|message| *message == FromEntry::Ready {});
    assert!(os.has_log(
        LogLevel::Error,
        "reverting again the network settings a revert left: 1 of 1 route(s) still in place"
    ));
    assert_eq!(os.kept(), Some(json!({"changes": [kept]})));

    os.admit();
    os.connect(1, &request(ATTEMPT));
    assert_eq!(
        os.revert(NetworkOutcome::Reverted { left: Vec::new() }),
        kept
    );
    assert!(matches!(os.change_network(), NetworkChange::LookUp { .. }));
    assert_eq!(os.kept(), None);
}
