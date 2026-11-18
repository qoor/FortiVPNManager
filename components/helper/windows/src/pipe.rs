//! The app side of the Helper: the named pipe the apps connect to, and the relay between those
//! connections and the Helper entry. It does what Swift's `EntryRelay` and XPC listener do on
//! macOS: it announces each connection to the entry and keeps it only if the entry admits it,
//! answers the build check itself, passes the other requests on with numbers of its own, sends
//! each answer, event and output batch to the connection it belongs to, in the order the entry
//! sent them, and changes the routing table as the entry asks.
//!
//! Every connection has a reader thread and a writer thread, so an app that stops reading holds up
//! only its own messages, never the entry; one that lets too many of them pile up is closed.

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::app_helper::{BypassRoutesReply, HelperError, HelperErrorCode};
use fvm_contract::helper_internal::{
    AppRequest, BypassRoutesChange, FromEntry, LogCategory, LogLevel, NetworkChange, ToEntry,
};
use fvm_contract::helper_pipe::{MAX_MESSAGE_BYTES, PipeMessage, PipeRequest};
use fvm_contract::stream;
use fvm_helper_core::entry::Inbox;
use fvm_helper_core::session::SessionManager;
use interprocess::os::windows::named_pipe::{
    DuplexPipeStream, PipeListener, PipeListenerOptions, PipeMode, RecvPipeStream, SendPipeStream,
    pipe_mode,
};
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use widestring::U16CString;
use zeroize::Zeroizing;

use crate::{BUILD, Log, network, routes};

/// How long a connection whose accept failed waits before the next accept, so a failing pipe
/// cannot spin.
const ACCEPT_RETRY: Duration = Duration::from_millis(500);
/// How many messages may wait for a connection that is slow to read. The app reads all the time; a
/// connection that lets this many pile up is closed, so another program cannot make the Helper keep
/// its answers without limit by asking and never reading.
const OUTGOING_LIMIT: usize = 256;

pub struct Options {
    /// `\\.\pipe\…`; `fvm_contract::helper_pipe::PIPE_NAME` in the service.
    pub pipe_name: String,
    /// The pipe's security descriptor in SDDL: who may connect, and its owner.
    pub security: String,
    /// The VPN client, with its run files where only the Helper may write.
    pub sessions: SessionManager,
}

/// How the entry wants the Helper to end.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Exit {
    /// Once what was sent to the apps has left, or after a short wait.
    AfterDraining,
    /// At once.
    Now,
}

/// The running Helper, from the service's or a test's side.
pub struct Helper {
    relay: Arc<Relay>,
    /// Read only by whoever waits for the exit, the service's main thread. The lock is there
    /// because the service's control handler, on another thread, holds the Helper to call
    /// `terminate`, and a receiver alone cannot be shared between threads.
    exits: Mutex<Receiver<Exit>>,
}

