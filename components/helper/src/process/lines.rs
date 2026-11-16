//! Turning one output stream of the client into Log tab lines.

use std::time::{Duration, Instant};

/// Longer lines are split, so output without line breaks still reaches the Log tab.
pub(crate) const MAX_LINE_BYTES: usize = 16_384;
const LF: u8 = b'\n';
const CR: u8 = b'\r';

/// Splits one output stream into display lines.
///
/// - Lines end at LF; one trailing CR is dropped.
/// - Bytes without a line break, such as a prompt, are shown once nothing arrived for the idle
///   period. The client may print the LF that ends such a line much later, together with its next
///   line, so an LF that only ends an already shown part adds no empty line, and text that continues
///   after a shown part is a line of its own.
/// - Lines longer than `MAX_LINE_BYTES` are split, never inside a UTF-8 scalar.
/// - Invalid UTF-8 becomes U+FFFD, and control characters other than TAB are shown in caret notation.
pub(crate) struct LineAssembler {
    idle_flush: Duration,
    /// Bytes of the current line not shown yet.
    pending: Vec<u8>,
    /// Part of the current line was already shown, by an idle flush or a split.
    shown_part: bool,
    /// When the last bytes arrived.
    last_input: Option<Instant>,
}

impl LineAssembler {
    pub(crate) fn new(idle_flush: Duration) -> Self {
        Self {
            idle_flush,
            pending: Vec::new(),
            shown_part: false,
            last_input: None,
        }
    }

    /// When `flush_if_idle` will show the unfinished line; `None` while it has nothing it could
    /// show, so a timer never spins on held-back bytes.
    pub(crate) fn idle_deadline(&self) -> Option<Instant> {
        let last_input = self.last_input?;
        (self.pending.len() > held_back(&self.pending)).then(|| last_input + self.idle_flush)
    }

    /// Feeds bytes read at `now`. An unfinished line whose idle period already passed is shown
    /// first, so the result does not depend on how promptly the caller's timer fired.
    pub(crate) fn append(&mut self, bytes: &[u8], now: Instant) -> Vec<String> {
        let mut lines: Vec<String> = self.flush_if_idle(now).into_iter().collect();
        if bytes.is_empty() {
            return lines;
        }
        self.last_input = Some(now);
        let mut pieces = bytes.split(|&byte| byte == LF).peekable();
        while let Some(piece) = pieces.next() {
            self.extend_line(piece, &mut lines);
            // Every piece but the last ended at an LF.
            if pieces.peek().is_some() {
                lines.extend(self.end_line());
            }
        }
        lines
    }

    /// Shows the unfinished line if nothing arrived for the idle period. A trailing CR or an
    /// incomplete UTF-8 sequence is held back, so a following LF or the rest of the scalar still
    /// pairs up with it.
    pub(crate) fn flush_if_idle(&mut self, now: Instant) -> Option<String> {
        let last_input = self.last_input?;
        if self.pending.is_empty() || now.duration_since(last_input) < self.idle_flush {
            return None;
        }
        let shown = self.pending.len() - held_back(&self.pending);
        if shown == 0 {
            return None;
        }
        let text = render(&self.pending[..shown]);
        self.pending.drain(..shown);
        self.shown_part = true;
        Some(text)
    }

    /// Shows what is left at the end of the stream.
    pub(crate) fn finish(&mut self) -> Option<String> {
        let content = without_trailing_cr(&self.pending);
        let text = (!content.is_empty()).then(|| render(content));
        self.pending.clear();
        self.shown_part = false;
        self.last_input = None;
        text
    }

    /// Adds bytes that hold no LF, splitting whenever the line would grow too long.
    fn extend_line(&mut self, bytes: &[u8], lines: &mut Vec<String>) {
        let mut index = 0;
        while index < bytes.len() {
            if self.pending.len() < MAX_LINE_BYTES {
                let end = bytes.len().min(index + MAX_LINE_BYTES - self.pending.len());
                self.pending.extend_from_slice(&bytes[index..end]);
                index = end;
                continue;
            }
            let byte = bytes[index];
            // A CR right at the limit may be the CR of a CRLF, so it may overhang by one.
            let may_overhang = byte == CR && self.pending.len() == MAX_LINE_BYTES;
            if !may_overhang {
                lines.push(self.split_before(byte));
            }
            self.pending.push(byte);
            index += 1;
        }
    }

    fn end_line(&mut self) -> Option<String> {
        let content = without_trailing_cr(&self.pending);
        let line = (!content.is_empty() || !self.shown_part).then(|| render(content));
        self.pending.clear();
        self.shown_part = false;
        line
    }

