//! Stopping the VPN for an update's install, and connecting again when the install does not start.

use fvm_contract::app_helper::{Milestone, StopReason};
use fvm_contract::app_internal::ButtonState;

use super::super::machine::{AfterStop, AttemptReason, Phase, Stage};
use super::super::{Effect, Failure, Input, OtpFailure, TimerKind};
use super::harness::*;

fn stopped(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|effect| **effect == Effect::StoppedForUpdate)
        .count()
}

fn connection_buttons(h: &Harness) -> [ButtonState; 3] {
    let section = h.cm.connection_section();
    [section.connect, section.disconnect, section.reconnect_now]
}

#[test]
fn a_session_stops_as_for_a_quit_and_the_stop_is_reported_at_its_exit() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    let stop = h.send(Input::StopForUpdate);
    assert_eq!(stop_reasons(&stop), [StopReason::AppQuit]);
    assert_eq!(
        timer_set(&stop, TimerKind::QuitDeadline),
        Some(h.now + secs(12.0))
    );
    assert_eq!(stopped(&stop), 0);
    assert!(!h.cm.wants_vpn());
    assert_eq!(h.stopping().map(|stop| stop.after), Some(AfterStop::Update));
    assert_eq!(
        connection_buttons(&h),
        [
            ButtonState::Disabled,
            ButtonState::Hidden,
            ButtonState::Disabled
        ]
    );
    for input in [Input::Connect, Input::Disconnect, Input::ReconnectNow] {
        assert_eq!(connection_effects(&h.send(input)), []);
    }
    assert_eq!(h.stopping().map(|stop| stop.after), Some(AfterStop::Update));

    h.advance(1.0);
    let exit = h.exit_for(
        &id,
        ended(Milestone::TunnelUp).by_helper(StopReason::AppQuit),
    );
    assert_eq!(stopped(&exit), 1);
    assert!(cancels(&exit, TimerKind::QuitDeadline));
    assert!(!exit.contains(&Effect::ApproveQuit));
    assert_eq!(h.cm.phase, Phase::Idle);
    assert_eq!(
        connection_buttons(&h),
        [
            ButtonState::Disabled,
            ButtonState::Hidden,
            ButtonState::Disabled
        ]
    );
}

#[test]
fn an_attempt_under_way_stops_and_one_not_running_yet_ends_at_once() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    let stop = h.send(Input::StopForUpdate);
    assert_eq!(stop_reasons(&stop), [StopReason::AppQuit]);
    assert_eq!(stopped(&h.exit(ended(Milestone::OtpPrompted))), 1);

    let mut h = Harness::new();
    h.launch();
    let stop = h.send(Input::StopForUpdate);
    assert!(stop_reasons(&stop).is_empty());
    assert_eq!(stopped(&stop), 1);
    assert_eq!(h.cm.phase, Phase::Idle);
    assert_eq!(h.timer(TimerKind::QuitDeadline), None);
}

#[test]
fn nothing_running_is_reported_stopped_at_once_and_a_failure_stays_shown() {
    let mut h = Harness::new();
    h.launch_with(false, None);
    assert_eq!(stopped(&h.send(Input::StopForUpdate)), 1);
    assert_eq!(h.cm.phase, Phase::Idle);

    let mut h = Harness::new();
    h.launch();
    h.send(Input::ReadinessChecked(ready()));
    let id = h.attempt().unwrap().id.clone();
    h.send(Input::OtpSessionFailed(id, OtpFailure::Authentication));
    assert_eq!(stopped(&h.send(Input::StopForUpdate)), 1);
    assert_eq!(h.cm.phase, Phase::Failed(Failure::GmailRejected));
    h.send(Input::UpdateFailed { reconnect: false });
    assert_eq!(h.cm.phase, Phase::Failed(Failure::GmailRejected));

    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.exit(ended(Milestone::Started));
    assert!(h.retry_wait().is_some());
    let stop = h.send(Input::StopForUpdate);
    assert_eq!(stopped(&stop), 1);
    assert!(cancels(&stop, TimerKind::Retry));
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn the_quit_time_limit_reports_the_stop_without_the_exit_once() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    h.send(Input::StopForUpdate);
    let deadline = h.fire(TimerKind::QuitDeadline);
    assert_eq!(stopped(&deadline), 1);
    assert!(!deadline.contains(&Effect::ApproveQuit));
    assert_eq!(h.timer(TimerKind::QuitDeadline), None);
    let exit = h.exit_for(
        &id,
        ended(Milestone::TunnelUp).by_helper(StopReason::AppQuit),
    );
    assert_eq!(stopped(&exit), 0);
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn a_failed_install_connects_again_only_when_asked() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    h.send(Input::StopForUpdate);
    h.exit_for(
        &id,
        ended(Milestone::TunnelUp).by_helper(StopReason::AppQuit),
    );
    h.send(Input::UpdateFailed { reconnect: true });
    assert_eq!(
        h.attempt().map(|attempt| attempt.reason),
        Some(AttemptReason::User)
    );
    assert!(h.cm.wants_vpn());
    assert_eq!(connection_buttons(&h)[1], ButtonState::Enabled);

    let mut h = Harness::new();
    h.launch_with(false, None);
    h.send(Input::StopForUpdate);
    assert_eq!(
        connection_effects(&h.send(Input::UpdateFailed { reconnect: false })),
        []
    );
    assert_eq!(h.cm.phase, Phase::Idle);
    assert_eq!(connection_buttons(&h)[0], ButtonState::Enabled);
    h.send(Input::Connect);
    assert!(h.attempt().is_some());
}

