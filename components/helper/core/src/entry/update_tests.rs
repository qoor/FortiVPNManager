use std::fs;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use fvm_contract::app_helper::{
    HelperErrorCode, PreparedUpdateReply, UpdateDiscardReply, UpdateInstallReply,
    UpdatePrepareReply, UpdatePrepareRequest, UpdateRefusal, Version,
};
use serde::de::DeserializeOwned;

use super::*;
use crate::auto_update::tests::{Setup, installer_name, names};
use crate::secrets::{SecretStore, StoredSecret};
use crate::session::Timing;
use crate::session::backend::{Backend, Link, SessionEnd, SessionRequest};

const APP: u64 = 1;

/// The entry's update requests reach no VPN session.
struct NoBackend;

impl Backend for NoBackend {
    fn init(&self) {}

    fn name(&self) -> &'static str {
        "none"
    }

    fn version(&self) -> &str {
        "0"
    }

    fn check(&self, _request: &RunRequest) -> Result<(), String> {
        Err("no VPN client in these tests".to_owned())
    }

    fn run(&self, _request: SessionRequest, _link: &Link) -> SessionEnd {
        unreachable!("no session starts in these tests")
    }
}

struct NoSecrets;

impl SecretStore for NoSecrets {
    fn save(&self, _name: &str, _secret: &[u8]) -> Result<(), String> {
        Err("no secret store in these tests".to_owned())
    }

    fn load_all(&self) -> Result<Vec<StoredSecret>, String> {
        Ok(Vec::new())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        Err(format!("no secret named {name}"))
    }
}

struct Test {
    setup: Setup,
    inbox: Inbox,
    messages: Receiver<FromEntry>,
}

struct Answer {
    id: u64,
    json: Option<String>,
    log: Vec<(LogLevel, String)>,
}

impl Answer {
    fn payload<T: DeserializeOwned>(&self) -> T {
        serde_json::from_str(self.json.as_deref().expect("a reply with data")).unwrap()
    }
}

impl Test {
    fn start(setup: Setup) -> Self {
        let installer = setup.another_installer();
        Self::start_with(setup, installer)
    }

    fn start_with(setup: Setup, installer: Installer) -> Self {
        let sessions = SessionManager::new(
            Arc::new(NoBackend),
            Arc::new(NoSecrets),
            setup.folder.join("run"),
            Timing::default(),
        );
        let (sender, events) = mpsc::channel();
        let inbox = Inbox(sender.clone());
        let (to_test, messages) = mpsc::channel();
        let os = OsSide(Box::new(move |message: &FromEntry| {
            let _ = to_test.send(message.clone());
        }));
        thread::spawn(move || serve(sessions, installer, os, sender, events));
        let test = Self {
            setup,
            inbox,
            messages,
        };
        loop {
            match test.next() {
                FromEntry::Log { .. } => {}
                FromEntry::Ready {} => return test,
                other => panic!("{other:?} before ready"),
            }
        }
    }

    fn next(&self) -> FromEntry {
        self.messages
            .recv_timeout(Duration::from_secs(30))
            .expect("a message from the entry")
    }

    fn ask(&self, id: u64, request: AppRequest) {
        self.inbox.deliver(ToEntry::Request {
            id,
            app: APP,
            request,
        });
    }

    fn answer(&self) -> Answer {
        let mut log = Vec::new();
        loop {
            match self.next() {
                FromEntry::Log {
                    level,
                    category: LogCategory::Helper,
                    message,
                } => log.push((level, message)),
                FromEntry::Reply { id, json } => return Answer { id, json, log },
                other => panic!("{other:?} instead of a reply"),
            }
        }
    }

    fn answer_to<T: DeserializeOwned>(&self, id: u64, request: AppRequest) -> (T, Answer) {
        self.ask(id, request);
        let answer = self.answer();
        assert_eq!(answer.id, id);
        (answer.payload(), answer)
    }

    fn prepared_version(&self, id: u64) -> PreparedUpdateReply {
        self.answer_to(id, AppRequest::ReadPreparedUpdate {}).0
    }
}

fn prepare(request: &UpdatePrepareRequest) -> AppRequest {
    AppRequest::PrepareUpdate {
        json: serde_json::to_string(request).unwrap(),
    }
}

fn version_of(name: &str) -> Version {
    let (version, _) = name
        .strip_prefix("FortiVPNManager-")
        .and_then(|rest| rest.rsplit_once('.'))
        .unwrap();
    Version::parse(version).unwrap()
}

