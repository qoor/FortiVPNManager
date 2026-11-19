//! The next login after an attempt that ended once its credentials went out, with no code
//! submitted: the gateway may have sent a code email that arrives late.

use std::time::SystemTime;

use fvm_contract::app_helper::{AttemptId, Milestone, StopReason};
use fvm_contract::app_internal::section::{ConnectProgress, ConnectionStatus};

use super::super::machine::{Phase, RetryReason, Stage};
use super::super::{Effect, Failure, Input, Note, OtpFailure, TimerKind};
use super::harness::*;

fn stage(h: &Harness) -> Option<Stage> {
    h.attempt().map(|attempt| attempt.stage)
}

fn waiting(h: &Harness) -> bool {
    matches!(stage(h), Some(Stage::WaitingForStaleOtp { .. }))
}

fn search(attempt: &AttemptId, since: SystemTime) -> Effect {
    Effect::TrashStaleOtp {
        attempt: attempt.clone(),
        since,
    }
}

fn close(attempt: &AttemptId, check_spam: bool) -> Effect {
    Effect::CloseOtpSession {
        attempt: attempt.clone(),
        check_spam,
    }
}

/// The first attempt after launch, at its OTP prompt: the credentials went out at t0 + 2 s, the
/// prompt came at t0 + 3 s.
fn at_the_prompt() -> (Harness, AttemptId) {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpPrompted);
    assert_eq!(h.now, t0() + secs(3.0));
    (h, id)
}

/// The first attempt after launch, its credentials sent at t0 + 2 s and no prompt yet.
fn after_the_credentials() -> (Harness, AttemptId) {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::ConnectedToGateway);
    assert_eq!(h.now, t0() + secs(2.0));
    (h, id)
}

/// Disconnect a second after the prompt, the run's exit half a second later.
fn disconnected_at_the_prompt() -> (Harness, AttemptId) {
    let (mut h, id) = at_the_prompt();
    h.advance(1.0);
    assert_eq!(
        connection_effects(&h.send(Input::Disconnect)),
        [
            Effect::StopRun {
                attempt: id.clone(),
                reason: StopReason::UserDisconnect
            },
            search(&id, t0() + secs(3.0)),
            Effect::CancelTimer(TimerKind::AttemptDeadline),
            Effect::CancelTimer(TimerKind::OtpDeadline),
            h.set(TimerKind::StaleOtp, h.now + secs(20.0)),
        ]
    );
    h.advance(0.5);
    // Idle, but the search keeps the app from being suspended.
    assert_eq!(
        connection_effects(
            &h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::UserDisconnect))
        ),
        []
    );
    assert_eq!(h.cm.phase, Phase::Idle);
    (h, id)
}

/// Connect, and the readiness check that finds everything in place.
fn connect_again(h: &mut Harness) -> (AttemptId, Vec<Effect>) {
    h.send(Input::Connect);
    let next = h.attempt().unwrap().id.clone();
    let effects = connection_effects(&h.send(Input::ReadinessChecked(ready())));
    (next, effects)
}

#[test]
fn the_next_login_waits_until_the_stopped_attempts_code_is_trashed() {
    let (mut h, stopped) = disconnected_at_the_prompt();
    h.advance(2.5);
    let connected_at = h.now;
    let (next, effects) = connect_again(&mut h);
    assert_eq!(
        effects,
        [Effect::Note {
            attempt: Some(next.clone()),
            note: Note::WaitingForStaleOtp
        }]
    );
    assert!(waiting(&h));
    assert!(
        matches!(
            h.cm.connection_section().status,
            ConnectionStatus::Connecting {
                progress: ConnectProgress::ConnectingToGmail,
                ..
            }
        ),
        "{:?}",
        h.cm.connection_section().status
    );
    assert_eq!(
        connection_effects(&h.send(Input::StaleOtpTrashed(stranger()))),
        []
    );
    h.advance(1.5);
    assert_eq!(
        connection_effects(&h.send(Input::StaleOtpTrashed(stopped.clone()))),
        [
            Effect::Note {
                attempt: Some(next.clone()),
                note: Note::StaleOtpTrashed
            },
            Effect::OpenOtpSession {
                attempt: next.clone()
            },
            // The wait does not count toward the attempt's time limit.
            h.set(TimerKind::AttemptDeadline, connected_at + secs(150.0 + 1.5)),
            Effect::CancelTimer(TimerKind::StaleOtp),
        ]
    );
    assert_eq!(
        run_starts(&h.send(Input::OtpSessionReady(next.clone()))),
        [account()]
    );
    h.run_milestones(&next, Milestone::TunnelUp);
    assert!(h.connected().is_some());
    assert_eq!(
        connection_effects(&h.send(Input::StaleOtpTrashed(stopped))),
        []
    );
}

