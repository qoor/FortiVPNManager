//! How the macOS app starts the runtime: Swift calls `fvm_runtime_start` once and talks to the
//! runtime only through the stream it returns, with the messages in `fvm_contract::app_internal`.

mod link;

use std::io;
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;

/// Starts the runtime and returns Swift's end of its socketpair, which Swift then owns.
#[allow(unsafe_code)] // The C symbol Swift calls; exporting a fixed name needs the unsafe attribute.
#[unsafe(no_mangle)]
pub extern "C" fn fvm_runtime_start() -> i32 {
    start().into_raw_fd()
}

/// Starts the runtime and returns Swift's end of the stream. Until the event loop exists, messages
/// from Swift are read and dropped.
pub fn start() -> UnixStream {
    let (ours, theirs) = socket_pair().expect("creating the socketpair to Swift");
    link::connect(ours, |_message| {});
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
    use std::mem;
    use std::sync::mpsc;

    use fvm_contract::app_internal::command::Command;
    use fvm_contract::app_internal::port::PortAction;
    use fvm_contract::app_internal::{FromRuntime, ToRuntime};
    use fvm_contract::stream;
    use rustix::net::sockopt::socket_nosigpipe;
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn both_ends_refuse_sigpipe() {
        let (ours, theirs) = socket_pair().unwrap();
        assert!(socket_nosigpipe(&ours).unwrap());
        assert!(socket_nosigpipe(&theirs).unwrap());
    }

    #[test]
    fn messages_cross_the_stream_both_ways() {
        let (ours, theirs) = socket_pair().unwrap();
        let (delivered, received) = mpsc::channel();
        let outgoing = link::connect(ours, move |message| delivered.send(message).unwrap());

        stream::send(
            &mut &theirs,
            &json!({"command": {"_0": {"reconnectNow": {}}}}),
        )
        .unwrap();
        assert_eq!(
            received.recv().unwrap(),
            ToRuntime::Command {
                command: Command::ReconnectNow {}
            }
        );

        outgoing
            .send(FromRuntime::PortAction {
                action: PortAction::HoldActivity { hold: true },
            })
            .unwrap();
        let message: Value = stream::receive(&mut BufReader::new(&theirs)).unwrap();
        assert_eq!(
            message,
            json!({"portAction": {"_0": {"holdActivity": {"hold": true}}}})
        );
        // Closing Swift's end would stop the reader thread with a panic, as it does in the app.
        mem::forget(theirs);
    }
}
