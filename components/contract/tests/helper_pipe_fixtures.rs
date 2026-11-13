//! `fixtures/helper-pipe/` pins the JSON of every message on the Windows Helper's pipe. Rust
//! decodes what the app sends and encodes what the app receives; the C# side checks its types the
//! opposite way against the same files.

use fvm_contract::helper_pipe::*;
use serde_json::Value;

const ATTEMPT: &str = "E621E1F8-C36C-495A-93FC-0C247A3E6E5F";

#[test]
fn requests_from_the_app() {
    let requests: Vec<PipeRequest> =
        serde_json::from_str(include_str!("fixtures/helper-pipe/requests.json")).unwrap();
    assert_eq!(
        requests,
        [
            PipeRequest::Build {
                id: 1,
                app: "20261112.1".into(),
            },
            PipeRequest::Connect {
                id: 2,
                json: format!(r#"{{"attemptID":"{ATTEMPT}"}}"#),
            },
            PipeRequest::SubmitOtp {
                id: 3,
                attempt: ATTEMPT.into(),
                code: "123456".into(),
            },
            PipeRequest::Disconnect {
                id: 4,
                attempt: ATTEMPT.into(),
                reason: "userDisconnect".into(),
            },
            PipeRequest::Status { id: 5 },
            PipeRequest::ExitWhenIdle { id: 6 },
            PipeRequest::ReadBypassRoutes { id: 7 },
            PipeRequest::WriteBypassRoutes {
                id: 8,
                json: r#"{"routes":[]}"#.into(),
            },
        ]
    );
    assert_eq!(
        requests.iter().map(PipeRequest::id).collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6, 7, 8]
    );
}

#[test]
fn messages_to_the_app() {
    let messages = [
        PipeMessage::Build {
            id: 1,
            build: "20261112.1".into(),
        },
        PipeMessage::Reply { id: 2, json: None },
        PipeMessage::Reply {
            id: 5,
            json: Some(r#"{"childAlive":false}"#.into()),
        },
        PipeMessage::ReplyFlag {
            id: 6,
            value: false,
        },
        PipeMessage::Event {
            json: format!(r#"{{"attemptID":"{ATTEMPT}"}}"#),
        },
        PipeMessage::Output { json: "[]".into() },
    ];
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/helper-pipe/messages.json")).unwrap();
    assert_eq!(serde_json::to_value(messages).unwrap(), fixture);
}

#[test]
fn debug_output_hides_the_password_and_the_otp() {
    let connect = PipeRequest::Connect {
        id: 2,
        json: r#"{"password":"hunter2"}"#.into(),
    };
    let otp = PipeRequest::SubmitOtp {
        id: 3,
        attempt: ATTEMPT.into(),
        code: "123456".into(),
    };
    let shown = format!("{connect:?} {otp:?}");
    assert!(
        !shown.contains("hunter2") && !shown.contains("123456"),
        "{shown}"
    );
}