#[test]
fn unreadable_update_requests_and_relative_paths_are_invalid() {
    let test = Test::start(Setup::new());
    for (id, request) in [
        (
            1,
            AppRequest::PrepareUpdate {
                json: "{}".to_owned(),
            },
        ),
        (
            2,
            AppRequest::InstallUpdate {
                json: "{}".to_owned(),
            },
        ),
    ] {
        test.ask(id, request);
        let answer = test.answer();
        assert_eq!(answer.id, id);
        assert!(answer.log.is_empty(), "{:?}", answer.log);
        let json = answer.json.unwrap();
        let code = match id {
            1 => {
                let UpdatePrepareReply::Failure { error } = serde_json::from_str(&json).unwrap()
                else {
                    panic!("{json}");
                };
                error.code
            }
            _ => {
                let UpdateInstallReply::Failure { error } = serde_json::from_str(&json).unwrap()
                else {
                    panic!("{json}");
                };
                error.code
            }
        };
        assert_eq!(code, HelperErrorCode::InvalidRequest);
    }

    let name = test.setup.newer();
    let request = test.setup.release(&name);
    for (id, relative) in [
        (
            3,
            UpdatePrepareRequest {
                installer_path: name.clone(),
                ..request.clone()
            },
        ),
        (
            4,
            UpdatePrepareRequest {
                checksums_path: "download/SHA256SUMS.txt".to_owned(),
                ..request.clone()
            },
        ),
        (
            5,
            UpdatePrepareRequest {
                signature_path: String::new(),
                ..request.clone()
            },
        ),
    ] {
        let (reply, answer) = test.answer_to(id, prepare(&relative));
        let UpdatePrepareReply::Failure { error } = reply else {
            panic!("{reply:?}");
        };
        assert_eq!(error.code, HelperErrorCode::InvalidRequest);
        assert_eq!(
            answer.log,
            [(
                LogLevel::Error,
                format!("update prepare invalid: {}", error.message)
            )]
        );
        assert!(error.message.ends_with(" is not an absolute path"));
    }
    assert!(!test.setup.records().exists());

    let missing = UpdatePrepareRequest {
        installer_path: test
            .setup
            .download("FortiVPNManager-99.0.0.pkg")
            .display()
            .to_string(),
        ..request
    };
    let (reply, answer) = test.answer_to(6, prepare(&missing));
    let UpdatePrepareReply::Refused { reason, message } = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(reason, UpdateRefusal::FilesUnreadable);
    assert!(
        message.starts_with("could not read the installer: "),
        "{message}"
    );
    assert_eq!(
        answer.log,
        [(
            LogLevel::Notice,
            format!("update prepare refused (FilesUnreadable): {message}")
        )]
    );
}

#[test]
fn an_update_is_prepared_answered_and_discarded() {
    let test = Test::start(Setup::new());
    assert_eq!(
        test.prepared_version(1),
        PreparedUpdateReply::NotPrepared {}
    );

    let name = test.setup.newer();
    let version = version_of(&name);
    let (reply, answer): (UpdatePrepareReply, _) =
        test.answer_to(2, prepare(&test.setup.release(&name)));
    assert_eq!(reply, UpdatePrepareReply::Prepared { version });
    assert_eq!(
        answer.log,
        [(
            LogLevel::Notice,
            format!("update prepared: {name} verified")
        )]
    );
    assert_eq!(names(&test.setup.records()), ["prepared"]);
    assert_eq!(
        test.prepared_version(3),
        PreparedUpdateReply::Prepared { version }
    );

    let (reply, answer): (UpdateDiscardReply, _) =
        test.answer_to(4, AppRequest::DiscardPreparedUpdate {});
    assert_eq!(reply, UpdateDiscardReply::Discarded {});
    assert_eq!(
        answer.log,
        [(LogLevel::Notice, "prepared update discarded".to_owned())]
    );
    assert_eq!(
        test.prepared_version(5),
        PreparedUpdateReply::NotPrepared {}
    );

    let install = AppRequest::InstallUpdate {
        json: r#"{"installID":42}"#.to_owned(),
    };
    let (reply, answer) = test.answer_to(6, install);
    let UpdateInstallReply::Refused { reason, message } = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(reason, UpdateRefusal::NothingPrepared);
    assert_eq!(
        answer.log,
        [(
            LogLevel::Notice,
            format!("update install 42 refused (NothingPrepared): {message}")
        )]
    );
    assert_eq!(names(&test.setup.records()), ["prepared"]);
}

#[test]
fn a_discard_while_the_installer_runs_is_busy_and_removes_nothing() {
    let mut setup = Setup::new();
    let name = setup.newer();
    setup.installer.prepare(&setup.release(&name)).unwrap();
    let (installer, stand_in) = setup.another_installer_while_installing();

    let test = Test::start_with(setup, installer);
    let (reply, answer): (UpdateDiscardReply, _) =
        test.answer_to(1, AppRequest::DiscardPreparedUpdate {});
    let UpdateDiscardReply::Failure { error } = reply else {
        panic!("{reply:?}");
    };
    assert_eq!(error.code, HelperErrorCode::Busy);
    assert_eq!(
        answer.log,
        [(
            LogLevel::Notice,
            format!("discarding the prepared update refused: {}", error.message)
        )]
    );
    assert_eq!(names(&test.setup.staging()).len(), 3);
    assert_eq!(
        test.prepared_version(2),
        PreparedUpdateReply::Prepared {
            version: version_of(&name)
        }
    );
    fs::write(stand_in, b"").unwrap();
}