#[test]
fn a_stop_after_the_credentials_before_the_prompt_searches_too() {
    // The gateway sends the email when it accepts the password, which may be before the prompt
    // reaches the app.
    let (mut h, id) = after_the_credentials();
    h.advance(0.5);
    let effects = h.send(Input::Disconnect);
    assert!(
        effects.contains(&search(&id, t0() + secs(2.0))),
        "{effects:?}"
    );
    h.exit(ended(Milestone::ConnectedToGateway).by_helper(StopReason::UserDisconnect));
    let (next, effects) = connect_again(&mut h);
    assert_eq!(
        effects,
        [Effect::Note {
            attempt: Some(next),
            note: Note::WaitingForStaleOtp
        }]
    );

    // The Helper's end, whose immediate reconnect waits.
    let (mut h, id) = after_the_credentials();
    let effects = h.send(Input::HelperConnectionLost);
    assert!(
        effects.contains(&search(&id, t0() + secs(2.0))),
        "{effects:?}"
    );
    assert_eq!(
        h.attempt().map(|attempt| attempt.reason),
        Some(retry(RetryReason::ForcedTermination))
    );
    let next = h.current_id();
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(ready()))),
        [Effect::Note {
            attempt: Some(next),
            note: Note::WaitingForStaleOtp
        }]
    );
}

#[test]
fn every_other_end_after_the_credentials_searches() {
    // The attempt's time limit, before and after the prompt.
    let (mut h, id) = after_the_credentials();
    let effects = h.fire(TimerKind::AttemptDeadline);
    assert!(
        effects.contains(&search(&id, t0() + secs(2.0))),
        "{effects:?}"
    );
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.milestone(Milestone::Spawned);
    h.milestone(Milestone::ConnectedToGateway);
    h.advance(120.0);
    h.milestone(Milestone::OtpPrompted);
    let prompted_at = h.now;
    let effects = h.fire(TimerKind::AttemptDeadline);
    assert!(effects.contains(&search(&id, prompted_at)), "{effects:?}");

    // The session's own end after the prompt, a network error.
    let (mut h, id) = at_the_prompt();
    let effects = h.exit(ended(Milestone::OtpPrompted));
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::Network)
    );
    assert!(
        effects.contains(&search(&id, t0() + secs(3.0))),
        "{effects:?}"
    );

    // The session's own end at login without the password asked again: a network error, not a
    // rejected login.
    let (mut h, id) = after_the_credentials();
    let effects = h.exit(ended(Milestone::ConnectedToGateway));
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::Network)
    );
    assert!(
        effects.contains(&search(&id, t0() + secs(2.0))),
        "{effects:?}"
    );
}

#[test]
fn no_search_after_a_rejected_login() {
    // The client asked for the password again: the gateway sends no code email for a rejected
    // password, and the next try is 90 s away.
    let (mut h, id) = after_the_credentials();
    let effects =
        h.exit(ended(Milestone::ConnectedToGateway).by_helper(StopReason::PromptRepeated));
    assert_eq!(
        h.retry_wait().map(|(_, reason)| reason),
        Some(RetryReason::AuthFailure(Failure::AuthenticationRejected))
    );
    assert!(effects.contains(&close(&id, false)), "{effects:?}");
    assert_eq!(timer_set(&effects, TimerKind::StaleOtp), None);
}

#[test]
fn no_search_after_the_otp_time_limit() {
    // A plain close with the spam check, so a hint that the email is in the spam folder comes at
    // once.
    let (mut h, id) = at_the_prompt();
    let effects = connection_effects(&h.fire(TimerKind::OtpDeadline));
    assert!(effects.contains(&close(&id, true)), "{effects:?}");
    assert_eq!(timer_set(&effects, TimerKind::StaleOtp), None);

    // The accepted gap: the Helper ending during that stop reconnects at once without a wait.
    h.send(Input::HelperConnectionLost);
    assert_eq!(
        h.attempt().map(|attempt| attempt.reason),
        Some(retry(RetryReason::ForcedTermination))
    );
    let next = h.attempt().unwrap().id.clone();
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(ready()))),
        [Effect::OpenOtpSession { attempt: next }]
    );
}

