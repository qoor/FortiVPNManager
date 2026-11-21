mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use fvm_contract::app_helper::{Milestone, StopReason};
use serde_json::{Value, json};

fn opened_then_returned() -> Script {
    script(|_, link| {
        trust_gateway(link);
        open(link);
        error_line(link, "Failed to open HTTPS connection to 127.0.0.1");
        ended(-EIO)
    })
}

fn setup_for(gateway: &FakeGateway) -> Setup {
    Setup::default().logging_out_to(gateway)
}

#[test]
fn an_opened_session_is_saved_off_the_session_thread_and_deleted_at_its_end() {
    let secrets = Arc::new(FakeSecrets::default());
    let release = secrets.hold_saves();
    let setup = Setup {
        secrets: Arc::clone(&secrets),
        ..Setup::default()
    };
    let mut session = Session::start(up_without_otp(settings("utun9", "10.10.0.5")), setup);
    session.bring_up(vec![gateway_route("192.0.2.1")]);
    assert_eq!(
        secrets.saves(),
        [(ATTEMPT.to_owned(), Some("fvm-session-secret".to_owned()))]
    );
    assert!(secrets.stored().is_empty(), "the held save finished");
    release.send(()).unwrap();
    let saved = || secrets.stored().get(ATTEMPT).cloned();
    assert!(session.wait(std::time::Duration::from_secs(5), |_| saved().is_some()));
    let blob: Value = serde_json::from_slice(&saved().unwrap().unwrap()).unwrap();
    assert_eq!(
        blob,
        json!({"host": GATEWAY, "port": 443, "pins": [PIN], "cookie": COOKIE})
    );
    assert!(secrets.deletes().is_empty());

    session.stop_and_revert(StopReason::UserDisconnect);
    assert_eq!(secrets.deletes(), [ATTEMPT]);
    assert!(secrets.stored().is_empty());
    session.assert_finished_cleanly();
}

#[test]
fn a_stopped_session_is_not_logged_out_by_the_helper() {
    let gateway = FakeGateway::start();
    let setup = setup_for(&gateway);
    let secrets = Arc::clone(&setup.secrets);
    let mut session = Session::start(up_without_otp(settings("utun9", "10.10.0.5")), setup);
    session.bring_up(vec![gateway_route("192.0.2.1")]);
    session.stop_and_revert(StopReason::UserDisconnect);
    assert!(gateway.connections().is_empty());
    assert!(
        !session
            .texts()
            .iter()
            .any(|text| text.contains("logout request"))
    );
    assert_eq!(secrets.deletes(), [ATTEMPT]);
}

#[test]
fn a_session_that_ends_by_itself_is_not_logged_out_by_the_helper() {
    let gateway = FakeGateway::start();
    let setup = setup_for(&gateway);
    let secrets = Arc::clone(&setup.secrets);
    let mut session = Session::start(opened_then_returned(), setup);
    let facts = session.wait_for_exit();
    assert_eq!(facts.furthest_milestone, Milestone::Authenticated);
    session.settle(Duration::from_millis(200));
    assert!(gateway.connections().is_empty());
    let texts = session.texts();
    assert!(texts.contains(&"‹session ended: code -5›"), "{texts:?}");
    assert!(
        !texts.iter().any(|text| text.contains("logout request")),
        "{texts:?}"
    );
    assert_eq!(secrets.deletes(), [ATTEMPT]);
    assert!(secrets.stored().is_empty());
    session.assert_finished_cleanly();
}

#[test]
fn a_session_that_never_opened_has_nothing_to_save_or_log_out() {
    let gateway = FakeGateway::start();
    let setup = setup_for(&gateway);
    let secrets = Arc::clone(&setup.secrets);
    let script = script(|_, link| {
        trust_gateway(link);
        ended(-EPERM)
    });
    let mut session = Session::start(script, setup);
    session.wait_for_exit();
    assert!(secrets.saves().is_empty());
    assert!(secrets.deletes().is_empty());
    assert!(gateway.connections().is_empty());
}

#[test]
fn a_secret_that_could_not_be_saved_is_logged_and_not_deleted() {
    let gateway = FakeGateway::start();
    let setup = Setup {
        secrets: Arc::new(FakeSecrets::refusing_saves()),
        ..setup_for(&gateway)
    };
    let secrets = Arc::clone(&setup.secrets);
    let mut session = Session::start(opened_then_returned(), setup);
    session.wait_for_exit();
    assert!(gateway.connections().is_empty());
    assert!(secrets.deletes().is_empty());
    assert!(
        session.logs().contains(
            &format!(
                "0277070470 run {ATTEMPT}: cannot save the session secret: the store is locked"
            )
            .as_str()
        ),
        "{:?}",
        session.logs()
    );
}

#[test]
fn the_run_reports_its_end_only_after_the_secret_is_deleted() {
    let secrets = Arc::new(FakeSecrets::default());
    let release = secrets.hold_deletes();
    let script = script(|_, link| {
        trust_gateway(link);
        open(link);
        ended(-EIO)
    });
    let setup = Setup {
        secrets: Arc::clone(&secrets),
        ..Setup::default()
    };
    let mut session = Session::start(script, setup);
    assert!(session.wait(std::time::Duration::from_secs(5), |_| {
        !secrets.deletes().is_empty()
    }));
    session.settle(std::time::Duration::from_millis(300));
    assert!(session.end_facts().is_none());
    assert!(!session.run.session_running());
    release.send(()).unwrap();
    session.wait_for_exit();
    assert_eq!(secrets.deletes(), [ATTEMPT]);
    assert!(secrets.stored().is_empty());
}
