//! Watches the folders whose changes may change config.yaml and tells the runtime when to check
//! the file again: once a burst of notifications settles, and when the watched folders change.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::orchestration::Input;

const QUIET: Duration = Duration::from_millis(100);
const LONGEST_WAIT: Duration = Duration::from_secs(2);

pub(super) enum Message {
    /// The folders to watch from now on.
    Folders(Vec<PathBuf>),
    /// Something changed in a watched folder, or a watch reported an error.
    Notified,
}

pub(super) fn start(inputs: Sender<Input>) -> Sender<Message> {
    let (messages, received) = mpsc::channel();
    let notified = messages.clone();
    thread::Builder::new()
        .name("fvm-runtime-config-watch".into())
        .spawn(move || run(&received, &notified, &inputs))
        .expect("starting the config watch thread");
    messages
}

#[derive(Clone, Copy)]
struct Burst {
    first: Instant,
    latest: Instant,
}

impl Burst {
    fn check_at(self) -> Instant {
        (self.latest + QUIET).min(self.first + LONGEST_WAIT)
    }
}

fn run(received: &Receiver<Message>, notified: &Sender<Message>, inputs: &Sender<Input>) {
    let mut watch = Watch::default();
    let mut burst: Option<Burst> = None;
    loop {
        let message = match burst {
            None => Ok(received
                .recv()
                .expect("the watch thread keeps a sender of its own")),
            Some(burst) => {
                received.recv_timeout(burst.check_at().saturating_duration_since(Instant::now()))
            }
        };
        match message {
            Ok(Message::Notified) => {
                let now = Instant::now();
                burst = Some(Burst {
                    first: burst.map_or(now, |burst| burst.first),
                    latest: now,
                });
            }
            Ok(Message::Folders(folders)) => {
                if watch.change(folders, notified, inputs) {
                    burst = None;
                    let _ = inputs.send(Input::ConfigFolderChanged);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                burst = None;
                let _ = inputs.send(Input::ConfigFolderChanged);
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("the watch thread keeps a sender of its own")
            }
        }
    }
}

#[derive(Default)]
struct Watch {
    watcher: Option<RecommendedWatcher>,
    watched: Vec<PathBuf>,
    failing: bool,
}

impl Watch {
    /// Watches `folders` in place of the folders watched so far and reports when watching starts
    /// or stops failing. Returns whether the folders being watched changed.
    fn change(
        &mut self,
        folders: Vec<PathBuf>,
        notified: &Sender<Message>,
        inputs: &Sender<Input>,
    ) -> bool {
        if folders == self.watched && !self.failing {
            return false;
        }
        let mut watching = Vec::new();
        let mut failures = Vec::new();
        self.watcher = if folders.is_empty() {
            None
        } else {
            let notified = notified.clone();
            match notify::recommended_watcher(move |_: notify::Result<notify::Event>| {
                let _ = notified.send(Message::Notified);
            }) {
                Ok(mut watcher) => {
                    for folder in folders {
                        match watcher.watch(&folder, RecursiveMode::NonRecursive) {
                            Ok(()) => watching.push(folder),
                            Err(error) => failures.push(format!("{}: {error}", folder.display())),
                        }
                    }
                    Some(watcher)
                }
                Err(error) => {
                    failures.push(error.to_string());
                    None
                }
            }
        };
        let report = match (self.failing, failures.is_empty()) {
            (false, false) => Some(Input::ConfigWatchFailed {
                error: failures.join("; "),
            }),
            (true, true) => Some(Input::ConfigWatchRestored),
            _ => None,
        };
        if let Some(report) = report {
            let _ = inputs.send(report);
        }
        self.failing = !failures.is_empty();
        let changed = watching != self.watched;
        self.watched = watching;
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_file::ConfigFile;
    use crate::config_file::tests::scratch;
    use crate::files;
    use std::fs;
    use std::path::Path;

    fn next(inputs: &Receiver<Input>) -> &'static str {
        match inputs.recv().expect("the watch thread keeps running") {
            Input::ConfigFolderChanged => "check",
            Input::ConfigWatchFailed { .. } => "failed",
            Input::ConfigWatchRestored => "restored",
            _ => panic!("an input the watch thread does not send"),
        }
    }

    fn watching(folders: &[&Path]) -> (Sender<Message>, Receiver<Input>) {
        let (inputs, received) = mpsc::channel();
        let watch = start(inputs);
        watch
            .send(Message::Folders(
                folders.iter().map(|folder| folder.to_path_buf()).collect(),
            ))
            .unwrap();
        assert_eq!(next(&received), "check", "the first watch is a change");
        (watch, received)
    }

    #[test]
    fn a_check_waits_for_a_quiet_moment_but_not_longer_than_the_longest_wait() {
        let start = Instant::now();
        let at = |milliseconds| start + Duration::from_millis(milliseconds);
        let burst = |first, latest| Burst {
            first: at(first),
            latest: at(latest),
        };
        assert_eq!(burst(0, 0).check_at(), at(100));
        assert_eq!(burst(0, 1_000).check_at(), at(1_100));
        assert_eq!(burst(0, 1_950).check_at(), at(2_000));
    }

    #[test]
    fn a_write_in_the_config_folder_asks_for_a_check() {
        let folder = scratch();
        files::make_directory(&folder).unwrap();
        let (_watch, inputs) = watching(&[&folder]);
        fs::write(folder.join("config.yaml"), "vpn:\n  port: 8443\n").unwrap();
        assert_eq!(next(&inputs), "check");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_config_folder_made_later_asks_for_a_check() {
        let config_home = scratch();
        files::make_directory(&config_home).unwrap();
        let (_watch, inputs) = watching(&[&config_home]);
        files::make_directory(&config_home.join("FortiVPNManager")).unwrap();
        assert_eq!(next(&inputs), "check");
        fs::remove_dir_all(config_home).unwrap();
    }

    #[test]
    fn a_deleted_config_folder_is_followed_until_it_comes_back() {
        let config_home = scratch();
        let folder = config_home.join("FortiVPNManager");
        let path = folder.join("config.yaml");
        files::make_directory(&folder).unwrap();
        fs::write(&path, "vpn:\n  port: 8443\n").unwrap();
        let config = ConfigFile::new(path.clone());
        let (watch, inputs) = watching(&[&folder]);
        fs::remove_dir_all(&folder).unwrap();
        assert_eq!(next(&inputs), "check");
        assert_eq!(config.watch_folders(), std::slice::from_ref(&config_home));
        watch
            .send(Message::Folders(config.watch_folders()))
            .unwrap();
        assert_eq!(next(&inputs), "check");
        files::make_directory(&folder).unwrap();
        assert_eq!(next(&inputs), "check");
        assert_eq!(config.watch_folders(), [folder]);
        watch
            .send(Message::Folders(config.watch_folders()))
            .unwrap();
        assert_eq!(next(&inputs), "check");
        fs::write(&path, "vpn:\n  port: 10443\n").unwrap();
        assert_eq!(next(&inputs), "check");
        fs::remove_dir_all(config_home).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_change_to_a_symlinked_file_asks_for_a_check() {
        let home = scratch();
        let folder = home.join(".config").join("FortiVPNManager");
        let dotfiles = home.join("dotfiles");
        files::make_directory(&folder).unwrap();
        files::make_directory(&dotfiles).unwrap();
        fs::write(dotfiles.join("config.yaml"), "vpn:\n  port: 8443\n").unwrap();
        std::os::unix::fs::symlink(dotfiles.join("config.yaml"), folder.join("config.yaml"))
            .unwrap();
        let folders = ConfigFile::new(folder.join("config.yaml")).watch_folders();
        let folders: Vec<&Path> = folders.iter().map(PathBuf::as_path).collect();
        let (_watch, inputs) = watching(&folders);
        fs::write(dotfiles.join("config.yaml"), "vpn:\n  port: 10443\n").unwrap();
        assert_eq!(next(&inputs), "check");
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn the_same_folders_ask_for_no_check() {
        let folder = scratch();
        files::make_directory(&folder).unwrap();
        let (watch, inputs) = watching(&[&folder]);
        watch.send(Message::Folders(vec![folder.clone()])).unwrap();
        watch
            .send(Message::Folders(vec![
                folder.clone(),
                folder.join("missing"),
            ]))
            .unwrap();
        assert_eq!(
            next(&inputs),
            "failed",
            "the same folders came before the failure"
        );
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_failure_is_reported_once_until_watching_recovers() {
        let folder = scratch();
        files::make_directory(&folder).unwrap();
        let (inputs, received) = mpsc::channel();
        let watch = start(inputs);
        let missing = vec![folder.join("missing")];
        watch.send(Message::Folders(missing.clone())).unwrap();
        assert_eq!(next(&received), "failed");
        watch.send(Message::Folders(missing)).unwrap();
        watch.send(Message::Folders(vec![folder.clone()])).unwrap();
        assert_eq!(
            next(&received),
            "restored",
            "the second failure came before the recovery"
        );
        assert_eq!(next(&received), "check");
        fs::remove_dir_all(folder).unwrap();
    }
}
