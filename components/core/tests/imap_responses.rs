//! Parsing what the IMAP server sends, in any chunking.

use std::time::{Duration, UNIX_EPOCH};

use fvm_core::imap::{
    Command, FetchResponse, ListEntry, MailboxSnapshot, ParseError, Parser, Response, ResponseCode,
    SpecialMailboxes, Status,
};
use fvm_core::otp_mail::ImapMessage;

const MAX_LITERAL: usize = 1 << 20;
const MAX_RESPONSE: usize = 4 << 20;

fn lines(lines: &[&str]) -> String {
    lines.iter().map(|line| format!("{line}\r\n")).collect()
}

fn literal(payload: &str) -> String {
    format!("{{{}}}\r\n{payload}", payload.len())
}

fn parse(transcript: &str) -> Vec<Response> {
    Parser::default().feed(transcript.as_bytes()).unwrap()
}

fn tagged(tag: &str, status: Status, code: Option<ResponseCode>, text: &str) -> Response {
    Response::Tagged {
        tag: tag.into(),
        status,
        code,
        text: text.into(),
    }
}

fn untagged(status: Status, code: Option<ResponseCode>, text: &str) -> Response {
    Response::UntaggedStatus {
        status,
        code,
        text: text.into(),
    }
}

fn code(name: &str, argument: Option<&str>) -> Option<ResponseCode> {
    Some(ResponseCode::Other {
        name: name.into(),
        argument: argument.map(Into::into),
    })
}

fn other(line: &str) -> Response {
    Response::Other(line.into())
}

fn fetch(uid: Option<u32>, internal_date: Option<u64>, header: Option<&[u8]>) -> Response {
    Response::Fetch(FetchResponse {
        uid,
        internal_date: internal_date.map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds)),
        header: header.map(<[u8]>::to_vec),
    })
}

fn list(attributes: &[&str], name: &str) -> ListEntry {
    ListEntry {
        attributes: attributes.iter().map(|a| a.to_string()).collect(),
        name: name.into(),
    }
}

// MARK: Framing

/// A session-shaped transcript with literals (one empty, one containing CRLF and `{n}` text),
/// quoted escapes, and untagged updates.
fn transcript() -> Vec<u8> {
    let header1 = "Subject: AuthCode: 482913\r\nDate: Wed, 14 Oct 2026 10:19:00 +0900\r\n\r\n";
    let header2 = "Subject: =?UTF-8?B?7J247Kad7L2U65OcIEF1dGhDb2RlOiA3NzExMjI=?=\r\n {7}\r\n\r\n";
    let mut text = lines(&[
        "* OK Gimap ready",
        "A0001 OK [CAPABILITY IMAP4rev1 IDLE MOVE] authenticated (Success)",
        r#"* LIST (\HasNoChildren \All) "/" "[Gmail]/&yATMtLz0rQDVaA-""#,
    ]);
    text += &format!(
        r#"* LIST (\HasNoChildren \Trash) "/" {}"#,
        literal("[Gmail]/&1zTJwNG1-")
    );
    text += "\r\n";
    text += &lines(&[
        "* OK [UIDVALIDITY 11] UIDs valid.",
        "* OK [UIDNEXT 500] Predicted next UID.",
        "+ idling",
        "* 499 EXISTS",
        r"* 498 FETCH (FLAGS (\Seen))",
        "* SEARCH 499 500",
    ]);
    text += &format!(
        "* 499 FETCH (UID 500 INTERNALDATE \"14-Oct-2026 10:19:00 +0900\" BODY[HEADER.FIELDS (SUBJECT DATE)] {})\r\n",
        literal(header1)
    );
    text += &format!(
        "* 500 FETCH (UID 501 INTERNALDATE \"14-Oct-2026 10:19:30 +0900\" BODY[HEADER.FIELDS (SUBJECT DATE)] {} X-EMPTY {})\r\n",
        literal(header2),
        literal("")
    );
    text += &lines(&[
        r#"* LIST (\HasNoChildren) "/" "a\"b""#,
        "A0009 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)",
        "* BYE bye",
    ]);
    text.into_bytes()
}

