//! Just enough of the mail header format (RFC 5322) and encoded-words (RFC 2047) to read a subject.

use std::borrow::Cow;

use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use encoding_rs::Encoding;

const BASE64: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

/// The first field named `name` in a header block, without regard to case: unfolded, trimmed and
/// with its encoded-words decoded.
pub(super) fn field(name: &str, block: &[u8]) -> Option<String> {
    raw_field(name, block).map(|value| decode_words(&value))
}

/// A header value as Gmail's API hands it over: trimmed and decoded.
pub(super) fn value(raw: &str) -> String {
    decode_words(raw.trim_matches(crate::is_blank))
}

fn raw_field(name: &str, block: &[u8]) -> Option<String> {
    // Header bytes should be ASCII, but raw 8-bit UTF-8 subjects occur; Latin-1 never fails.
    let text: Cow<str> = match std::str::from_utf8(block) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => Cow::Owned(block.iter().map(|byte| char::from(*byte)).collect()),
    };
    let matches = |field: &(String, String)| field.0.eq_ignore_ascii_case(name);
    let mut current: Option<(String, String)> = None;
    for line in lines(&text) {
        if line.is_empty() {
            break;
        }
        if line.starts_with([' ', '\t']) {
            // A folded line: unfolding removes only the line break (RFC 5322 §2.2.3).
            if let Some((_, value)) = &mut current {
                value.push_str(line);
            }
            continue;
        }
        if let Some(field) = current.as_ref().filter(|field| matches(field)) {
            return Some(field.1.trim_matches(crate::is_blank).to_owned());
        }
        current = line.split_once(':').map(|(field_name, value)| {
            (
                field_name.trim_matches(crate::is_blank).to_owned(),
                value.to_owned(),
            )
        });
    }
    current
        .filter(matches)
        .map(|field| field.1.trim_matches(crate::is_blank).to_owned())
}

/// Lines split at CRLF, LF or CR.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let current = rest?;
        match current.find(['\r', '\n']) {
            Some(end) => {
                let skip = if current[end..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
                rest = Some(&current[end + skip..]);
                Some(&current[..end])
            }
            None => {
                rest = None;
                Some(current)
            }
        }
    })
}

/// Decodes `=?charset?B|Q?text?=` words, only when `=?` occurs. Whitespace between adjacent words
/// is dropped, and adjacent words in the same charset are joined before conversion, so a character
/// split across words still decodes. Malformed words and unknown charsets stay as written.
fn decode_words(text: &str) -> String {
    if !text.contains("=?") {
        return text.to_owned();
    }
    let bytes = text.as_bytes();
    let mut output = String::new();
    let mut literal_start = 0;
    let mut pending: Option<Run> = None;
    let mut index = 0;
    while index + 1 < bytes.len() {
        let word = if bytes[index] == b'=' && bytes[index + 1] == b'?' {
            parse_word(bytes, index)
        } else {
            None
        };
        let Some(word) = word else {
            index += 1;
            continue;
        };
        let end = word.end;
        let gap = &text[literal_start..index];
        let gap_is_whitespace = gap
            .bytes()
            .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'));
        match pending.take() {
            Some(mut run) if gap_is_whitespace => {
                if run.encoding == word.encoding {
                    run.decoded.extend_from_slice(&word.decoded);
                    run.end = word.end;
                    pending = Some(run);
                } else {
                    output.push_str(&run.rendered(text));
                    pending = Some(word);
                }
            }
            run => {
                if let Some(run) = run {
                    output.push_str(&run.rendered(text));
                }
                output.push_str(gap);
                pending = Some(word);
            }
        }
        index = end;
        literal_start = end;
    }
    if let Some(run) = pending {
        output.push_str(&run.rendered(text));
    }
    output.push_str(&text[literal_start..]);
    output
}

/// One or more adjacent encoded-words in the same charset.
struct Run {
    start: usize,
    end: usize,
    encoding: &'static Encoding,
    decoded: Vec<u8>,
}

impl Run {
    /// The words as written when their bytes are not valid in the charset.
    fn rendered<'a>(&self, source: &'a str) -> Cow<'a, str> {
        match self
            .encoding
            .decode_without_bom_handling_and_without_replacement(&self.decoded)
        {
            Some(decoded) => Cow::Owned(decoded.into_owned()),
            None => Cow::Borrowed(&source[self.start..self.end]),
        }
    }
}

