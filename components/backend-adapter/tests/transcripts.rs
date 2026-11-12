//! Replays the synthetic transcripts in `fixtures/`, reconstructed from openfortivpn 1.24.1 and
//! Apple pppd source strings, under many ways the output can be split into reads.
#![cfg(unix)] // openfortivpn runs only on Unix.

mod common;

use std::collections::BTreeSet;
use std::net::Ipv4Addr;
use std::ops::Range;

use common::Run;
use fvm_contract::app_helper::{Milestone, OutputSource};
use fvm_contract::backend::{Finding, OutputInterpreter, Prompt};

const DIGEST: &str = "b0cf91f4d820f99d16e54f757dea3f71c2e717b62c6860cfd8357ed43429ba11";

struct Scenario {
    name: &'static str,
    stdout: &'static [u8],
    stderr: &'static [u8],
    prompts: &'static [Prompt],
    findings: fn() -> Vec<Finding>,
    routes_left_unrestored: bool,
}

fn tunnel_up() -> Vec<Finding> {
    vec![
        Finding::Milestone(Milestone::ConnectedToGateway),
        Finding::Milestone(Milestone::Authenticated),
        Finding::TunnelIp(Ipv4Addr::new(10, 212, 134, 200)),
        Finding::Milestone(Milestone::TunnelUp),
    ]
}

const SCENARIOS: [Scenario; 6] = [
    Scenario {
        name: "success-tokeninfo",
        stdout: include_bytes!("fixtures/synthetic-success-tokeninfo.stdout.txt"),
        stderr: include_bytes!("fixtures/synthetic-success-tokeninfo.stderr.txt"),
        prompts: &[Prompt::Password, Prompt::Otp],
        findings: tunnel_up,
        // The transcript ends while the tunnel is up.
        routes_left_unrestored: true,
    },
    Scenario {
        name: "server-terminated",
        stdout: include_bytes!("fixtures/synthetic-server-terminated.stdout.txt"),
        stderr: include_bytes!("fixtures/synthetic-server-terminated.stderr.txt"),
        prompts: &[Prompt::Password, Prompt::Otp],
        findings: tunnel_up,
        routes_left_unrestored: false,
    },
    Scenario {
        name: "sigterm-teardown",
        stdout: include_bytes!("fixtures/synthetic-sigterm-teardown.stdout.txt"),
        stderr: include_bytes!("fixtures/synthetic-sigterm-teardown.stderr.txt"),
        prompts: &[Prompt::Password, Prompt::Otp],
        findings: tunnel_up,
        routes_left_unrestored: false,
    },
    Scenario {
        name: "cert-rejected",
        stdout: include_bytes!("fixtures/synthetic-cert-rejected.stdout.txt"),
        stderr: include_bytes!("fixtures/synthetic-cert-rejected.stderr.txt"),
        prompts: &[Prompt::Password],
        findings: || vec![Finding::CertificateRejected(DIGEST.into())],
        routes_left_unrestored: false,
    },
    Scenario {
        name: "auth-failed",
        stdout: include_bytes!("fixtures/synthetic-auth-failed.stdout.txt"),
        stderr: include_bytes!("fixtures/synthetic-auth-failed.stderr.txt"),
        prompts: &[Prompt::Password],
        findings: || vec![Finding::Milestone(Milestone::ConnectedToGateway)],
        routes_left_unrestored: false,
    },
    // No fixture was reconstructed for this run; it uses the same openfortivpn strings.
    Scenario {
        name: "tunnel-mode-rejected",
        stdout: b"VPN account password: \n\
INFO:   Connected to gateway.\n\
Two-factor authentication token: \n\
INFO:   Authenticated.\n\
INFO:   Remote gateway has allocated a VPN.\n\
ERROR:  Could not authenticate to the gateway. Please make sure tunnel mode is allowed by the gateway, check the realm, etc.\n\
INFO:   Closed connection to gateway.\n\
INFO:   Logged out.\n",
        stderr: b"",
        prompts: &[Prompt::Password, Prompt::Otp],
        findings: || {
            vec![
                Finding::Milestone(Milestone::ConnectedToGateway),
                Finding::Milestone(Milestone::Authenticated),
                Finding::TunnelModeRejected,
            ]
        },
        routes_left_unrestored: false,
    },
];

/// Offsets right after each prompt, before its line break: openfortivpn blocks on stdin there, so
/// a read always ends at them.
fn prompt_pauses(stdout: &[u8]) -> BTreeSet<usize> {
    let mut pauses = BTreeSet::new();
    let mut offset = 0;
    for line in stdout.split(|&byte| byte == b'\n') {
        if line == b"VPN account password: " || line == b"Two-factor authentication token: " {
            pauses.insert(offset + line.len());
        }
        offset += line.len() + 1;
    }
    pauses
}

