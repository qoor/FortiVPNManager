//! Misuse that would leave a run waiting forever fails at once.

mod common;

use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use common::*;
use fvm_contract::app_helper::StopReason;
use fvm_watchdog::{Report, Run};

#[test]
fn submit_otp_and_stop_from_on_report_panic() {
    let cell: Arc<OnceLock<Run>> = Arc::new(OnceLock::new());
    let shared = Arc::clone(&cell);
    let (results, received) = mpsc::channel();
    let launch = shell(
        "/bin/sh",
        vec![
            "-c".to_owned(),
            "/bin/sleep 0.2; echo late; exit 0".to_owned(),
        ],
    );
    let run = Run::start(
        &fake_backend(Duration::from_secs(2)),
        &launch,
        PASSWORD.to_owned(),
        Setup::default().timing,
        move |report| {
            if let (Report::Output(_), Some(run)) = (&report, shared.get()) {
                let stop =
                    panic::catch_unwind(AssertUnwindSafe(|| run.stop(StopReason::UserDisconnect)));
                let submit =
                    panic::catch_unwind(AssertUnwindSafe(|| run.submit_otp(OTP.to_owned())));
                let _ = results.send((stop.is_err(), submit.is_err()));
            }
        },
    );
    assert!(cell.set(run).is_ok());
    assert_eq!(
        received.recv_timeout(Duration::from_secs(5)).unwrap(),
        (true, true)
    );
}

#[test]
#[should_panic(expected = "a route repair was dropped unfinished")]
fn dropping_an_unfinished_route_repair_panics() {
    let launch = shell(
        "/bin/sh",
        vec![
            "-c".to_owned(),
            "echo 'INFO:   Setting new routes...'; exit 1".to_owned(),
        ],
    );
    let (sender, reports) = mpsc::channel();
    let _run = Run::start(
        &fake_backend(Duration::from_secs(2)),
        &launch,
        PASSWORD.to_owned(),
        Setup::default().timing,
        move |report| {
            let _ = sender.send(report);
        },
    );
    while let Ok(report) = reports.recv_timeout(Duration::from_secs(5)) {
        if let Report::RepairRoutes(repair) = report {
            drop(repair);
        }
    }
}
