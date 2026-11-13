//! Helper entry: the Helper's front, the same on every OS. It admits the app connections the OS side
//! passes on, decides who may do what with the one run, answers the app's requests from what the
//! watchdog tells it, and decides when the Helper exits. Everything about the VPN client, from
//! checking it to stopping what an earlier Helper left, is the watchdog's, which the OS's start
//! hands over with the backend adapter and the run directory it chose. The OS side, Swift's OS
//! integration Helper on macOS and the service's own part on Windows, takes the app connections and
//! does what the entry asks of it; the two exchange the messages in `fvm_contract::helper_internal`.
//! This crate uses no OS-specific API.

mod entry;

use std::sync::mpsc::{self, Sender};
use std::thread;

use fvm_contract::backend::Backend;
use fvm_contract::helper_internal::{FromEntry, ToEntry};
use fvm_watchdog::Watchdog;

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

/// Starts the entry on its own thread with `watchdog` for the VPN client. `send` takes the entry's
/// messages for the OS side, in order, on the entry's thread; the first is "ready", once the
/// watchdog cleaned up after an earlier Helper.
pub fn start<B>(watchdog: Watchdog<B>, send: impl Fn(&FromEntry) + Send + 'static) -> Inbox
where
    B: Backend + Send + Sync + 'static,
{
    let (sender, events) = mpsc::channel();
    let inbox = Inbox(sender.clone());
    thread::Builder::new()
        .name("fvm-helper-entry".into())
        .spawn(move || {
            entry::serve(watchdog, entry::OsSide(Box::new(send)), sender, events);
        })
        .expect("starting the Helper entry thread");
    inbox
}
