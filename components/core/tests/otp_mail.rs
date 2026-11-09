//! Picking the OTP mail (SPEC F-OT-01..04).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fvm_core::gmail_api::MessageMetadata;
use fvm_core::imap::Parser;
use fvm_core::imap::{Command, UidSet};
use fvm_core::imap::{FetchResponse, Response};
use fvm_core::otp_mail::{
    ImapMessage, code_in_subject, gmail_has_fresh_code, imap_has_fresh_code, imap_messages,
    select_gmail, select_imap,
};

/// 14-Oct-2026 10:19:00 +0900.
fn prompted_at() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_791_940_740)
}

fn offset(seconds: f64) -> SystemTime {
    if seconds >= 0.0 {
        prompted_at() + Duration::from_secs_f64(seconds)
    } else {
        prompted_at() - Duration::from_secs_f64(-seconds)
    }
}

fn imap(uid: u32, seconds_after_prompt: f64, subject: &str) -> ImapMessage {
    ImapMessage {
        uid,
        internal_date: offset(seconds_after_prompt),
        header_block: format!(
            "Subject: {subject}\r\nDate: Wed, 14 Oct 2026 10:19:03 +0900\r\n\r\n"
        )
        .into_bytes(),
    }
}

fn fresh(uid: u32) -> ImapMessage {
    imap(uid, 3.0, "AuthCode: 482913")
}

fn gmail(
    id: &str,
    seconds_after_prompt: Option<f64>,
    subject: Option<&str>,
    labels: &[&str],
) -> MessageMetadata {
    let mut headers =
        vec![serde_json::json!({"name": "Date", "value": "Wed, 14 Oct 2026 10:19:03 +0900"})];
    if let Some(subject) = subject {
        headers.push(serde_json::json!({"name": "Subject", "value": subject}));
    }
    let mut json =
        serde_json::json!({"id": id, "labelIds": labels, "payload": {"headers": headers}});
    if let Some(seconds) = seconds_after_prompt {
        // Gmail sends `internalDate` as milliseconds in a string.
        let millis = ((1_791_940_740.0 + seconds) * 1000.0_f64).round() as i64;
        json["internalDate"] = serde_json::Value::String(millis.to_string());
    }
    MessageMetadata::from_response(200, json.to_string().as_bytes()).unwrap()
}

fn fresh_gmail(id: &str) -> MessageMetadata {
    gmail(
        id,
        Some(3.2),
        Some("AuthCode: 482913"),
        &["UNREAD", "INBOX"],
    )
}

// MARK: Subject pattern

#[test]
fn code_is_extracted() {
    for (subject, code) in [
        ("AuthCode: 123456", "123456"),
        ("AuthCode:123456", "123456"),
        ("AuthCode:   12345678", "12345678"),
        ("AuthCode:\t1234567 (valid for 60 seconds)", "1234567"),
        ("AuthCode:\u{00A0}654321", "654321"),
        ("Fwd: AuthCode: 654321", "654321"),
        (
            "[SPAM] \u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 000001.",
            "000001",
        ),
        ("AuthCode: abc / AuthCode: 111222", "111222"),
    ] {
        assert_eq!(code_in_subject(subject), Some(code), "{subject:?}");
    }
}

#[test]
fn non_matching_subjects() {
    for subject in [
        "AuthCode: 12345",                                            // too short
        "AuthCode: 123456789", // too long: must not truncate to 8 digits
        "authcode: 123456",    // marker is case-sensitive
        "AuthCode 123456",     // colon required
        "AuthCode: \u{FF11}\u{FF12}\u{FF13}\u{FF14}\u{FF15}\u{FF16}", // full-width digits are not ASCII
        "AuthCode: 12 3456",
        "Your code is 123456",
        "",
    ] {
        assert_eq!(code_in_subject(subject), None, "{subject:?}");
    }
}

// MARK: IMAP selection

#[test]
fn selects_a_fresh_imap_message() {
    let selection = select_imap(prompted_at(), &[fresh(500)]).unwrap();
    assert_eq!(selection.code.0, "482913");
    assert_eq!(selection.uid, 500);
    assert_eq!(selection.all_candidate_uids, vec![500]);
}

