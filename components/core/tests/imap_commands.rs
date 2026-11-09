//! IMAP commands, UID sets and login rejections (SPEC F-OT-03).

use base64::Engine;
use fvm_contract::app_internal::SecretText;
use fvm_core::imap::{
    Command, CommandError, IDLE_DONE, Parser, Response, Status, TagGenerator, UidSet,
    is_credential_rejection,
};

fn wire(command: &Command) -> String {
    String::from_utf8(command.encode("A0001").unwrap()).unwrap()
}

fn plain(username: &str, password: &str) -> Command {
    Command::AuthenticatePlain {
        username: username.into(),
        password: SecretText(password.into()),
    }
}

#[test]
fn the_tag_generator_counts_with_at_least_four_digits() {
    let mut tags = TagGenerator::default();
    assert_eq!(tags.next_tag(), "A0001");
    assert_eq!(tags.next_tag(), "A0002");
    let mut copy = tags.clone();
    assert_eq!(copy.next_tag(), "A0003");
    assert_eq!(tags.next_tag(), "A0003");
    for _ in 3..9_999 {
        tags.next_tag();
    }
    assert_eq!(tags.next_tag(), "A10000");
}

#[test]
fn authenticate_plain_uses_the_sasl_initial_response() {
    assert_eq!(
        wire(&plain("user@example.com", "abcd efgh ijkl mnop")),
        "A0001 AUTHENTICATE PLAIN AHVzZXJAZXhhbXBsZS5jb20AYWJjZCBlZmdoIGlqa2wgbW5vcA==\r\n"
    );
}

#[test]
fn authenticate_plain_encodes_utf8_credentials() {
    let username = "\u{C0AC}\u{C6A9}\u{C790}@example.com";
    let line = wire(&plain(username, "p\u{E4}ssw\u{F6}rd"));
    let encoded = line
        .strip_prefix("A0001 AUTHENTICATE PLAIN ")
        .unwrap()
        .strip_suffix("\r\n")
        .unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    let mut expected = vec![0];
    expected.extend(username.as_bytes());
    expected.push(0);
    expected.extend("p\u{E4}ssw\u{F6}rd".as_bytes());
    assert_eq!(decoded, expected);
}

#[test]
fn authenticate_plain_rejects_invalid_credentials() {
    for command in [
        plain("", "pw"),
        plain("us\0er", "pw"),
        plain("user", "p\0w"),
    ] {
        assert_eq!(command.encode("A1"), Err(CommandError::InvalidCredentials));
    }
}

#[test]
fn debug_output_redacts_the_password() {
    assert!(!format!("{:?}", plain("user@example.com", "s3cret-app-pw")).contains("s3cret"));
}