#[test]
fn a_failed_install_after_the_time_limit_reconnects_after_the_exit() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    h.send(Input::StopForUpdate);
    h.fire(TimerKind::QuitDeadline);
    h.send(Input::UpdateFailed { reconnect: true });
    assert_eq!(
        h.stopping().map(|stop| stop.after),
        Some(AfterStop::Reconnect(AttemptReason::User))
    );
    h.exit_for(
        &id,
        ended(Milestone::TunnelUp).by_helper(StopReason::AppQuit),
    );
    assert_eq!(
        h.attempt().map(|attempt| attempt.reason),
        Some(AttemptReason::User)
    );
}

#[test]
fn a_quit_during_the_stop_is_a_normal_quit() {
    let mut h = Harness::new();
    h.launch_connected();
    let id = h.connected().unwrap().attempt.clone();
    h.send(Input::StopForUpdate);
    let quit = h.send(Input::QuitRequested);
    assert!(stop_reasons(&quit).is_empty());
    assert_eq!(h.stopping().map(|stop| stop.after), Some(AfterStop::Quit));
    let exit = h.exit_for(
        &id,
        ended(Milestone::TunnelUp).by_helper(StopReason::AppQuit),
    );
    assert_eq!(stopped(&exit), 0);
    assert_eq!(exit.last(), Some(&Effect::ApproveQuit));
    assert_eq!(
        connection_effects(&h.send(Input::UpdateFailed { reconnect: true })),
        []
    );
}

#[test]
fn the_schedule_does_not_connect_during_the_stop() {
    let mut h = Harness::with_settings(scheduled(weekday_hours()));
    h.launch_with(false, None);
    assert!(h.timer(TimerKind::Schedule).is_some());
    h.send(Input::StopForUpdate);
    assert_eq!(h.timer(TimerKind::Schedule), None);
    h.send(Input::UpdateFailed { reconnect: false });
    assert!(h.timer(TimerKind::Schedule).is_some());
}

#[test]
fn a_reconnect_after_a_failed_install_waits_for_the_stopped_attempts_late_code() {
    for trashed in [true, false] {
        let mut h = Harness::new();
        h.launch();
        let id = h.start_tunnel();
        h.run_milestones(&id, Milestone::OtpPrompted);
        let prompted = h.now;
        h.advance(1.0);
        let stop = h.send(Input::StopForUpdate);
        assert_eq!(stop_reasons(&stop), [StopReason::AppQuit]);
        assert!(stop.contains(&Effect::TrashStaleOtp {
            attempt: id.clone(),
            since: prompted,
        }));
        assert_eq!(
            timer_set(&stop, TimerKind::StaleOtp),
            Some(h.now + secs(20.0))
        );
        h.advance(0.5);
        let exit = h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::AppQuit));
        assert_eq!(stopped(&exit), 1);

        h.send(Input::UpdateFailed { reconnect: true });
        let login = h.send(Input::ReadinessChecked(ready()));
        assert!(
            matches!(
                h.attempt().map(|attempt| attempt.stage),
                Some(Stage::WaitingForStaleOtp { .. })
            ),
            "{:?}",
            h.cm.phase
        );
        assert!(
            !login
                .iter()
                .any(|effect| matches!(effect, Effect::OpenOtpSession { .. }))
        );

        let over = if trashed {
            h.send(Input::StaleOtpTrashed(id.clone()))
        } else {
            h.fire(TimerKind::StaleOtp)
        };
        assert!(
            over.iter()
                .any(|effect| matches!(effect, Effect::OpenOtpSession { .. })),
            "{over:?}"
        );
        assert_eq!(
            h.attempt().map(|attempt| attempt.stage),
            Some(Stage::PreparingOtp)
        );
    }
}

#[test]
fn a_quit_still_skips_the_late_search() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    let quit = h.send(Input::QuitRequested);
    assert!(
        !quit
            .iter()
            .any(|effect| matches!(effect, Effect::TrashStaleOtp { .. }))
    );
}
