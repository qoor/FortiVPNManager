//! The Helper over its pipe, as an app sees it: the build check and what it unlocks, the answers it
//! gives itself, answers going to the connection that asked, one route request at a time, the
//! message size limit, bad messages, connections that do not read, and the exit on a termination
//! request. The tests write and read
//! the JSON on the pipe as the app's C# side does. Each test runs its own Helper on its own pipe,
//! open to every authenticated user, the one running the tests among them, instead of only the
//! install-time user. They get the install-time user's rights and the right to create the pipe's
//! instances, which the Helper needs since it runs as the same user here; the service's own test
//! checks that only the install-time user gets in, with those rights alone.
#![cfg(windows)]

use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use fvm_contract::app_helper::{BypassRoutesReply, HelperError, HelperErrorCode, HelperStatus};
use fvm_contract::helper_pipe::MAX_MESSAGE_BYTES;
use fvm_contract::stream;
use fvm_helper_entry_windows::{BUILD, Exit, Helper, Options};
use interprocess::os::windows::named_pipe::{DuplexPipeStream, pipe_mode};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

struct Test {
    helper: Helper,
    pipe: String,
    log: Arc<Mutex<Vec<String>>>,
}

fn start() -> Test {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "fvm-helper-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let pipe = format!(r"\\.\pipe\{name}");
    let log = Arc::new(Mutex::new(Vec::new()));
    let lines = Arc::clone(&log);
    let helper = Helper::start(
        Options {
            pipe_name: pipe.clone(),
            security: "D:P(A;;GA;;;SY)(A;;0x12019f;;;AU)".to_owned(),
            run_directory: env::temp_dir().join(name),
        },
        Arc::new(move |_, _, message| lines.lock().unwrap().push(message.to_owned())),
    )
    .unwrap();
    Test { helper, pipe, log }
}

impl Test {
    fn connect(&self) -> DuplexPipeStream<pipe_mode::Bytes> {
        DuplexPipeStream::connect_by_path(self.pipe.as_str()).unwrap()
    }
}

fn ask(connection: &DuplexPipeStream<pipe_mode::Bytes>, request: Value) -> Value {
    stream::send(&mut &*connection, &request).unwrap();
    stream::receive(&mut &*connection).unwrap()
}

fn build_check(connection: &DuplexPipeStream<pipe_mode::Bytes>, app: &str) {
    let answer = ask(connection, json!({ "build": { "id": 1, "app": app } }));
    assert_eq!(answer, json!({ "build": { "id": 1, "build": BUILD } }));
}

/// The payload of `answer`, which must be the reply to request `id`.
fn reply<T: DeserializeOwned>(answer: &Value, id: u64) -> T {
    assert_eq!(answer["reply"]["id"], id, "{answer}");
    let json = answer["reply"]["json"]
        .as_str()
        .unwrap_or_else(|| panic!("a reply without a payload: {answer}"));
    serde_json::from_str(json).unwrap()
}

fn connect_request(id: u64) -> Value {
    json!({ "connect": { "id": id, "json": "{}" } })
}

#[test]
fn only_a_connection_of_the_same_build_may_connect() {
    let test = start();
    let connection = test.connect();
    let refused: HelperError = reply(&ask(&connection, connect_request(2)), 2);
    assert_eq!(refused.code, HelperErrorCode::NotAuthorized);

    build_check(&connection, "another build");
    let refused: HelperError = reply(&ask(&connection, connect_request(3)), 3);
    assert_eq!(refused.code, HelperErrorCode::NotAuthorized);

    build_check(&connection, BUILD);
    // The entry got it: it judges the request itself.
    let judged: HelperError = reply(&ask(&connection, connect_request(4)), 4);
    assert_eq!(judged.code, HelperErrorCode::InvalidRequest, "{judged:?}");
}

#[test]
fn the_helper_never_exits_when_idle() {
    let test = start();
    let connection = test.connect();
    assert_eq!(
        ask(&connection, json!({ "exitWhenIdle": { "id": 7 } })),
        json!({ "replyFlag": { "id": 7, "value": false } })
    );
    // Still serving.
    build_check(&connection, BUILD);
}

#[test]
fn each_connection_gets_the_answers_to_its_own_requests() {
    let test = start();
    let first = test.connect();
    let second = test.connect();
    for connection in [&first, &second] {
        stream::send(&mut &*connection, &json!({ "status": { "id": 1 } })).unwrap();
    }
    for connection in [&first, &second] {
        let answer: Value = stream::receive(&mut &*connection).unwrap();
        let status: HelperStatus = reply(&answer, 1);
        assert!(!status.client_running);
    }
}