fn expected() -> Vec<Response> {
    Parser::default().feed(&transcript()).unwrap()
}

#[test]
fn a_one_shot_parse_is_complete() {
    let responses = expected();
    assert_eq!(responses.len(), 15);
    assert!(
        !responses
            .iter()
            .any(|response| matches!(response, Response::Other(_)))
    );
    let Response::Fetch(second) = &responses[11] else {
        panic!("expected fetch, got {:?}", responses[11])
    };
    assert_eq!(second.uid, Some(501));
    assert_eq!(
        ImapMessage::from_fetch(second)
            .unwrap()
            .subject()
            .as_deref(),
        Some("\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 771122 {7}")
    );
}

#[test]
fn split_at_every_offset() {
    let bytes = transcript();
    let expected = expected();
    for offset in 0..=bytes.len() {
        let mut parser = Parser::default();
        let mut responses = parser.feed(&bytes[..offset]).unwrap();
        responses.extend(parser.feed(&bytes[offset..]).unwrap());
        assert_eq!(responses, expected, "split at {offset}");
    }
}

#[test]
fn byte_by_byte() {
    let mut parser = Parser::default();
    let mut responses = Vec::new();
    for byte in transcript() {
        responses.extend(parser.feed(&[byte]).unwrap());
    }
    assert_eq!(responses, expected());
}

/// Deterministic generator for chunk boundaries.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[test]
fn random_chunkings() {
    let bytes = transcript();
    let expected = expected();
    for seed in [1, 7, 42, 1234, 99_999] {
        let mut rng = SplitMix64(seed);
        for _ in 0..50 {
            let mut parser = Parser::default();
            let mut responses = Vec::new();
            let mut index = 0;
            while index < bytes.len() {
                let end = (index + (rng.next() % 17) as usize).min(bytes.len()); // empty chunks included
                responses.extend(parser.feed(&bytes[index..end]).unwrap());
                index = end;
            }
            assert_eq!(responses, expected);
        }
    }
}

#[test]
fn a_literal_split_inside_its_count_and_crlf() {
    let line = b"* 1 FETCH (UID 2 BODY[HEADER] {12}\r\nSubject: x\r\n)\r\n";
    let expected = vec![fetch(Some(2), None, Some(b"Subject: x\r\n"))];
    for first in 0..=line.len() {
        for second in first..=line.len() {
            let mut parser = Parser::default();
            let mut responses = parser.feed(&line[..first]).unwrap();
            responses.extend(parser.feed(&line[first..second]).unwrap());
            responses.extend(parser.feed(&line[second..]).unwrap());
            assert_eq!(responses, expected, "splits at {first}, {second}");
        }
    }
}

// MARK: Status responses

#[test]
fn greeting_and_capability() {
    let capability = "* CAPABILITY IMAP4rev1 UNSELECT IDLE NAMESPACE QUOTA ID XLIST CHILDREN X-GM-EXT-1 UIDPLUS COMPRESS=DEFLATE ENABLE MOVE CONDSTORE ESEARCH UTF8=ACCEPT LIST-EXTENDED LIST-STATUS LITERAL- SPECIAL-USE APPENDLIMIT=35651584";
    let responses = parse(&lines(&[
        "* OK Gimap ready for requests from 203.0.113.7 a1b2c3",
        capability,
        "A0001 OK [CAPABILITY IMAP4rev1 IDLE MOVE SPECIAL-USE] user@example.com authenticated (Success)",
    ]));
    assert_eq!(
        responses,
        vec![
            untagged(
                Status::Ok,
                None,
                "Gimap ready for requests from 203.0.113.7 a1b2c3"
            ),
            other(capability),
            tagged(
                "A0001",
                Status::Ok,
                code("CAPABILITY", Some("IMAP4rev1 IDLE MOVE SPECIAL-USE")),
                "user@example.com authenticated (Success)"
            ),
        ]
    );
}