#[test]
fn the_wait_is_decided_at_the_next_login() {
    // After the session's own end at the prompt the next try comes 10 s later, while the search
    // still runs.
    let (mut h, id) = at_the_prompt();
    h.exit(ended(Milestone::OtpPrompted));
    h.fire(TimerKind::Retry);
    let next = h.attempt().unwrap().id.clone();
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(ready()))),
        [Effect::Note {
            attempt: Some(next.clone()),
            note: Note::WaitingForStaleOtp
        }]
    );
    let effects = connection_effects(&h.fire(TimerKind::StaleOtp));
    assert!(effects.contains(&close(&id, false)), "{effects:?}");
    assert!(
        effects.contains(&Effect::OpenOtpSession { attempt: next }),
        "{effects:?}"
    );

    // A login after the limit goes ahead although its timer has not fired yet.
    let (mut h, stopped) = disconnected_at_the_prompt();
    h.advance(25.0);
    let (next, effects) = connect_again(&mut h);
    assert_eq!(effects, [Effect::OpenOtpSession { attempt: next }]);
    assert_eq!(
        connection_effects(&h.fire(TimerKind::StaleOtp)),
        [close(&stopped, false)]
    );
    assert_eq!(stage(&h), Some(Stage::PreparingOtp));
}

#[test]
fn the_wait_ends_when_the_stopped_attempts_session_fails() {
    let (mut h, stopped) = disconnected_at_the_prompt();
    h.advance(2.0);
    let connected_at = h.now;
    let (next, effects) = connect_again(&mut h);
    assert_eq!(
        effects,
        [Effect::Note {
            attempt: Some(next.clone()),
            note: Note::WaitingForStaleOtp
        }]
    );
    h.advance(1.0);
    assert_eq!(
        connection_effects(&h.send(Input::OtpSessionFailed(
            stopped.clone(),
            OtpFailure::Network
        ))),
        [
            Effect::Note {
                attempt: Some(next.clone()),
                note: Note::StaleOtpSessionEnded
            },
            Effect::OpenOtpSession {
                attempt: next.clone()
            },
            h.set(TimerKind::AttemptDeadline, connected_at + secs(150.0 + 1.0)),
            Effect::CancelTimer(TimerKind::StaleOtp),
        ]
    );
    // The new attempt's own session is untouched.
    assert_eq!(
        run_starts(&h.send(Input::OtpSessionReady(next))),
        [account()]
    );

    // With no login waiting, the next one does not wait either.
    let (mut h, stopped) = disconnected_at_the_prompt();
    assert_eq!(
        connection_effects(&h.send(Input::OtpSessionFailed(stopped, OtpFailure::Authentication))),
        [
            Effect::CancelTimer(TimerKind::StaleOtp),
            Effect::HoldActivity(false)
        ]
    );
    let (next, effects) = connect_again(&mut h);
    assert_eq!(effects, [Effect::OpenOtpSession { attempt: next }]);
}

#[test]
fn the_wait_runs_out_twenty_seconds_after_the_stop() {
    let (mut h, stopped) = disconnected_at_the_prompt();
    let limit = h.timer(TimerKind::StaleOtp).unwrap();
    h.advance(15.0);
    let (next, effects) = connect_again(&mut h);
    assert_eq!(
        effects,
        [Effect::Note {
            attempt: Some(next.clone()),
            note: Note::WaitingForStaleOtp
        }]
    );
    let effects = connection_effects(&h.fire(TimerKind::StaleOtp));
    assert_eq!(h.now, limit);
    assert_eq!(
        effects,
        [
            // The stopped attempt's session stops looking.
            close(&stopped, false),
            Effect::Note {
                attempt: Some(next.clone()),
                note: Note::StaleOtpWaitRanOut
            },
            Effect::OpenOtpSession { attempt: next },
            // Waited from the readiness check, right at Connect, to the limit.
            h.set(TimerKind::AttemptDeadline, limit + secs(150.0)),
        ]
    );
    assert_eq!(stage(&h), Some(Stage::PreparingOtp));
    assert_eq!(
        connection_effects(&h.send(Input::StaleOtpTrashed(stopped))),
        []
    );
}