#[test]
fn imap_date_window_boundary() {
    for (seconds, included) in [
        (-6.0, false),
        (-5.5, false),
        (-5.0, true),
        (-4.0, true),
        (0.0, true),
        (40.0, true),
    ] {
        let selection = select_imap(prompted_at(), &[imap(501, seconds, "AuthCode: 482913")]);
        assert_eq!(
            selection.is_some_and(|s| s.uid == 501),
            included,
            "{seconds}"
        );
    }
}

#[test]
fn highest_qualifying_uid_wins_and_all_candidates_are_reported() {
    let messages = [
        imap(503, 3.0, "AuthCode: 333333"),
        imap(505, -60.0, "AuthCode: 555555"), // stale, still trashed
        imap(504, 3.0, "Lunch on Friday?"),   // not an OTP
        imap(502, 3.0, "AuthCode: 222222"),
        imap(503, 3.0, "AuthCode: 333333"), // duplicate fetch
    ];
    let selection = select_imap(prompted_at(), &messages).unwrap();
    assert_eq!(selection.code.0, "333333");
    assert_eq!(selection.uid, 503);
    assert_eq!(selection.all_candidate_uids, vec![502, 503, 505]);
}

#[test]
fn only_stale_imap_candidates_yield_none() {
    assert!(select_imap(prompted_at(), &[imap(501, -30.0, "AuthCode: 482913")]).is_none());
    assert!(select_imap(prompted_at(), &[]).is_none());
    assert!(select_imap(prompted_at(), &[imap(501, 3.0, "Hello")]).is_none());
}

#[test]
fn encoded_imap_subjects() {
    for subject in [
        "=?UTF-8?B?7J247Kad7L2U65OcIEF1dGhDb2RlOiA0ODI5MTM=?=",
        "=?UTF-8?Q?AuthCode:_482913?=",
        "=?UTF-8?Q?=EC=9D=B8=EC=A6=9D?= AuthCode: 482913",
        "=?EUC-KR?B?wM7B9cTateUgQXV0aENvZGU6IDQ4MjkxMw==?=",
    ] {
        let selection = select_imap(prompted_at(), &[imap(500, 3.0, subject)]);
        assert_eq!(
            selection.map(|s| s.code.0),
            Some("482913".to_owned()),
            "{subject}"
        );
    }
}

#[test]
fn folded_encoded_subject_in_header_block() {
    let message = ImapMessage {
        uid: 600,
        internal_date: prompted_at(),
        header_block: b"Date: x\r\nSubject: =?UTF-8?B?7J247Kad7L2U65OcIA==?=\r\n =?UTF-8?Q?AuthCode:_24680135?=\r\n\r\n".to_vec(),
    };
    assert_eq!(
        message.subject().as_deref(),
        Some("\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 24680135")
    );
    assert_eq!(
        select_imap(prompted_at(), &[message]).unwrap().code.0,
        "24680135"
    );
}

#[test]
fn fresh_auth_code_in_imap_junk() {
    assert!(imap_has_fresh_code(
        prompted_at(),
        &[imap(3, 10.0, "AuthCode: 482913")]
    ));
    assert!(!imap_has_fresh_code(
        prompted_at(),
        &[imap(3, -6.0, "AuthCode: 482913")]
    ));
    assert!(!imap_has_fresh_code(prompted_at(), &[imap(3, 3.0, "Hi")]));
}

fn literal(text: &str) -> String {
    format!("{{{}}}\r\n{text}", text.len())
}

#[test]
fn fetch_replies_feed_the_matcher() {
    let header = "Subject: AuthCode: 482913\r\nDate: Wed, 14 Oct 2026 10:19:02 +0900\r\n\r\n";
    let stale = "Subject: AuthCode: 111111\r\nDate: Wed, 14 Oct 2026 10:00:00 +0900\r\n\r\n";
    let transcript = format!(
        "* SEARCH 500 501\r\n\
         * 11 FETCH (UID 500 INTERNALDATE \"14-Oct-2026 10:00:00 +0900\" BODY[HEADER.FIELDS (SUBJECT DATE)] {})\r\n\
         * 12 FETCH (UID 501 INTERNALDATE \"14-Oct-2026 10:19:02 +0900\" BODY[HEADER.FIELDS (SUBJECT DATE)] {})\r\n\
         * 12 FETCH (FLAGS (\\Seen))\r\n\
         A0010 OK Success\r\n",
        literal(stale),
        literal(header)
    );
    let responses = Parser::default().feed(transcript.as_bytes()).unwrap();
    let messages = imap_messages(&responses);
    assert_eq!(
        messages.iter().map(|m| m.uid).collect::<Vec<_>>(),
        vec![500, 501]
    );
    let selection = select_imap(prompted_at(), &messages).unwrap();
    assert_eq!(selection.code.0, "482913");
    assert_eq!(selection.uid, 501);
    assert_eq!(selection.all_candidate_uids, vec![500, 501]);
    let mov = Command::UidMove {
        uids: UidSet::new(selection.all_candidate_uids.clone()).unwrap(),
        mailbox: "[Gmail]/&1zTJwNG1-".into(),
    }
    .encode("A0011")
    .unwrap();
    assert_eq!(
        String::from_utf8(mov).unwrap(),
        "A0011 UID MOVE 500:501 \"[Gmail]/&1zTJwNG1-\"\r\n"
    );
}

