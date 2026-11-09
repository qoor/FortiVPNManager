//! The core's timers on the wall clock (SPEC F-RT-01 item 3). A timer is due when the wall clock
//! reaches its time, so it also fires after a sleep or a clock change; the thread looks again at
//! least every few seconds and whenever it is nudged.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use fvm_core::connection::{TimerId, TimerKind};

/// How late a timer may fire after the wall clock jumped forward without a nudge.
const LONGEST_NAP: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Pending {
    timers: BTreeMap<TimerKind, (TimerId, SystemTime)>,
}

pub(crate) struct Timers {
    shared: Arc<(Mutex<Pending>, Condvar)>,
}

impl Timers {
    /// Starts the timer thread; `fire` runs on it for each due timer.
    pub(crate) fn start(fire: impl Fn(TimerId) + Send + 'static) -> Self {
        let shared = Arc::new((Mutex::new(Pending::default()), Condvar::new()));
        let thread_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("fvm-runtime-timers".into())
            .spawn(move || run(&thread_shared, &fire))
            .expect("starting the timer thread");
        Self { shared }
    }

    /// Sets `timer` to `at`, replacing a timer of the same kind.
    pub(crate) fn set(&self, timer: TimerId, at: SystemTime) {
        let (pending, wake) = &*self.shared;
        pending
            .lock()
            .expect("the timer lock")
            .timers
            .insert(timer.kind, (timer, at));
        wake.notify_one();
    }

    pub(crate) fn cancel(&self, kind: TimerKind) {
        let (pending, wake) = &*self.shared;
        pending.lock().expect("the timer lock").timers.remove(&kind);
        wake.notify_one();
    }

    /// Looks at the wall clock again now, after a wake or a clock change.
    pub(crate) fn nudge(&self) {
        self.shared.1.notify_one();
    }
}

fn run(shared: &(Mutex<Pending>, Condvar), fire: &impl Fn(TimerId)) {
    let (pending, wake) = shared;
    let mut guard = pending.lock().expect("the timer lock");
    loop {
        let now = SystemTime::now();
        let due: Vec<TimerId> = guard
            .timers
            .values()
            .filter(|(_, at)| *at <= now)
            .map(|(timer, _)| *timer)
            .collect();
        for timer in &due {
            guard.timers.remove(&timer.kind);
        }
        if !due.is_empty() {
            drop(guard);
            due.into_iter().for_each(fire);
            guard = pending.lock().expect("the timer lock");
            continue;
        }
        let nap = guard
            .timers
            .values()
            .map(|(_, at)| at.duration_since(now).unwrap_or_default())
            .min()
            .map_or(LONGEST_NAP, |until| until.min(LONGEST_NAP));
        guard = wake.wait_timeout(guard, nap).expect("the timer lock").0;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn timer(kind: TimerKind, generation: u64) -> TimerId {
        TimerId { kind, generation }
    }

    #[test]
    fn a_due_timer_fires_once_and_a_cancelled_one_never() {
        let (fired, received) = mpsc::channel();
        let timers = Timers::start(move |timer| fired.send(timer).unwrap());
        let now = SystemTime::now();
        timers.set(timer(TimerKind::Retry, 1), now + Duration::from_millis(50));
        timers.set(
            timer(TimerKind::OtpDeadline, 2),
            now + Duration::from_millis(60),
        );
        timers.cancel(TimerKind::OtpDeadline);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            timer(TimerKind::Retry, 1)
        );
        assert!(received.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn setting_a_kind_again_replaces_it() {
        let (fired, received) = mpsc::channel();
        let timers = Timers::start(move |timer| fired.send(timer).unwrap());
        let now = SystemTime::now();
        timers.set(timer(TimerKind::Retry, 1), now + Duration::from_secs(60));
        timers.set(timer(TimerKind::Retry, 2), now);
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            timer(TimerKind::Retry, 2)
        );
        assert!(received.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
