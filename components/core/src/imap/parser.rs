use super::{FetchResponse, ListEntry, Response, ResponseCode, Status, date};

const MAX_LITERAL_BYTES: u64 = 1 << 20;
const MAX_RESPONSE_BYTES: usize = 4 << 20;
/// Deeper parenthesized lists make a response `Other`, so the grammar's recursion stays bounded.
const MAX_NESTING: usize = 32;

/// A framing failure; the parser stays failed and the connection is dropped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParseError {
    /// A literal (`{n}`) announced more than 1 MiB, or a length too large to read.
    LiteralTooLarge { declared: u64 },
    /// One response, literals included, grew beyond 4 MiB.
    ResponseTooLarge,
}

/// Turns the server's bytes into responses, in any chunking: literals (`{n}` CRLF and n bytes) may
/// be split anywhere.
#[derive(Clone, Debug, Default)]
pub struct Parser {
    /// The response being assembled: lines and literal contents, line breaks included.
    buffer: Vec<u8>,
    /// Where the current line starts in `buffer`, just after the last literal's contents.
    line_start: usize,
    literal_remaining: usize,
    failure: Option<ParseError>,
}

impl Parser {
    /// Takes the bytes as they arrive and returns the responses they complete, in order.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Response>, ParseError> {
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        let mut responses = Vec::new();
        for &byte in bytes {
            if self.buffer.len() >= MAX_RESPONSE_BYTES {
                return Err(self.fail(ParseError::ResponseTooLarge));
            }
            self.buffer.push(byte);
            if self.literal_remaining > 0 {
                self.literal_remaining -= 1;
                if self.literal_remaining == 0 {
                    self.line_start = self.buffer.len();
                }
                continue;
            }
            if byte != b'\n' {
                continue;
            }
            let mut content_end = self.buffer.len() - 1;
            if content_end > self.line_start && self.buffer[content_end - 1] == b'\r' {
                content_end -= 1;
            }
            match literal_length(&self.buffer, self.line_start, content_end) {
                None => {
                    responses.push(decode(&self.buffer[..content_end]));
                    self.buffer.clear();
                    self.line_start = 0;
                }
                Some(declared) => {
                    if declared > MAX_LITERAL_BYTES {
                        return Err(self.fail(ParseError::LiteralTooLarge { declared }));
                    }
                    let declared = usize::try_from(declared).expect("at most 1 MiB");
                    if self.buffer.len() + declared > MAX_RESPONSE_BYTES {
                        return Err(self.fail(ParseError::ResponseTooLarge));
                    }
                    self.literal_remaining = declared;
                    if declared == 0 {
                        self.line_start = self.buffer.len();
                    }
                }
            }
        }
        Ok(responses)
    }

    fn fail(&mut self, error: ParseError) -> ParseError {
        self.failure = Some(error);
        self.buffer = Vec::new();
        error
    }
}

/// The literal length when the line ends with `{digits}` or `{digits+}`; saturates at `u64::MAX`.
/// A continuation request (`+ ...`) never announces one.
fn literal_length(buffer: &[u8], line_start: usize, content_end: usize) -> Option<u64> {
    let line = &buffer[line_start..content_end];
    let inner = line.strip_suffix(b"}")?;
    if line_start == 0 && line.first() == Some(&b'+') {
        return None;
    }
    let inner = inner.strip_suffix(b"+").unwrap_or(inner);
    let digit_count = inner
        .iter()
        .rev()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    let (before, digits) = inner.split_at(inner.len() - digit_count);
    if digit_count == 0 || before.last() != Some(&b'{') {
        return None;
    }
    Some(digits.iter().fold(0u64, |value, digit| {
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(digit - b'0')))
            .unwrap_or(u64::MAX)
    }))
}

