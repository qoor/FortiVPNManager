use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::entry;

const LOCK: &str = "the watched loops are never held across a panic";

pub(crate) type Fault = Arc<dyn Fn(&str) + Send + Sync>;

static FAULT: OnceLock<Fault> = OnceLock::new();

pub(crate) fn report_to(fault: Fault) {
    assert!(FAULT.set(fault).is_ok(), "the fault channel is set once");
}

pub(crate) struct Watchdog {
    loops: Arc<Mutex<Loops>>,
    period: Duration,
}

#[derive(Default)]
struct Loops {
    next_id: u64,
    watched: HashMap<u64, Watched>,
}

struct Watched {
    name: String,
    beaten: Instant,
}

pub(crate) struct Beat {
    loops: Arc<Mutex<Loops>>,
    id: u64,
    period: Duration,
}

impl Watchdog {
    pub(crate) fn start(period: Duration, stalled_after: Duration, resume_gap: Duration) -> Self {
        let loops: Arc<Mutex<Loops>> = Arc::default();
        let monitored = Arc::downgrade(&loops);
        thread::Builder::new()
            .name("fvm-watchdog".to_owned())
            .spawn(move || monitor(&monitored, period, stalled_after, resume_gap))
            .expect("starting the watchdog thread");
        Self { loops, period }
    }

    pub(crate) fn watch(&self, name: String) -> Beat {
        let mut loops = self.loops.lock().expect(LOCK);
        let id = loops.next_id;
        loops.next_id += 1;
        loops.watched.insert(
            id,
            Watched {
                name,
                beaten: Instant::now(),
            },
        );
        Beat {
            loops: Arc::clone(&self.loops),
            id,
            period: self.period,
        }
    }
}

impl Beat {
    pub(crate) fn beat(&self) {
        self.loops
            .lock()
            .expect(LOCK)
            .watched
            .get_mut(&self.id)
            .expect("a loop stays watched while it holds its beat")
            .beaten = Instant::now();
    }

    pub(crate) fn period(&self) -> Duration {
        self.period
    }
}

impl Drop for Beat {
    fn drop(&mut self) {
        self.loops.lock().expect(LOCK).watched.remove(&self.id);
    }
}

fn monitor(
    monitored: &Weak<Mutex<Loops>>,
    period: Duration,
    stalled_after: Duration,
    resume_gap: Duration,
) {
    let mut judged = Instant::now();
    let stalled = loop {
        thread::sleep(period);
        let Some(shared) = monitored.upgrade() else {
            return;
        };
        let mut loops = shared.lock().expect(LOCK);
        let now = Instant::now();
        let gap = now.duration_since(judged);
        judged = now;
        if gap > resume_gap {
            for watched in loops.watched.values_mut() {
                watched.beaten = now;
            }
            continue;
        }
        let longest = loops
            .watched
            .values()
            .map(|watched| (watched, now.duration_since(watched.beaten)))
            .max_by_key(|(_, silent)| *silent);
        if let Some((watched, silent)) = longest
            && silent >= stalled_after
        {
            break format!(
                "watchdog: {} made no progress for {:.1}s; aborting so the OS restarts the Helper",
                watched.name,
                silent.as_secs_f64()
            );
        }
    };
    let fault = FAULT
        .get()
        .expect("report_panics set the fault channel before the Helper started");
    fault(&stalled);
    entry::end_for_restart();
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::io::{self, Write};
    use std::process::{Command, ExitStatus};

    use super::*;
    use crate::entry::report_panics;

    #[cfg(unix)]
    fn aborted(status: ExitStatus) -> bool {
        use std::os::unix::process::ExitStatusExt;
        const SIGABRT: i32 = 6;
        status.signal() == Some(SIGABRT)
    }

    #[cfg(windows)]
    fn aborted(status: ExitStatus) -> bool {
        status.code() == Some(entry::RESTART_EXIT_CODE.cast_signed())
    }

    #[test]
    fn a_monitor_that_could_not_judge_for_longer_than_the_resume_gap_restarts_every_count() {
        let child = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "watchdog::tests::block_the_monitor_past_the_limit",
                "--ignored",
            ])
            .output()
            .unwrap();
        let stdout = String::from_utf8(child.stdout).unwrap();
        assert!(aborted(child.status), "{} {stdout}", child.status);
        let silent: f64 = stdout
            .lines()
            .find_map(|line| {
                line.strip_prefix("watchdog: silent loop made no progress for ")?
                    .strip_suffix("s; aborting so the OS restarts the Helper")?
                    .parse()
                    .ok()
            })
            .unwrap_or_else(|| panic!("{stdout}"));
        assert!((1.0..2.0).contains(&silent), "{stdout}");
    }

    #[test]
    #[ignore = "the child of a_monitor_that_could_not_judge_for_longer_than_the_resume_gap_restarts_every_count"]
    fn block_the_monitor_past_the_limit() {
        report_panics(|line| {
            io::stdout()
                .write_all(format!("{line}\n").as_bytes())
                .unwrap();
        });
        let watchdog = Watchdog::start(
            Duration::from_millis(100),
            Duration::from_secs(1),
            Duration::from_millis(500),
        );
        let _beat = watchdog.watch("silent loop".to_owned());
        {
            let _blocked = watchdog.loops.lock().unwrap();
            thread::sleep(Duration::from_millis(2500));
        }
        thread::sleep(Duration::from_secs(10));
    }
}