fn reads(length: usize, cuts: &BTreeSet<usize>) -> Vec<Range<usize>> {
    let mut points: Vec<usize> = cuts
        .iter()
        .copied()
        .filter(|&cut| cut > 0 && cut < length)
        .collect();
    points.insert(0, 0);
    points.push(length);
    points.windows(2).map(|pair| pair[0]..pair[1]).collect()
}

struct Replay {
    /// The prompts reported between one stdin pause and the next, the last entry after the last pause.
    prompts_per_pause: Vec<Vec<Prompt>>,
    run: Run,
}

fn replay(
    scenario: &Scenario,
    stdout_cuts: &BTreeSet<usize>,
    stderr_cuts: &BTreeSet<usize>,
) -> Replay {
    let pauses = prompt_pauses(scenario.stdout);
    let mut run = Run::new();
    let mut prompts_per_pause = vec![Vec::new()];
    let cuts = stdout_cuts.union(&pauses).copied().collect();
    for range in reads(scenario.stdout.len(), &cuts) {
        let end = range.end;
        let prompt = run.read(OutputSource::Stdout, &scenario.stdout[range]);
        prompts_per_pause.last_mut().unwrap().extend(prompt);
        if pauses.contains(&end) {
            prompts_per_pause.push(Vec::new());
        }
    }
    for range in reads(scenario.stderr.len(), stderr_cuts) {
        assert_eq!(
            run.read(OutputSource::Stderr, &scenario.stderr[range]),
            None
        );
    }
    Replay {
        prompts_per_pause,
        run,
    }
}

fn check(scenario: &Scenario, replay: &Replay) -> Result<(), String> {
    let mut expected_prompts: Vec<Vec<Prompt>> = scenario
        .prompts
        .iter()
        .map(|prompt| vec![*prompt])
        .collect();
    expected_prompts.push(Vec::new());
    if replay.prompts_per_pause != expected_prompts {
        return Err(format!("prompts {:?}", replay.prompts_per_pause));
    }
    if replay.run.findings != (scenario.findings)() {
        return Err(format!("findings {:?}", replay.run.findings));
    }
    if replay.run.interpreter.routes_left_unrestored() != scenario.routes_left_unrestored {
        return Err("routes left unrestored".into());
    }
    Ok(())
}

/// Deterministic read boundaries.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn chunkings(length: usize) -> Vec<(String, BTreeSet<usize>)> {
    let mut chunkings = vec![("whole".to_owned(), BTreeSet::new())];
    for size in [1, 2, 3, 5, 8, 13, 64, 4_096] {
        chunkings.push((
            format!("every {size} bytes"),
            (size..length).step_by(size).collect(),
        ));
    }
    for seed in 1..=12 {
        let mut random = SplitMix64(seed);
        let mut cuts = BTreeSet::new();
        let mut position = 0;
        loop {
            position += 1 + (random.next() % 24) as usize;
            if position >= length {
                break;
            }
            cuts.insert(position);
        }
        chunkings.push((format!("random #{seed}"), cuts));
    }
    chunkings
}

#[test]
fn replays_each_transcript_under_many_chunkings() {
    for scenario in &SCENARIOS {
        let stdout_chunkings = chunkings(scenario.stdout.len());
        let stderr_chunkings = chunkings(scenario.stderr.len());
        for ((name, stdout_cuts), (_, stderr_cuts)) in
            stdout_chunkings.iter().zip(&stderr_chunkings)
        {
            let replay = replay(scenario, stdout_cuts, stderr_cuts);
            if let Err(problem) = check(scenario, &replay) {
                panic!("{} {name}: {problem}", scenario.name);
            }
        }
    }
}

#[test]
fn replays_each_transcript_split_at_every_offset() {
    for scenario in &SCENARIOS {
        let failing: Vec<usize> = (0..=scenario.stdout.len())
            .filter(|&offset| {
                let replay = replay(scenario, &BTreeSet::from([offset]), &BTreeSet::new());
                check(scenario, &replay).is_err()
            })
            .collect();
        assert!(failing.is_empty(), "{}: {failing:?}", scenario.name);
    }
}

#[test]
fn reads_nothing_from_pppd_on_stderr() {
    for scenario in &SCENARIOS {
        let mut run = Run::new();
        assert_eq!(run.read(OutputSource::Stderr, scenario.stderr), None);
        assert_eq!(run.findings, [], "{}", scenario.name);
        assert!(
            !run.interpreter.routes_left_unrestored(),
            "{}",
            scenario.name
        );
    }
}
