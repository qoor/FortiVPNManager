//! Helper entry (macOS): the Helper's Rust side. Swift reaches it only through the socket returned
//! by `fvm_helper_entry_start`, using the messages in `fvm_contract::helper_internal`, and the
//! shared Helper entry runs the VPN client with the openfortivpn backend adapter and macOS's
//! process and file handling.

mod platform;

use std::io::{self, BufReader};
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;

use fvm_backend_adapter::openfortivpn::OpenFortiVpn;
use fvm_contract::backend::Backend;
use fvm_contract::stream;

/// Root-only: it holds the config file of the current run and the pidfile.
const RUN_DIRECTORY: &str = "/var/run/com.qoor.FortiVPNManager.Helper";

/// Starts the Helper entry and returns Swift's end of its socketpair, which Swift then owns.
#[allow(unsafe_code)] // The C symbol Swift calls; exporting a fixed name needs the unsafe attribute.
#[unsafe(no_mangle)]
pub extern "C" fn fvm_helper_entry_start() -> i32 {
    start(OpenFortiVpn, PathBuf::from(RUN_DIRECTORY)).into_raw_fd()
}

/// Starts the Helper entry for `backend`'s client, keeping its files in `run_directory`, and returns
/// Swift's end of the stream. The entry says "ready" once it has cleaned up after an earlier Helper.
pub fn start<B>(backend: B, run_directory: PathBuf) -> UnixStream
where
    B: Backend + Send + 'static,
    B::Interpreter: 'static,
{
    let (ours, theirs) = socket_pair().expect("creating the socketpair to Swift");
    let writer = ours.try_clone().expect("cloning the socket to Swift");
    let inbox = fvm_helper_entry::start(backend, platform::MacOs, run_directory, move |message| {
        stream::send(&mut &writer, message).expect("writing a message to Swift");
    });
    thread::Builder::new()
        .name("fvm-helper-entry-read".into())
        .spawn(move || {
            let mut reader = BufReader::new(ours);
            loop {
                let message = stream::receive(&mut reader).expect("reading a message from Swift");
                inbox.deliver(message);
            }
        })
        .expect("starting the Helper entry's reader thread");
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