#[test]
fn status_responses() {
    for (line, expected) in [
        (
            "A0001 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)",
            tagged(
                "A0001",
                Status::No,
                code("AUTHENTICATIONFAILED", None),
                "Invalid credentials (Failure)",
            ),
        ),
        (
            "A0001 NO [ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)",
            tagged(
                "A0001",
                Status::No,
                Some(ResponseCode::Alert),
                "Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)",
            ),
        ),
        (
            "A0002 BAD Could not parse command",
            tagged("A0002", Status::Bad, None, "Could not parse command"),
        ),
        (
            "a7 ok [READ-WRITE] [Gmail]/All Mail selected. (Success)",
            tagged(
                "a7",
                Status::Ok,
                code("READ-WRITE", None),
                "[Gmail]/All Mail selected. (Success)",
            ),
        ),
        ("A0003 OK", tagged("A0003", Status::Ok, None, "")),
        (
            "A0004 NO [WEBALERT https://accounts.google.com/signin/continue?x=1] Web login required.",
            tagged(
                "A0004",
                Status::No,
                code(
                    "WEBALERT",
                    Some("https://accounts.google.com/signin/continue?x=1"),
                ),
                "Web login required.",
            ),
        ),
        (
            "A0005 NO [TRYCREATE] No folder [Gmail]/Nope (Failure)",
            tagged(
                "A0005",
                Status::No,
                code("TRYCREATE", None),
                "No folder [Gmail]/Nope (Failure)",
            ),
        ),
        (
            "A0006 OK [READ-ONLY] Examined",
            tagged("A0006", Status::Ok, code("READ-ONLY", None), "Examined"),
        ),
        (
            "* NO [UNAVAILABLE] Temporary System Problem",
            untagged(
                Status::No,
                Some(ResponseCode::Unavailable),
                "Temporary System Problem",
            ),
        ),
        (
            "* BAD [CLIENTBUG] Unknown command",
            untagged(Status::Bad, code("CLIENTBUG", None), "Unknown command"),
        ),
        (
            "* BYE System error",
            Response::Bye {
                code: None,
                text: "System error".into(),
            },
        ),
        (
            "* BYE [UNAVAILABLE] Server shutting down",
            Response::Bye {
                code: Some(ResponseCode::Unavailable),
                text: "Server shutting down".into(),
            },
        ),
        ("+ idling", Response::Continuation("idling".into())),
        ("+", Response::Continuation(String::new())),
        ("+ ", Response::Continuation(String::new())),
    ] {
        assert_eq!(parse(&lines(&[line])), vec![expected], "{line}");
    }
}

#[test]
fn select_replies_produce_a_snapshot() {
    let responses = parse(&lines(&[
        r"* FLAGS (\Answered \Flagged \Draft \Deleted \Seen $NotPhishing $Phishing)",
        r"* OK [PERMANENTFLAGS (\Answered \Flagged \Draft \Deleted \Seen $NotPhishing $Phishing \*)] Flags permitted.",
        "* OK [UIDVALIDITY 11] UIDs valid.",
        "* 48213 EXISTS",
        "* 0 RECENT",
        "* OK [UIDNEXT 512345] Predicted next UID.",
        "* OK [HIGHESTMODSEQ 9876543]",
        "A0003 OK [READ-WRITE] [Gmail]/&yATMtLz0rQDVaA- selected. (Success)",
    ]));
    assert_eq!(
        responses,
        vec![
            other(r"* FLAGS (\Answered \Flagged \Draft \Deleted \Seen $NotPhishing $Phishing)"),
            untagged(
                Status::Ok,
                code(
                    "PERMANENTFLAGS",
                    Some(r"(\Answered \Flagged \Draft \Deleted \Seen $NotPhishing $Phishing \*)")
                ),
                "Flags permitted.",
            ),
            untagged(
                Status::Ok,
                Some(ResponseCode::UidValidity(11)),
                "UIDs valid."
            ),
            Response::Exists(48213),
            other("* 0 RECENT"),
            untagged(
                Status::Ok,
                Some(ResponseCode::UidNext(512_345)),
                "Predicted next UID."
            ),
            untagged(Status::Ok, code("HIGHESTMODSEQ", Some("9876543")), ""),
            tagged(
                "A0003",
                Status::Ok,
                code("READ-WRITE", None),
                "[Gmail]/&yATMtLz0rQDVaA- selected. (Success)"
            ),
        ]
    );
    assert_eq!(
        MailboxSnapshot::from_select_responses(&responses),
        Some(MailboxSnapshot {
            uid_validity: 11,
            uid_next: 512_345
        })
    );
}

