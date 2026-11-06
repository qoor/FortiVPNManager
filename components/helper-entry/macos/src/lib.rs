//! Helper entry (macOS): the Helper's Rust side. Swift reaches it only through the socket returned
//! by `fvm_helper_entry_start`, using the messages in `fvm_contract::helper_internal`. It cleans up
//! what an earlier Helper left behind, handles the app requests Swift passes on and runs the VPN
//! client through the watchdog with the openfortivpn backend adapter.

mod entry;
mod leftovers;
mod processes;
mod run_directory;

use std::io;
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;

use fvm_backend_adapter::openfortivpn::OpenFortiVpn;
use fvm_contract::backend::Backend;

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
    thread::Builder::new()
        .name("fvm-helper-entry".into())
        .spawn(move || entry::serve(backend, run_directory, ours))
        .expect("starting the Helper entry thread");
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
    use std::io::BufReader;

    use fvm_contract::stream;

    use super::*;

    #[test]
    fn socket_pair_connects_both_ends() {
        let (ours, theirs) = socket_pair().unwrap();
        stream::send(&mut &theirs, &"from Swift").unwrap();
        let received: String = stream::receive(&mut BufReader::new(&ours)).unwrap();
        assert_eq!(received, "from Swift");
    }
}
