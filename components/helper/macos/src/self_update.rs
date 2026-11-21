//! Detects that the helper's own executable was replaced (the app bundle was reinstalled).
//!
//! A running helper whose on-disk binary changed can no longer pass the app's code-signing check
//! (the static code no longer matches what is running), so it must exit once idle and let launchd
//! start the new binary on the next XPC request. The Helper entry decides when exiting is safe.

use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::Duration;
use std::{env, fs, thread};

const PERIOD: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Fingerprint {
    inode: u64,
    seconds: i64,
    nanoseconds: i64,
}

pub(crate) struct Watcher {
    path: PathBuf,
    original: Option<Fingerprint>,
}

impl Watcher {
    pub fn new() -> Self {
        let path = env::current_exe()
            .unwrap_or_else(|_| PathBuf::from(env::args_os().next().unwrap_or_default()));
        let original = fingerprint(&path);
        Self { path, original }
    }

    /// True when the file at our executable path is no longer the binary we were launched from.
    pub fn is_replaced(&self) -> bool {
        self.original
            .is_some_and(|original| fingerprint(&self.path) != Some(original))
    }

    /// Checks periodically and calls `on_replaced` once the binary was replaced; a replacement is
    /// permanent, so checking stops there.
    pub fn start(self: std::sync::Arc<Self>, on_replaced: impl FnOnce() + Send + 'static) {
        thread::Builder::new()
            .name("fvm-self-update".into())
            .spawn(move || {
                loop {
                    thread::sleep(PERIOD);
                    if self.is_replaced() {
                        on_replaced();
                        return;
                    }
                }
            })
            .expect("starting the self-update watcher");
    }
}

fn fingerprint(path: &PathBuf) -> Option<Fingerprint> {
    let metadata = fs::metadata(path).ok()?;
    Some(Fingerprint {
        inode: metadata.ino(),
        seconds: metadata.mtime(),
        nanoseconds: metadata.mtime_nsec(),
    })
}