impl Helper {
    /// Starts the entry and, once it cleaned up after an earlier Helper, opens the pipe and returns
    /// with connections accepted; until then an app finds no pipe and waits for it, as on macOS.
    /// Fails when the pipe cannot be created, such as when another program created a pipe of that
    /// name first; the entry then has no run and the Helper exits.
    pub fn start(options: Options, log: Log) -> io::Result<Self> {
        let security = U16CString::from_str(&options.security)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "SDDL holds a NUL"))?;
        let security = SecurityDescriptor::deserialize(&security)?;
        let (exit_sender, exits) = mpsc::channel();
        let (route_changes, route_queue) = mpsc::channel();
        let relay = Arc::new(Relay {
            inbox: OnceLock::new(),
            log,
            state: Mutex::new(State::default()),
            exits: exit_sender,
            route_changes,
        });
        let (ready_sender, ready) = mpsc::channel();
        let entry_side = Arc::clone(&relay);
        let inbox = fvm_helper_core::entry::start(options.sessions, move |message| {
            if let FromEntry::Ready {} = message {
                // The entry says it once; nothing waits for it afterwards.
                let _ = ready_sender.send(());
            }
            entry_side.from_entry(message);
        });
        assert!(relay.inbox.set(inbox).is_ok(), "the inbox is set once");
        let route_side = Arc::clone(&relay);
        thread::Builder::new()
            .name("fvm-helper-routes".into())
            .spawn(move || route_side.serve_routes(route_queue))
            .expect("starting the route thread");
        ready.recv().expect("the entry says ready once");
        let listener: PipeListener<pipe_mode::Bytes, pipe_mode::Bytes> = PipeListenerOptions::new()
            .path(options.pipe_name.as_str())
            .mode(PipeMode::Bytes)
            .security_descriptor(Some(security))
            .create_duplex::<pipe_mode::Bytes>()?;
        let accept_side = Arc::clone(&relay);
        thread::Builder::new()
            .name("fvm-helper-accept".into())
            .spawn(move || accept_side.accept(listener))
            .expect("starting the accept thread");
        Ok(Self {
            relay,
            exits: Mutex::new(exits),
        })
    }

    /// The OS asked the Helper to stop: the entry ends the run, reports it and then says how to
    /// exit.
    pub fn terminate(&self) {
        self.relay.inbox().deliver(ToEntry::TerminationRequested {});
    }

    /// Blocks until the entry says to exit.
    pub fn wait_for_exit(&self) -> Exit {
        self.exits
            .lock()
            .expect("never held across a panic")
            .recv()
            .expect("the relay lives as long as the Helper")
    }

    /// Returns once everything sent to the apps has been read, or after `limit`.
    pub fn drain(&self, limit: Duration) {
        let writers: Vec<SyncSender<Outgoing>> = self
            .relay
            .lock()
            .connections
            .values()
            .map(|connection| connection.writer.clone())
            .collect();
        let (done, drained) = mpsc::channel();
        // A connection whose queue is full does not read; it is not waited for.
        let pending = writers
            .iter()
            .filter(|writer| writer.try_send(Outgoing::Drained(done.clone())).is_ok())
            .count();
        let deadline = Instant::now() + limit;
        for _ in 0..pending {
            let left = deadline.saturating_duration_since(Instant::now());
            if drained.recv_timeout(left).is_err() {
                return;
            }
        }
    }
}

struct Relay {
    inbox: OnceLock<Inbox>,
    log: Log,
    state: Mutex<State>,
    exits: Sender<Exit>,
    route_changes: Sender<RouteChange>,
}

enum RouteChange {
    BypassRoutes(BypassRoutesChange),
    Network(NetworkChange),
}

#[derive(Default)]
struct State {
    connections: HashMap<u64, Connection>,
    /// Announced connections waiting for the entry's answer.
    admissions: HashMap<u64, Sender<bool>>,
    /// The entry's request numbers with what they answer.
    replies: HashMap<u64, Pending>,
    last_app: u64,
    last_request: u64,
}

struct Connection {
    /// Holds at most `OUTGOING_LIMIT` messages.
    writer: SyncSender<Outgoing>,
    /// The app sent this build in its build check.
    same_build: bool,
    /// An exception-route request of this connection waits for the entry's answer.
    routing: bool,
}

/// A request passed on to the entry.
struct Pending {
    app: u64,
    /// The app's number for it.
    id: u64,
    /// An exception-route request, of which a connection has one at a time.
    route: bool,
}

enum Outgoing {
    Message(PipeMessage),
    /// Answered once everything before it has been read by the app.
    Drained(Sender<()>),
}