/// One complete response, literal contents inline and the final line break removed. Never fails:
/// what the grammar does not recognize becomes `Other`.
fn decode(bytes: &[u8]) -> Response {
    let mut scanner = Scanner { bytes, position: 0 };
    response(&mut scanner).unwrap_or_else(|| Response::Other(lossy(bytes)))
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A data item: an atom, a string (quoted or literal), `NIL` or a parenthesized list.
enum Value {
    Atom(String),
    String(Vec<u8>),
    Nil,
    List(Vec<Value>),
}

impl Value {
    fn string_bytes(self) -> Option<Vec<u8>> {
        match self {
            Value::String(bytes) => Some(bytes),
            Value::Atom(atom) => Some(atom.into_bytes()),
            Value::Nil | Value::List(_) => None,
        }
    }
}

struct Scanner<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Scanner<'_> {
    fn is_at_end(&self) -> bool {
        self.position >= self.bytes.len()
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.position + offset).copied()
    }

    fn consume(&mut self, byte: u8) -> bool {
        let found = self.peek(0) == Some(byte);
        if found {
            self.position += 1;
        }
        found
    }

    /// Whether there was at least one space.
    fn consume_spaces(&mut self) -> bool {
        let start = self.position;
        while self.peek(0) == Some(b' ') {
            self.position += 1;
        }
        self.position > start
    }

    fn rest_text(&mut self) -> String {
        let rest = lossy(self.bytes.get(self.position..).unwrap_or_default());
        self.position = self.bytes.len();
        rest
    }

    /// A lenient atom. A `[` opens a section that runs to its matching `]` and may hold spaces and
    /// parentheses, so `BODY[HEADER.FIELDS (SUBJECT DATE)]` is one atom.
    fn atom(&mut self, allow_close_bracket: bool) -> Option<String> {
        let start = self.position;
        while let Some(byte) = self.peek(0) {
            if byte == b'[' {
                let Some(end) = self.matching_bracket(self.position) else {
                    break;
                };
                self.position = end + 1;
            } else if is_atom_byte(byte) || (allow_close_bracket && byte == b']') {
                self.position += 1;
            } else {
                break;
            }
        }
        (self.position > start).then(|| lossy(&self.bytes[start..self.position]))
    }

    /// The index of the `]` that matches the `[` at `index`, on the same line.
    fn matching_bracket(&self, index: usize) -> Option<usize> {
        let mut depth = 0usize;
        for (offset, byte) in self.bytes[index..].iter().enumerate() {
            match byte {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(index + offset);
                    }
                }
                b'\r' | b'\n' => return None,
                _ => {}
            }
        }
        None
    }

    /// A 32-bit unsigned number; `None` without digits or on overflow.
    fn number(&mut self) -> Option<u32> {
        let start = self.position;
        let mut value: u32 = 0;
        while let Some(byte) = self.peek(0).filter(u8::is_ascii_digit) {
            let Some(next) = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u32::from(byte - b'0')))
            else {
                self.position = start;
                return None;
            };
            value = next;
            self.position += 1;
        }
        (self.position > start).then_some(value)
    }

    /// A quoted string with `\` escapes, or a literal (`{n}` or `~{n}`, CRLF and n bytes).
    fn string(&mut self) -> Option<Vec<u8>> {
        let start = self.position;
        let string = self.string_from_here();
        if string.is_none() {
            self.position = start;
        }
        string
    }

    fn string_from_here(&mut self) -> Option<Vec<u8>> {
        if self.consume(b'"') {
            let mut string = Vec::new();
            while let Some(byte) = self.peek(0) {
                self.position += 1;
                match byte {
                    b'"' => return Some(string),
                    b'\\' => {
                        string.push(self.peek(0)?);
                        self.position += 1;
                    }
                    b'\r' | b'\n' => return None,
                    _ => string.push(byte),
                }
            }
            return None;
        }
        if self.peek(0) == Some(b'~') && self.peek(1) == Some(b'{') {
            self.position += 1;
        }
        if !self.consume(b'{') {
            return None;
        }
        let count = usize::try_from(self.number()?).expect("u32 fits in usize");
        self.consume(b'+');
        if !self.consume(b'}') {
            return None;
        }
        self.consume(b'\r');
        if !self.consume(b'\n') || self.bytes.len() - self.position < count {
            return None;
        }
        let contents = self.bytes[self.position..self.position + count].to_vec();
        self.position += count;
        Some(contents)
    }

    /// Any data item; `NIL` in any case is `Value::Nil`.
    fn value(&mut self, depth: usize) -> Option<Value> {
        match self.peek(0)? {
            b'(' => {
                if depth >= MAX_NESTING {
                    return None;
                }
                let start = self.position;
                self.position += 1;
                let mut items = Vec::new();
                loop {
                    self.consume_spaces();
                    if self.consume(b')') {
                        return Some(Value::List(items));
                    }
                    let Some(item) = self.value(depth + 1) else {
                        self.position = start;
                        return None;
                    };
                    items.push(item);
                }
            }
            b'"' | b'{' => self.string().map(Value::String),
            b'~' if self.peek(1) == Some(b'{') => self.string().map(Value::String),
            _ => {
                let atom = self.atom(true)?;
                Some(if atom.to_uppercase() == "NIL" {
                    Value::Nil
                } else {
                    Value::Atom(atom)
                })
            }
        }
    }
}

fn is_atom_byte(byte: u8) -> bool {
    !matches!(byte, 0..=0x20 | 0x7F | b'(' | b')' | b'{' | b'"' | b']')
}

fn status(keyword: &str) -> Option<Status> {
    match keyword {
        "OK" => Some(Status::Ok),
        "NO" => Some(Status::No),
        "BAD" => Some(Status::Bad),
        _ => None,
    }
}

fn response(s: &mut Scanner) -> Option<Response> {
    if s.consume(b'+') {
        s.consume(b' ');
        return Some(Response::Continuation(s.rest_text()));
    }
    if s.consume(b'*') {
        if !s.consume_spaces() {
            return None;
        }
        return untagged(s);
    }
    let tag = s.atom(true).filter(|tag| !tag.contains('+'))?;
    if !s.consume_spaces() {
        return None;
    }
    let status = status(&s.atom(false)?.to_uppercase())?;
    let (code, text) = response_text(s);
    Some(Response::Tagged {
        tag,
        status,
        code,
        text,
    })
}

