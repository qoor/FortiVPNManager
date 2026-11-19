//! The macOS Helper's Rust start. Swift reaches the Helper's Rust side only through the socket
//! returned by `fvm_helper_entry_start`, using the messages in `fvm_contract::helper_internal`, and
//! the shared Helper entry has the session manager run the VPN client with its backend adapter,
//! keeping the session secrets in the System keychain.

mod keychain;

use std::io::{self, BufReader};
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use fvm_backend_adapter::openconnect::OpenConnect;
use fvm_contract::stream;
use fvm_helper_core::secrets::SecretStore;
use fvm_helper_core::session::backend::Backend;
use fvm_helper_core::session::{SessionManager, Timing};
use keychain::Keychain;

/// Root-only: it holds the run record.
const RUN_DIRECTORY: &str = "/var/run/com.qoor.FortiVPNManager.Helper";

/// Starts the Helper entry and returns Swift's end of its socketpair, which Swift then owns.
// The C symbol Swift calls and the C function Swift passes: exporting a fixed name needs the unsafe
// attribute, and calling the function an unsafe block.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn fvm_helper_entry_start(fault: unsafe extern "C" fn(*const u8, usize)) -> i32 {
    fvm_helper_core::entry::report_panics(move |line| {
        // SAFETY: Swift's function reads the `length` bytes at `text` during the call, which `line`
        // holds.
        unsafe { fault(line.as_ptr(), line.len()) }
    });
    start(
        Arc::new(OpenConnect::default()),
        Arc::new(Keychain),
        PathBuf::from(RUN_DIRECTORY),
    )
    .into_raw_fd()
}

/// Starts the Helper entry for the client of `backend`, keeping its session secrets in `secrets`
/// and its files in `run_directory`, and returns Swift's end of the stream. The entry says "ready"
/// once it has cleaned up after an earlier Helper.
pub fn start(
    backend: Arc<dyn Backend>,
    secrets: Arc<dyn SecretStore>,
    run_directory: PathBuf,
) -> UnixStream {
    let (ours, theirs) = socket_pair().expect("creating the socketpair to Swift");
    let writer = ours.try_clone().expect("cloning the socket to Swift");
    let sessions = SessionManager::new(backend, secrets, run_directory, Timing::default());
    let inbox = fvm_helper_core::entry::start(sessions, move |message| {
        stream::send(&mut &writer, message).expect("writing a message to Swift");
    });
    thread::Builder::new()
        .name("fvm-entry-read".into())
        .spawn(move || {
            let mut reader = BufReader::new(ours);
            loop {
                let message = stream::receive(&mut reader).expect("reading a message from Swift");
                inbox.deliver(message);
            }
        })
        .expect("starting the entry reader thread");
    theirs
}

/// Rust keeps the first end; Swift gets the second.
fn socket_pair() -> io::Result<(UnixStream, UnixStream)> {
    let (ours, theirs) = UnixStream::pair()?;
    // Rust's `main`, which ignores SIGPIPE, never runs in a library linked into a Swift process, so
    // a write to a closed peer would kill the process instead of failing with EPIPE.
    socket2::SockRef::from(&ours).set_nosigpipe(true)?;
    socket2::SockRef::from(&theirs).set_nosigpipe(true)?;
    Ok((ours, theirs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_pair_connects_both_ends() {
        let (ours, theirs) = socket_pair().unwrap();
        stream::send(&mut &theirs, &"from Swift").unwrap();
        let received: String = stream::receive(&mut BufReader::new(&ours)).unwrap();
        assert_eq!(received, "from Swift");
    }
}