#[test]
fn the_search_holds_activity_until_its_limit_when_no_login_waits() {
    let (mut h, stopped) = disconnected_at_the_prompt();
    assert_eq!(
        connection_effects(&h.fire(TimerKind::StaleOtp)),
        [close(&stopped, false), Effect::HoldActivity(false)]
    );
    h.advance(1.0);
    let (next, effects) = connect_again(&mut h);
    assert_eq!(effects, [Effect::OpenOtpSession { attempt: next }]);
}

#[test]
fn no_search_on_quit_or_sleep() {
    // The quit: a plain close, approved at the run's exit as before.
    let (mut h, id) = at_the_prompt();
    let effects = h.send(Input::QuitRequested);
    assert_eq!(stop_reasons(&effects), [StopReason::AppQuit]);
    assert!(effects.contains(&close(&id, false)), "{effects:?}");
    assert_eq!(timer_set(&effects, TimerKind::StaleOtp), None);
    h.advance(0.5);
    let exit = h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::AppQuit));
    assert_eq!(exit.last(), Some(&Effect::ApproveQuit));

    // A quit while the next login waits is approved at once.
    let (mut h, _) = disconnected_at_the_prompt();
    let (next, effects) = connect_again(&mut h);
    assert_eq!(
        effects,
        [Effect::Note {
            attempt: Some(next),
            note: Note::WaitingForStaleOtp
        }]
    );
    let quit = connection_effects(&h.send(Input::QuitRequested));
    assert_eq!(quit.last(), Some(&Effect::ApproveQuit));
    assert!(!opens_otp_session(&quit), "{quit:?}");

    // The sleep: the next login comes only after the wake.
    let (mut h, id) = at_the_prompt();
    let effects = h.sleep();
    assert!(effects.contains(&close(&id, false)), "{effects:?}");
    assert_eq!(timer_set(&effects, TimerKind::StaleOtp), None);
    h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::Sleep));
    h.advance(600.0);
    h.wake();
    h.settle(observe().dead());
    let next = h.attempt().unwrap().id.clone();
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(ready()))),
        [Effect::OpenOtpSession { attempt: next }]
    );
}

#[test]
fn no_search_without_credentials_or_once_the_code_was_submitted() {
    // Stopped before the credentials went out.
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::Spawned);
    let effects = h.send(Input::Disconnect);
    assert!(effects.contains(&close(&id, false)), "{effects:?}");
    assert_eq!(timer_set(&effects, TimerKind::StaleOtp), None);
    h.exit(ended(Milestone::Spawned).by_helper(StopReason::UserDisconnect));
    let (next, effects) = connect_again(&mut h);
    assert_eq!(effects, [Effect::OpenOtpSession { attempt: next }]);

    // Stopped after the submission: the session trashed the used email and closed itself.
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::OtpSubmitted);
    let effects = connection_effects(&h.send(Input::Disconnect));
    assert_eq!(
        effects,
        [
            Effect::StopRun {
                attempt: id,
                reason: StopReason::UserDisconnect
            },
            Effect::CancelTimer(TimerKind::AttemptDeadline),
        ]
    );
    h.exit(ended(Milestone::OtpSubmitted).by_helper(StopReason::UserDisconnect));
    let (next, effects) = connect_again(&mut h);
    assert_eq!(effects, [Effect::OpenOtpSession { attempt: next }]);

    // The Gmail session failed after the prompt: no session is left to search.
    let (mut h, id) = at_the_prompt();
    let effects = h.send(Input::OtpSessionFailed(id, OtpFailure::Network));
    assert_eq!(timer_set(&effects, TimerKind::StaleOtp), None);
    h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::OtpUnavailable));
    h.fire(TimerKind::Retry);
    let next = h.attempt().unwrap().id.clone();
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(ready()))),
        [Effect::OpenOtpSession { attempt: next }]
    );
}

#[test]
fn the_failure_rules_stay_as_they_are() {
    let (mut h, _) = at_the_prompt();
    h.fire(TimerKind::OtpDeadline);
    h.exit(ended(Milestone::OtpPrompted).by_helper(StopReason::OtpTimeout));
    let failed_at = h.now;
    assert_eq!(
        h.retry_wait(),
        Some((
            failed_at + secs(90.0),
            RetryReason::AuthFailure(Failure::OtpNotReceived { in_spam: false })
        ))
    );
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);

    // A forced end at the prompt reconnects at once, uncounted, as before.
    let (mut h, _) = at_the_prompt();
    h.send(Input::HelperConnectionLost);
    assert_eq!(
        h.attempt().map(|attempt| attempt.reason),
        Some(retry(RetryReason::ForcedTermination))
    );
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 0);
}
