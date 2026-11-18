use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use fvm_helper_core::session::backend::Link;
use zeroize::Zeroizing;

#[test]
fn a_cancel_writes_only_while_the_session_is_armed() {
    let link = Link::new(|_| {}, |_| {});
    let writes = Arc::new(AtomicUsize::new(0));
    assert!(link.arm_cancel({
        let writes = Arc::clone(&writes);
        move || {
            writes.fetch_add(1, Ordering::SeqCst);
        }
    }));
    assert!(link.cancel());
    assert_eq!(writes.load(Ordering::SeqCst), 1);
    assert!(link.cancel());
    assert_eq!(writes.load(Ordering::SeqCst), 2);
    link.disarm_cancel();
    assert!(!link.cancel());
    assert_eq!(writes.load(Ordering::SeqCst), 2);

    let cancelled = Link::new(|_| {}, |_| {});
    assert!(!cancelled.cancel());
    assert!(!cancelled.arm_cancel(|| unreachable!("a cancelled session is never armed")));
}

#[test]
fn the_otp_wait_ends_with_the_submitted_code_or_a_cancel() {
    let link = Arc::new(Link::new(|_| {}, |_| {}));
    let wait = |link: &Arc<Link>| {
        let link = Arc::clone(link);
        thread::spawn(move || link.otp())
    };
    let waiting = wait(&link);
    link.submit_otp(Zeroizing::new("042137".into()));
    assert_eq!(
        waiting.join().unwrap().as_deref().map(String::as_str),
        Some("042137")
    );
    let waiting = wait(&link);
    link.cancel();
    assert!(waiting.join().unwrap().is_none());
}

#[test]
fn a_session_opens_once() {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let link = Link::new(|_| {}, {
        let opened = Arc::clone(&opened);
        move |cookie: Zeroizing<String>| opened.lock().unwrap().push(cookie.to_string())
    });
    link.opened(Zeroizing::new("SVPNCOOKIE=1".into()));
    assert_eq!(*opened.lock().unwrap(), ["SVPNCOOKIE=1"]);
    let again = panic::catch_unwind(AssertUnwindSafe(|| {
        link.opened(Zeroizing::new("SVPNCOOKIE=2".into()));
    }));
    assert!(again.is_err());
    assert_eq!(opened.lock().unwrap().len(), 1);
}
