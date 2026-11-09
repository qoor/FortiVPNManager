//! The readiness gate, account changes and what the connection section shows.

use fvm_contract::app_helper::Milestone;
use fvm_contract::app_internal::ButtonState;
use fvm_contract::app_internal::section::{ConnectionIcon, ConnectionStatus};

use super::super::machine::{AttemptReason, Phase, Stage};
use super::super::readiness::SetupNeed;
use super::super::{
    Effect, Failure, FailureLedger, HelperFacts, Input, OtpFailure, ReadinessFacts, Settings,
    TimerKind, VpnAccount,
};
use super::harness::*;

fn reason(h: &Harness) -> Option<AttemptReason> {
    h.attempt().map(|attempt| attempt.reason)
}

fn without_password() -> ReadinessFacts {
    ReadinessFacts {
        vpn_password_stored: false,
        ..ready()
    }
}

fn with_account(account: VpnAccount) -> Settings {
    Settings {
        account,
        ..settings()
    }
}

#[test]
fn missing_setup_blocks_the_attempt() {
    let mut h = Harness::new();
    h.launch_with(false, None);
    h.send(Input::Connect);
    let facts = ReadinessFacts {
        helper: HelperFacts::NotRegistered,
        ..without_password()
    };
    let effects = connection_effects(&h.send(Input::ReadinessChecked(facts)));
    // The first readiness result of the launch finds setup missing, so Settings opens once.
    assert_eq!(
        effects,
        [
            Effect::OpenSettings,
            Effect::CancelTimer(TimerKind::AttemptDeadline),
            Effect::HoldActivity(false)
        ]
    );
    assert!(!opens_otp_session(&effects));
    assert_eq!(
        h.cm.phase,
        Phase::SetupRequired(vec![SetupNeed::VpnPassword, SetupNeed::HelperApproval])
    );
    let section = h.cm.connection_section();
    assert_eq!(section.icon, ConnectionIcon::Error);
    assert!(
        matches!(section.status, ConnectionStatus::SetupRequired { ref items } if items.len() == 2),
        "{:?}",
        section.status
    );
    assert_eq!(section.connect, ButtonState::Enabled);
    assert_eq!(section.disconnect, ButtonState::Enabled);
    assert_ne!(section.reconnect_now, ButtonState::Enabled);
}

#[test]
fn completing_the_setup_while_a_connection_is_wanted_connects() {
    let mut h = Harness::new();
    h.launch();
    h.send(Input::ReadinessChecked(ReadinessFacts {
        gmail_credential_stored: false,
        ..ready()
    }));
    assert_eq!(
        h.cm.phase,
        Phase::SetupRequired(vec![SetupNeed::GmailCredential])
    );
    assert!(h.cm.wants_connection);
    let effects = connection_effects(&h.send(Input::ReadinessChecked(ready())));
    assert_eq!(
        effects,
        [
            Effect::OpenOtpSession { attempt: id(2) },
            h.set(TimerKind::AttemptDeadline, h.now + secs(150.0)),
            Effect::HoldActivity(true),
        ]
    );
    assert_eq!(reason(&h), Some(AttemptReason::SetupCompleted));
    assert_eq!(h.attempt().unwrap().stage, Stage::PreparingOtp);
    h.run_to_connected();
}

#[test]
fn completing_the_setup_while_disconnected_goes_idle() {
    let mut h = Harness::new();
    assert_eq!(
        connection_effects(&h.launch_with(false, None)),
        [Effect::CheckReadiness]
    );
    h.send(Input::ReadinessChecked(without_password()));
    assert_eq!(
        h.cm.phase,
        Phase::SetupRequired(vec![SetupNeed::VpnPassword])
    );
    let section = h.cm.connection_section();
    assert_eq!(section.connect, ButtonState::Enabled);
    assert_eq!(section.disconnect, ButtonState::Hidden);
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(ready()))),
        []
    );
    assert_eq!(h.cm.phase, Phase::Idle);
}

#[test]
fn losing_the_setup_while_waiting_shows_the_setup() {
    let mut h = Harness::new();
    h.launch();
    h.start_tunnel();
    h.exit(ended(Milestone::Spawned));
    let mismatch = ReadinessFacts {
        helper: HelperFacts::Answered {
            build: "99".into(),
            status: None,
        },
        ..ready()
    };
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(mismatch))),
        [
            Effect::CancelTimer(TimerKind::Retry),
            Effect::HoldActivity(false)
        ]
    );
    assert_eq!(
        h.cm.phase,
        Phase::SetupRequired(vec![SetupNeed::HelperVersion])
    );
}

#[test]
fn readiness_changes_during_a_run_apply_to_the_next_attempt() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(without_password()))),
        []
    );
    h.run_milestones(&id, Milestone::TunnelUp);
    assert!(h.connected().is_some());
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(without_password()))),
        []
    );
    assert!(h.connected().is_some());
}