impl Relay {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .expect("the relay's state is never held across a panic")
    }

    /// Set right after the entry starts; route repairs during its cleanup may ask before that.
    fn inbox(&self) -> &Inbox {
        self.inbox.wait()
    }

    fn log(&self, level: LogLevel, message: &str) {
        (self.log)(level, LogCategory::Helper, message);
    }

    /// Runs on the entry's thread, one message at a time, so what reaches each app keeps the
    /// entry's order.
    fn from_entry(self: &Arc<Self>, message: &FromEntry) {
        match message {
            // The start learns of it from the callback and only then opens the pipe.
            FromEntry::Ready {} => {}
            FromEntry::Admitted { app, accept } => {
                let decided = self
                    .lock()
                    .admissions
                    .remove(app)
                    .expect("the entry answers only announced connections");
                // The accept thread waits for the answer.
                let _ = decided.send(*accept);
            }
            FromEntry::Reply { id, json } => {
                self.answer(*id, |id| PipeMessage::Reply {
                    id,
                    json: json.clone(),
                });
            }
            FromEntry::ReplyFlag { id, value } => {
                self.answer(*id, |id| PipeMessage::ReplyFlag { id, value: *value });
            }
            FromEntry::SendEvent { app, json } => {
                self.send(*app, PipeMessage::Event { json: json.clone() });
            }
            FromEntry::SendOutput { app, json } => {
                self.send(*app, PipeMessage::Output { json: json.clone() });
            }
            FromEntry::RepairRoutes { gateway } => {
                let gateway = *gateway;
                let relay = Arc::clone(self);
                // Changing routes must not hold up the entry's other messages.
                thread::Builder::new()
                    .name("fvm-helper-route-repair".into())
                    .spawn(move || {
                        let outcome = routes::repair(gateway, &relay.log);
                        relay.inbox().deliver(ToEntry::RoutesRepaired { outcome });
                    })
                    .expect("starting the route repair thread");
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
            FromEntry::ExitAfterDraining {} => {
                let _ = self.exits.send(Exit::AfterDraining);
            }
            FromEntry::ExitNow {} => {
                let _ = self.exits.send(Exit::Now);
            }
            FromEntry::Log {
                level,
                category,
                message,
            } => (self.log)(*level, *category, message),
        }
    }

    /// Sends the entry's answer to request `id` to the connection that asked, if it is still open.
    fn answer(&self, id: u64, message: impl FnOnce(u64) -> PipeMessage) {
        let pending = {
            let mut state = self.lock();
            let pending = state
                .replies
                .remove(&id)
                .expect("the entry answers each request once");
            // Cleared before the answer leaves, so the app's next route request is taken.
            if pending.route
                && let Some(connection) = state.connections.get_mut(&pending.app)
            {
                connection.routing = false;
            }
            pending
        };
        self.send(pending.app, message(pending.id));
    }

    /// Queues `message` for connection `app`; one that is gone gets nothing. A connection whose
    /// queue is full is dropped as if its app had ended, so the entry stops the run it started. Its
    /// pipe closes only when the app next writes or ends: nothing can cut a pipe that a reader and
    /// a writer are blocked on.
    fn send(&self, app: u64, message: PipeMessage) {
        let mut state = self.lock();
        let Some(connection) = state.connections.get(&app) else {
            return;
        };
        // The writer ends only after the connection was removed, so it is never disconnected here.
        if let Err(TrySendError::Full(_)) = connection.writer.try_send(Outgoing::Message(message)) {
            // Told under the lock, so no request of this connection reaches the entry after it.
            state.connections.remove(&app);
            self.inbox().deliver(ToEntry::AppDisconnected { app });
            drop(state);
            let line = format!("dropping connection {app}: it does not read what the helper sends");
            self.log(LogLevel::Error, &line);
        }
    }

    /// Accepting is retried until it works again. Only the first failure and the recovery are
    /// logged, so a failure that lasts does not flood the event log.
    fn accept(self: Arc<Self>, listener: PipeListener<pipe_mode::Bytes, pipe_mode::Bytes>) {
        let mut failing = false;
        loop {
            match listener.accept() {
                Ok(stream) => {
                    if failing {
                        failing = false;
                        self.log(LogLevel::Notice, "accepting connections again");
                    }
                    self.admit(stream);
                }
                Err(error) => {
                    if !failing {
                        failing = true;
                        let line = format!(
                            "accepting a connection failed: {error}; retrying without logging until it works"
                        );
                        self.log(LogLevel::Error, &line);
                    }
                    thread::sleep(ACCEPT_RETRY);
                }
            }
        }
    }

    /// Announces the connection to the entry and, if admitted, starts its reader and writer.
    fn admit(self: &Arc<Self>, stream: DuplexPipeStream<pipe_mode::Bytes>) {
        let session = match stream.client_session_id() {
            Ok(session) => session,
            Err(error) => {
                let line = format!("refusing a connection: its session is unknown ({error})");
                self.log(LogLevel::Error, &line);
                return;
            }
        };
        let (decide, decided) = mpsc::channel();
        // The entry refuses connections once it decided to exit.
        let app = {
            let mut state = self.lock();
            state.last_app += 1;
            let app = state.last_app;
            state.admissions.insert(app, decide);
            app
        };
        self.inbox()
            .deliver(ToEntry::AppConnected { app, uid: session });
        if !decided.recv().expect("the entry answers every connection") {
            self.log(
                LogLevel::Notice,
                "refusing a connection: the helper is exiting",
            );
            return;
        }
        let (receiver, sender) = stream.split();
        let (writer, outgoing) = mpsc::sync_channel(OUTGOING_LIMIT);
        self.lock().connections.insert(
            app,
            Connection {
                writer,
                same_build: false,
                routing: false,
            },
        );
        self.log(
            LogLevel::Notice,
            &format!("accepted connection {app} from session {session}"),
        );
        thread::Builder::new()
            .name(format!("fvm-helper-write-{app}"))
            .spawn(move || write_messages(sender, outgoing))
            .expect("starting a connection's writer");
        let relay = Arc::clone(self);
        thread::Builder::new()
            .name(format!("fvm-helper-read-{app}"))
            .spawn(move || relay.read_requests(app, receiver))
            .expect("starting a connection's reader");
    }

    /// Handles the connection's requests until it ends, sends something unreadable or was dropped
    /// for not reading, then forgets it.
    fn read_requests(self: Arc<Self>, app: u64, receiver: RecvPipeStream<pipe_mode::Bytes>) {
        loop {
            match receive(&receiver) {
                Ok(_) if !self.lock().connections.contains_key(&app) => break,
                Ok(request) => self.request(app, request),
                Err(error) => {
                    if error.kind() != io::ErrorKind::UnexpectedEof {
                        let line = format!("closing connection {app}: {error}");
                        self.log(LogLevel::Error, &line);
                    }
                    break;
                }
            }
        }
        // Dropping the writer's sender ends the writer once it wrote what was queued. A connection
        // dropped for not reading was already reported.
        let mut state = self.lock();
        if state.connections.remove(&app).is_some() {
            self.inbox().deliver(ToEntry::AppDisconnected { app });
        }
    }

    fn request(&self, app: u64, request: PipeRequest) {
        let id = request.id();
        let same_build = self
            .lock()
            .connections
            .get(&app)
            .is_some_and(|connection| connection.same_build);
        let unchecked = || HelperError {
            code: HelperErrorCode::NotAuthorized,
            message: "the connection has not passed the build check".to_owned(),
        };
        match request {
            PipeRequest::Build { app: build, .. } => {
                if let Some(connection) = self.lock().connections.get_mut(&app) {
                    connection.same_build = build == BUILD;
                }
                self.send(
                    app,
                    PipeMessage::Build {
                        id,
                        build: BUILD.to_owned(),
                    },
                );
            }
            PipeRequest::ExitWhenIdle { .. } => {
                self.log(
                    LogLevel::Notice,
                    "not exiting on request: Windows starts the helper again only through a reinstall",
                );
                self.send(app, PipeMessage::ReplyFlag { id, value: false });
            }
            // The password and the OTP are wiped where they go no further, as the entry wipes
            // what reaches it.
            PipeRequest::Connect { json, .. } if !same_build => {
                drop(Zeroizing::new(json));
                self.send(app, reply(id, &unchecked()));
            }
            PipeRequest::SubmitOtp { code, .. } if !same_build => {
                drop(Zeroizing::new(code));
                self.send(app, reply(id, &unchecked()));
            }
            PipeRequest::ReadBypassRoutes { .. } | PipeRequest::WriteBypassRoutes { .. }
                if !same_build =>
            {
                let failure = BypassRoutesReply::Failure { error: unchecked() };
                self.send(app, reply(id, &failure));
            }
            PipeRequest::Connect { json, .. } => {
                self.forward(app, id, AppRequest::Connect { json }, false)
            }
            PipeRequest::SubmitOtp { attempt, code, .. } => {
                self.forward(app, id, AppRequest::SubmitOtp { attempt, code }, false);
            }
            PipeRequest::Disconnect {
                attempt, reason, ..
            } => self.forward(app, id, AppRequest::Disconnect { attempt, reason }, false),
            PipeRequest::Status { .. } => self.forward(app, id, AppRequest::Status {}, false),
            PipeRequest::ReadBypassRoutes { .. } => {
                self.forward_route(app, id, AppRequest::ReadBypassRoutes {});
            }
            PipeRequest::WriteBypassRoutes { json, .. } => {
                self.forward_route(app, id, AppRequest::WriteBypassRoutes { json });
            }
        }
    }

    /// Passes a request to the entry under a number of the relay's own, since the apps' numbers
    /// may repeat across connections.
    /// Delivered under the lock, so it never follows the connection's AppDisconnected.
    /// `route` tells an exception-route request, of which a connection has one at a time.
    fn forward(&self, app: u64, id: u64, request: AppRequest, route: bool) {
        let mut state = self.lock();
        if !state.connections.contains_key(&app) {
            // It does not reach the entry, which would wipe the password and the OTP.
            match request {
                AppRequest::Connect { json } => drop(Zeroizing::new(json)),
                AppRequest::SubmitOtp { code, .. } => drop(Zeroizing::new(code)),
                _ => {}
            }
            return;
        }
        state.last_request += 1;
        let number = state.last_request;
        state.replies.insert(number, Pending { app, id, route });
        self.inbox().deliver(ToEntry::Request {
            id: number,
            app,
            request,
        });
    }

    /// Passes on an exception-route request. A connection may have one at a time, as the app
    /// sends them, so another program cannot pile them up.
    fn forward_route(&self, app: u64, id: u64, request: AppRequest) {
        let busy = match self.lock().connections.get_mut(&app) {
            Some(connection) => std::mem::replace(&mut connection.routing, true),
            None => return,
        };
        if busy {
            let failure = BypassRoutesReply::Failure {
                error: HelperError {
                    code: HelperErrorCode::InvalidRequest,
                    message: "the connection's previous route request is still running".to_owned(),
                },
            };
            self.send(app, reply(id, &failure));
            return;
        }
        self.forward(app, id, request, true);
    }

    fn serve_routes(&self, changes: Receiver<RouteChange>) {
        for change in changes {
            let message = match change {
                RouteChange::BypassRoutes(change) => ToEntry::BypassRoutesChanged {
                    outcome: routes::change_bypass_routes(&change, &self.log),
                },
                RouteChange::Network(change) => ToEntry::NetworkChanged {
                    outcome: network::change(&change, &self.log),
                },
            };
            self.inbox().deliver(message);
        }
    }
}

