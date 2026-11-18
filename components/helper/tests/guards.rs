mod common;

use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use common::*;
use fvm_contract::app_helper::StopReason;
use fvm_helper_core::recovery::RecordSlot;
use fvm_helper_core::session::{Report, Run, SessionManager};

fn start(script: Script, on_report: impl FnMut(Report) + Send + 'static) -> (Run, RunDirectory) {
    let directory = RunDirectory::new();
    let sessions = SessionManager::new(
        Arc::new(FakeBackend::new([script])),
        Arc::new(FakeSecrets::default()),
        directory.path.clone(),
        timing(),
    );
    let run = sessions
        .start(&mut request(ATTEMPT), RecordSlot::First, None, on_report)
        .unwrap();
    (run, directory)
}

#[test]
fn submit_otp_and_stop_from_on_report_panic() {
    let cell: Arc<OnceLock<Run>> = Arc::new(OnceLock::new());
    let shared = Arc::clone(&cell);
    let (results, received) = mpsc::channel();
    let script = script(|_, link| {
        std::thread::sleep(Duration::from_millis(200));
        line(link, "late");
        ended(-EIO)
    });
    let (run, _directory) = start(script, move |report| {
        if let (Report::Output(_), Some(run)) = (&report, shared.get()) {
            let stop =
                panic::catch_unwind(AssertUnwindSafe(|| run.stop(StopReason::UserDisconnect)));
            let submit = panic::catch_unwind(AssertUnwindSafe(|| run.submit_otp(OTP.to_owned())));
            let _ = results.send((stop.is_err(), submit.is_err()));
        }
    });
    assert!(cell.set(run).is_ok());
    assert_eq!(
        received.recv_timeout(Duration::from_secs(5)).unwrap(),
        (true, true)
    );
}

#[test]
#[should_panic(expected = "a network change was dropped unanswered")]
fn dropping_an_unanswered_network_change_panics() {
    let (sender, reports) = mpsc::channel();
    let (_run, _directory) = start(
        up_without_otp(settings("utun9", "10.10.0.5")),
        move |report| {
            let _ = sender.send(report);
        },
    );
    while let Ok(report) = reports.recv_timeout(Duration::from_secs(5)) {
        if let Report::ChangeNetwork(job) = report {
            drop(job);
        }
    }
}