#[test]
fn commands_serialize_exactly() {
    for (command, expected) in [
        (Command::List, r#"A0001 LIST "" "*""#),
        (
            Command::Select {
                mailbox: "INBOX".into(),
            },
            r#"A0001 SELECT "INBOX""#,
        ),
        (
            Command::Select {
                mailbox: "[Gmail]/&yATMtLz0rQDVaA-".into(),
            },
            r#"A0001 SELECT "[Gmail]/&yATMtLz0rQDVaA-""#,
        ),
        (
            Command::Select {
                mailbox: r#"odd "name" \ here"#.into(),
            },
            r#"A0001 SELECT "odd \"name\" \\ here""#,
        ),
        (
            Command::Examine {
                mailbox: "[Gmail]/&wqTTONVo-".into(),
            },
            r#"A0001 EXAMINE "[Gmail]/&wqTTONVo-""#,
        ),
        (
            Command::UidSearch(UidSet::starting_at(42)),
            "A0001 UID SEARCH UID 42:*",
        ),
        (
            Command::UidSearchSubject("AuthCode".into()),
            r#"A0001 UID SEARCH SUBJECT "AuthCode""#,
        ),
        (
            Command::UidFetchHeaders(UidSet::new([10, 5, 6, 7]).unwrap()),
            "A0001 UID FETCH 5:7,10 (UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT DATE)])",
        ),
        (
            Command::UidFetchHeaders(UidSet::starting_at(300)),
            "A0001 UID FETCH 300:* (UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT DATE)])",
        ),
        (Command::Idle, "A0001 IDLE"),
        (
            Command::UidMove {
                uids: UidSet::new([9, 7]).unwrap(),
                mailbox: "[Gmail]/&1zTJwNG1-".into(),
            },
            r#"A0001 UID MOVE 7,9 "[Gmail]/&1zTJwNG1-""#,
        ),
        (Command::Logout, "A0001 LOGOUT"),
    ] {
        assert_eq!(wire(&command), format!("{expected}\r\n"));
    }
}

#[test]
fn idle_done_is_untagged() {
    assert_eq!(IDLE_DONE, b"DONE\r\n");
}

#[test]
fn mailbox_names_that_cannot_be_quoted_are_rejected() {
    for name in [
        "Trash\r\nA2 LOGOUT",
        "Trash\n",
        "Tr\0sh",
        "[Gmail]/\u{D734}\u{C9C0}\u{D1B5}",
    ] {
        assert_eq!(
            Command::Select {
                mailbox: name.into()
            }
            .encode("A1"),
            Err(CommandError::InvalidMailboxName)
        );
        let mov = Command::UidMove {
            uids: UidSet::new([1]).unwrap(),
            mailbox: name.into(),
        };
        assert_eq!(mov.encode("A1"), Err(CommandError::InvalidMailboxName));
    }
}

#[test]
fn a_search_text_with_a_line_break_is_rejected() {
    assert_eq!(
        Command::UidSearchSubject("AuthCode\r\nA2 LOGOUT".into()).encode("A1"),
        Err(CommandError::InvalidSearchText)
    );
}

#[test]
fn an_empty_or_zero_uid_set_is_refused() {
    assert_eq!(UidSet::new(Vec::<u32>::new()), None);
    assert_eq!(UidSet::new([0, 1]), None);
}

#[test]
fn explicit_uid_sets_are_compacted() {
    for (uids, expected) in [
        (vec![1], "1"),
        (vec![3, 1, 2], "1:3"),
        (vec![5, 5, 6, 9, 10, 11, 20], "5:6,9:11,20"),
        (vec![u32::MAX, 1], "1,4294967295"),
        (vec![u32::MAX - 1, u32::MAX], "4294967294:4294967295"),
    ] {
        assert_eq!(UidSet::new(uids).unwrap().to_string(), expected);
    }
}

#[test]
fn open_ended_uid_ranges() {
    assert_eq!(UidSet::starting_at(7).to_string(), "7:*");
    assert_eq!(UidSet::starting_at(0).to_string(), "1:*");
}

fn rejection(line: &str) -> bool {
    let responses = Parser::default()
        .feed(format!("{line}\r\n").as_bytes())
        .unwrap();
    match responses.as_slice() {
        [
            Response::Tagged {
                status: Status::No,
                code,
                text,
                ..
            },
        ] => is_credential_rejection(code.as_ref(), text),
        other => panic!("expected one tagged NO, got {other:?}"),
    }
}

#[test]
fn permanent_refusals_are_credential_rejections() {
    for line in [
        "A0001 NO [AUTHENTICATIONFAILED] Invalid credentials (Failure)",
        "A0001 NO [ALERT] Application-specific password required: https://support.google.com/accounts/answer/185833 (Failure)",
        "A0001 NO [ALERT] Please log in via your web browser: https://support.google.com/mail/accounts/answer/78754 (Failure)",
        // Google flagged the sign-in: retrying from here only repeats a suspicious login.
        "A0001 NO [WEBALERT https://accounts.google.com/ContinueSignIn?sarp=1&scc=1&plt=AKgnsbs] Web login required.",
        // IMAP disabled by the user or a Workspace admin.
        "A0001 NO [ALERT] Your account is not enabled for IMAP use. Please visit your Gmail settings page and enable your account for IMAP access. (Failure)",
        "A0001 NO [ALERT] IMAP access is disabled for your domain. Please contact your domain administrator for questions about this feature. (Failure)",
        "A0001 NO [AUTHORIZATIONFAILED] Not allowed",
        "A0001 NO [EXPIRED] Password expired",
        "A0001 NO Invalid credentials",
    ] {
        assert!(rejection(line), "{line}");
    }
}

#[test]
fn transient_refusals_are_not_credential_rejections() {
    for line in [
        "A0001 NO [UNAVAILABLE] Temporary System Problem. Try again later.",
        "A0001 NO [LIMIT] Too many connections",
        "A0001 NO [INUSE] Mailbox in use",
        "A0001 NO [SERVERBUG] Internal error",
        "A0001 NO [ALERT] Too many simultaneous connections. (Failure)",
        "A0001 NO [ALERT] Account exceeded command or bandwidth limits. (Failure)",
        "A0001 NO [ALERT] Temporary problem, please try again later",
    ] {
        assert!(!rejection(line), "{line}");
    }
}