fn reply(id: u64, payload: &impl serde::Serialize) -> PipeMessage {
    PipeMessage::Reply {
        id,
        json: Some(serde_json::to_string(payload).expect("contract types serialize")),
    }
}

/// Reads one framed request; a length over the limit ends the connection before anything is
/// allocated for it. Only a pipe that ends is a hang-up: JSON that ends early is bad input like
/// any other, though serde_json reports it as an early end. The frame may hold the password or
/// the OTP, so it is wiped once read.
fn receive(receiver: &RecvPipeStream<pipe_mode::Bytes>) -> io::Result<PipeRequest> {
    let mut reader = receiver;
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header);
    if length > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a message of {length} bytes is over the limit"),
        ));
    }
    let mut payload = Zeroizing::new(vec![0; length as usize]);
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Writes the queued messages until the connection is forgotten. After a failed write the rest is
/// dropped, but drain requests are still answered.
fn write_messages(sender: SendPipeStream<pipe_mode::Bytes>, outgoing: Receiver<Outgoing>) {
    let mut broken = false;
    for item in outgoing {
        match item {
            Outgoing::Message(message) => {
                // `send` flushes, which waits until the app has read the message.
                broken = broken || stream::send(&mut &sender, &message).is_err();
            }
            Outgoing::Drained(done) => {
                let _ = done.send(());
            }
        }
    }
    // Nothing is left to send; the app does not need to read what follows the last flush.
    sender.assume_flushed();
}
