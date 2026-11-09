//! Judgment core: every decision of the app and every data format it reads, with no outside
//! communication. It reads no files, opens no connections, runs no timers and knows neither the VPN
//! client's name nor any sentence the user sees. Given the same inputs in the same order it gives
//! the same results, so the runtime feeds it every input and carries out what it decides.

pub mod bypass_rules;
pub mod config;
pub mod connection;
pub mod gmail_api;
pub mod imap;
pub mod oauth;
pub mod otp_mail;

/// Tab and the Unicode space separators: what mail headers and IMAP response codes are trimmed of,
/// unlike line breaks.
pub(crate) fn is_blank(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}