#[test]
fn a_username_change_resets_the_ledger_and_needs_setup_when_the_password_is_missing() {
    let mut h = Harness::new();
    h.launch();
    let id = h.start_tunnel();
    h.run_milestones(&id, Milestone::ConnectedToGateway);
    h.exit(ended(Milestone::ConnectedToGateway));
    assert_eq!(h.cm.ledger.consecutive_auth_failures, 1);
    let effects = connection_effects(&h.send(Input::SettingsChanged(with_account(bob()))));
    assert_eq!(
        effects,
        [
            Effect::CheckReadiness,
            Effect::SaveLedger(FailureLedger::new("bob@vpn.example.com".into()))
        ]
    );
    assert_eq!(h.cm.settings.account, bob());
    h.send(Input::ReadinessChecked(without_password()));
    assert_eq!(
        h.cm.phase,
        Phase::SetupRequired(vec![SetupNeed::VpnPassword])
    );
    assert!(
        h.connection_timers().is_empty(),
        "{:?}",
        h.connection_timers()
    );
    // Entering the password completes the setup; a connection is still wanted.
    h.send(Input::ReadinessChecked(ready()));
    assert_eq!(reason(&h), Some(AttemptReason::SetupCompleted));
    h.start_tunnel();
    assert_eq!(run_starts(&h.effects)[0].username, "bob");
}

/// An account change applies from the next connection: the attempt under way keeps the account its
/// readiness check was for and never starts the client for the unchecked one.
#[test]
fn an_account_change_while_the_gmail_session_starts_applies_to_the_next_attempt() {
    let mut h = Harness::new();
    h.launch();
    let id = h.attempt().unwrap().id.clone();
    h.send(Input::ReadinessChecked(ready()));
    assert_eq!(h.attempt().unwrap().stage, Stage::PreparingOtp);
    assert!(
        h.send(Input::SettingsChanged(with_account(bob())))
            .contains(&Effect::CheckReadiness)
    );
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(without_password()))),
        []
    );
    let effects = h.send(Input::OtpSessionReady(id.clone()));
    assert_eq!(run_starts(&effects), [account()]);
    assert_eq!(h.cm.settings.account, bob());
    // The next attempt checks readiness for bob and stops at the setup.
    h.exit_for(&id, ended(Milestone::Spawned));
    h.fire(TimerKind::Retry);
    assert!(h.effects.contains(&Effect::CheckReadiness));
    h.send(Input::ReadinessChecked(without_password()));
    assert_eq!(
        h.cm.phase,
        Phase::SetupRequired(vec![SetupNeed::VpnPassword])
    );
    // Once bob's password is stored, bob is used.
    h.send(Input::ReadinessChecked(ready()));
    h.start_tunnel();
    assert_eq!(run_starts(&h.effects), [bob()]);
}

#[test]
fn an_account_change_while_checking_readiness_keeps_the_attempt_on_its_own_account() {
    let mut h = Harness::new();
    h.launch();
    let id = h.attempt().unwrap().id.clone();
    assert_eq!(h.attempt().unwrap().stage, Stage::CheckingReadiness);
    h.send(Input::SettingsChanged(with_account(bob())));
    // The launch check (alice) answers first, then the check for bob.
    h.send(Input::ReadinessChecked(ready()));
    assert_eq!(
        connection_effects(&h.send(Input::ReadinessChecked(without_password()))),
        []
    );
    assert_eq!(run_starts(&h.send(Input::OtpSessionReady(id))), [account()]);
}

#[test]
fn an_account_change_while_idle_shows_the_setup() {
    let mut h = Harness::new();
    h.launch_with(false, None);
    h.send(Input::ReadinessChecked(ready()));
    let mut other = account();
    other.host = "other.example.com".into();
    other.trusted_certs = vec![];
    assert!(
        h.send(Input::SettingsChanged(with_account(other)))
            .contains(&Effect::CheckReadiness)
    );
    h.send(Input::ReadinessChecked(without_password()));
    assert_eq!(
        h.cm.phase,
        Phase::SetupRequired(vec![SetupNeed::VpnPassword])
    );
    assert_eq!(h.cm.connection_section().icon, ConnectionIcon::Error);
}

#[test]
fn an_account_change_while_connected_keeps_the_session_until_it_ends() {
    let mut h = Harness::new();
    h.launch_connected();
    h.send(Input::SettingsChanged(with_account(bob())));
    h.send(Input::ReadinessChecked(without_password()));
    assert!(h.connected().is_some());
    assert!(h.cm.connection_section().account_applies_next_connection);
    h.advance(3.0 * 3600.0);
    h.exit(ended(Milestone::TunnelUp).up(3.0 * 3600.0));
    h.fire(TimerKind::Retry);
    assert!(
        h.effects.contains(&Effect::CheckReadiness),
        "the next attempt checks readiness again"
    );
    h.send(Input::ReadinessChecked(without_password()));
    assert_eq!(
        h.cm.phase,
        Phase::SetupRequired(vec![SetupNeed::VpnPassword])
    );
}

