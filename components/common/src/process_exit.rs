use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::event::{EvFlags, EventFilter, FilterFlag, KEvent, Kqueue};
use nix::sys::time::TimeSpec;

pub struct ProcessExit(Kqueue);

impl ProcessExit {
    pub fn watch(pid: u32) -> Option<Self> {
        let kqueue = Kqueue::new().expect("creating a kqueue");
        let exit = KEvent::new(
            pid as usize,
            EventFilter::EVFILT_PROC,
            EvFlags::EV_ADD | EvFlags::EV_ONESHOT,
            FilterFlag::NOTE_EXIT,
            0,
            0,
        );
        let now = *TimeSpec::from_duration(Duration::ZERO).as_ref();
        match kqueue.kevent(&[exit], &mut [], Some(now)) {
            Ok(_) => Some(Self(kqueue)),
            Err(Errno::ESRCH) => None,
            Err(error) => panic!("watching process {pid} for its exit: {error}"),
        }
    }

    pub fn wait(&self, limit: Option<Duration>) -> bool {
        let deadline = limit.map(|limit| Instant::now() + limit);
        let mut exited = [KEvent::new(
            0,
            EventFilter::EVFILT_PROC,
            EvFlags::empty(),
            FilterFlag::empty(),
            0,
            0,
        )];
        loop {
            let timeout = deadline.map(|deadline| {
                *TimeSpec::from_duration(deadline.saturating_duration_since(Instant::now()))
                    .as_ref()
            });
            match self.0.kevent(&[], &mut exited, timeout) {
                Ok(count) => return count > 0,
                Err(Errno::EINTR) => {}
                Err(error) => panic!("waiting for a process to exit: {error}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn a_running_process_is_waited_for_until_it_exits_or_the_limit_passes() {
        let mut child = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let exit = ProcessExit::watch(child.id()).unwrap();
        let started = Instant::now();
        assert!(!exit.wait(Some(Duration::from_millis(300))));
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(300) && waited < Duration::from_secs(2),
            "{waited:?}"
        );
        child.kill().unwrap();
        assert!(exit.wait(Some(Duration::from_secs(5))));
        child.wait().unwrap();
        assert!(ProcessExit::watch(child.id()).is_none());
    }

    #[test]
    fn a_process_that_exited_but_was_not_collected_is_not_watched() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "exit 3"])
            .spawn()
            .unwrap();
        if let Some(exit) = ProcessExit::watch(child.id()) {
            assert!(exit.wait(None));
        }
        assert!(ProcessExit::watch(child.id()).is_none());
        assert_eq!(child.wait().unwrap().code(), Some(3));
    }
}