    /// Shows the first `MAX_LINE_BYTES` of `pending` because `next` would make the line too long.
    /// The cut moves back to the start of a scalar that continues past the limit.
    fn split_before(&mut self, next: u8) -> String {
        let mut cut = MAX_LINE_BYTES;
        let after_cut = self.pending.get(cut).copied().unwrap_or(next);
        if is_continuation(after_cut) {
            let lowest = cut.saturating_sub(3).max(1);
            let mut index = cut - 1;
            while index >= lowest && is_continuation(self.pending[index]) {
                index -= 1;
            }
            if index >= lowest && self.pending[index] >= 0xC0 {
                cut = index;
            }
        }
        let text = render(&self.pending[..cut]);
        self.pending.drain(..cut);
        self.shown_part = true;
        text
    }
}

fn without_trailing_cr(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(&[CR]).unwrap_or(bytes)
}

fn is_continuation(byte: u8) -> bool {
    byte & 0xC0 == 0x80
}

/// How many trailing bytes an idle flush keeps back: a CR, or an incomplete UTF-8 sequence.
fn held_back(bytes: &[u8]) -> usize {
    let Some(&last) = bytes.last() else {
        return 0;
    };
    if last == CR {
        return 1;
    }
    let lowest = bytes.len().saturating_sub(4);
    let mut index = bytes.len() - 1;
    while index > lowest && is_continuation(bytes[index]) {
        index -= 1;
    }
    let expected = match bytes[index] {
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => 1,
    };
    let available = bytes.len() - index;
    if available < expected { available } else { 0 }
}

