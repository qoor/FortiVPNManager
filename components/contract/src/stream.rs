//! Message stream between Swift and Rust in one process: each message is a 4-byte big-endian
//! length followed by that many bytes of UTF-8 JSON. Neither side ends the stream on purpose, so its
//! end is an error like any other. The Swift side is `components/contract/macos/MessageStream.swift`.

use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Writes one message with a single `write_all`.
pub fn send(writer: &mut impl Write, message: &impl Serialize) -> io::Result<()> {
    let mut frame = vec![0; 4];
    serde_json::to_writer(&mut frame, message)?;
    let length = u32::try_from(frame.len() - 4)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "message exceeds 4 GiB"))?;
    frame[..4].copy_from_slice(&length.to_be_bytes());
    writer.write_all(&frame)?;
    writer.flush()
}

/// Reads the next message. The end of the stream, even between two messages, is `UnexpectedEof`.
pub fn receive<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let mut payload = vec![0; u32::from_be_bytes(header) as usize];
    reader.read_exact(&mut payload)?;
    Ok(serde_json::from_slice(&payload)?)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::{Value, json};

    use super::*;

    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn length_is_big_endian_and_counts_the_payload() {
        let mut out = Vec::new();
        send(&mut out, &"x".repeat(256)).unwrap();
        assert_eq!(&out[..4], &[0, 0, 1, 2]);
        assert_eq!(out.len(), 4 + 258);
    }

    #[test]
    fn messages_arrive_in_order() {
        let mut stream = Vec::new();
        for message in [json!({"a": 1}), json!("‹ü›"), json!([])] {
            send(&mut stream, &message).unwrap();
        }
        let mut reader = Cursor::new(stream);
        assert_eq!(receive::<Value>(&mut reader).unwrap(), json!({"a": 1}));
        assert_eq!(receive::<Value>(&mut reader).unwrap(), json!("‹ü›"));
        assert_eq!(receive::<Value>(&mut reader).unwrap(), json!([]));
    }

    #[test]
    fn large_message_round_trips() {
        let text = "a".repeat(1 << 20);
        let mut stream = Vec::new();
        send(&mut stream, &text).unwrap();
        let received: String = receive(&mut Cursor::new(stream)).unwrap();
        assert_eq!(received, text);
    }

    #[test]
    fn any_end_of_the_stream_is_an_error() {
        let mut between_messages = Cursor::new(frame(b"[1,2]"));
        receive::<Value>(&mut between_messages).unwrap();
        let inside_the_header = Cursor::new(vec![0, 0]);
        let mut inside_the_payload = frame(b"[1,2]");
        inside_the_payload.pop();
        for mut reader in [
            between_messages,
            inside_the_header,
            Cursor::new(inside_the_payload),
        ] {
            let error = receive::<Value>(&mut reader).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        }
    }

    #[test]
    fn empty_or_unexpected_payload_is_an_error() {
        for payload in [&b""[..], b"{", b"\"text\""] {
            let result = receive::<Vec<u8>>(&mut Cursor::new(frame(payload)));
            assert!(result.is_err(), "{payload:?}");
        }
    }
}