#[test]
fn a_snapshot_needs_both_codes() {
    let without_next = parse(&lines(&[
        "* OK [UIDVALIDITY 11] UIDs valid.",
        "A1 OK [READ-WRITE] done",
    ]));
    assert_eq!(MailboxSnapshot::from_select_responses(&without_next), None);
    let bad_number = parse(&lines(&[
        "* OK [UIDVALIDITY 11]",
        "* OK [UIDNEXT 99999999999]",
    ]));
    assert_eq!(
        bad_number[1],
        untagged(Status::Ok, code("UIDNEXT", Some("99999999999")), "")
    );
    assert_eq!(MailboxSnapshot::from_select_responses(&bad_number), None);
}

// MARK: LIST

#[test]
fn a_gmail_list_with_special_use_and_korean_names() {
    let responses = parse(&lines(&[
        r#"* LIST (\HasNoChildren) "/" "INBOX""#,
        r#"* LIST (\HasChildren \Noselect) "/" "[Gmail]""#,
        r#"* LIST (\HasNoChildren \All) "/" "[Gmail]/&yATMtLz0rQDVaA-""#,
        r#"* LIST (\HasNoChildren \Sent) "/" "[Gmail]/&vPSwuNO4ycDVaA-""#,
        r#"* LIST (\Flagged \HasNoChildren) "/" "[Gmail]/&vMTUXNO4ycDVaA-""#,
        r#"* LIST (\HasNoChildren \Important) "/" "[Gmail]/&yRHGlA-""#,
        r#"* LIST (\HasNoChildren \Junk) "/" "[Gmail]/&wqTTONVo-""#,
        r#"* LIST (\HasNoChildren \Trash) "/" "[Gmail]/&1zTJwNG1-""#,
        r#"* LIST (\HasNoChildren) "/" "Tom &- Jerry""#,
        "A0002 OK Success",
    ]));
    let entries: Vec<&ListEntry> = responses
        .iter()
        .filter_map(|response| match response {
            Response::List(entry) => Some(entry),
            _ => None,
        })
        .collect();
    assert_eq!(entries.len(), 9);
    assert_eq!(
        *entries[2],
        list(&[r"\HasNoChildren", r"\All"], "[Gmail]/&yATMtLz0rQDVaA-")
    );
    assert_eq!(
        entries[2].display_name(),
        "[Gmail]/\u{C804}\u{CCB4}\u{BCF4}\u{AD00}\u{D568}"
    );
    assert!(entries[2].has_attribute(r"\All"));
    assert!(!entries[1].is_selectable());
    assert_eq!(entries[8].display_name(), "Tom & Jerry");
    let special = SpecialMailboxes::from_responses(&responses);
    assert_eq!(special.all.unwrap().name, "[Gmail]/&yATMtLz0rQDVaA-");
    let trash = special.trash.unwrap();
    assert_eq!(trash.name, "[Gmail]/&1zTJwNG1-");
    assert_eq!(trash.display_name(), "[Gmail]/\u{D734}\u{C9C0}\u{D1B5}");
    assert_eq!(
        special.junk.unwrap().display_name(),
        "[Gmail]/\u{C2A4}\u{D338}\u{D568}"
    );
    assert_eq!(
        responses.last(),
        Some(&tagged("A0002", Status::Ok, None, "Success"))
    );
}