#[test]
fn the_bypass_routes_are_read_after_the_build_check() {
    let test = start();
    let connection = test.connect();
    let answer = ask(&connection, json!({ "readBypassRoutes": { "id": 2 } }));
    let BypassRoutesReply::Failure { error } = reply(&answer, 2) else {
        panic!("read without the build check: {answer}");
    };
    assert_eq!(error.code, HelperErrorCode::NotAuthorized);

    build_check(&connection, BUILD);
    let answer = ask(&connection, json!({ "readBypassRoutes": { "id": 3 } }));
    // The test PC has a wired network.
    let BypassRoutesReply::Success { snapshot } = reply(&answer, 3) else {
        panic!("{answer}");
    };
    assert!(snapshot.service_id.starts_with('{'), "{snapshot:?}");
    assert!(snapshot.router.parse::<std::net::Ipv4Addr>().is_ok());
}

#[test]
fn a_connection_has_one_route_request_at_a_time() {
    let test = start();
    let connection = test.connect();
    build_check(&connection, BUILD);
    for id in [2, 3] {
        stream::send(
            &mut &connection,
            &json!({ "readBypassRoutes": { "id": id } }),
        )
        .unwrap();
    }
    // The second is refused at once; the first is answered once PowerShell is done.
    let refused: Value = stream::receive(&mut &connection).unwrap();
    let BypassRoutesReply::Failure { error } = reply(&refused, 3) else {
        panic!("{refused}");
    };
    assert_eq!(error.code, HelperErrorCode::InvalidRequest);
    let answered: Value = stream::receive(&mut &connection).unwrap();
    let first: BypassRoutesReply = reply(&answered, 2);
    assert!(
        matches!(first, BypassRoutesReply::Success { .. }),
        "{answered}"
    );
    // Once it is answered, the next one is taken.
    let next = ask(&connection, json!({ "readBypassRoutes": { "id": 4 } }));
    let next: BypassRoutesReply = reply(&next, 4);
    assert!(
        matches!(next, BypassRoutesReply::Success { .. }),
        "{next:?}"
    );
}

#[test]
fn a_message_over_the_limit_ends_the_connection() {
    let test = start();
    let connection = test.connect();
    let header = (MAX_MESSAGE_BYTES + 1).to_be_bytes();
    std::io::Write::write_all(&mut &connection, &header).unwrap();
    let ended = stream::receive::<Value>(&mut &connection);
    assert!(ended.is_err(), "{ended:?}");
    assert!(
        test.log
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("over the limit")),
        "{:?}",
        test.log.lock().unwrap()
    );
    // The Helper still serves others.
    build_check(&test.connect(), BUILD);
}

#[test]
fn a_message_that_is_not_json_ends_the_connection_and_is_logged() {
    let test = start();
    for payload in [&b""[..], b"{\"status\":{\"id\""] {
        let connection = test.connect();
        let length = u32::try_from(payload.len()).unwrap().to_be_bytes();
        std::io::Write::write_all(&mut &connection, &length).unwrap();
        std::io::Write::write_all(&mut &connection, payload).unwrap();
        let ended = stream::receive::<Value>(&mut &connection);
        assert!(ended.is_err(), "{ended:?}");
    }
    let closed = test
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|line| line.starts_with("closing connection"))
        .count();
    assert_eq!(closed, 2, "{:?}", test.log.lock().unwrap());
}

#[test]
fn a_connection_that_does_not_read_its_answers_is_closed() {
    let test = start();
    let connection = test.connect();
    // Asks without ever reading, so the answers pile up until the Helper closes the connection.
    // Asking may block once the Helper stops reading, so it happens on a thread of its own.
    thread::spawn(move || {
        for id in 0..1000 {
            let request = json!({ "build": { "id": id, "app": BUILD } });
            if stream::send(&mut &connection, &request).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let closed = || {
        test.log
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("does not read"))
    };
    while !closed() {
        assert!(Instant::now() < deadline, "{:?}", test.log.lock().unwrap());
        thread::sleep(Duration::from_millis(100));
    }
    // The Helper still serves others.
    build_check(&test.connect(), BUILD);
}

#[test]
fn a_termination_request_without_a_run_exits_after_draining() {
    let test = start();
    let connection = test.connect();
    build_check(&connection, BUILD);
    test.helper.terminate();
    assert_eq!(test.helper.wait_for_exit(), Exit::AfterDraining);
    test.helper.drain(Duration::from_secs(1));
}