fn untagged(s: &mut Scanner) -> Option<Response> {
    if s.peek(0).is_some_and(|byte| byte.is_ascii_digit()) {
        let number = s.number()?;
        if !s.consume_spaces() {
            return None;
        }
        return match s.atom(false)?.to_uppercase().as_str() {
            "EXISTS" => s.is_at_end().then_some(Response::Exists(number)),
            "FETCH" => {
                if !s.consume_spaces() {
                    return None;
                }
                fetch(s)
            }
            _ => None,
        };
    }
    let keyword = s.atom(false)?.to_uppercase();
    if let Some(status) = status(&keyword) {
        let (code, text) = response_text(s);
        return Some(Response::UntaggedStatus { status, code, text });
    }
    match keyword.as_str() {
        "BYE" => {
            let (code, text) = response_text(s);
            Some(Response::Bye { code, text })
        }
        "LIST" | "XLIST" => list(s),
        "SEARCH" => {
            let mut uids = Vec::new();
            loop {
                s.consume_spaces();
                // The end, or a trailing CONDSTORE `(MODSEQ n)`, which is not needed.
                if s.is_at_end() || s.peek(0) == Some(b'(') {
                    break;
                }
                uids.push(s.number()?);
            }
            Some(Response::Search(uids))
        }
        _ => None,
    }
}

/// `[SP] ["[" code "]" SP] text`; the text may be missing.
fn response_text(s: &mut Scanner) -> (Option<ResponseCode>, String) {
    s.consume_spaces();
    let mut code = None;
    if s.peek(0) == Some(b'[')
        && let Some(end) = s.matching_bracket(s.position)
    {
        code = Some(response_code(&lossy(&s.bytes[s.position + 1..end])));
        s.position = end + 1;
        s.consume_spaces();
    }
    (code, s.rest_text())
}

fn response_code(inner: &str) -> ResponseCode {
    let inner = inner.trim_matches(crate::is_blank);
    let (name, argument) = match inner.split_once(' ') {
        Some((name, argument)) => (name, Some(argument.trim_matches(crate::is_blank))),
        None => (inner, None),
    };
    let name = name.to_uppercase();
    let number = || argument.and_then(|argument| argument.parse::<u32>().ok());
    match name.as_str() {
        "ALERT" => return ResponseCode::Alert,
        "UNAVAILABLE" => return ResponseCode::Unavailable,
        "LIMIT" => return ResponseCode::Limit,
        "UIDVALIDITY" => {
            if let Some(value) = number() {
                return ResponseCode::UidValidity(value);
            }
        }
        "UIDNEXT" => {
            if let Some(value) = number() {
                return ResponseCode::UidNext(value);
            }
        }
        _ => {}
    }
    ResponseCode::Other {
        name,
        argument: argument.map(ToOwned::to_owned),
    }
}

/// `(attributes) SP delimiter SP mailbox [SP extended data]`.
fn list(s: &mut Scanner) -> Option<Response> {
    if !s.consume_spaces() {
        return None;
    }
    let Value::List(values) = s.value(0)? else {
        return None;
    };
    let attributes = values
        .into_iter()
        .map(|value| match value {
            Value::Atom(attribute) => Some(attribute),
            _ => None,
        })
        .collect::<Option<Vec<String>>>()?;
    // The hierarchy delimiter, a string or NIL, is not needed.
    if !s.consume_spaces() || !matches!(s.value(0)?, Value::Nil | Value::String(_)) {
        return None;
    }
    if !s.consume_spaces() {
        return None;
    }
    let name = s.value(0)?.string_bytes()?;
    Some(Response::List(ListEntry {
        attributes,
        name: lossy(&name),
    }))
}

/// `(name value name value ...)`. Unknown items are skipped; known ones must have the right type.
fn fetch(s: &mut Scanner) -> Option<Response> {
    let Value::List(items) = s.value(0)? else {
        return None;
    };
    if items.len() % 2 != 0 {
        return None;
    }
    let mut fetch = FetchResponse::default();
    let mut items = items.into_iter();
    while let (Some(name), Some(value)) = (items.next(), items.next()) {
        let Value::Atom(name) = name else {
            return None;
        };
        let name = name.to_uppercase();
        if name == "UID" {
            let Value::Atom(digits) = value else {
                return None;
            };
            fetch.uid = Some(digits.parse().ok()?);
        } else if name == "INTERNALDATE" {
            let Value::String(bytes) = value else {
                return None;
            };
            fetch.internal_date = date::parse(&lossy(&bytes));
        } else if name == "RFC822.HEADER"
            || name
                .strip_prefix("BODY[")
                .is_some_and(|section| section.starts_with("HEADER"))
        {
            fetch.header = match value {
                Value::String(bytes) => Some(bytes),
                Value::Nil => None,
                _ => return None,
            };
        }
    }
    Some(Response::Fetch(fetch))
}