#[test]
fn an_imap_message_needs_all_parts() {
    let header = b"Subject: AuthCode: 1\r\n\r\n".to_vec();
    let complete = FetchResponse {
        uid: Some(5),
        internal_date: Some(prompted_at()),
        header: Some(header.clone()),
    };
    assert_eq!(
        ImapMessage::from_fetch(&complete),
        Some(ImapMessage {
            uid: 5,
            internal_date: prompted_at(),
            header_block: header.clone()
        })
    );
    for partial in [
        FetchResponse {
            uid: None,
            ..complete.clone()
        },
        FetchResponse {
            internal_date: None,
            ..complete.clone()
        },
        FetchResponse {
            header: None,
            ..complete.clone()
        },
    ] {
        assert_eq!(ImapMessage::from_fetch(&partial), None);
        assert!(imap_messages(&[Response::Fetch(partial)]).is_empty());
    }
}

// MARK: Gmail API selection

#[test]
fn selects_a_fresh_gmail_message() {
    let selection = select_gmail(prompted_at(), &[fresh_gmail("19a01")], None).unwrap();
    assert_eq!(selection.code.0, "482913");
    assert_eq!(selection.message_id, "19a01");
    assert_eq!(selection.arrived_at, offset(3.2));
    assert_eq!(selection.all_candidate_ids, vec!["19a01"]);
}

#[test]
fn gmail_date_window_boundary() {
    for (seconds, included) in [
        (-6.0, false),
        (-5.001, false),
        (-5.0, true),
        (-4.0, true),
        (0.0, true),
        (45.0, true),
    ] {
        let message = gmail("m", Some(seconds), Some("AuthCode: 482913"), &["INBOX"]);
        let selection = select_gmail(prompted_at(), &[message], None);
        assert_eq!(
            selection.is_some_and(|s| s.message_id == "m"),
            included,
            "{seconds}"
        );
    }
}

#[test]
fn newest_by_internal_date_wins_and_every_candidate_is_reported() {
    let inbox = &["UNREAD", "INBOX"];
    let messages = [
        gmail("19a05", Some(2.0), Some("AuthCode: 222222"), inbox),
        gmail("19a03", Some(6.0), Some("AuthCode: 333333"), inbox), // newest by date
        gmail("19a09", Some(-60.0), Some("AuthCode: 999999"), inbox), // stale, still trashed
        gmail("19a07", Some(3.2), Some("Lunch on Friday?"), inbox),
        gmail("19a08", Some(3.2), None, inbox),
    ];
    let selection = select_gmail(prompted_at(), &messages, None).unwrap();
    assert_eq!(selection.code.0, "333333");
    assert_eq!(selection.message_id, "19a03");
    assert_eq!(selection.arrived_at, offset(6.0));
    assert_eq!(selection.all_candidate_ids, vec!["19a09", "19a05", "19a03"]);
}

#[test]
fn the_same_instant_prefers_the_later_id() {
    let messages = [
        gmail("19a0f", Some(4.0), Some("AuthCode: 111111"), &["INBOX"]),
        gmail("19a100", Some(4.0), Some("AuthCode: 222222"), &["INBOX"]),
        gmail("19a0e", Some(4.0), Some("AuthCode: 333333"), &["INBOX"]),
    ];
    let selection = select_gmail(prompted_at(), &messages, None).unwrap();
    assert_eq!(selection.code.0, "222222");
    assert_eq!(
        selection.all_candidate_ids,
        vec!["19a0e", "19a0f", "19a100"]
    );
}

