//! The file as text: its size, its encoding, the raw characters it may hold, and positions in it.

use fvm_contract::app_internal::problem::{ConfigProblem, ConfigProblemKind, LineColumn};

use super::MAX_FILE_BYTES;

/// Checks what applies to the whole file, and returns its text without a leading byte order mark.
pub(super) fn check(bytes: &[u8]) -> Result<&str, ConfigProblem> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ConfigProblem {
            kind: ConfigProblemKind::FileTooLarge {
                byte_count: bytes.len() as u64,
                limit: MAX_FILE_BYTES as u64,
            },
            location: None,
        });
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => without_bom(text),
        Err(error) => {
            let valid = std::str::from_utf8(&bytes[..error.valid_up_to()])
                .expect("the bytes before the first invalid one are UTF-8");
            let mut position = Position::default();
            without_bom(valid).chars().for_each(|c| position.advance(c));
            return Err(ConfigProblem {
                kind: ConfigProblemKind::InvalidUtf8 {},
                location: Some(position.location()),
            });
        }
    };
    let mut position = Position::default();
    for character in text.chars() {
        if !readable(character) {
            return Err(ConfigProblem {
                kind: ConfigProblemKind::DisallowedCharacter {
                    scalar: u32::from(character),
                },
                location: Some(position.location()),
            });
        }
        position.advance(character);
    }
    Ok(text)
}

/// The first character a string value may not hold: a control character, a line or paragraph
/// separator, a byte order mark or a noncharacter.
pub(super) fn disallowed_in_value(value: &str) -> Option<char> {
    value.chars().find(|&c| {
        let scalar = u32::from(c);
        c.is_control()
            || matches!(c, '\u{2028}' | '\u{2029}' | '\u{FEFF}')
            || (0xFDD0..=0xFDEF).contains(&scalar)
            || scalar & 0xFFFE == 0xFFFE
    })
}

/// 1-based, counting Unicode scalars.
pub(super) fn location(marker: &saphyr_parser::Marker) -> LineColumn {
    LineColumn {
        line: marker.line() as u32,
        column: marker.col() as u32 + 1,
    }
}

fn without_bom(text: &str) -> &str {
    text.strip_prefix('\u{FEFF}').unwrap_or(text)
}

/// The characters libyaml reads, the parser of the earlier app, less NEL, LS and PS: YAML 1.1 reads
/// those as line breaks and YAML 1.2 as text, so a file holding them raw could be read two ways
/// (REFACTOR-PLAN §2, exception 1).
fn readable(character: char) -> bool {
    matches!(
        u32::from(character),
        0x09 | 0x0A | 0x0D | 0x20..=0x7E | 0xA0..=0x2027 | 0x202A..=0xD7FF | 0xE000..=0xFFFD | 0x10000..
    )
}

/// Counts lines the way the parser does: CR, LF and CRLF each end a line. NEL, LS and PS end one
/// too, as in the earlier app, which matters only for an invalid byte after one of them.
#[derive(Default)]
struct Position {
    line: u32,
    column: u32,
    after_cr: bool,
}

impl Position {
    fn location(&self) -> LineColumn {
        LineColumn {
            line: self.line + 1,
            column: self.column + 1,
        }
    }

    fn advance(&mut self, character: char) {
        match character {
            '\n' if self.after_cr => {}
            '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}' => {
                self.line += 1;
                self.column = 0;
            }
            _ => self.column += 1,
        }
        self.after_cr = character == '\r';
    }
}
