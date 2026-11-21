//! Connects the XPC side of the Helper to the Helper entry: announces app connections, passes app
//! requests through unread, and carries out what the entry sends back.

use std::collections::HashMap;
use std::process;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::helper_internal::{
    AppRequest, BypassRoutesChange, FromEntry, LogCategory, LogLevel, NetworkChange, ToEntry,
};
use fvm_helper_core::entry::{self, Inbox};
use fvm_helper_core::session::SessionManager;
use fvm_os_api_macos::ipc::{DataReply, FlagReply, OptionalDataReply, Peer};
use fvm_os_api_macos::log::Level;
use objc2::sel;

use crate::{log, network, routes};

/// How long exiting waits for an app that stops reading.
const DRAIN_LIMIT: Duration = Duration::from_secs(1);

/// Where the entry's answer to one request goes.
pub(crate) enum Reply {
    /// connect, submitOTP and disconnect; `None` is the empty (success) reply.
    OptionalData(OptionalDataReply),
    /// status, the exception-route requests and the update requests.
    Data(DataReply),
    /// exitWhenIdle.
    Flag(FlagReply),
}

/// One accepted app connection: its number in messages to the entry, who is calling, and the
/// callback channel (`event`/`output`) for the runs it starts.
pub(crate) struct ClientLink {
    pub id: u64,
    pub uid: u32,
    pub pid: i32,
    peer: Peer,
}

impl ClientLink {
    /// An encoded `HelperEvent`.
    fn send_event(&self, data: &[u8]) {
        self.peer.send(sel!(event:), data, callback_failed);
    }

    fn send_output(&self, data: &[u8]) {
        self.peer.send(sel!(output:), data, callback_failed);
    }
}

fn callback_failed(error: String) {
    log(
        LogCategory::Helper,
        Level::Debug,
        &format!("callback to the app failed: {error}"),
    );
}

#[derive(Default)]
struct State {
    clients: HashMap<u64, Arc<ClientLink>>,
    /// Announced connections waiting for the entry's answer.
    admissions: HashMap<u64, (Arc<ClientLink>, Sender<()>)>,
    last_client_id: u64,
    replies: HashMap<u64, Reply>,
    last_request_id: u64,
    /// Cleared when the entry says to exit.
    exiting: bool,
    replacement_reported: bool,
}

enum RouteChange {
    BypassRoutes(BypassRoutesChange),
    Network(NetworkChange),
}

pub(crate) struct Relay {
    inbox: OnceLock<Inbox>,
    state: Mutex<State>,
    route_changes: Sender<RouteChange>,
}

impl Relay {
    /// Starts the entry and returns once app connections may be accepted: the entry first cleans up
    /// after an earlier Helper that died during a run.
    pub fn start(sessions: SessionManager) -> Arc<Self> {
        let (route_changes, route_queue) = mpsc::channel();
        let relay = Arc::new(Self {
            inbox: OnceLock::new(),
            state: Mutex::new(State::default()),
            route_changes,
        });
        let (ready_sender, ready) = mpsc::channel();
        let entry_side = Arc::clone(&relay);
        let inbox = entry::start(sessions, move |message| {
            entry_side.receive(message, &ready_sender);
        });
        assert!(relay.inbox.set(inbox).is_ok(), "the inbox is set once");
        let route_side = Arc::clone(&relay);
        thread::Builder::new()
            .name("fvm-helper-routes".into())
            .spawn(move || route_side.serve_routes(route_queue))
            .expect("starting the route thread");
        ready.recv().expect("the entry says ready once");
        relay
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("the relay's state is never held across a panic")
    }

    fn inbox(&self) -> &Inbox {
        self.inbox.wait()
    }

    /// Announces a connection that passed the signature check and waits for the entry to admit or
    /// refuse it; `None` when refused. A replacement of the executable is announced first, so the
    /// entry can refuse the connection.
    pub fn admit(
        &self,
        peer: Peer,
        uid: u32,
        pid: i32,
        executable_replaced: bool,
    ) -> Option<Arc<ClientLink>> {
        let (decided_sender, decided) = mpsc::channel();
        let id = {
            // Sent under the lock, so a replacement the watcher reports meanwhile cannot come
            // after it.
            let mut state = self.lock();
            if state.exiting {
                return None;
            }
            if executable_replaced {
                self.report_replacement(&mut state);
            }
            state.last_client_id += 1;
            let id = state.last_client_id;
            let client = Arc::new(ClientLink { id, uid, pid, peer });
            state.admissions.insert(id, (client, decided_sender));
            self.inbox().deliver(ToEntry::AppConnected { app: id, uid });
            id
        };
        decided
            .recv()
            .expect("the entry answers every announced connection");
        self.lock().clients.get(&id).cloned()
    }

    /// The connection is gone, whether the app quit, crashed or was killed.
    pub fn disconnected(&self, client: &ClientLink) {
        self.lock().clients.remove(&client.id);
        self.inbox()
            .deliver(ToEntry::AppDisconnected { app: client.id });
    }

    /// The self-update watcher saw the executable replaced.
    pub fn executable_replaced(&self) {
        let mut state = self.lock();
        self.report_replacement(&mut state);
    }

