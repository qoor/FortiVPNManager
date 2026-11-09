//! Gmail API payloads and error classification (SPEC F-OT-02).

use std::time::{Duration, UNIX_EPOCH};

use fvm_core::gmail_api::{
    ApiError, HistoryPage, MessageMetadata, Profile, is_valid_history_id, message_metadata,
    messages_from_response, trash,
};
use fvm_core::otp_mail::gmail_subject;

#[test]
fn profile() {
    let json = r#"{"emailAddress": "otp@example.com", "messagesTotal": 1234, "threadsTotal": 900, "historyId": "9876543"}"#;
    let profile = Profile::from_response(200, json.as_bytes()).unwrap();
    assert_eq!(profile.email_address, "otp@example.com");
    assert_eq!(profile.history_id, "9876543");
}

#[test]
fn history_list() {
    let json = r#"{
      "history": [
        {"id": "100", "messages": [{"id": "a1", "threadId": "a1"}],
         "messagesAdded": [{"message": {"id": "19a0000000000001", "threadId": "t1", "labelIds": ["UNREAD", "INBOX"]}}]},
        {"id": "101", "labelsAdded": [{"message": {"id": "x"}, "labelIds": ["STARRED"]}]},
        {"id": "102", "messagesAdded": [
          {"message": {"id": "19a0000000000002", "threadId": "t2", "labelIds": ["SPAM"]}},
          {"message": {"id": "19a0000000000001", "threadId": "t1"}}
        ]}
      ],
      "nextPageToken": "page2",
      "historyId": "103"
    }"#;
    let page = HistoryPage::from_response(200, json.as_bytes()).unwrap();
    assert_eq!(page.history_id.as_deref(), Some("103"));
    assert_eq!(page.next_page_token.as_deref(), Some("page2"));
    assert_eq!(
        page.added_messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["19a0000000000001", "19a0000000000002"]
    );
    assert_eq!(
        page.added_messages
            .iter()
            .map(|m| m.label_ids.clone())
            .collect::<Vec<_>>(),
        [
            vec!["UNREAD".to_owned(), "INBOX".to_owned()],
            vec!["SPAM".to_owned()]
        ]
    );
}

