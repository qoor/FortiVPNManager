//! The entry's exception routes on Windows, where a route belongs to an interface, with the test as
//! the OS side.
#![cfg(windows)]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_contract::app_helper::{
    BypassRoute, BypassRoutesReply, BypassRoutesRequest, BypassRoutesSnapshot,
};
use fvm_contract::helper_internal::{
    AddedRoute, AppRequest, BypassRoutesChange, BypassRoutesOutcome, FromEntry, NetworkCheck,
    ToEntry,
};
use fvm_helper_core::entry::{self, Inbox};
use fvm_helper_core::process::{Timing, Watchdog};

const WIFI: &str = "{6B29FC40-CA47-1067-B31D-00DD010662DA}";
const ETHERNET: &str = "{3F2504E0-4F89-11D3-9A0C-0305E82C3301}";

fn network(service_id: &str, service_name: &str) -> BypassRoutesSnapshot {
    BypassRoutesSnapshot {
        service_id: service_id.to_owned(),
        service_name: service_name.to_owned(),
        router: "10.20.161.1".to_owned(),
        address: "10.20.161.20".to_owned(),
        subnet_mask: "255.255.255.0".to_owned(),
    }
}

fn host() -> BypassRoute {
    BypassRoute {
        destination: "10.30.161.16".to_owned(),
        subnet_mask: "255.255.255.255".to_owned(),
        gateway: "10.20.161.1".to_owned(),
    }
}

fn added_through(service_id: &str) -> AddedRoute {
    AddedRoute {
        route: host(),
        service_id: service_id.to_owned(),
    }
}

/// The OS side of an entry with app 1 admitted.
struct OsSide {
    folder: PathBuf,
    inbox: Inbox,
    messages: Receiver<FromEntry>,
}

impl OsSide {
    fn start() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let folder = env::temp_dir().join(format!(
            "fvm-entry-windows-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&folder).unwrap();
        let watchdog = Watchdog::new(Arc::new(OpenConnect), folder.join("run"), Timing::default());
        let (sender, messages) = mpsc::channel();
        let inbox = entry::start(watchdog, move |message| {
            let _ = sender.send(message.clone());
        });
        let os = Self {
            folder,
            inbox,
            messages,
        };
        assert_eq!(os.next(), FromEntry::Ready {});
        os.inbox.deliver(ToEntry::AppConnected { app: 1, uid: 1 });
        assert_eq!(
            os.next(),
            FromEntry::Admitted {
                app: 1,
                accept: true
            }
        );
        os
    }

    /// The entry's next message but log lines.
    fn next(&self) -> FromEntry {
        loop {
            match self.messages.recv_timeout(Duration::from_secs(10)) {
                Ok(FromEntry::Log { .. }) => {}
                Ok(message) => return message,
                Err(error) => panic!("no message from the entry: {error}"),
            }
        }
    }

    fn write(&self, id: u64, network: &BypassRoutesSnapshot) {
        let request = BypassRoutesRequest::Write {
            expected: network.clone(),
            routes: vec![host()],
        };
        let json = serde_json::to_string(&request).unwrap();
        self.inbox.deliver(ToEntry::Request {
            id,
            app: 1,
            request: AppRequest::WriteBypassRoutes { json },
        });
    }

    fn change(&self) -> BypassRoutesChange {
        match self.next() {
            FromEntry::ChangeBypassRoutes { change } => change,
            other => panic!("not a change: {other:?}"),
        }
    }

    fn changed(&self, network: &BypassRoutesSnapshot, added: Vec<BypassRoute>) {
        let found = added.clone();
        let outcome = BypassRoutesOutcome::Changed {
            network: Some(network.clone()),
            added,
            found,
            left: Vec::new(),
        };
        self.inbox.deliver(ToEntry::BypassRoutesChanged { outcome });
    }

    fn reply(&self, id: u64) -> BypassRoutesReply {
        match self.next() {
            FromEntry::Reply {
                id: answered,
                json: Some(json),
            } if answered == id => serde_json::from_str(&json).unwrap(),
            other => panic!("not the reply to {id}: {other:?}"),
        }
    }

    fn recorded(&self) -> Vec<AddedRoute> {
        let path = self.folder.join("run").join("exception-routes.json");
        let record: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        serde_json::from_value(record["routes"].clone()).unwrap()
    }

    /// Adds the route through `network`, none of it in the table there.
    fn write_added(&self, id: u64, network: &BypassRoutesSnapshot) -> BypassRoutesChange {
        self.write(id, network);
        assert_eq!(self.change().look_up, vec![host()]);
        self.changed(network, Vec::new());
        let change = self.change();
        self.changed(network, vec![host()]);
        assert_eq!(
            self.reply(id),
            BypassRoutesReply::Success {
                snapshot: network.clone(),
                routes: vec![host()],
            }
        );
        change
    }
}

impl Drop for OsSide {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

/// Another interface with the same router takes over, as when the PC moves from Wi-Fi to its cable
/// on the same network: the route through the Wi-Fi interface is another route, so the one the
/// Helper added there goes and the route is added through the cable's.
#[test]
fn a_new_interface_with_the_same_router_gets_the_route_through_it() {
    let os = OsSide::start();
    let wifi = network(WIFI, "Wi-Fi");
    let ethernet = network(ETHERNET, "Ethernet");
    os.write_added(1, &wifi);
    assert_eq!(os.recorded(), vec![added_through(WIFI)]);

    let change = os.write_added(2, &ethernet);
    assert_eq!(
        change,
        BypassRoutesChange {
            network: NetworkCheck::Expect { network: ethernet },
            remove: vec![added_through(WIFI)],
            add: vec![host()],
            look_up: vec![host()],
        }
    );
    assert_eq!(os.recorded(), vec![added_through(ETHERNET)]);
}
