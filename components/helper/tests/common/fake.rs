use std::collections::{BTreeMap, VecDeque};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, TcpListener};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::app_helper::RunRequest;
use fvm_contract::helper_internal::{
    FamilySettings, IpNetwork, NetworkSettings, TunnelDevice, TunnelRoutes,
};
use fvm_helper_core::secrets::{SecretStore, StoredSecret};
use fvm_helper_core::session::backend::{
    Backend, LineLevel, Link, SessionEnd, SessionEvent, SessionRequest,
};
use zeroize::Zeroizing;

pub const VERSION: &str = "9.21";
pub const PIN: &str = "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=";
pub const OTHER_PIN: &str = "pin-sha256:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
pub const COOKIE: &str = "SVPNCOOKIE=Ab12Cd34Ef56";
pub const GATEWAY: &str = "203.0.113.10";
pub const EINTR: i32 = 4;
pub const EIO: i32 = 5;
pub const EPERM: i32 = 1;

pub type Script = Box<dyn FnOnce(SessionRequest, &Link) -> SessionEnd + Send>;

pub fn script(run: impl FnOnce(SessionRequest, &Link) -> SessionEnd + Send + 'static) -> Script {
    Box::new(run)
}

#[derive(Default)]
pub struct FakeBackend {
    scripts: Mutex<VecDeque<Script>>,
}

impl FakeBackend {
    pub fn new(scripts: impl IntoIterator<Item = Script>) -> Self {
        Self {
            scripts: Mutex::new(scripts.into_iter().collect()),
        }
    }

    pub fn push(&self, script: Script) {
        self.scripts.lock().unwrap().push_back(script);
    }
}

impl Backend for FakeBackend {
    fn init(&self) {}

    fn name(&self) -> &'static str {
        "openconnect"
    }

    fn version(&self) -> &str {
        VERSION
    }

    fn check(&self, request: &RunRequest) -> Result<(), String> {
        if request.port == 0 {
            Err("port 0 is refused".to_owned())
        } else {
            Ok(())
        }
    }

    fn run(&self, request: SessionRequest, link: &Link) -> SessionEnd {
        let script = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .expect("a script for every session");
        script(request, link)
    }
}

pub fn ended(code: i32) -> SessionEnd {
    SessionEnd { code }
}

pub fn line(link: &Link, text: &str) {
    link.report(SessionEvent::Line(LineLevel::Info, text.to_owned()));
}

pub fn error_line(link: &Link, text: &str) {
    link.report(SessionEvent::Line(LineLevel::Error, text.to_owned()));
}

pub fn trust_gateway(link: &Link) {
    line(
        link,
        "Connected to HTTPS on 203.0.113.10 with ciphersuite (TLS1.3)",
    );
    link.report(SessionEvent::GatewayTrusted);
}

pub fn ask_otp(link: &Link) -> Option<Zeroizing<String>> {
    link.report(SessionEvent::OtpPrompted);
    link.otp()
}

pub fn open(link: &Link) {
    link.opened(Zeroizing::new(COOKIE.to_owned()));
    link.report(SessionEvent::Authenticated);
}

pub fn tunnel_ready(link: &Link, settings: NetworkSettings) {
    line(link, "Connected as 10.10.0.5, using SSL");
    link.report(SessionEvent::TunnelReady(Ok(Box::new(settings))));
}

pub fn wait_for_cancel(link: &Link) {
    let (cancelled, wait) = mpsc::channel();
    if link.arm_cancel(move |_| {
        let _ = cancelled.send(());
    }) {
        let _ = wait.recv();
    }
    link.disarm_cancel();
}

pub fn up(settings: NetworkSettings) -> Script {
    script(move |_, link| {
        trust_gateway(link);
        if ask_otp(link).is_none() {
            return ended(-EIO);
        }
        open(link);
        tunnel_ready(link, settings);
        wait_for_cancel(link);
        ended(-EINTR)
    })
}

pub fn up_without_otp(settings: NetworkSettings) -> Script {
    script(move |_, link| {
        trust_gateway(link);
        open(link);
        tunnel_ready(link, settings);
        wait_for_cancel(link);
        ended(-EINTR)
    })
}

pub fn network(address: &str, prefix_length: u8) -> IpNetwork {
    IpNetwork {
        address: address.parse().unwrap(),
        prefix_length,
    }
}

