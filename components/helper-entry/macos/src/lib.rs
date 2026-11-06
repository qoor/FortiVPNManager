//! Helper entry (macOS): the Helper's Rust side. Swift reaches it only through the socket returned
//! by `fvm_helper_entry_start`, using the messages in `fvm_contract::helper_internal`.

use std::io::{self, BufReader};
use std::os::fd::{IntoRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::thread;

use fvm_contract::helper_internal::ToEntry;
use fvm_contract::stream;

/// Starts the Helper entry and returns Swift's end of its socketpair, which Swift then owns.
#[allow(unsafe_code)] // The C symbol Swift calls; exporting a fixed name needs the unsafe attribute.
#[unsafe(no_mangle)]
pub extern "C" fn fvm_helper_entry_start() -> i32 {
    let (ours, theirs) = socket_pair().expect("creating the socketpair to Swift");
    thread::Builder::new()
        .name("fvm-helper-entry".into())
        .spawn(move || serve(&ours))
        .expect("starting the Helper entry thread");
    theirs.into_raw_fd()
}

/// Rust keeps the first end; Swift gets the second.
fn socket_pair() -> io::Result<(UnixStream, OwnedFd)> {
    let (ours, theirs) = UnixStream::pair()?;
    // Rust's `main`, which ignores SIGPIPE, never runs in a library linked into a Swift process, so
    // a write to a closed peer would kill the process instead of failing with EPIPE.
    socket2::SockRef::from(&ours).set_nosigpipe(true)?;
    socket2::SockRef::from(&theirs).set_nosigpipe(true)?;
    Ok((ours, theirs.into()))
}

fn serve(socket: &UnixStream) {
    let mut reader = BufReader::new(socket);
    loop {
        stream::receive::<ToEntry>(&mut reader).expect("reading a message from Swift");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_pair_connects_both_ends() {
        let (ours, theirs) = socket_pair().unwrap();
        let theirs = UnixStream::from(theirs);
        stream::send(&mut &theirs, &"from Swift").unwrap();
        let received: String = stream::receive(&mut BufReader::new(&ours)).unwrap();
        assert_eq!(received, "from Swift");
    }
}
