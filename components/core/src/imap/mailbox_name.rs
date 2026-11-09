//! Mailbox names in modified UTF-7 (RFC 3501 §5.1.3): printable ASCII as it is, `&` as `&-`, and
//! everything else as `&`, modified base64 of UTF-16BE (`,` for `/`, no padding), `-`.

use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// `None` when `wire` is not valid modified UTF-7.
pub(super) fn decode(wire: &str) -> Option<String> {
    let bytes = wire.as_bytes();
    let mut name = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if !(0x20..=0x7E).contains(&byte) {
            return None;
        }
        if byte != b'&' {
            name.push(char::from(byte));
            index += 1;
            continue;
        }
        let dash = index + 1 + bytes[index + 1..].iter().position(|byte| *byte == b'-')?;
        if dash == index + 1 {
            name.push('&');
        } else {
            name.push_str(&decode_run(&bytes[index + 1..dash])?);
        }
        index = dash + 1;
    }
    Some(name)
}

fn decode_run(run: &[u8]) -> Option<String> {
    if !run
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b','))
    {
        return None;
    }
    let base64: Vec<u8> = run
        .iter()
        .map(|byte| if *byte == b',' { b'/' } else { *byte })
        .collect();
    let utf16 = LENIENT.decode(base64).ok()?;
    if utf16.is_empty() || utf16.len() % 2 != 0 {
        return None;
    }
    let units = utf16
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair));
    char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_wire_names() {
        for (wire, name) in [
            // Korean Gmail system folders: All Mail, Trash, Spam, Sent Mail.
            (
                "[Gmail]/&yATMtLz0rQDVaA-",
                "[Gmail]/\u{C804}\u{CCB4}\u{BCF4}\u{AD00}\u{D568}",
            ),
            ("[Gmail]/&1zTJwNG1-", "[Gmail]/\u{D734}\u{C9C0}\u{D1B5}"),
            ("[Gmail]/&wqTTONVo-", "[Gmail]/\u{C2A4}\u{D338}\u{D568}"),
            (
                "[Gmail]/&vPSwuNO4ycDVaA-",
                "[Gmail]/\u{BCF4}\u{B0B8}\u{D3B8}\u{C9C0}\u{D568}",
            ),
            // RFC 3501 §5.1.3.
            (
                "~peter/mail/&U,BTFw-/&ZeVnLIqe-",
                "~peter/mail/\u{53F0}\u{5317}/\u{65E5}\u{672C}\u{8A9E}",
            ),
            ("Tom &- Jerry", "Tom & Jerry"),
            ("INBOX", "INBOX"),
            ("&2D3eAA-", "\u{1F600}"),
        ] {
            assert_eq!(decode(wire).as_deref(), Some(name), "{wire:?}");
        }
    }

    #[test]
    fn invalid_names_do_not_decode() {
        for wire in [
            "&yATMtLz0rQDVaA",  // unterminated
            "&yAT!-",           // not modified base64
            "&yA/-",            // "/" is not modified base64
            "&yA-",             // an odd number of bytes
            "&2D0-",            // a lone surrogate
            "tab\there",        // not printable ASCII
            "\u{D55C}\u{AE00}", // raw 8-bit
        ] {
            assert_eq!(decode(wire), None, "{wire:?}");
        }
    }
}