pub fn settings(device: &str, address: &str) -> NetworkSettings {
    NetworkSettings {
        tunnel: TunnelDevice {
            name: device.to_owned(),
            index: None,
        },
        gateway: GATEWAY.parse().unwrap(),
        mtu: Some(1354),
        ipv4: FamilySettings {
            address: IpNetwork {
                address: address.parse().unwrap(),
                prefix_length: 32,
            },
            routes: TunnelRoutes::Split {
                include: vec![network("10.20.0.0", 16)],
            },
        },
        ipv6: None,
        split_exclude: vec![network("10.20.161.16", 32)],
        dns_servers: vec![IpAddr::V4(Ipv4Addr::new(10, 20, 0, 53))],
        default_domains: vec!["corp.example.com".to_owned()],
        split_dns_domains: Vec::new(),
        wins_servers: Vec::new(),
    }
}

#[derive(Default)]
pub struct FakeSecrets {
    stored: Mutex<BTreeMap<String, Result<Vec<u8>, String>>>,
    saves: Mutex<Vec<(String, Option<String>)>>,
    deletes: Mutex<Vec<String>>,
    hold_save: Mutex<Option<Receiver<()>>>,
    hold_delete: Mutex<Option<Receiver<()>>>,
    refuse_saves: bool,
}

impl FakeSecrets {
    pub fn holding(stored: impl IntoIterator<Item = (String, Result<Vec<u8>, String>)>) -> Self {
        Self {
            stored: Mutex::new(stored.into_iter().collect()),
            ..Self::default()
        }
    }

    pub fn refusing_saves() -> Self {
        Self {
            refuse_saves: true,
            ..Self::default()
        }
    }

    pub fn hold_saves(&self) -> Sender<()> {
        let (release, hold) = mpsc::channel();
        *self.hold_save.lock().unwrap() = Some(hold);
        release
    }

    pub fn hold_deletes(&self) -> Sender<()> {
        let (release, hold) = mpsc::channel();
        *self.hold_delete.lock().unwrap() = Some(hold);
        release
    }

    pub fn stored(&self) -> BTreeMap<String, Result<Vec<u8>, String>> {
        self.stored.lock().unwrap().clone()
    }

    pub fn saves(&self) -> Vec<(String, Option<String>)> {
        self.saves.lock().unwrap().clone()
    }

    pub fn deletes(&self) -> Vec<String> {
        self.deletes.lock().unwrap().clone()
    }
}

impl SecretStore for FakeSecrets {
    fn save(&self, name: &str, secret: &[u8]) -> Result<(), String> {
        let thread = thread::current().name().map(str::to_owned);
        self.saves.lock().unwrap().push((name.to_owned(), thread));
        if let Some(hold) = self.hold_save.lock().unwrap().take() {
            let _ = hold.recv();
        }
        if self.refuse_saves {
            return Err("the store is locked".to_owned());
        }
        self.stored
            .lock()
            .unwrap()
            .insert(name.to_owned(), Ok(secret.to_vec()));
        Ok(())
    }

    fn load_all(&self) -> Result<Vec<StoredSecret>, String> {
        Ok(self
            .stored
            .lock()
            .unwrap()
            .iter()
            .map(|(name, secret)| (name.clone(), secret.clone().map(Zeroizing::new)))
            .collect())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        self.deletes.lock().unwrap().push(name.to_owned());
        if let Some(hold) = self.hold_delete.lock().unwrap().take() {
            let _ = hold.recv();
        }
        self.stored
            .lock()
            .unwrap()
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| format!("no secret named {name}"))
    }
}

pub fn secret_json(host: &str, port: u16) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "host": host,
        "port": port,
        "pins": [PIN],
        "cookie": COOKIE,
    }))
    .unwrap()
}

pub struct FakeGateway {
    pub port: u16,
    connections: Arc<Mutex<Vec<Vec<u8>>>>,
    release: Option<Sender<()>>,
}

impl FakeGateway {
    pub fn start() -> Self {
        Self::serve(None)
    }

    pub fn holding() -> Self {
        let (release, held) = mpsc::channel();
        Self {
            release: Some(release),
            ..Self::serve(Some(held))
        }
    }

    fn serve(held: Option<Receiver<()>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&connections);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut start = [0; 1];
                let read = stream.read(&mut start).unwrap_or(0);
                seen.lock().unwrap().push(start[..read].to_vec());
                if let Some(held) = &held {
                    let _ = held.recv();
                }
            }
        });
        Self {
            port,
            connections,
            release: None,
        }
    }

    pub fn release(&self) {
        self.release
            .as_ref()
            .expect("only a holding gateway releases")
            .send(())
            .unwrap();
    }

    pub fn connections(&self) -> Vec<Vec<u8>> {
        self.connections.lock().unwrap().clone()
    }

    pub fn wait_for_connections(&self, count: usize) -> Vec<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.connections().len() < count && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        self.connections()
    }
}

pub const TLS_HANDSHAKE: [u8; 1] = [0x16];