#[test]
fn excluded_labels() {
    for labels in [&["SPAM"][..], &["TRASH", "INBOX"], &["DRAFT"]] {
        let excluded = gmail("m", Some(3.2), Some("AuthCode: 482913"), labels);
        assert!(select_gmail(prompted_at(), std::slice::from_ref(&excluded), None).is_none());
        let other = gmail("n", Some(3.2), Some("AuthCode: 654321"), &["INBOX"]);
        let selection = select_gmail(prompted_at(), &[excluded, other], None).unwrap();
        assert_eq!(selection.code.0, "654321");
        assert_eq!(selection.all_candidate_ids, vec!["n"]);
    }
}

#[test]
fn no_labels_still_count() {
    let message = gmail("m", Some(3.2), Some("AuthCode: 482913"), &[]);
    assert_eq!(
        select_gmail(prompted_at(), &[message], None)
            .unwrap()
            .message_id,
        "m"
    );
}

#[test]
fn a_missing_internal_date_is_a_candidate_but_never_selected() {
    let undated = gmail("m", None, Some("AuthCode: 482913"), &["INBOX"]);
    assert!(select_gmail(prompted_at(), std::slice::from_ref(&undated), None).is_none());
    let selection = select_gmail(prompted_at(), &[undated, fresh_gmail("n")], None).unwrap();
    assert_eq!(selection.message_id, "n");
    assert_eq!(selection.all_candidate_ids, vec!["m", "n"]);
}

#[test]
fn candidates_not_before_limits_trashing_but_not_selection() {
    let not_before = offset(-30.0);
    let messages = [
        gmail("old", Some(-3600.0), Some("AuthCode: 111111"), &["INBOX"]),
        gmail("nodate", None, Some("AuthCode: 222222"), &["INBOX"]),
        gmail("recent", Some(-20.0), Some("AuthCode: 333333"), &["INBOX"]),
        gmail("fresh", Some(1.0), Some("AuthCode: 444444"), &["INBOX"]),
    ];
    let selection = select_gmail(prompted_at(), &messages, Some(not_before)).unwrap();
    assert_eq!(selection.code.0, "444444");
    assert_eq!(selection.message_id, "fresh");
    assert_eq!(selection.arrived_at, offset(1.0));
    assert_eq!(selection.all_candidate_ids, vec!["recent", "fresh"]);
    assert_eq!(
        select_gmail(prompted_at(), &messages, None)
            .unwrap()
            .all_candidate_ids,
        vec!["nodate", "old", "recent", "fresh"]
    );
}

#[test]
fn only_stale_or_unrelated_gmail_messages_yield_none() {
    assert!(select_gmail(prompted_at(), &[], None).is_none());
    for message in [
        gmail("m", Some(-30.0), Some("AuthCode: 482913"), &["INBOX"]),
        gmail("m", Some(3.2), Some("AuthCode: 12345"), &["INBOX"]),
        gmail("m", Some(3.2), Some("Your code is 123456"), &["INBOX"]),
    ] {
        assert!(select_gmail(prompted_at(), &[message], None).is_none());
    }
}

#[test]
fn gmail_subject_forms() {
    for subject in [
        "=?UTF-8?B?7J247Kad7L2U65OcIEF1dGhDb2RlOiA0ODI5MTM=?=",
        "=?UTF-8?Q?AuthCode:_482913?=",
        "\u{C778}\u{C99D}\u{CF54}\u{B4DC} AuthCode: 482913",
        "[External] AuthCode: 482913 (valid 60 s)",
    ] {
        let message = gmail("m", Some(3.2), Some(subject), &["INBOX"]);
        assert_eq!(
            select_gmail(prompted_at(), &[message], None)
                .unwrap()
                .code
                .0,
            "482913",
            "{subject}"
        );
    }
}

#[test]
fn the_gmail_junk_check_ignores_labels_but_not_the_window() {
    let spam = &["SPAM"];
    assert!(gmail_has_fresh_code(
        prompted_at(),
        &[gmail("s", Some(3.2), Some("AuthCode: 482913"), spam)]
    ));
    assert!(!gmail_has_fresh_code(
        prompted_at(),
        &[gmail("s", Some(-6.0), Some("AuthCode: 482913"), spam)]
    ));
    assert!(!gmail_has_fresh_code(
        prompted_at(),
        &[gmail("s", None, Some("AuthCode: 482913"), spam)]
    ));
    assert!(!gmail_has_fresh_code(
        prompted_at(),
        &[gmail("s", Some(3.2), Some("Hi"), spam)]
    ));
}
