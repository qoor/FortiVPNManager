use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

use fvm_contract::app_helper::RunRequest;
use fvm_contract::helper_internal::NetworkSettings;
use zeroize::Zeroizing;

const LOCK: &str = "a link's lock is never held across a panic";

pub trait Backend: Send + Sync {
    fn init(&self);

    fn name(&self) -> &'static str;

    fn version(&self) -> &str;

    /// Checks the values of `request` before a run starts.
    ///
    /// # Errors
    ///
    /// Returns an English reason for the app when a value of `request` is one the VPN client
    /// refuses.
    fn check(&self, request: &RunRequest) -> Result<(), String>;

    fn run(&self, request: SessionRequest, link: &Link) -> SessionEnd;
}

pub struct SessionRequest {
    pub host: String,
    pub port: u16,
    pub realm: Option<String>,
    pub username: String,
    pub password: Zeroizing<String>,
    pub pins: Vec<String>,
    pub dtls: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SessionEnd {
    pub code: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Cancel,
    Detach,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SessionEvent {
    GatewayTrusted,
    CertificateRejected(String),
    OtpPrompted,
    FormRepeated(FormKind),
    Authenticated,
    Line(LineLevel, String),
    TunnelReady(Result<Box<NetworkSettings>, String>),
    TunnelFailed(Option<Ipv4Addr>, Option<String>),
    Reconnected,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormKind {
    Login,
    Otp,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineLevel {
    Error,
    Info,
}

type OtpCode = Option<Zeroizing<String>>;

type Opened = Box<dyn FnOnce(Zeroizing<String>) + Send>;

pub struct Link {
    report: Box<dyn Fn(SessionEvent) + Send + Sync>,
    opened: Mutex<Option<Opened>>,
    otp_sender: Sender<OtpCode>,
    otp: Mutex<Receiver<OtpCode>>,
    cancel: Mutex<Cancel>,
}

#[derive(Default)]
struct Cancel {
    requested: bool,
    write: Option<Box<dyn Fn(Command) + Send>>,
}

impl Link {
    pub fn new(
        report: impl Fn(SessionEvent) + Send + Sync + 'static,
        opened: impl FnOnce(Zeroizing<String>) + Send + 'static,
    ) -> Self {
        let (otp_sender, otp) = mpsc::channel();
        Self {
            report: Box::new(report),
            opened: Mutex::new(Some(Box::new(opened))),
            otp_sender,
            otp: Mutex::new(otp),
            cancel: Mutex::default(),
        }
    }

    pub fn report(&self, event: SessionEvent) {
        (self.report)(event);
    }

    /// Waits for the OTP the app submits; `None` when the run is cancelled.
    ///
    /// # Panics
    ///
    /// Panics if the link's lock is poisoned.
    pub fn otp(&self) -> Option<Zeroizing<String>> {
        self.otp.lock().expect(LOCK).recv().ok().flatten()
    }

    /// Hands `code` to the session waiting in `otp`.
    ///
    /// # Panics
    ///
    /// Panics if the link no longer holds the OTP receiver.
    pub fn submit_otp(&self, code: Zeroizing<String>) {
        self.otp_sender
            .send(Some(code))
            .expect("the link holds the OTP receiver");
    }

    /// Reports the session's cookie once the tunnel is open.
    ///
    /// # Panics
    ///
    /// Panics if the link's lock is poisoned. Panics on a second call.
    pub fn opened(&self, cookie: Zeroizing<String>) {
        let opened = self
            .opened
            .lock()
            .expect(LOCK)
            .take()
            .expect("a session opens once");
        opened(cookie);
    }

    /// Sets how a cancel or a detach reaches the running backend; `false` when the run was already
    /// cancelled.
    ///
    /// # Panics
    ///
    /// Panics if the link's lock is poisoned.
    pub fn arm_cancel(&self, write: impl Fn(Command) + Send + 'static) -> bool {
        let mut cancel = self.cancel.lock().expect(LOCK);
        if cancel.requested {
            return false;
        }
        cancel.write = Some(Box::new(write));
        true
    }

    /// Forgets the writer that `arm_cancel` set.
    ///
    /// # Panics
    ///
    /// Panics if the link's lock is poisoned.
    pub fn disarm_cancel(&self) {
        self.cancel.lock().expect(LOCK).write = None;
    }

    /// Cancels the run: asks the armed backend to cancel and ends a wait for the OTP. Returns
    /// whether a backend was armed.
    ///
    /// # Panics
    ///
    /// Panics if the link's lock is poisoned. Panics if the link no longer holds the OTP receiver.
    pub fn cancel(&self) -> bool {
        let mut cancel = self.cancel.lock().expect(LOCK);
        cancel.requested = true;
        let armed = cancel.write.is_some();
        if let Some(write) = &cancel.write {
            write(Command::Cancel);
        }
        self.otp_sender
            .send(None)
            .expect("the link holds the OTP receiver");
        armed
    }

    /// Asks the armed backend to detach from the tunnel; `false` when none is armed.
    ///
    /// # Panics
    ///
    /// Panics if the link's lock is poisoned.
    pub fn detach(&self) -> bool {
        let cancel = self.cancel.lock().expect(LOCK);
        let Some(write) = &cancel.write else {
            return false;
        };
        write(Command::Detach);
        true
    }
}
