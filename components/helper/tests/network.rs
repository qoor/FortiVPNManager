mod common;

use std::fs;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use common::*;
use fvm_contract::app_helper::{ExitFacts, Milestone, StopInitiator, StopReason};
use fvm_contract::helper_internal::{
    NetworkChange, NetworkOutcome, NetworkSettings, NetworkStep, OutsideChanges, OutsideRoute,
    TunnelDevice,
};
use fvm_helper_core::session::NetworkJob;
use fvm_helper_core::session::backend::{Link, SessionEvent};
use serde_json::Value;

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
    assert!(session.exit_facts().is_none());
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
    assert_eq!(facts.furthest_milestone, Some(Milestone::TunnelUp));
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
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
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
    assert_eq!(facts.stop_initiator, StopInitiator::Client {});
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
    assert_eq!(facts.stop_initiator, StopInitiator::Client {});
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
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
    assert_eq!(facts.stop_initiator, StopInitiator::Client {});
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
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

fn address_in_use_end(new: &mut Session) -> ExitFacts {
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
    assert_eq!(facts.stop_initiator, StopInitiator::Client {});
    assert_eq!(facts.furthest_milestone, Some(Milestone::Authenticated));
    assert!(new.jobs.is_empty());
    assert!(old.exit_facts().is_none());
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