fn parse_word(bytes: &[u8], start: usize) -> Option<Run> {
    let mut index = start + 2;
    let charset_start = index;
    while index < bytes.len() && bytes[index] != b'?' {
        if !(0x21..=0x7E).contains(&bytes[index]) {
            return None;
        }
        index += 1;
    }
    if index >= bytes.len() || index == charset_start {
        return None;
    }
    let charset = &bytes[charset_start..index];
    // An RFC 2231 language suffix, such as `UTF-8*ko`.
    let charset = charset
        .iter()
        .position(|byte| *byte == b'*')
        .map_or(charset, |star| &charset[..star]);
    index += 1;
    if index + 1 >= bytes.len() || bytes[index + 1] != b'?' {
        return None;
    }
    let method = bytes[index].to_ascii_lowercase();
    if method != b'b' && method != b'q' {
        return None;
    }
    index += 2;
    let text_start = index;
    while index + 1 < bytes.len() && !(bytes[index] == b'?' && bytes[index + 1] == b'=') {
        if !(0x21..=0x7E).contains(&bytes[index]) || bytes[index] == b'?' {
            return None;
        }
        index += 1;
    }
    if index + 1 >= bytes.len() {
        return None;
    }
    let encoded = &bytes[text_start..index];
    let encoding = encoding(charset)?;
    let decoded = if method == b'b' {
        decode_b(encoded)?
    } else {
        decode_q(encoded)?
    };
    Some(Run {
        start,
        end: index + 2,
        encoding,
        decoded,
    })
}

/// The encoding a MIME charset name stands for, by the WHATWG labels; CP949, a common name for
/// Korean that the labels lack, is EUC-KR, which the WHATWG defines as Windows-949.
fn encoding(charset: &[u8]) -> Option<&'static Encoding> {
    if charset.eq_ignore_ascii_case(b"cp949") {
        return Some(encoding_rs::EUC_KR);
    }
    Encoding::for_label_no_replacement(charset)
}

fn decode_b(text: &[u8]) -> Option<Vec<u8>> {
    if text.len() % 4 == 1 {
        return None;
    }
    let mut padded = text.to_vec();
    padded.resize(text.len().div_ceil(4) * 4, b'=');
    BASE64.decode(padded).ok()
}