#[test]
fn list_variants() {
    let transcript = format!(
        r#"* LIST (\HasNoChildren \Trash) "/" {}"#,
        literal("[Gmail]/Trash")
    ) + "\r\n"
        + &lines(&[
            r#"* LIST (\Noselect) NIL """#,
            r#"* LIST () "." INBOX"#,
            r#"* LIST (\HasNoChildren) "/" "quote\"and\\slash""#,
            r#"* LIST (\HasNoChildren) "/" [Gmail]/Bare"#,
            r#"* LIST (\all \hasnochildren) "/" "lower""#,
            r#"* XLIST (\HasNoChildren \AllMail) "/" "[Gmail]/All Mail""#,
            r#"* LIST (\HasNoChildren \Trash) "/" "[Gmail]/Bin" ("CHILDINFO" ("SUBSCRIBED"))"#,
        ]);
    let responses = parse(&transcript);
    let expected: Vec<Response> = [
        list(&[r"\HasNoChildren", r"\Trash"], "[Gmail]/Trash"),
        list(&[r"\Noselect"], ""),
        list(&[], "INBOX"),
        list(&[r"\HasNoChildren"], r#"quote"and\slash"#),
        list(&[r"\HasNoChildren"], "[Gmail]/Bare"),
        list(&[r"\all", r"\hasnochildren"], "lower"),
        list(&[r"\HasNoChildren", r"\AllMail"], "[Gmail]/All Mail"),
        list(&[r"\HasNoChildren", r"\Trash"], "[Gmail]/Bin"),
    ]
    .into_iter()
    .map(Response::List)
    .collect();
    assert_eq!(responses, expected);
    // Attribute matching is case-insensitive.
    let Response::List(lower) = &responses[5] else {
        unreachable!()
    };
    assert!(lower.has_attribute(r"\All"));
    // The quoted name re-serializes with the same escapes.
    let Response::List(quoted) = &responses[3] else {
        unreachable!()
    };
    let select = Command::Select {
        mailbox: quoted.name.clone(),
    }
    .encode("A9")
    .unwrap();
    assert_eq!(
        String::from_utf8(select).unwrap(),
        "A9 SELECT \"quote\\\"and\\\\slash\"\r\n"
    );
}

// MARK: SEARCH and FETCH

#[test]
fn search_results() {
    for (line, expected) in [
        ("* SEARCH", vec![]),
        ("* SEARCH ", vec![]),
        ("* SEARCH 4021", vec![4021]),
        ("* SEARCH 5 6 4294967295", vec![5, 6, 4_294_967_295]),
        ("* search 7 8 (MODSEQ 917162500)", vec![7, 8]),
    ] {
        assert_eq!(
            parse(&lines(&[line])),
            vec![Response::Search(expected)],
            "{line}"
        );
    }
}

#[test]
fn a_fetch_with_a_header_literal() {
    let header = "Subject: AuthCode: 482913\r\nDate: Wed, 14 Oct 2026 10:19:00 +0900\r\n\r\n";
    let transcript = format!(
        "* 48214 FETCH (UID 512345 INTERNALDATE \"14-Oct-2026 10:19:00 +0900\" BODY[HEADER.FIELDS (SUBJECT DATE)] {})\r\n",
        literal(header)
    ) + &lines(&["A0005 OK Success"]);
    assert_eq!(
        parse(&transcript),
        vec![
            fetch(Some(512_345), Some(1_791_940_740), Some(header.as_bytes())),
            tagged("A0005", Status::Ok, None, "Success"),
        ]
    );
}

#[test]
fn fetch_item_order_flags_and_unknown_items() {
    let header = "SUBJECT: AuthCode: 111222\r\n\r\n";
    let transcript = format!(
        r#"* 3 FETCH (X-GM-LABELS ("\\Inbox" "\\Important" Work) FLAGS (\Seen $Label1) BODY[HEADER.FIELDS ("SUBJECT" "DATE")] {} RFC822.SIZE 4512 INTERNALDATE " 4-Oct-2026 00:00:00 +0000" UID 77 MODSEQ (12345))"#,
        literal(header)
    ) + "\r\n";
    assert_eq!(
        parse(&transcript),
        vec![fetch(
            Some(77),
            Some(1_791_072_000),
            Some(header.as_bytes())
        )]
    );
}

#[test]
fn a_fetch_with_a_quoted_or_nil_header() {
    let responses = parse(
        &(lines(&[
            r#"* 1 FETCH (UID 10 BODY[HEADER.FIELDS (SUBJECT DATE)] "Subject: x\"y")"#,
            r"* 2 FETCH (UID 11 BODY[HEADER.FIELDS (SUBJECT DATE)] NIL)",
            r#"* 3 FETCH (UID 12 RFC822.HEADER "Subject: z")"#,
        ]) + "* 4 FETCH (UID 13 BODY[HEADER] {0}\r\n)\r\n"),
    );
    assert_eq!(
        responses,
        vec![
            fetch(Some(10), None, Some(br#"Subject: x"y"#)),
            fetch(Some(11), None, None),
            fetch(Some(12), None, Some(b"Subject: z")),
            fetch(Some(13), None, Some(b"")),
        ]
    );
}

#[test]
fn a_fetch_with_an_unparsable_internal_date_keeps_other_items() {
    assert_eq!(
        parse(&lines(&[r#"* 1 FETCH (UID 10 INTERNALDATE "yesterday")"#])),
        vec![fetch(Some(10), None, None)]
    );
}

#[test]
fn untagged_updates_interleaved_during_idle() {
    let responses = parse(&lines(&[
        "+ idling",
        "* 48215 EXISTS",
        "* 120 EXPUNGE",
        r"* 48213 FETCH (FLAGS (\Seen))",
        "* 48214 FETCH (UID 512346 FLAGS ())",
        "* 48215 EXISTS",
        "A0007 OK IDLE terminated (Success)",
    ]));
    assert_eq!(
        responses,
        vec![
            Response::Continuation("idling".into()),
            Response::Exists(48215),
            other("* 120 EXPUNGE"),
            fetch(None, None, None),
            fetch(Some(512_346), None, None),
            Response::Exists(48215),
            tagged("A0007", Status::Ok, None, "IDLE terminated (Success)"),
        ]
    );
}

#[test]
fn move_replies() {
    let responses = parse(&lines(&[
        "* OK [COPYUID 7 512345:512346 901:902] (Success)",
        "* 48214 EXPUNGE",
        "* 48214 EXPUNGE",
        "* 48212 EXISTS",
        "A0008 OK Success",
    ]));
    assert_eq!(
        responses,
        vec![
            untagged(
                Status::Ok,
                code("COPYUID", Some("7 512345:512346 901:902")),
                "(Success)"
            ),
            other("* 48214 EXPUNGE"),
            other("* 48214 EXPUNGE"),
            Response::Exists(48212),
            tagged("A0008", Status::Ok, None, "Success"),
        ]
    );
}

#[test]
fn malformed_or_unmodelled_lines_become_other() {
    for line in [
        "* LIST garbage",
        "* LIST (\\HasNoChildren) \"/\"",
        "* 99999999999 EXISTS",
        "* 12 EXISTS extra",
        "* 12 FETCH UID 5",
        "* 12 FETCH (UID)",
        "* 12 FETCH (UID abc)",
        "* 12 FETCH (UID 5",
        "* SEARCH 1 two 3",
        "* ENABLED UTF8=ACCEPT",
        "* ID (\"name\" \"GImap\")",
        "*",
        "* ",
        "A0001 MAYBE later",
        "A0001",
        "",
        "garbage without status",
        "* LIST (\\HasNoChildren) \"/\" \"unterminated",
    ] {
        assert_eq!(parse(&lines(&[line])), vec![other(line)], "{line:?}");
    }
}

#[test]
fn deeply_nested_lists_are_refused_without_unbounded_recursion() {
    let depth = 100_000;
    let line = format!(
        "* 1 FETCH (X {}{} UID 5)",
        "(".repeat(depth),
        ")".repeat(depth)
    );
    assert_eq!(parse(&lines(&[&line])), vec![other(&line)]);
    let allowed = format!("* 1 FETCH (X {}{} UID 5)", "(".repeat(31), ")".repeat(31));
    assert_eq!(parse(&lines(&[&allowed])), vec![fetch(Some(5), None, None)]);
}

#[test]
fn bare_line_feeds_are_tolerated() {
    assert_eq!(
        parse("* 3 EXISTS\nA1 OK done\n* 1 FETCH (UID 4 BODY[HEADER] {2}\nab)\n"),
        vec![
            Response::Exists(3),
            tagged("A1", Status::Ok, None, "done"),
            fetch(Some(4), None, Some(b"ab")),
        ]
    );
}

#[test]
fn a_literal_may_contain_crlf_and_braces() {
    let header = "Subject: weird {5}\r\nX: )\r\n\r\n";
    let transcript = format!("* 9 FETCH (BODY[HEADER] {} UID 99)\r\n", literal(header));
    assert_eq!(
        parse(&transcript),
        vec![fetch(Some(99), None, Some(header.as_bytes()))]
    );
}

#[test]
fn multiple_literals_in_one_response() {
    let transcript = format!(
        "* 1 FETCH (BODY[HEADER] {} X-ONE {} X-ZERO ~{} UID 5)\r\n",
        literal("H\r\n"),
        literal("abc"),
        literal("")
    );
    assert_eq!(
        parse(&transcript),
        vec![fetch(Some(5), None, Some(b"H\r\n"))]
    );
}

#[test]
fn eight_bit_bytes_in_a_literal_are_kept() {
    let mut payload = b"Subject: ".to_vec();
    payload.extend([0xC0, 0xCE, 0xFF, 0x00, 0x0D, 0x0A]);
    let mut bytes = format!("* 2 FETCH (UID 8 BODY[HEADER] {{{}}}\r\n", payload.len()).into_bytes();
    bytes.extend(&payload);
    bytes.extend(b")\r\n");
    assert_eq!(
        Parser::default().feed(&bytes).unwrap(),
        vec![fetch(Some(8), None, Some(&payload))]
    );
}

#[test]
fn partial_input_stays_buffered() {
    let mut parser = Parser::default();
    assert!(parser.feed(b"* 5 EXI").unwrap().is_empty());
    assert_eq!(
        parser.feed(b"STS\r\n* 6").unwrap(),
        vec![Response::Exists(5)]
    );
    assert_eq!(
        parser.feed(b" EXISTS\r\n").unwrap(),
        vec![Response::Exists(6)]
    );
    assert!(parser.feed(b"").unwrap().is_empty());
}

#[test]
fn an_oversized_literal_is_fatal() {
    let too_large = MAX_LITERAL as u64 + 1;
    let mut parser = Parser::default();
    let failure = Err(ParseError::LiteralTooLarge {
        declared: too_large,
    });
    assert_eq!(
        parser.feed(format!("* 1 FETCH (BODY[HEADER] {{{too_large}}}\r\n").as_bytes()),
        failure
    );
    // The parser stays failed.
    assert_eq!(parser.feed(b"* 1 EXISTS\r\n"), failure);
    let mut overflow = Parser::default();
    assert_eq!(
        overflow.feed(b"* 1 FETCH (BODY[HEADER] {99999999999999999999999}\r\n"),
        Err(ParseError::LiteralTooLarge { declared: u64::MAX })
    );
}

#[test]
fn an_oversized_response_is_fatal() {
    // "* OK " + text + CRLF is exactly the limit: fine.
    let text = "x".repeat(MAX_RESPONSE - 7);
    assert_eq!(
        Parser::default()
            .feed(format!("* OK {text}\r\n").as_bytes())
            .unwrap(),
        vec![untagged(Status::Ok, None, &text)]
    );
    // One byte more is not.
    assert_eq!(
        Parser::default().feed(format!("* OK x{text}\r\n").as_bytes()),
        Err(ParseError::ResponseTooLarge)
    );
    // A literal that would take the response past the limit fails when it is announced.
    let padding = "x".repeat(MAX_RESPONSE - MAX_LITERAL);
    assert_eq!(
        Parser::default().feed(
            format!("* 1 FETCH (X-PAD \"{padding}\" BODY[HEADER] {{{MAX_LITERAL}}}\r\n").as_bytes()
        ),
        Err(ParseError::ResponseTooLarge)
    );
}