    /// SIGTERM: the entry ends the run and says when to exit.
    pub fn termination_requested(&self) {
        log(LogCategory::Helper, Level::Notice, "SIGTERM received");
        self.inbox().deliver(ToEntry::TerminationRequested {});
    }

    /// Passes an app request to the entry; `reply` gets its answer.
    pub fn forward(&self, request: AppRequest, client: &ClientLink, reply: Reply) {
        let id = {
            let mut state = self.lock();
            state.last_request_id += 1;
            let id = state.last_request_id;
            state.replies.insert(id, reply);
            id
        };
        self.inbox().deliver(ToEntry::Request {
            id,
            app: client.id,
            request,
        });
    }

    fn report_replacement(&self, state: &mut State) {
        if state.replacement_reported {
            return;
        }
        state.replacement_reported = true;
        self.inbox().deliver(ToEntry::ExecutableReplaced {});
    }

    /// Runs on the entry's thread, one message at a time, so callbacks reach each app in the order
    /// the entry sent them.
    fn receive(&self, message: &FromEntry, ready: &Sender<()>) {
        match message {
            FromEntry::Ready {} => {
                let _ = ready.send(());
            }
            FromEntry::Admitted { app, accept } => {
                let decided = {
                    let mut state = self.lock();
                    let Some((client, decided)) = state.admissions.remove(app) else {
                        panic!(
                            "the Helper entry answered app {app}, which is not waiting for admission"
                        );
                    };
                    if *accept {
                        state.clients.insert(*app, client);
                    }
                    decided
                };
                let _ = decided.send(());
            }
            FromEntry::Reply { id, json } => match self.take_reply(*id) {
                Reply::OptionalData(reply) => reply.send(json.as_deref().map(str::as_bytes)),
                Reply::Data(reply) => {
                    let Some(json) = json else {
                        panic!("the Helper entry answered request {id} without data");
                    };
                    reply.send(json.as_bytes());
                }
                Reply::Flag(_) => {
                    panic!("the Helper entry answered request {id} with data instead of a flag")
                }
            },
            FromEntry::ReplyFlag { id, value } => {
                let Reply::Flag(reply) = self.take_reply(*id) else {
                    panic!("the Helper entry answered request {id} with a flag instead of data");
                };
                reply.send(*value);
            }
            FromEntry::SendEvent { app, json } => {
                if let Some(client) = self.client(*app) {
                    client.send_event(json.as_bytes());
                }
            }
            FromEntry::SendOutput { app, json } => {
                if let Some(client) = self.client(*app) {
                    client.send_output(json.as_bytes());
                }
            }
            FromEntry::ChangeBypassRoutes { change } => {
                self.route_changes
                    .send(RouteChange::BypassRoutes(change.clone()))
                    .expect("the route thread runs as long as the Helper");
            }
            FromEntry::ChangeNetwork { change } => {
                self.route_changes
                    .send(RouteChange::Network(change.clone()))
                    .expect("the route thread runs as long as the Helper");
            }
            FromEntry::ExitAfterDraining {} => self.exit_after_draining(|| process::exit(0)),
            FromEntry::ExitNow {} => process::exit(0),
            FromEntry::AbortAfterDraining {} => {
                self.exit_after_draining(|| entry::end_for_restart());
            }
            FromEntry::Log {
                level,
                category,
                message,
            } => {
                let level = match level {
                    LogLevel::Info => Level::Info,
                    LogLevel::Notice => Level::Notice,
                    LogLevel::Error => Level::Error,
                };
                log(*category, level, message);
            }
        }
    }

    /// `None` once the app is gone: the entry may still send to it until it reads
    /// `AppDisconnected`.
    fn client(&self, id: u64) -> Option<Arc<ClientLink>> {
        self.lock().clients.get(&id).cloned()
    }

    fn take_reply(&self, id: u64) -> Reply {
        let Some(reply) = self.lock().replies.remove(&id) else {
            panic!("the Helper entry answered request {id}, which is not pending");
        };
        reply
    }

    /// Every change runs here, one at a time and off the entry's thread, so tunnel control never
    /// waits behind `/sbin/route`.
    fn serve_routes(&self, changes: Receiver<RouteChange>) {
        for change in changes {
            let message = match change {
                RouteChange::BypassRoutes(change) => ToEntry::BypassRoutesChanged {
                    outcome: routes::change(&change),
                },
                RouteChange::Network(change) => ToEntry::NetworkChanged {
                    outcome: network::change(&change),
                },
            };
            self.inbox().deliver(message);
        }
    }

    /// Ends the process with `end` once everything sent to the apps has left, or after
    /// `DRAIN_LIMIT`.
    fn exit_after_draining(&self, end: fn() -> !) {
        let clients: Vec<Arc<ClientLink>> = {
            let mut state = self.lock();
            state.exiting = true;
            state.clients.values().cloned().collect()
        };
        let (drained_sender, drained) = mpsc::channel();
        for client in &clients {
            let drained_sender = drained_sender.clone();
            client.peer.after_pending_sends(move || {
                let _ = drained_sender.send(());
            });
        }
        thread::Builder::new()
            .name("fvm-helper-exit".into())
            .spawn(move || {
                let deadline = Instant::now() + DRAIN_LIMIT;
                for _ in &clients {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if drained.recv_timeout(left).is_err() {
                        break;
                    }
                }
                end();
            })
            .expect("starting the exit thread");
    }
}