fn decode_q(text: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        match text[index] {
            b'_' => {
                decoded.push(b' ');
                index += 1;
            }
            b'=' => {
                let high = hex_value(*text.get(index + 1)?)?;
                let low = hex_value(*text.get(index + 2)?)?;
                decoded.push(high << 4 | low);
                index += 3;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    Some(decoded)
}

fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .map(|digit| u8::try_from(digit).expect("a hex digit"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(block: &str) -> Option<String> {
        field("Subject", block.as_bytes())
    }

    const VERIFICATION_CODE: &str = "\u{C778}\u{C99D}\u{CF54}\u{B4DC}";

    #[test]
    fn plain_subject_with_date() {
        assert_eq!(
            subject("Subject: AuthCode: 482913\r\nDate: Wed, 14 Oct 2026 10:19:00 +0900\r\n\r\n")
                .as_deref(),
            Some("AuthCode: 482913")
        );
        assert_eq!(
            field("date", b"Subject: x\r\nDATE: Wed, 14 Oct 2026\r\n\r\n").as_deref(),
            Some("Wed, 14 Oct 2026")
        );
    }

    #[test]
    fn field_names_ignore_case_and_the_first_wins() {
        assert_eq!(
            subject("SUBJECT:AuthCode: 1\r\nsubject: second\r\n").as_deref(),
            Some("AuthCode: 1")
        );
        assert_eq!(
            subject("Date: x\r\nsubject :  spaced  \r\n").as_deref(),
            Some("spaced")
        );
    }

    #[test]
    fn folded_subjects_are_unfolded() {
        assert_eq!(
            subject("Subject: Your one-time\r\n\tpassword\r\n AuthCode: 123456\r\nDate: x\r\n")
                .as_deref(),
            Some("Your one-time\tpassword AuthCode: 123456")
        );
        assert_eq!(
            subject("Date: x\nSubject: bare\n  LF\n\n").as_deref(),
            Some("bare  LF")
        );
    }

    #[test]
    fn missing_subject_and_header_end() {
        assert_eq!(
            subject("Date: Wed, 14 Oct 2026 10:19:00 +0900\r\n\r\n"),
            None
        );
        assert_eq!(subject(""), None);
        assert_eq!(subject("Date: x\r\n\r\nSubject: in the body\r\n"), None);
        assert_eq!(subject("X-Subject: no\r\n"), None);
    }

    #[test]
    fn raw_eight_bit_subjects() {
        assert_eq!(
            subject(&format!(
                "Subject: {VERIFICATION_CODE} AuthCode: 123456\r\n"
            )),
            Some(format!("{VERIFICATION_CODE} AuthCode: 123456"))
        );
        let latin1 = [
            b"Subject: caf".as_slice(),
            &[0xE9],
            b" AuthCode: 654321\r\n",
        ]
        .concat();
        assert_eq!(
            field("Subject", &latin1).as_deref(),
            Some("caf\u{E9} AuthCode: 654321")
        );
    }

    #[test]
    fn encoded_words_are_decoded() {
        let cases = [
            (
                "=?UTF-8?B?7J247Kad7L2U65OcIEF1dGhDb2RlOiA0ODI5MTM=?=",
                format!("{VERIFICATION_CODE} AuthCode: 482913"),
            ),
            // Lowercase, without padding.
            (
                "=?utf-8?b?7J247Kad7L2U65OcIEF1dGhDb2RlOiA0ODI5MTM?=",
                format!("{VERIFICATION_CODE} AuthCode: 482913"),
            ),
            ("=?UTF-8?Q?AuthCode:_123456?=", "AuthCode: 123456".into()),
            (
                "=?UTF-8?Q?=EC=9D=B8=EC=A6=9D_AuthCode:_123456?=",
                "\u{C778}\u{C99D} AuthCode: 123456".into(),
            ),
            ("=?utf-8?q?=ec=9d=b8?=", "\u{C778}".into()),
            (
                "=?EUC-KR?B?wM7B9cTateUgQXV0aENvZGU6IDQ4MjkxMw==?=",
                format!("{VERIFICATION_CODE} AuthCode: 482913"),
            ),
            (
                "=?euc-kr?B?wM7B9cTateUgQXV0aENvZGU6IDQ4MjkxMw==?=",
                format!("{VERIFICATION_CODE} AuthCode: 482913"),
            ),
            (
                "=?ks_c_5601-1987?B?wM7B9cTateUgQXV0aENvZGU6IDQ4MjkxMw==?=",
                format!("{VERIFICATION_CODE} AuthCode: 482913"),
            ),
            (
                "=?ISO-8859-1?Q?caf=E9?= AuthCode: 1",
                "caf\u{E9} AuthCode: 1".into(),
            ),
            // An RFC 2231 language suffix.
            ("=?UTF-8*ko?B?7J247Kad?=", "\u{C778}\u{C99D}".into()),
            // Whitespace, folding included, between adjacent words is dropped.
            (
                "=?UTF-8?Q?AuthCode:?= =?UTF-8?Q?_123456?=",
                "AuthCode: 123456".into(),
            ),
            (
                "=?UTF-8?Q?Auth?=\r\n =?UTF-8?Q?Code:_1?=",
                "AuthCode: 1".into(),
            ),
            // A character split across adjacent words still decodes.
            (
                "=?UTF-8?B?7J247A==?= =?UTF-8?B?pp0=?=",
                "\u{C778}\u{C99D}".into(),
            ),
            // Adjacent words in different charsets.
            (
                "=?EUC-KR?B?wM4=?= =?UTF-8?B?7Kad?=",
                "\u{C778}\u{C99D}".into(),
            ),
            // Whitespace between a word and plain text stays.
            ("=?UTF-8?Q?AuthCode:?= 123456", "AuthCode: 123456".into()),
            (
                "Re: =?UTF-8?B?7J247Kad?= AuthCode: 1",
                "Re: \u{C778}\u{C99D} AuthCode: 1".into(),
            ),
            // Lenient: a word glued to text.
            ("AuthCode:=?UTF-8?Q?_777888?=", "AuthCode: 777888".into()),
            ("=?UTF-8?Q??=", String::new()),
        ];
        for (encoded, decoded) in cases {
            assert_eq!(decode_words(encoded), decoded, "{encoded:?}");
        }
    }

    #[test]
    fn malformed_words_stay_as_written() {
        for text in [
            "AuthCode: 123456",
            "100% =? sure",
            "=?UTF-8?B?7J247Kad",           // unterminated
            "=?x-unknown-charset?B?QUJD?=", // unknown charset
            "=?UTF-8?X?QUJD?=",             // unknown encoding
            "=?UTF-8?Q?bad=ZZ?=",           // bad hex
            "=?UTF-8?Q?a=E?=",              // truncated hex
            "=?UTF-8?B?Q?=",                // impossible base64 length
            "=?UTF-8?B?/w==?=",             // decodes to invalid UTF-8
            "=?UTF-8?Q?has space?=",        // spaces are not allowed inside
            "=??B?QUJD?=",                  // empty charset
            "=?ISO-2022-KR?B?QUJD?=",       // a charset the WHATWG replaces wholesale
        ] {
            assert_eq!(decode_words(text), text, "{text:?}");
        }
    }

    #[test]
    fn encoded_subject_inside_a_header_block() {
        let block = "Subject: =?UTF-8?B?7J247Kad7L2U65OcIA==?=\r\n =?UTF-8?B?QXV0aENvZGU6IDQ4MjkxMw==?=\r\nDate: x\r\n\r\n";
        assert_eq!(
            subject(block),
            Some(format!("{VERIFICATION_CODE} AuthCode: 482913"))
        );
    }

    #[test]
    fn known_charsets() {
        for charset in [
            "UTF-8",
            "utf-8",
            "utf8",
            "EUC-KR",
            "euc-kr",
            "ISO-8859-1",
            "us-ascii",
            "ks_c_5601-1987",
            "CP949",
            "windows-1252",
        ] {
            assert!(encoding(charset.as_bytes()).is_some(), "{charset}");
        }
        assert_eq!(encoding(b"UTF-8"), Some(encoding_rs::UTF_8));
        assert_eq!(encoding(b"x-no-such-charset"), None);
    }
}
