//! The threads that carry the stream to Swift, so the runtime never waits on Swift. Swift never
//! closes its end on purpose: an unreadable message, the end of the stream and a failed write all
//! stop the process (SPEC F-RT-02).

use std::io::{BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use fvm_contract::app_internal::{FromRuntime, ToRuntime};
use fvm_contract::stream;

/// Starts reading `socket`, handing each message to `deliver` in arrival order, and returns the
/// sender whose messages are written to `socket` in order.
pub(crate) fn connect(
    socket: UnixStream,
    deliver: impl FnMut(ToRuntime) + Send + 'static,
) -> Sender<FromRuntime> {
    let reader = socket.try_clone().expect("cloning the socket to Swift");
    thread::Builder::new()
        .name("fvm-runtime-read".into())
        .spawn(move || read(BufReader::new(reader), deliver))
        .expect("starting the runtime's reader thread");
    let (outgoing, messages) = mpsc::channel();
    thread::Builder::new()
        .name("fvm-runtime-write".into())
        .spawn(move || write(socket, messages))
        .expect("starting the runtime's writer thread");
    outgoing
}

fn read(mut reader: impl Read, mut deliver: impl FnMut(ToRuntime)) {
    loop {
        deliver(stream::receive(&mut reader).expect("reading a message from Swift"));
    }
}

fn write(mut writer: impl Write, messages: Receiver<FromRuntime>) {
    for message in messages {
        stream::send(&mut writer, &message).expect("writing a message to Swift");
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    #[should_panic(expected = "reading a message from Swift")]
    fn an_unreadable_message_stops_the_process() {
        read(
            Cursor::new(frame(br#"{"command":{"_0":{"fly":{}}}}"#)),
            |_| {},
        );
    }

    #[test]
    #[should_panic(expected = "reading a message from Swift")]
    fn the_end_of_the_stream_stops_the_process() {
        read(
            Cursor::new(frame(br#"{"command":{"_0":{"connect":{}}}}"#)),
            |_| {},
        );
    }
}
