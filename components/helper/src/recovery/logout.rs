#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

use std::env;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use fvm_contract::validation::{self, MAX_HOST_BYTES, PIN_PREFIX, certificate_value};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

#[cfg(unix)]
use unix::Child;
#[cfg(windows)]
use windows::Child;

#[cfg(windows)]
const CURL: &str = r"C:\Windows\System32\curl.exe";
#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(windows)]
const SYSTEM_VARIABLES: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATH",
    "PATHEXT",
    "TEMP",
    "TMP",
];
#[cfg(not(windows))]
const CURL: &str = "/usr/bin/curl";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";
#[cfg(not(windows))]
const SYSTEM_VARIABLES: &[&str] = &["PATH"];
const LOGOUT_SECONDS: &str = "10";
const LOGOUT_LIMIT: Duration = Duration::from_secs(15);
const OUTPUT_BYTES: u64 = 256;
const ENCODED_BYTES: usize = 1024;

#[derive(Serialize, Deserialize)]
pub(crate) struct SessionSecret {
    pub host: String,
    pub port: u16,
    pub pins: Vec<String>,
    pub cookie: String,
}

impl SessionSecret {
    pub(crate) fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut bytes = Zeroizing::new(Vec::with_capacity(ENCODED_BYTES + self.cookie.len()));
        serde_json::to_writer(&mut *bytes, self).expect("a session secret serializes");
        bytes
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, String> {
        let secret: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        validation::check_host(&secret.host, MAX_HOST_BYTES)?;
        if secret.port == 0 {
            return Err("port must be between 1 and 65535".to_owned());
        }
        if let Some(index) = secret
            .pins
            .iter()
            .position(|pin| certificate_value(pin).is_none())
        {
            return Err(format!("pin {index} is not a pin-sha256: value"));
        }
        Ok(secret)
    }
}

impl Drop for SessionSecret {
    fn drop(&mut self) {
        self.cookie.zeroize();
    }
}

pub(crate) fn log_out(secret: &SessionSecret) -> String {
    let pins = secret
        .pins
        .iter()
        .map(|pin| {
            let hash = pin.strip_prefix(PIN_PREFIX).expect("the pins were checked");
            format!("sha256//{hash}")
        })
        .collect::<Vec<_>>()
        .join(";");
    let url = format!("https://{}:{}/remote/logout", secret.host, secret.port);
    let environment: Vec<_> = SYSTEM_VARIABLES
        .iter()
        .filter_map(|&name| Some((name, env::var_os(name)?)))
        .collect();
    let spawned = Child::spawn(
        CURL,
        &[
            "-q",
            "-s",
            "-k",
            "--noproxy",
            "*",
            "--max-time",
            LOGOUT_SECONDS,
            "--pinnedpubkey",
            &pins,
            "-H",
            "@-",
            "-o",
            NULL_DEVICE,
            "-w",
            "HTTP %{http_code}",
            &url,
        ],
        &environment,
    );
    let (child, mut stdin, stdout) = match spawned {
        Ok(spawned) => spawned,
        Err(error) => return format!("logout request not sent: {CURL}: {error}"),
    };
    let mut header = Zeroizing::new(Vec::with_capacity(secret.cookie.len() + 9));
    header.extend_from_slice(b"Cookie: ");
    header.extend_from_slice(secret.cookie.as_bytes());
    header.push(b'\n');
    let _ = stdin.write_all(&header);
    drop(stdin);
    let (sender, output) = mpsc::channel();
    thread::Builder::new()
        .name("fvm-logout-output".to_owned())
        .spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.take(OUTPUT_BYTES).read_to_end(&mut bytes);
            let _ = sender.send(bytes);
        })
        .expect("starting the logout output thread");
    let output = output.recv_timeout(LOGOUT_LIMIT);
    if output.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    let Ok(output) = output else {
        return format!(
            "logout request did not finish within {}s; killed",
            LOGOUT_LIMIT.as_secs()
        );
    };
    let output = String::from_utf8_lossy(&output);
    let answer = output.lines().next().map_or("", str::trim);
    if status.success() {
        format!("logout request sent: {answer}")
    } else {
        format!("logout request failed: {answer} ({status})")
    }
}