#[test]
fn an_empty_history_list() {
    let page = HistoryPage::from_response(200, br#"{"historyId": "555"}"#).unwrap();
    assert_eq!(page.history_id.as_deref(), Some("555"));
    assert_eq!(page.next_page_token, None);
    assert!(page.added_messages.is_empty());
    assert_eq!(
        HistoryPage::from_response(200, b"{}").unwrap().history_id,
        None
    );
}

#[test]
fn message_list() {
    let body = br#"{"messages": [{"id": "m2", "threadId": "t"}, {"id": "m1", "threadId": "t"}], "resultSizeEstimate": 2}"#;
    let messages = messages_from_response(200, body).unwrap();
    assert_eq!(
        messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["m2", "m1"]
    );
    assert!(
        messages_from_response(200, br#"{"resultSizeEstimate": 0}"#)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn message_metadata_payload() {
    let json = r#"{
      "id": "19a1b2c3d4e5f607",
      "threadId": "19a1b2c3d4e5f607",
      "labelIds": ["UNREAD", "IMPORTANT", "CATEGORY_UPDATES", "INBOX"],
      "snippet": "",
      "payload": {
        "partId": "", "mimeType": "text/plain", "filename": "",
        "headers": [
          {"name": "Date", "value": "Wed, 14 Oct 2026 10:19:03 +0900"},
          {"name": "SUBJECT", "value": " =?UTF-8?Q?AuthCode:_482913?= "}
        ]
      },
      "sizeEstimate": 3051,
      "historyId": "9876600",
      "internalDate": "1791940743123"
    }"#;
    let message = MessageMetadata::from_response(200, json.as_bytes()).unwrap();
    assert_eq!(message.id, "19a1b2c3d4e5f607");
    assert_eq!(
        message.label_ids,
        ["UNREAD", "IMPORTANT", "CATEGORY_UPDATES", "INBOX"]
    );
    assert_eq!(
        message.internal_date,
        Some(UNIX_EPOCH + Duration::from_millis(1_791_940_743_123))
    );
    assert_eq!(gmail_subject(&message).as_deref(), Some("AuthCode: 482913"));
    assert_eq!(
        message.header("date"),
        Some("Wed, 14 Oct 2026 10:19:03 +0900")
    );
    assert_eq!(message.header("From"), None);
}

#[test]
fn sparse_metadata() {
    let message =
        MessageMetadata::from_response(200, br#"{"id": "m", "internalDate": 1791940743000}"#)
            .unwrap();
    assert_eq!(
        message.internal_date,
        Some(UNIX_EPOCH + Duration::from_secs(1_791_940_743))
    );
    assert!(message.label_ids.is_empty());
    assert!(message.headers.is_empty());
    assert_eq!(gmail_subject(&message), None);
    for body in [
        &br#"{"id": "m", "internalDate": "-5"}"#[..],
        br#"{"id": "m", "internalDate": "12x"}"#,
    ] {
        assert_eq!(
            MessageMetadata::from_response(200, body)
                .unwrap()
                .internal_date,
            None
        );
    }
    assert!(
        MessageMetadata::from_response(200, br#"{"id": "m", "payload": {}}"#)
            .unwrap()
            .headers
            .is_empty()
    );
    assert!(MessageMetadata::from_response(200, br#"{"labelIds": []}"#).is_err());
}

#[test]
fn only_safe_identifiers_go_into_a_request_path() {
    let long = "a".repeat(513);
    for (value, safe) in [
        ("19a1b2c3d4e5f607", true),
        ("abc-DEF_09", true),
        ("", false),
        ("../profile", false),
        ("a/b", false),
        ("a b", false),
        ("a%2F", false),
        (long.as_str(), false),
    ] {
        assert_eq!(message_metadata(value).is_some(), safe, "{value}");
        assert_eq!(trash(value).is_some(), safe, "{value}");
    }
}

#[test]
fn history_ids() {
    for (value, valid) in [
        ("1", true),
        ("18446744073709551615", true),
        ("18446744073709551616", false),
        ("", false),
        ("-1", false),
        ("1 ", false),
        ("0x10", false),
    ] {
        assert_eq!(is_valid_history_id(value), valid, "{value:?}");
    }
}

fn classify(status: u16, body: &str) -> Option<ApiError> {
    ApiError::classify(status, body.as_bytes())
}

#[test]
fn success_is_no_error() {
    assert_eq!(classify(200, ""), None);
    assert_eq!(classify(204, ""), None);
}

#[test]
fn status_classes() {
    let unauthorized = r#"{"error": {"code": 401, "message": "Request had invalid authentication credentials.", "status": "UNAUTHENTICATED"}}"#;
    assert_eq!(classify(401, unauthorized), Some(ApiError::Unauthorized));
    let not_found = r#"{"error": {"code": 404, "message": "Requested entity was not found.", "status": "NOT_FOUND"}}"#;
    assert_eq!(classify(404, not_found), Some(ApiError::NotFound));
    assert_eq!(classify(429, ""), Some(ApiError::RateLimited));
    assert_eq!(classify(500, ""), Some(ApiError::Server { status: 500 }));
    assert_eq!(
        classify(503, "<html>"),
        Some(ApiError::Server { status: 503 })
    );
    let invalid = r#"{"error": {"code": 400, "status": "INVALID_ARGUMENT"}}"#;
    assert_eq!(
        classify(400, invalid),
        Some(ApiError::Server { status: 400 })
    );
}

#[test]
fn forbidden_reasons() {
    let forbidden = |reason: &str| {
        Some(ApiError::Forbidden {
            reason: reason.into(),
        })
    };
    for (body, expected) in [
        (
            r#"{"error": {"code": 403, "errors": [{"domain": "usageLimits", "reason": "rateLimitExceeded"}], "status": "PERMISSION_DENIED"}}"#,
            Some(ApiError::RateLimited),
        ),
        (
            r#"{"error": {"errors": [{"reason": "userRateLimitExceeded"}]}}"#,
            Some(ApiError::RateLimited),
        ),
        (
            r#"{"error": {"code": 403, "errors": [{"domain": "global", "reason": "insufficientPermissions"}], "status": "PERMISSION_DENIED"}}"#,
            forbidden("insufficientPermissions"),
        ),
        (
            r#"{"error": {"code": 403, "status": "PERMISSION_DENIED", "details": [{"@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "ACCESS_TOKEN_SCOPE_INSUFFICIENT"}]}}"#,
            forbidden("ACCESS_TOKEN_SCOPE_INSUFFICIENT"),
        ),
        (
            r#"{"error": {"code": 403, "status": "PERMISSION_DENIED", "details": [{"reason": "SERVICE_DISABLED"}], "errors": [{"reason": "accessNotConfigured"}]}}"#,
            forbidden("accessNotConfigured"),
        ),
        (
            r#"{"error": {"code": 403, "status": "PERMISSION_DENIED"}}"#,
            forbidden("PERMISSION_DENIED"),
        ),
        (
            r#"{"error": {"errors": [{"reason": "bad reason <x>"}]}}"#,
            forbidden("badreasonx"),
        ),
        (
            r#"{"error": {"errors": [{"reason": "!!"}], "status": "PERMISSION_DENIED"}}"#,
            forbidden("PERMISSION_DENIED"),
        ),
        ("", forbidden("forbidden")),
        ("not json", forbidden("forbidden")),
    ] {
        assert_eq!(classify(403, body), expected, "{body}");
    }
}
