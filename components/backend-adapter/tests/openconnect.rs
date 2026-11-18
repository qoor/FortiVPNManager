use std::io::{ErrorKind, Read};
use std::net::{TcpListener, TcpStream};
use std::process::{self, Command};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Once};
use std::time::{Duration, Instant};
use std::{env, fs, thread};

use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_helper_core::session::backend::{Backend, Link, SessionEnd, SessionEvent, SessionRequest};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection};
use zeroize::Zeroizing;

const EINTR: i32 = 4;
const TLS_HANDSHAKE: u8 = 0x16;
const GATEWAY_CERT: &[u8] = include_bytes!("gateway/cert.der");
const GATEWAY_KEY: &[u8] = include_bytes!("gateway/key.der");
const GATEWAY_PIN: &str = "pin-sha256:MLXLDsm7DieFNnn7zYx1QArmPMlMb2vn3VZQHWavoXY=";
const OPENSSL_CONFIG_WITHOUT_ALGORITHMS: &str = "openssl_conf = openssl_init
[openssl_init]
providers = providers
[providers]
null = null
[null]
activate = 1
";

fn backend() -> OpenConnect {
    static INIT: Once = Once::new();
    let backend = OpenConnect::default();
    INIT.call_once(|| backend.init());
    backend
}

fn request(port: u16) -> SessionRequest {
    SessionRequest {
        host: "127.0.0.1".into(),
        port,
        realm: None,
        username: "user".into(),
        password: Zeroizing::new("password".into()),
        pins: Vec::new(),
        dtls: false,
    }
}

fn link() -> (Arc<Link>, Receiver<SessionEvent>) {
    let (sender, events) = mpsc::channel();
    let link = Link::new(
        move |event| {
            let _ = sender.send(event);
        },
        |_| unreachable!("the session never opens"),
    );
    (Arc::new(link), events)
}

#[test]
fn the_linked_library_is_openconnect_9_21() {
    let backend = OpenConnect::default();
    assert_eq!(backend.name(), "openconnect");
    assert!(
        backend.version().starts_with("9.21"),
        "{}",
        backend.version()
    );
}

#[test]
fn a_cancel_during_the_tls_handshake_ends_it_at_once() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (link, _) = link();
    let session = thread::spawn({
        let link = Arc::clone(&link);
        move || {
            let end = backend().run(request(port), &link);
            (end, Instant::now())
        }
    });
    let mut gateway = accept(&listener);
    let mut record = [0];
    gateway.read_exact(&mut record).unwrap();
    assert_eq!(record, [TLS_HANDSHAKE]);
    thread::sleep(Duration::from_secs(1));
    assert!(
        !session.is_finished(),
        "the handshake waits for the gateway"
    );
    let cancelled = Instant::now();
    link.cancel();
    let (_, ended) = session.join().unwrap();
    let waited = ended - cancelled;
    assert!(waited < Duration::from_secs(3), "{waited:?}");
}

#[test]
fn a_detach_while_the_gateway_never_answers_ends_the_session_at_once() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (link, _) = link();
    let session = thread::spawn({
        let link = Arc::clone(&link);
        move || {
            let mut request = request(port);
            request.pins = vec![GATEWAY_PIN.to_owned()];
            let end = backend().run(request, &link);
            (end, Instant::now())
        }
    });
    let mut stream = accept(&listener);
    let mut tls = ServerConnection::new(gateway_config()).unwrap();
    let mut gateway = rustls::Stream::new(&mut tls, &mut stream);
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        gateway.read_exact(&mut byte).unwrap();
        head.push(byte[0]);
    }
    assert!(
        head.starts_with(b"GET /"),
        "{}",
        String::from_utf8_lossy(&head)
    );
    thread::sleep(Duration::from_secs(1));
    assert!(
        !session.is_finished(),
        "the session waits for the gateway's answer"
    );
    let detached = Instant::now();
    assert!(link.detach());
    let (end, ended) = session.join().unwrap();
    let waited = ended - detached;
    assert!(waited < Duration::from_secs(3), "{waited:?}");
    assert_ne!(end.code, 0);
}

#[test]
fn an_openssl_config_file_is_never_read() {
    let config = env::temp_dir().join(format!("fvm-openssl-{}.cnf", process::id()));
    fs::write(&config, OPENSSL_CONFIG_WITHOUT_ALGORITHMS).unwrap();
    let output = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "a_cancel_during_the_tls_handshake_ends_it_at_once",
        ])
        .env("OPENSSL_CONF", &config)
        .output()
        .unwrap();
    fs::remove_file(&config).unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_session_cancelled_before_it_starts_never_connects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let (link, events) = link();
    link.cancel();
    let end = backend().run(request(listener.local_addr().unwrap().port()), &link);
    assert_eq!(end, SessionEnd { code: -EINTR });
    assert_eq!(listener.accept().unwrap_err().kind(), ErrorKind::WouldBlock);
    assert_eq!(events.try_iter().count(), 0);
}

fn gateway_config() -> Arc<ServerConfig> {
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(GATEWAY_CERT)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(GATEWAY_KEY)),
            )
            .unwrap();
    Arc::new(config)
}

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                return stream;
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("the session did not connect: {error}"),
        }
    }
}