/// Decodes UTF-8, with U+FFFD for invalid bytes, and makes control characters visible: `^[`,
/// `^@`, `^?`, and `M-^E` for the C1 controls.
fn render(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut shown = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\t' => shown.push(character),
            '\0'..='\x1f' => {
                shown.push('^');
                shown.push(char::from(character as u8 + 0x40));
            }
            '\x7f' => shown.push_str("^?"),
            '\u{80}'..='\u{9f}' => {
                shown.push_str("M-^");
                shown.push(char::from(character as u8 - 0x80 + 0x40));
            }
            _ => shown.push(character),
        }
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: usize = MAX_LINE_BYTES;

    struct Clock(Instant);

    impl Clock {
        fn at(&self, milliseconds: u64) -> Instant {
            self.0 + Duration::from_millis(milliseconds)
        }
    }

    fn setup() -> (LineAssembler, Clock) {
        (
            LineAssembler::new(Duration::from_millis(250)),
            Clock(Instant::now()),
        )
    }

    fn whole(bytes: &[u8]) -> Vec<String> {
        let (mut lines, clock) = setup();
        lines.append(bytes, clock.at(0))
    }

    #[test]
    fn splits_at_lf_and_drops_one_trailing_cr() {
        assert_eq!(whole(b"a\nb\r\nc\r\r\n"), ["a", "b", "c^M"]);
    }

    #[test]
    fn keeps_empty_lines_after_a_complete_line() {
        assert_eq!(whole(b"x\n\n\r\n"), ["x", "", ""]);
    }

    #[test]
    fn joins_a_line_split_across_reads() {
        let (mut lines, clock) = setup();
        assert!(lines.append(b"INFO:   Conn", clock.at(0)).is_empty());
        assert!(lines.append(b"ected to gate", clock.at(10)).is_empty());
        assert_eq!(
            lines.append(b"way.\nINFO:", clock.at(20)),
            ["INFO:   Connected to gateway."]
        );
        assert_eq!(
            lines.append(b"   Authenticated.\n", clock.at(30)),
            ["INFO:   Authenticated."]
        );
    }

    #[test]
    fn shows_an_unfinished_line_only_after_the_idle_period() {
        let (mut lines, clock) = setup();
        assert!(
            lines
                .append(b"VPN account password: ", clock.at(0))
                .is_empty()
        );
        assert_eq!(lines.idle_deadline(), Some(clock.at(250)));
        assert_eq!(lines.flush_if_idle(clock.at(249)), None);
        assert_eq!(
            lines.flush_if_idle(clock.at(250)).as_deref(),
            Some("VPN account password: ")
        );
        assert_eq!(lines.idle_deadline(), None);
        assert_eq!(lines.flush_if_idle(clock.at(5_000)), None);
    }

    #[test]
    fn counts_the_idle_period_from_the_last_bytes() {
        let (mut lines, clock) = setup();
        lines.append(b"ab", clock.at(0));
        lines.append(b"c", clock.at(200));
        assert_eq!(lines.flush_if_idle(clock.at(300)), None);
        // An empty read is no activity.
        lines.append(b"", clock.at(400));
        assert_eq!(lines.idle_deadline(), Some(clock.at(450)));
        assert_eq!(lines.flush_if_idle(clock.at(450)).as_deref(), Some("abc"));
    }

    #[test]
    fn adds_no_empty_line_for_the_lf_that_ends_a_shown_prompt() {
        let (mut lines, clock) = setup();
        lines.append(b"VPN account password: ", clock.at(0));
        assert_eq!(
            lines.flush_if_idle(clock.at(300)).as_deref(),
            Some("VPN account password: ")
        );
        assert_eq!(
            lines.append(b"\nINFO:   Connected to gateway.\n", clock.at(900)),
            ["INFO:   Connected to gateway."]
        );
    }

    #[test]
    fn adds_no_empty_line_for_a_lone_lf_or_crlf_after_a_shown_part() {
        let (mut lines, clock) = setup();
        lines.append(b"Two-factor authentication token: ", clock.at(0));
        lines.flush_if_idle(clock.at(250));
        assert!(lines.append(b"\n", clock.at(400)).is_empty());
        assert_eq!(
            lines.append(b"INFO:   Authenticated.\n", clock.at(401)),
            ["INFO:   Authenticated."]
        );
        lines.append(b"prompt: ", clock.at(500));
        lines.flush_if_idle(clock.at(750));
        assert!(lines.append(b"\r\n", clock.at(800)).is_empty());
    }

    #[test]
    fn keeps_the_empty_line_after_a_prompt_answered_within_the_idle_period() {
        let (mut lines, clock) = setup();
        lines.append(b"VPN account password: ", clock.at(0));
        assert_eq!(lines.flush_if_idle(clock.at(100)), None);
        assert_eq!(
            lines.append(b"\nINFO:   Connected to gateway.\n\n", clock.at(120)),
            [
                "VPN account password: ",
                "INFO:   Connected to gateway.",
                ""
            ]
        );
    }

    #[test]
    fn shows_text_continuing_after_a_shown_part_as_its_own_line() {
        let (mut lines, clock) = setup();
        lines.append(b"abc", clock.at(0));
        assert_eq!(lines.flush_if_idle(clock.at(250)).as_deref(), Some("abc"));
        assert_eq!(lines.append(b"def\nghi\n", clock.at(260)), ["def", "ghi"]);
        lines.append(b"Fri Apr  5 11:31:24 2024 : ", clock.at(300));
        assert_eq!(
            lines.flush_if_idle(clock.at(550)).as_deref(),
            Some("Fri Apr  5 11:31:24 2024 : ")
        );
        lines.append(b"Modem hangup", clock.at(900));
        assert_eq!(
            lines.flush_if_idle(clock.at(1_150)).as_deref(),
            Some("Modem hangup")
        );
        assert!(lines.append(b"\n", clock.at(1_500)).is_empty());
    }

    #[test]
    fn shows_an_overdue_line_before_new_bytes() {
        let (mut lines, clock) = setup();
        lines.append(b"Two-factor authentication token: ", clock.at(0));
        assert_eq!(
            lines.append(b"\nINFO:   Authenticated.\n", clock.at(20_000)),
            [
                "Two-factor authentication token: ",
                "INFO:   Authenticated."
            ]
        );
    }

    #[test]
    fn holds_back_a_trailing_cr() {
        let (mut lines, clock) = setup();
        lines.append(b"abc\r", clock.at(0));
        assert_eq!(lines.flush_if_idle(clock.at(250)).as_deref(), Some("abc"));
        assert_eq!(lines.idle_deadline(), None);
        assert_eq!(lines.flush_if_idle(clock.at(500)), None);
        assert!(lines.append(b"\n", clock.at(600)).is_empty());

        lines.append(b"50%\r", clock.at(700));
        lines.flush_if_idle(clock.at(950));
        assert_eq!(lines.append(b"100%\n", clock.at(1_000)), ["^M100%"]);
    }

    #[test]
    fn holds_back_an_incomplete_utf8_sequence() {
        let (mut lines, clock) = setup();
        // "ab" and two of the three bytes of "€".
        lines.append(&[0x61, 0x62, 0xE2, 0x82], clock.at(0));
        assert_eq!(lines.idle_deadline(), Some(clock.at(250)));
        assert_eq!(lines.flush_if_idle(clock.at(250)).as_deref(), Some("ab"));
        assert_eq!(lines.idle_deadline(), None);
        assert_eq!(lines.append(&[0xAC, 0x0A], clock.at(300)), ["€"]);

        lines.append(&[0xF0, 0x9F], clock.at(400));
        assert_eq!(lines.idle_deadline(), None);
        assert_eq!(lines.flush_if_idle(clock.at(1_400)), None);
        assert_eq!(lines.finish().as_deref(), Some("\u{FFFD}"));
    }

    #[test]
    fn splits_long_lines() {
        let first = "a".repeat(LIMIT);
        let second = "b".repeat(LIMIT);
        assert_eq!(
            whole(format!("{first}{second}GHIJ\n").as_bytes()),
            [first, second, "GHIJ".to_owned()]
        );
        let lengths: Vec<usize> = whole(&[[b'x'; 40_000].as_slice(), b"\n"].concat())
            .iter()
            .map(String::len)
            .collect();
        assert_eq!(lengths, [16_384, 16_384, 7_232]);
    }

    #[test]
    fn does_not_split_a_line_of_exactly_the_limit() {
        let line = "x".repeat(LIMIT);
        let (mut lines, clock) = setup();
        assert_eq!(
            lines.append(format!("{line}\n").as_bytes(), clock.at(0)),
            [line.as_str()]
        );
        assert_eq!(
            lines.append(format!("{line}\r\n").as_bytes(), clock.at(0)),
            [line.as_str()]
        );
        assert_eq!(
            lines.append(format!("{line}\rX\n").as_bytes(), clock.at(0)),
            [line.as_str(), "^MX"]
        );
    }

    #[test]
    fn adds_no_empty_line_after_a_split_that_took_the_whole_line() {
        let (a, b, c) = ("a".repeat(LIMIT), "b".repeat(LIMIT), "c".repeat(LIMIT));
        let (mut lines, clock) = setup();
        assert_eq!(lines.append(format!("{a}{b}").as_bytes(), clock.at(0)), [a]);
        assert_eq!(lines.append(b"8\n", clock.at(0)), [b, "8".to_owned()]);
        assert_eq!(lines.append(format!("{c}e").as_bytes(), clock.at(0)), [c]);
        assert_eq!(lines.flush_if_idle(clock.at(300)).as_deref(), Some("e"));
        assert!(lines.append(b"\n", clock.at(301)).is_empty());
    }

    #[test]
    fn splits_between_utf8_scalars() {
        let text = "a\u{AC00}\u{B098}😀b€\u{B2E4}cd\u{B77C}\u{B9C8}😀😀\u{BC14}";
        for bytes_before_limit in 0..=text.len() {
            let padding = "x".repeat(LIMIT - bytes_before_limit);
            let (mut lines, clock) = setup();
            let mut shown = lines.append(padding.as_bytes(), clock.at(0));
            for &byte in text.as_bytes().iter().chain(b"\n") {
                shown.extend(lines.append(&[byte], clock.at(0)));
            }
            assert_eq!(shown.concat(), format!("{padding}{text}"));
            assert!(
                shown
                    .iter()
                    .all(|line| !line.contains('\u{FFFD}') && line.len() <= LIMIT)
            );
        }
    }

    #[test]
    fn replaces_invalid_utf8() {
        assert_eq!(whole(&[0x61, 0xFF, 0x62, 0x0A]), ["a\u{FFFD}b"]);
        assert_eq!(whole(&[0xC3, 0x28, 0x0A]), ["\u{FFFD}("]);
        // A UTF-16 surrogate written as UTF-8.
        assert_eq!(
            whole(&[0xED, 0xA0, 0x80, 0x0A]),
            ["\u{FFFD}\u{FFFD}\u{FFFD}"]
        );
        assert_eq!(
            whole("\u{D55C}\u{AE00} OK\n".as_bytes()),
            ["\u{D55C}\u{AE00} OK"]
        );
    }

    #[test]
    fn shows_control_characters_in_caret_notation() {
        for (raw, shown) in [
            (
                "\u{1B}[0;31mERROR:  x\u{1B}[0;0m",
                "^[[0;31mERROR:  x^[[0;0m",
            ),
            ("a\u{0}b", "a^@b"),
            ("del\u{7F}", "del^?"),
            ("bell\u{7}", "bell^G"),
            ("us\u{1F}", "us^_"),
            ("mid\rcr", "mid^Mcr"),
            ("tab\tkept", "tab\tkept"),
            ("nel\u{85}c1\u{9B}", "nelM-^Ec1M-^["),
            ("plain text: é ü 中", "plain text: é ü 中"),
        ] {
            assert_eq!(whole(format!("{raw}\n").as_bytes()), [shown], "{raw:?}");
        }
    }

    #[test]
    fn finish_shows_the_rest_once() {
        let (mut lines, clock) = setup();
        lines.append(b"done\nINFO:   Logged", clock.at(0));
        assert_eq!(lines.finish().as_deref(), Some("INFO:   Logged"));
        assert_eq!(lines.finish(), None);

        lines.append(b"abc\r", clock.at(0));
        assert_eq!(lines.finish().as_deref(), Some("abc"));
        lines.append(b"\r", clock.at(0));
        assert_eq!(lines.finish(), None);

        lines.append(b"Two-factor authentication token: ", clock.at(0));
        lines.flush_if_idle(clock.at(250));
        assert_eq!(lines.finish(), None);
        assert_eq!(lines.idle_deadline(), None);
    }
}