#[test]
fn a_prepared_update_outlives_the_helper() {
    let mut setup = Setup::new();
    let name = setup.newer();
    let request = setup.release(&name);
    setup.installer.prepare(&request).unwrap();

    let test = Test::start(setup);
    assert_eq!(
        test.prepared_version(1),
        PreparedUpdateReply::Prepared {
            version: version_of(&name)
        }
    );
    assert_eq!(names(&test.setup.staging()).len(), 3);
}

#[test]
fn a_prepared_update_that_is_not_newer_is_answered_as_not_prepared_and_removed() {
    let setup = Setup::new();
    let installed = installer_name(setup.current());
    fs::create_dir_all(setup.staging()).unwrap();
    fs::write(setup.staging().join(&installed), b"").unwrap();

    let test = Test::start(setup);
    let (reply, answer): (PreparedUpdateReply, _) =
        test.answer_to(1, AppRequest::ReadPreparedUpdate {});
    assert_eq!(reply, PreparedUpdateReply::NotPrepared {});
    let [(LogLevel::Notice, line)] = &answer.log[..] else {
        panic!("{:?}", answer.log);
    };
    assert!(line.starts_with("removed the prepared update"), "{line}");
    assert_eq!(names(&test.setup.staging()), Vec::<String>::new());
}

#[test]
fn update_requests_are_answered_in_the_order_they_came() {
    let test = Test::start(Setup::new());
    let first = test.setup.newer_by(1);
    let first_request = test.setup.release(&first);
    test.answer_to::<UpdatePrepareReply>(1, prepare(&first_request));

    let second = test.setup.newer_by(2);
    let second_request = test.setup.release(&second);
    test.ask(2, AppRequest::DiscardPreparedUpdate {});
    test.ask(3, prepare(&second_request));
    test.ask(4, AppRequest::ReadPreparedUpdate {});
    test.ask(5, prepare(&second_request));
    test.ask(6, AppRequest::DiscardPreparedUpdate {});
    test.ask(7, AppRequest::ReadPreparedUpdate {});

    let answers: Vec<Answer> = (0..6).map(|_| test.answer()).collect();
    let ids: Vec<u64> = answers.iter().map(|answer| answer.id).collect();
    assert_eq!(ids, [2, 3, 4, 5, 6, 7]);
    assert_eq!(
        answers[0].payload::<UpdateDiscardReply>(),
        UpdateDiscardReply::Discarded {}
    );
    let second_version = version_of(&second);
    assert_eq!(
        answers[1].payload::<UpdatePrepareReply>(),
        UpdatePrepareReply::Prepared {
            version: second_version
        }
    );
    assert_eq!(
        answers[2].payload::<PreparedUpdateReply>(),
        PreparedUpdateReply::Prepared {
            version: second_version
        }
    );
    assert_eq!(
        answers[3].payload::<UpdatePrepareReply>(),
        UpdatePrepareReply::Prepared {
            version: second_version
        }
    );
    assert_eq!(
        answers[4].payload::<UpdateDiscardReply>(),
        UpdateDiscardReply::Discarded {}
    );
    assert_eq!(
        answers[5].payload::<PreparedUpdateReply>(),
        PreparedUpdateReply::NotPrepared {}
    );
}

#[test]
fn a_long_prepare_does_not_hold_up_the_vpn_requests() {
    let test = Test::start(Setup::new());
    let name = test.setup.newer();
    let contents = vec![0x5a; 64 << 20];
    let request = test.setup.release_of(&name, &contents);

    test.ask(1, prepare(&request));
    test.inbox
        .deliver(ToEntry::AppConnected { app: APP, uid: 501 });
    test.ask(2, AppRequest::Status {});
    test.ask(
        3,
        AppRequest::Disconnect {
            attempt: "E621E1F8-C36C-495A-93FC-0C247A3E6E5F".to_owned(),
            reason: "userDisconnect".to_owned(),
        },
    );

    let mut order = Vec::new();
    while order.len() < 4 {
        match test.next() {
            FromEntry::Admitted { app, accept } => {
                assert_eq!((app, accept), (APP, true));
                order.push("admitted".to_owned());
            }
            FromEntry::Reply { id, json } => {
                if id == 1 {
                    let reply: UpdatePrepareReply =
                        serde_json::from_str(json.as_deref().unwrap()).unwrap();
                    assert_eq!(
                        reply,
                        UpdatePrepareReply::Prepared {
                            version: version_of(&name)
                        }
                    );
                }
                order.push(format!("reply {id}"));
            }
            FromEntry::Log { .. } => {}
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(order, ["admitted", "reply 2", "reply 3", "reply 1"]);
    assert_eq!(
        fs::metadata(test.setup.staging().join(&name))
            .unwrap()
            .len(),
        64 << 20
    );
}
