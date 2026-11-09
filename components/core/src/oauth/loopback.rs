//! What the loopback listener reads from the browser and answers it during the sign-in.

use fvm_contract::app_internal::{SecretText, SignInPages};

/// Larger request heads are refused; a real redirect is well under 2 KiB.
const MAX_HEAD_BYTES: usize = 8 * 1024;
/// Google's authorization codes are about 73 characters.
const MAX_CODE_BYTES: usize = 2048;

/// The request line of a browser request; header fields are not needed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LoopbackRequest {
    pub method: String,
    /// The target's path without query or fragment, such as `/` or `/favicon.ico`.
    pub path: String,
    /// Decoded, in order.
    pub query: Vec<(String, String)>,
}

impl LoopbackRequest {
    /// The first value of `name`.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(own, _)| own == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum HeadParse {
    /// The blank line that ends the head has not arrived yet.
    Incomplete,
    Request(LoopbackRequest),
    /// Not an HTTP/1.x request line, bad percent-encoding, or a head over the size limit. Answer
    /// it like `CallbackDecision::Rejected`.
    Malformed,
}

/// Reads the bytes received so far on one connection.
pub fn parse_request_head(bytes: &[u8]) -> HeadParse {
    let bytes = &bytes[..bytes.len().min(MAX_HEAD_BYTES + 4)];
    let Some(end) = head_end(bytes) else {
        return if bytes.len() > MAX_HEAD_BYTES {
            HeadParse::Malformed
        } else {
            HeadParse::Incomplete
        };
    };
    if end > MAX_HEAD_BYTES {
        return HeadParse::Malformed;
    }
    let mut line_end = bytes.iter().position(|byte| *byte == b'\n').unwrap_or(end);
    if line_end > 0 && bytes[line_end - 1] == b'\r' {
        line_end -= 1;
    }
    let line = &bytes[..line_end];
    if !line.is_ascii() {
        return HeadParse::Malformed;
    }
    let line = std::str::from_utf8(line).expect("ASCII is UTF-8");
    request_line(line).map_or(HeadParse::Malformed, HeadParse::Request)
}

fn request_line(line: &str) -> Option<LoopbackRequest> {
    let parts: Vec<&str> = line.split(' ').collect();
    let [method, target, version] = parts.as_slice() else {
        return None;
    };
    if method.is_empty() || !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return None;
    }
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (*target, None),
    };
    let mut items = Vec::new();
    for pair in query.into_iter().flat_map(|query| query.split('&')) {
        if pair.is_empty() {
            continue;
        }
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        items.push((form_decoded(name)?, form_decoded(value)?));
    }
    let path = path.split_once('#').map_or(path, |(path, _)| path);
    Some(LoopbackRequest {
        method: (*method).to_owned(),
        path: path.to_owned(),
        query: items,
    })
}

/// The index just past the blank line (`CRLF CRLF` or `LF LF`).
fn head_end(bytes: &[u8]) -> Option<usize> {
    bytes.iter().enumerate().find_map(|(index, byte)| {
        if *byte != b'\n' {
            return None;
        }
        match &bytes[index + 1..] {
            [b'\n', ..] => Some(index + 2),
            [b'\r', b'\n', ..] => Some(index + 3),
            _ => None,
        }
    })
}

/// A form-encoded component: `+` is a space and `%XX` a byte of UTF-8.
fn form_decoded(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut input = text.bytes();
    while let Some(byte) = input.next() {
        match byte {
            b'+' => bytes.push(b' '),
            b'%' => {
                let high = hex_value(input.next()?)?;
                let low = hex_value(input.next()?)?;
                bytes.push(high << 4 | low);
            }
            _ => bytes.push(byte),
        }
    }
    String::from_utf8(bytes).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .map(|digit| u8::try_from(digit).expect("a hex digit"))
}

/// What the listener does with one browser request during the sign-in.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CallbackDecision {
    /// The redirect with the expected `state` and a code: the sign-in goes on with it.
    Authorized { code: SecretText },
    /// The redirect with the expected `state` and an error, such as `access_denied`: the sign-in
    /// ends. The code holds only `[A-Za-z0-9_]`, or is `unknown_error`.
    Denied { error: String },
    /// A redirect without the expected `state`, or with neither code nor error; also a malformed
    /// request, or one after the sign-in ended. The listener keeps waiting.
    Rejected,
    /// Another path, such as `/favicon.ico`. The listener keeps waiting.
    NotFound,
}

impl CallbackDecision {
    pub fn decide(request: &LoopbackRequest, expected_state: &str) -> Self {
        if request.path != "/" {
            return CallbackDecision::NotFound;
        }
        if request.method != "GET"
            || expected_state.is_empty()
            || request.value("state") != Some(expected_state)
        {
            return CallbackDecision::Rejected;
        }
        if let Some(error) = request.value("error") {
            return CallbackDecision::Denied {
                error: sanitized_error_code(error),
            };
        }
        match request.value("code") {
            Some(code)
                if !code.is_empty()
                    && code.len() <= MAX_CODE_BYTES
                    && code.bytes().all(|byte| byte.is_ascii_graphic()) =>
            {
                CallbackDecision::Authorized {
                    code: SecretText(code.to_owned()),
                }
            }
            _ => CallbackDecision::Rejected,
        }
    }

    /// The complete HTTP answer, with the UI's page text; the connection is closed after it.
    pub fn response(&self, pages: &SignInPages) -> Vec<u8> {
        match self {
            CallbackDecision::Authorized { .. } => html(200, "OK", &pages.completed),
            CallbackDecision::Denied { .. } => html(200, "OK", &pages.not_completed),
            CallbackDecision::Rejected => html(400, "Bad Request", &pages.invalid),
            CallbackDecision::NotFound => html(404, "Not Found", "Not Found"),
        }
    }
}

/// OAuth error codes are `[A-Za-z0-9_]`; anything else, or a very long value, becomes
/// `unknown_error` so it is safe to show and log.
fn sanitized_error_code(raw: &str) -> String {
    let valid = (1..=64).contains(&raw.len())
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if valid { raw } else { "unknown_error" }.to_owned()
}

/// A tiny HTML page with `message`.
fn html(status: u16, reason: &str, message: &str) -> Vec<u8> {
    let body = format!(
        "<!doctype html>\n\
         <html><head><meta charset=\"utf-8\"><title>FortiVPNManager</title></head>\n\
         <body style=\"font-family: -apple-system, sans-serif; margin: 3em;\"><p>{}</p></body></html>\n",
        escape_html(message)
    );
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Referrer-Policy: no-referrer\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    [head.into_bytes(), body.into_bytes()].concat()
}

fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}
