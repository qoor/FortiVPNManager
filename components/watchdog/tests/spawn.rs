//! The conditions the client starts under.
#![cfg(unix)] // The fake clients are shell scripts; tests/windows.rs covers Windows.

mod common;

use std::env;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::*;
use fvm_contract::app_helper::{
    Milestone, OutputSource, ProcessExitStatus, StopInitiator, StopReason,
};
use nix::sys::signal::{SigSet, Signal};
use rustix::process::Pid;

/// Set by the outer test for the inner one.
const SIGTERM_IGNORED: &str = "FVM_WATCHDOG_SIGTERM_IGNORED";

fn output_texts(session: &Session) -> Vec<&str> {
    session
        .lines()
        .iter()
        .filter(|line| line.source != OutputSource::Helper)
        .map(|line| line.text.as_str())
        .collect()
}

#[test]
fn only_the_three_pipes_are_inherited() {
    // dup leaves close-on-exec off, so this descriptor would normally survive exec.
    let leaked = nix::unistd::dup(File::open("/dev/null").unwrap()).unwrap();
    let fd = leaked.as_raw_fd();
    let mut session = script(
        &format!(
            "for fd in 0 1 2 {fd}; do if [ -e /dev/fd/$fd ]; then echo \"$fd open\"; else echo \"$fd closed\"; fi; done"
        ),
        Setup::default(),
    );
    session.wait_for_exit();
    assert_eq!(
        output_texts(&session),
        ["0 open", "1 open", "2 open", &format!("{fd} closed")]
    );
    drop(leaked);
}

/// This process ignores SIGPIPE, as the Helper must, yet the client gets the default: `yes` dies
/// of SIGPIPE once `head` stops reading, instead of failing with EPIPE.
#[test]
fn sigpipe_is_at_its_default_in_the_client() {
    let mut session = script(
        "( /usr/bin/yes; echo \"yes_status=$?\" >> \"$1\" ) | /usr/bin/head -c1 >/dev/null",
        Setup::default(),
    );
    session.wait_for_exit();
    // 128 + SIGPIPE.
    assert_eq!(session.record.value("yes_status").as_deref(), Some("141"));
}

#[test]
fn the_environment_is_exactly_the_launch_one() {
    let mut launch = shell("/usr/bin/env", Vec::new());
    launch.env.push(("FVM_TEST".to_owned(), "a=b c".to_owned()));
    let mut session = Session::start(launch, RecordFile::new(), Setup::default());
    let facts = session.wait_for_exit();
    assert_eq!(facts.exit_status, ProcessExitStatus::Exited { code: 0 });
    let mut texts = output_texts(&session);
    texts.sort_unstable();
    assert_eq!(texts, ["FVM_TEST=a=b c", &format!("PATH={PATH}")]);
}

#[test]
fn the_client_leads_its_own_process_group() {
    let mut session = fixture("hang", &["spawned"], Setup::default());
    session.wait_for(Milestone::Spawned);
    let pid = Pid::from_raw(session.run.process_id().unwrap().try_into().unwrap()).unwrap();
    let group = rustix::process::getpgid(Some(pid)).unwrap();
    assert_eq!(group, pid);
    assert_ne!(group, rustix::process::getpgrp());
    session.run.stop(StopReason::UserDisconnect);
    session.wait_for_exit();
}

/// Ignoring SIGTERM takes sigaction, which Rust offers only as unsafe. A shell sets it up and runs
/// this test binary again for the inner test; an ignored signal stays ignored across exec.
#[test]
fn sigterm_ends_the_client_at_once_although_this_process_ignores_and_blocks_it() {
    let output = Command::new("/bin/sh")
        .args([
            "-c",
            "trap '' TERM; exec \"$0\" --exact \"$1\" --ignored --test-threads=1",
        ])
        .arg(env::current_exe().unwrap())
        .arg("sigterm_inner")
        .env(SIGTERM_IGNORED, "1")
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{report}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(report.contains("1 passed"), "{report}");
}

#[test]
#[ignore = "run by the test above, in a process that ignores SIGTERM"]
fn sigterm_inner() {
    if env::var_os(SIGTERM_IGNORED).is_none() {
        return;
    }
    // With SIGTERM's default action this would end the test process.
    rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::TERM)
        .unwrap();

    let (started, session) = mpsc::channel();
    thread::spawn(move || {
        let mut blocked = SigSet::empty();
        blocked.add(Signal::SIGTERM);
        blocked.thread_block().unwrap();
        started
            .send(fixture("hang", &["spawned"], Setup::default()))
            .unwrap();
    });
    let mut session = session.recv().unwrap();
    session.wait_for(Milestone::Spawned);
    // The script has to be past its start before the signal counts as stopping a running client.
    assert!(session.wait(Duration::from_secs(5), |session| {
        session.record.value("started").is_some()
    }));
    let stopped_at = Instant::now();
    session.run.stop(StopReason::UserDisconnect);
    let facts = session.wait_for_exit_within(Duration::from_secs(1));
    assert!(stopped_at.elapsed() < Duration::from_secs(1));
    assert_eq!(
        facts.exit_status,
        ProcessExitStatus::Signaled { signal: 15 }
    );
    assert_eq!(
        facts.stop_initiator,
        StopInitiator::Helper {
            reason: StopReason::UserDisconnect
        }
    );
}
