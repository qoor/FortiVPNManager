//! Helper entry: the Helper's work that is the same on every OS. It cleans up what an earlier Helper
//! left behind, handles the app requests the OS side passes on and runs the VPN client through the
//! watchdog with the backend adapter it is given. The OS side, Swift's OS integration Helper on
//! macOS, takes the app connections and does what the entry asks of it; the two exchange the
//! messages in `fvm_contract::helper_internal`. What differs per OS, telling processes apart,
//! stopping leftover ones and keeping the run directory private, comes from the OS's start as a
//! `Platform`, so this crate uses no OS-specific API.

mod entry;
mod leftovers;
mod platform;
mod run_directory;

pub use platform::{Platform, ProcessIdentity};
pub use run_directory::Failure;

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

use fvm_contract::backend::Backend;
use fvm_contract::helper_internal::{FromEntry, ToEntry};

/// Takes the OS side's messages to the entry.
pub struct Inbox(Sender<entry::Event>);

impl Inbox {
    /// Passes on a message, in the order the OS side received them.
    pub fn deliver(&self, message: ToEntry) {
        self.0
            .send(entry::Event::FromOs(message))
            .expect("the entry runs as long as the process");
    }
}

/// Starts the entry on its own thread for `backend`'s client on `platform`, keeping its files in
/// `run_directory`. `send` takes the entry's messages for the OS side, in order, on the entry's
/// thread; the first is "ready", once the entry cleaned up after an earlier Helper.
pub fn start<B, P>(
    backend: B,
    platform: P,
    run_directory: PathBuf,
    send: impl Fn(&FromEntry) + Send + 'static,
) -> Inbox
where
    B: Backend + Send + 'static,
    B::Interpreter: 'static,
    P: Platform + Send + 'static,
{
    let (sender, events) = mpsc::channel();
    let inbox = Inbox(sender.clone());
    thread::Builder::new()
        .name("fvm-helper-entry".into())
        .spawn(move || {
            entry::serve(
                backend,
                platform,
                run_directory,
                entry::OsSide(Box::new(send)),
                sender,
                events,
            );
        })
        .expect("starting the Helper entry thread");
    inbox
}