#[test]
fn an_account_change_keeps_failed() {
    let mut h = Harness::new();
    h.launch();
    h.send(Input::ReadinessChecked(ready()));
    h.send(Input::OtpSessionFailed(id(1), OtpFailure::Authentication));
    let failed = Phase::Failed(Failure::GmailRejected);
    assert_eq!(h.cm.phase, failed);
    h.send(Input::SettingsChanged(with_account(bob())));
    h.send(Input::ReadinessChecked(without_password()));
    assert_eq!(h.cm.phase, failed);
    h.send(Input::ReadinessChecked(ready()));
    assert_eq!(h.cm.phase, failed);
}

#[test]
fn a_realm_only_change_needs_no_recheck() {
    let mut h = Harness::new();
    h.launch_with(false, None);
    let mut account = account();
    account.realm = "staff".into();
    assert_eq!(
        connection_effects(&h.send(Input::SettingsChanged(with_account(account)))),
        []
    );
    assert_eq!(h.cm.settings.account.realm, "staff");
}

#[test]
fn readiness_facts_map_to_setup_items() {
    let check = |facts: ReadinessFacts| {
        let mut h = Harness::new();
        h.launch_with(false, None);
        h.send(Input::Connect);
        h.send(Input::ReadinessChecked(facts));
        h.cm.phase.clone()
    };
    assert!(matches!(check(ready()), Phase::Connecting(_)));
    assert_eq!(
        check(ReadinessFacts {
            config_valid: false,
            ..ready()
        }),
        Phase::SetupRequired(vec![SetupNeed::ConfigFile])
    );
    assert_eq!(
        check(ReadinessFacts {
            account_complete: false,
            vpn_password_stored: false,
            gmail_credential_stored: false,
            ..ready()
        }),
        Phase::SetupRequired(vec![
            SetupNeed::AccountDetails,
            SetupNeed::VpnPassword,
            SetupNeed::GmailCredential
        ])
    );
    assert_eq!(
        check(ReadinessFacts {
            helper: HelperFacts::RequiresApproval,
            ..ready()
        }),
        Phase::SetupRequired(vec![SetupNeed::HelperApproval])
    );
    assert_eq!(
        check(ReadinessFacts {
            helper: HelperFacts::Answered {
                build: "99".into(),
                status: None
            },
            ..ready()
        }),
        Phase::SetupRequired(vec![SetupNeed::HelperVersion])
    );
}

// MARK: Presentation

#[test]
fn the_icon_and_buttons_follow_the_state() {
    let check = |h: &Harness, icon, connect, disconnect, reconnect_now| {
        let section = h.cm.connection_section();
        assert_eq!(section.icon, icon, "{:?}", h.cm.phase);
        assert_eq!(section.connect, connect, "{:?}", h.cm.phase);
        assert_eq!(section.disconnect, disconnect, "{:?}", h.cm.phase);
        assert_eq!(
            section.reconnect_now == ButtonState::Enabled,
            reconnect_now,
            "{:?}",
            h.cm.phase
        );
    };
    use ButtonState::{Enabled, Hidden};
    use ConnectionIcon::*;

    let mut h = Harness::new();
    h.launch_with(false, None);
    check(&h, Disconnected, Enabled, Hidden, false);
    h.send(Input::Connect);
    check(&h, InProgress, Hidden, Enabled, false);
    h.run_to_connected();
    check(&h, Connected, Hidden, Enabled, true);
    h.send(Input::Disconnect);
    check(&h, InProgress, Enabled, Hidden, false);

    let mut waiting = Harness::new();
    waiting.launch();
    let id = waiting.start_tunnel();
    waiting.run_milestones(&id, Milestone::OtpSubmitted);
    waiting.exit(ended(Milestone::OtpSubmitted));
    check(&waiting, InProgress, Hidden, Enabled, true);

    let mut offline_wait = Harness::with(settings(), Some(offline()));
    offline_wait.launch();
    check(&offline_wait, InProgress, Hidden, Enabled, false);

    let mut failed = Harness::new();
    failed.launch();
    failed.send(Input::ReadinessChecked(ready()));
    failed.send(Input::OtpSessionFailed(
        id_of(&failed),
        OtpFailure::Authentication,
    ));
    check(&failed, Error, Enabled, Enabled, false);
}

fn id_of(h: &Harness) -> fvm_contract::app_helper::AttemptId {
    h.attempt().unwrap().id.clone()
}
