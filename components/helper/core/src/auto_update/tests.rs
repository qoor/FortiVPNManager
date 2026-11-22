use std::fmt::Write as _;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use std::{env, process};

use fvm_contract::app_helper::UpdateResult;
use ring::digest;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};

#[cfg(windows)]
use super::os::ProcessIdentity;
use super::*;

const ID: u64 = 13_835_058_055_282_163_713;
pub(crate) const CONTENTS: &[u8] = b"an installer";
const STAND_IN_STOP: &str = "FVM_TEST_STAND_IN_STOP";
const STAND_IN_READY: &str = "FVM_TEST_STAND_IN_READY";
const STAND_IN_START_LIMIT: Duration = Duration::from_secs(300);
const STAND_IN_EXIT_CODE: &str = "FVM_TEST_STAND_IN_EXIT_CODE";

pub(crate) struct Setup {
    pub(crate) folder: PathBuf,
    key: Ed25519KeyPair,
    pub(crate) installer: Installer,
    log: Log,
}

fn key_pair() -> Ed25519KeyPair {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest::digest(&digest::SHA256, bytes)
        .as_ref()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

pub(crate) fn installer_name(version: Version) -> String {
    format!("FortiVPNManager-{version}.{}", os::INSTALLER_EXTENSION)
}

fn line(hash: &str, name: &str) -> String {
    format!("{hash}  {name}\n")
}

impl Setup {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let folder = env::temp_dir().join(format!(
            "fvm-auto-update-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("download")).unwrap();
        let key = key_pair();
        let installer = Installer::new(folder.join("records"), Self::public_key_of(&key));
        Self {
            folder,
            key,
            installer,
            log: Arc::new(|_, _| {}),
        }
    }

    fn public_key_of(key: &Ed25519KeyPair) -> [u8; 32] {
        key.public_key().as_ref().try_into().unwrap()
    }

    pub(crate) fn another_installer(&self) -> Installer {
        Installer::new(self.records(), Self::public_key_of(&self.key))
    }

    pub(crate) fn another_installer_while_installing(&self) -> (Installer, PathBuf) {
        let mut installer = self.another_installer();
        let (child, stop) = start_stand_in(0);
        installer.runner = Some(runner(child));
        (installer, stop)
    }

    fn install(&mut self, install_id: u64) -> Result<Started, NotDone> {
        let log = Arc::clone(&self.log);
        self.installer.install(install_id, &log)
    }

    pub(crate) fn current(&self) -> Version {
        self.installer.current
    }

    pub(crate) fn download(&self, name: &str) -> PathBuf {
        self.folder.join("download").join(name)
    }

    pub(crate) fn newer(&self) -> String {
        self.newer_by(1)
    }

    pub(crate) fn newer_by(&self, majors: u32) -> String {
        installer_name(Version {
            major: self.current().major + majors,
            minor: 0,
            patch: 0,
        })
    }

    pub(crate) fn request(&self, installer: &str) -> UpdatePrepareRequest {
        UpdatePrepareRequest {
            installer_path: self.download(installer).display().to_string(),
            checksums_path: self.download(CHECKSUMS_FILE).display().to_string(),
            signature_path: self
                .download(CHECKSUMS_SIGNATURE_FILE)
                .display()
                .to_string(),
        }
    }

    fn download_files(&self, installer: &str, contents: &[u8], list: &[u8], signature: &[u8]) {
        fs::write(self.download(installer), contents).unwrap();
        fs::write(self.download(CHECKSUMS_FILE), list).unwrap();
        fs::write(self.download(CHECKSUMS_SIGNATURE_FILE), signature).unwrap();
    }

    fn signed(&self, installer: &str, contents: &[u8], list: &[u8]) -> UpdatePrepareRequest {
        let signature = self.key.sign(list);
        self.download_files(installer, contents, list, signature.as_ref());
        self.request(installer)
    }

    pub(crate) fn release_of(&self, installer: &str, contents: &[u8]) -> UpdatePrepareRequest {
        let other = if os::INSTALLER_EXTENSION == "pkg" {
            installer.replace(".pkg", ".msi")
        } else {
            installer.replace(".msi", ".pkg")
        };
        let list = line(&sha256_hex(b"the other installer"), &other)
            + &line(&sha256_hex(contents), installer);
        self.signed(installer, contents, list.as_bytes())
    }

    pub(crate) fn release(&self, installer: &str) -> UpdatePrepareRequest {
        self.release_of(installer, CONTENTS)
    }

    pub(crate) fn staging(&self) -> PathBuf {
        self.records().join(PREPARED)
    }

    pub(crate) fn records(&self) -> PathBuf {
        self.folder.join("records")
    }

    fn refusal(&mut self, request: &UpdatePrepareRequest) -> UpdateRefusal {
        match self.installer.prepare(request) {
            Err(NotDone::Refused(reason, _)) => reason,
            Err(other) => panic!("not refused: {other:?}"),
            Ok(prepared) => panic!("{} was accepted", prepared.installer),
        }
    }

    fn assert_refused(&mut self, request: &UpdatePrepareRequest, expected: UpdateRefusal) {
        assert_eq!(self.refusal(request), expected, "{request:?}");
        assert_eq!(names(&self.staging()), Vec::<String>::new());
    }

    fn prepared_version(&self) -> Option<Version> {
        self.installer
            .prepared()
            .unwrap()
            .map(|prepared| prepared.version)
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
    }
}

pub(crate) fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn staged_names(installer: &str) -> Vec<String> {
    let mut names = vec![
        installer.to_owned(),
        CHECKSUMS_FILE.into(),
        CHECKSUMS_SIGNATURE_FILE.into(),
    ];
    names.sort();
    names
}

fn start_stand_in(exit_code: i32) -> (Child, PathBuf) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = format!(
        "fvm-stand-in-{}-{}",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let stop = env::temp_dir().join(format!("{name}.stop"));
    let ready = env::temp_dir().join(format!("{name}.ready"));
    let mut child = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "auto_update::tests::stand_in",
            "--ignored",
            "--test-threads=1",
        ])
        .env(STAND_IN_STOP, &stop)
        .env(STAND_IN_READY, &ready)
        .env(STAND_IN_EXIT_CODE, exit_code.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + STAND_IN_START_LIMIT;
    while !ready.exists() {
        assert!(
            child.try_wait().unwrap().is_none(),
            "the stand-in ended before it was ready"
        );
        assert!(Instant::now() < deadline, "the stand-in did not start");
        thread::sleep(Duration::from_millis(50));
    }
    fs::remove_file(&ready).unwrap();
    (child, stop)
}

#[test]
#[ignore = "run by the tests that need a process of their own"]
fn stand_in() {
    let Some(stop) = env::var_os(STAND_IN_STOP) else {
        return;
    };
    fs::write(env::var_os(STAND_IN_READY).unwrap(), b"").unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !Path::new(&stop).exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    let _ = fs::remove_file(&stop);
    process::exit(env::var(STAND_IN_EXIT_CODE).unwrap().parse().unwrap());
}

#[cfg(unix)]
fn runner(child: Child) -> os::Runner {
    os::Runner(rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap())
}

#[cfg(windows)]
fn runner(mut child: Child) -> os::Runner {
    os::Runner {
        pid: child.id(),
        waiter: thread::spawn(move || {
            child.wait().unwrap();
        }),
    }
}

fn stop(stop: &Path) {
    fs::write(stop, b"").unwrap();
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "{what}");
        thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_newer_installer_that_verifies_is_kept_prepared_without_starting() {
    let mut setup = Setup::new();
    let name = setup.newer();
    let request = setup.release(&name);

    let prepared = setup.installer.prepare(&request).unwrap();
    assert_eq!(prepared.installer, name);
    assert_eq!(installer_name(prepared.version), name);
    assert_eq!(names(&setup.staging()), staged_names(&name));
    for (copy, original) in [
        (name.as_str(), &request.installer_path),
        (CHECKSUMS_FILE, &request.checksums_path),
        (CHECKSUMS_SIGNATURE_FILE, &request.signature_path),
    ] {
        assert_eq!(
            fs::read(setup.staging().join(copy)).unwrap(),
            fs::read(original).unwrap()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&setup.staging()), 0o700);
        assert_eq!(mode(&setup.staging().join(&name)), 0o600);
    }
    assert!(setup.installer.runner.is_none());
    assert_eq!(names(&setup.records()), [PREPARED]);
    assert_eq!(setup.prepared_version(), Some(prepared.version));
}

#[test]
fn recording_an_install_removes_earlier_records_but_not_the_prepared_update() {
    let mut setup = Setup::new();
    let name = setup.newer();
    setup.installer.prepare(&setup.release(&name)).unwrap();
    for earlier in ["5.started", "5.result", "6.result.tmp", "notes.txt"] {
        fs::write(setup.records().join(earlier), b"").unwrap();
    }

    let started = setup.installer.record(ID).unwrap();
    assert_eq!(started, setup.records().join(format!("{ID}.started")));
    assert_eq!(
        names(&setup.records()),
        [
            format!("{ID}.started"),
            "6.result.tmp".into(),
            "notes.txt".into(),
            PREPARED.into()
        ]
    );
    assert_eq!(fs::read(&started).unwrap(), b"");
    assert_eq!(names(&setup.staging()), staged_names(&name));
}

#[test]
fn the_records_folder_is_created_when_missing() {
    let setup = Setup::new();
    setup.installer.record(ID).unwrap();
    assert_eq!(names(&setup.records()), [format!("{ID}.started")]);
}

#[test]
fn a_new_prepare_replaces_the_prepared_update() {
    let mut setup = Setup::new();
    let first = setup.newer_by(1);
    setup.installer.prepare(&setup.release(&first)).unwrap();
    fs::write(setup.staging().join("left by a cut prepare.tmp"), b"").unwrap();

    let second = setup.newer_by(2);
    let request = setup.release_of(&second, b"another installer");
    let prepared = setup.installer.prepare(&request).unwrap();
    assert_eq!(prepared.installer, second);
    assert_eq!(names(&setup.staging()), staged_names(&second));
    assert_eq!(
        fs::read(setup.staging().join(&second)).unwrap(),
        b"another installer"
    );
    assert_eq!(setup.prepared_version(), Some(prepared.version));

    setup.assert_refused(
        &setup.release(&first.replace(".0.0.", ".0.")),
        UpdateRefusal::VersionNotNewer,
    );
    assert_eq!(setup.prepared_version(), None);
}

#[test]
fn the_prepared_update_is_the_installer_whose_copy_was_verified() {
    let setup = Setup::new();
    assert_eq!(setup.prepared_version(), None);
    let name = setup.newer();
    fs::create_dir_all(setup.staging()).unwrap();
    for left in [
        format!("{name}{DRAFT_SUFFIX}"),
        CHECKSUMS_FILE.into(),
        CHECKSUMS_SIGNATURE_FILE.into(),
    ] {
        fs::write(setup.staging().join(left), b"").unwrap();
    }
    assert_eq!(setup.prepared_version(), None);
    let Err(NotDone::Refused(reason, _)) = setup.another_installer().install(ID, &setup.log) else {
        panic!("an install without a prepared update started");
    };
    assert_eq!(reason, UpdateRefusal::NothingPrepared);
    assert_eq!(names(&setup.records()), [PREPARED]);
}

#[test]
fn a_prepared_update_that_is_not_newer_is_removed() {
    let setup = Setup::new();
    let installed = installer_name(setup.current());
    let leave = || {
        fs::create_dir_all(setup.staging()).unwrap();
        for name in staged_names(&installed) {
            fs::write(setup.staging().join(name), b"").unwrap();
        }
    };

    leave();
    let Err(NotDone::Refused(reason, message)) = setup.installer.prepared() else {
        panic!("a prepared update that is not newer was answered");
    };
    assert_eq!(reason, UpdateRefusal::VersionNotNewer);
    assert!(
        message.starts_with("removed the prepared update"),
        "{message}"
    );
    assert_eq!(names(&setup.staging()), Vec::<String>::new());
    assert_eq!(setup.prepared_version(), None);

    leave();
    let Err(NotDone::Refused(reason, _)) = setup.another_installer().install(ID, &setup.log) else {
        panic!("an update that is not newer started");
    };
    assert_eq!(reason, UpdateRefusal::VersionNotNewer);
    assert_eq!(names(&setup.staging()), Vec::<String>::new());
    assert_eq!(names(&setup.records()), [PREPARED]);
}

#[test]
fn discarding_removes_the_prepared_update() {
    let mut setup = Setup::new();
    setup.installer.discard().unwrap();
    assert!(!setup.records().exists());

    setup
        .installer
        .prepare(&setup.release(&setup.newer()))
        .unwrap();
    setup.installer.discard().unwrap();
    assert_eq!(names(&setup.staging()), Vec::<String>::new());
    assert_eq!(setup.prepared_version(), None);
    setup.installer.discard().unwrap();
}

#[test]
fn an_install_starts_the_runner_on_the_prepared_update() {
    let mut setup = Setup::new();
    let Err(NotDone::Refused(reason, _)) = setup.install(ID) else {
        panic!("an install without a prepared update started");
    };
    assert_eq!(reason, UpdateRefusal::NothingPrepared);
    assert!(!setup.records().exists());

    let name = setup.newer();
    setup.installer.prepare(&setup.release(&name)).unwrap();
    let started = setup.install(ID).unwrap();
    assert_eq!(started.installer, name);
    #[cfg(unix)]
    assert_eq!(started.installer_process, None);
    #[cfg(windows)]
    assert_eq!(started.installer_process, Some(started.runner));
    assert!(setup.installer.runner.is_some());
    let result = setup.records().join(format!("{ID}.result"));
    wait_until("the runner wrote no result", || result.exists());
    let result: UpdateResult = serde_json::from_slice(&fs::read(&result).unwrap()).unwrap();
    assert_eq!(result.install_id, ID);
    #[cfg(unix)]
    assert!(
        result.exit_code.is_some_and(|code| code != 0),
        "the test's installer is not a package"
    );
    #[cfg(windows)]
    assert_eq!(
        result.exit_code,
        Some(1620),
        "the test's installer is not a package"
    );
    wait_until("the runner left the prepared update", || {
        names(&setup.staging()).is_empty()
            && !setup.records().join(format!("{ID}{RUN_SUFFIX}")).exists()
    });
    let records: Vec<String> = names(&setup.records())
        .into_iter()
        .filter(|name| name != "update.log")
        .collect();
    assert_eq!(
        records,
        [
            format!("{ID}.result"),
            format!("{ID}.started"),
            PREPARED.into()
        ]
    );
    wait_until("the runner did not exit", || !setup.installer.installing());
}

#[test]
fn a_signature_that_does_not_verify_is_refused() {
    let mut setup = Setup::new();
    let name = setup.newer();
    let list = line(&sha256_hex(CONTENTS), &name);

    let by_another_key = key_pair().sign(list.as_bytes());
    setup.download_files(&name, CONTENTS, list.as_bytes(), by_another_key.as_ref());
    setup.assert_refused(&setup.request(&name), UpdateRefusal::SignatureInvalid);

    let signature = setup.key.sign(list.as_bytes());
    let changed = line(&sha256_hex(b"another installer"), &name);
    setup.download_files(&name, CONTENTS, changed.as_bytes(), signature.as_ref());
    setup.assert_refused(&setup.request(&name), UpdateRefusal::SignatureInvalid);

    let signature = signature.as_ref();
    let mut longer = signature.to_vec();
    longer.push(0);
    for wrong_length in [&signature[..63], &longer[..], &[], &signature[..32]] {
        setup.download_files(&name, CONTENTS, list.as_bytes(), wrong_length);
        setup.assert_refused(&setup.request(&name), UpdateRefusal::SignatureInvalid);
    }
}

#[test]
fn an_installer_that_does_not_match_its_checksum_is_refused() {
    let mut setup = Setup::new();
    let name = setup.newer();
    let list = line(&sha256_hex(CONTENTS), &name);
    let request = setup.signed(&name, b"another installer", list.as_bytes());
    setup.assert_refused(&request, UpdateRefusal::ChecksumMismatch);
}

#[test]
fn the_list_must_name_the_installer_exactly_once() {
    let mut setup = Setup::new();
    let name = setup.newer();
    let hash = sha256_hex(CONTENTS);
    for list in [
        line(&hash, "FortiVPNManager-0.0.1.pkg"),
        line(&hash, &name.to_uppercase()),
        line(&hash, &format!("download/{name}")),
        line(&hash, &name) + &line(&hash, &name),
        line(&hash, &name) + &line(&sha256_hex(b"another installer"), &name),
    ] {
        let request = setup.signed(&name, CONTENTS, list.as_bytes());
        setup.assert_refused(&request, UpdateRefusal::ChecksumMismatch);
    }
}

#[test]
fn a_list_that_is_not_sha256sum_output_is_refused() {
    let mut setup = Setup::new();
    let name = setup.newer();
    let hash = sha256_hex(CONTENTS);
    let valid = line(&hash, &name);
    let mut not_text = valid.clone().into_bytes();
    not_text.insert(0, 0xff);
    for list in [
        Vec::new(),
        b"\n".to_vec(),
        valid.trim_end().as_bytes().to_vec(),
        format!("{}  {name}\n", hash.to_uppercase()).into_bytes(),
        format!("{hash} {name}\n").into_bytes(),
        format!("{hash}\t{name}\n").into_bytes(),
        format!("{hash} *{name}\n").into_bytes(),
        format!("{hash}   {name}\n").into_bytes(),
        format!("{}  {name}\n", &hash[..63]).into_bytes(),
        format!("{hash}0  {name}\n").into_bytes(),
        format!("{hash}  \n").into_bytes(),
        format!("\\{hash}  {name}\n").into_bytes(),
        format!("{valid}\n").into_bytes(),
        format!("\n{valid}").into_bytes(),
        format!("\u{feff}{valid}").into_bytes(),
        format!("{valid}# comment\n").into_bytes(),
        not_text,
    ] {
        let request = setup.signed(&name, CONTENTS, &list);
        setup.assert_refused(&request, UpdateRefusal::ChecksumMismatch);
    }
}

#[test]
fn listed_checksums() {
    let hash = sha256_hex(CONTENTS);
    let other = sha256_hex(b"the other installer");
    let list =
        line(&other, "FortiVPNManager-27.0.0.msi") + &line(&hash, "FortiVPNManager-27.0.0.pkg");
    assert_eq!(
        listed_checksum(list.as_bytes(), "FortiVPNManager-27.0.0.pkg"),
        Ok(hash.as_str())
    );
    assert_eq!(
        listed_checksum(list.as_bytes(), "FortiVPNManager-27.0.0.msi"),
        Ok(other.as_str())
    );
    let names_with_spaces = line(&hash, "A file  with spaces.pkg");
    assert_eq!(
        listed_checksum(names_with_spaces.as_bytes(), "A file  with spaces.pkg"),
        Ok(hash.as_str())
    );
    let malformed_elsewhere = list + "not a checksum line\n";
    assert_eq!(
        listed_checksum(malformed_elsewhere.as_bytes(), "FortiVPNManager-27.0.0.pkg"),
        Err("SHA256SUMS.txt is not a sha256sum list".to_owned())
    );
}

#[test]
fn an_installer_that_is_not_newer_is_refused() {
    let mut setup = Setup::new();
    let current = setup.current();
    let mut older = vec![Version {
        major: 0,
        minor: 0,
        patch: 0,
    }];
    if current.patch > 0 {
        older.push(Version {
            patch: current.patch - 1,
            ..current
        });
    }
    if current.minor > 0 {
        older.push(Version {
            minor: current.minor - 1,
            patch: 99,
            ..current
        });
    }
    if current.major > 0 {
        older.push(Version {
            major: current.major - 1,
            minor: 99,
            patch: 99,
        });
    }
    for version in [current].into_iter().chain(older) {
        let request = setup.release(&installer_name(version));
        setup.assert_refused(&request, UpdateRefusal::VersionNotNewer);
    }
    let next_patch = Version {
        patch: current.patch + 1,
        ..current
    };
    setup
        .installer
        .prepare(&setup.release(&installer_name(next_patch)))
        .unwrap();
}

#[test]
fn an_installer_not_named_for_this_os_is_refused() {
    let mut setup = Setup::new();
    let newer = setup.newer();
    let other_extension = if os::INSTALLER_EXTENSION == "pkg" {
        newer.replace(".pkg", ".msi")
    } else {
        newer.replace(".msi", ".pkg")
    };
    let without_patch = newer.replacen(".0.0.", ".0.", 1);
    for name in [
        other_extension,
        without_patch,
        format!("Installer.{}", os::INSTALLER_EXTENSION),
        newer.replace("FortiVPNManager", "fortivpnmanager"),
    ] {
        let hash = sha256_hex(CONTENTS);
        let request = setup.signed(&name, CONTENTS, line(&hash, &name).as_bytes());
        setup.assert_refused(&request, UpdateRefusal::VersionNotNewer);
    }
}

#[test]
fn preparing_and_installing_are_refused_while_the_runner_runs() {
    let mut setup = Setup::new();
    let first = setup.newer_by(1);
    setup.installer.prepare(&setup.release(&first)).unwrap();
    let second = setup.newer_by(2);
    let request = setup.release(&second);
    let (child, stand_in) = start_stand_in(0);
    setup.installer.runner = Some(runner(child));

    assert_eq!(setup.refusal(&request), UpdateRefusal::AlreadyInstalling);
    let Err(NotDone::Refused(reason, _)) = setup.install(ID) else {
        panic!("a second install started");
    };
    assert_eq!(reason, UpdateRefusal::AlreadyInstalling);
    assert_eq!(names(&setup.staging()), staged_names(&first));
    assert_eq!(names(&setup.records()), [PREPARED]);

    stop(&stand_in);
    wait_until("the runner did not exit", || !setup.installer.installing());
    assert!(setup.installer.runner.is_none());
    setup.installer.prepare(&request).unwrap();
    assert_eq!(names(&setup.staging()), staged_names(&second));
}

#[test]
fn discarding_removes_nothing_while_the_runner_runs() {
    let mut setup = Setup::new();
    let name = setup.newer();
    setup.installer.prepare(&setup.release(&name)).unwrap();
    let (child, stand_in) = start_stand_in(0);
    setup.installer.runner = Some(runner(child));

    let Err(NotDone::Refused(reason, _)) = setup.installer.discard() else {
        panic!("discarded while the runner runs");
    };
    assert_eq!(reason, UpdateRefusal::AlreadyInstalling);
    assert_eq!(names(&setup.staging()), staged_names(&name));

    stop(&stand_in);
    wait_until("the runner did not exit", || !setup.installer.installing());
    setup.installer.discard().unwrap();
    assert_eq!(names(&setup.staging()), Vec::<String>::new());
}

#[test]
fn unreadable_files_are_refused() {
    let mut setup = Setup::new();
    let name = setup.newer();
    let request = setup.release(&name);
    for missing in [
        UpdatePrepareRequest {
            installer_path: setup
                .download("FortiVPNManager-99.0.0.pkg")
                .display()
                .to_string(),
            ..request.clone()
        },
        UpdatePrepareRequest {
            checksums_path: setup.download("missing.txt").display().to_string(),
            ..request.clone()
        },
        UpdatePrepareRequest {
            signature_path: setup.download("missing.sig").display().to_string(),
            ..request.clone()
        },
        UpdatePrepareRequest {
            installer_path: setup.folder.join("download").display().to_string(),
            ..request.clone()
        },
        UpdatePrepareRequest {
            installer_path: setup.download("..").display().to_string(),
            ..request.clone()
        },
    ] {
        setup.assert_refused(&missing, UpdateRefusal::FilesUnreadable);
    }
    setup.installer.prepare(&request).unwrap();
}

#[cfg(unix)]
#[test]
fn a_pipe_in_place_of_a_file_is_refused_without_waiting() {
    let mut setup = Setup::new();
    let name = setup.newer();
    let request = setup.release(&name);
    fs::remove_file(setup.download(&name)).unwrap();
    nix::unistd::mkfifo(
        &setup.download(&name),
        nix::sys::stat::Mode::from_bits_truncate(0o600),
    )
    .unwrap();
    setup.assert_refused(&request, UpdateRefusal::FilesUnreadable);
}

#[cfg(windows)]
#[test]
fn only_files_on_a_drive_are_read() {
    let mut setup = Setup::new();
    let name = setup.newer();
    let request = setup.release(&name);
    for path in [
        format!(r"\\.\pipe\fvm-auto-update-{}", process::id()),
        r"\\localhost\C$\Windows\win.ini".to_owned(),
    ] {
        let request = UpdatePrepareRequest {
            installer_path: path,
            ..request.clone()
        };
        setup.assert_refused(&request, UpdateRefusal::FilesUnreadable);
    }
}

#[cfg(windows)]
fn leave_run(setup: &Setup, install_id: u64, process: Option<ProcessIdentity>, version: Version) {
    let installer = installer_name(version);
    fs::create_dir_all(setup.staging()).unwrap();
    for name in staged_names(&installer) {
        fs::write(setup.staging().join(name), b"").unwrap();
    }
    let record = os::RunRecord {
        process,
        installer,
        version,
    };
    fs::write(
        setup.records().join(format!("{install_id}{RUN_SUFFIX}")),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

#[cfg(windows)]
fn identity_of(child: &Child) -> ProcessIdentity {
    let mut identity = None;
    wait_until("the stand-in is not visible", || {
        identity = os::process(child.id());
        identity.is_some()
    });
    identity.unwrap()
}

#[cfg(windows)]
fn result_of(setup: &Setup, install_id: u64) -> UpdateResult {
    let result = setup.records().join(format!("{install_id}.result"));
    serde_json::from_slice(&fs::read(result).unwrap()).unwrap()
}

#[cfg(windows)]
#[test]
fn a_restarted_helper_waits_for_the_installer_an_earlier_one_started() {
    let mut setup = Setup::new();
    let version = Version {
        major: setup.current().major + 1,
        minor: 0,
        patch: 0,
    };
    let (child, stand_in) = start_stand_in(42);
    leave_run(&setup, ID, Some(identity_of(&child)), version);

    let image = env::current_exe().unwrap();
    let runner = os::recover_runs(&setup.installer, &image, &setup.log);
    setup.installer.runner = Some(runner.expect("the live stand-in is waited for"));
    thread::sleep(Duration::from_millis(500));
    assert!(setup.installer.installing());
    assert!(!setup.records().join(format!("{ID}.result")).exists());

    stop(&stand_in);
    wait_until("the wait did not end", || !setup.installer.installing());
    assert_eq!(
        result_of(&setup, ID),
        UpdateResult {
            install_id: ID,
            exit_code: Some(42)
        }
    );
    assert_eq!(names(&setup.staging()), Vec::<String>::new());
    assert_eq!(
        names(&setup.records()),
        [format!("{ID}.result"), PREPARED.into()]
    );
}

#[cfg(windows)]
#[test]
fn a_restarted_helper_records_no_exit_code_when_the_installer_is_gone() {
    let setup = Setup::new();
    let current = setup.current();
    let newer = Version {
        major: current.major + 1,
        minor: 0,
        patch: 0,
    };
    let (child, stand_in) = start_stand_in(42);
    let not_msiexec = identity_of(&child);
    let restarted = ProcessIdentity {
        pid: process::id(),
        start_seconds: 0,
        path: r"C:\Windows\System32\msiexec.exe".to_owned(),
    };
    leave_run(&setup, 1, Some(restarted), current);
    leave_run(&setup, 2, None, newer);
    leave_run(&setup, 3, Some(not_msiexec), newer);
    leave_run(&setup, 4, None, current);
    fs::write(
        setup.records().join("4.result"),
        br#"{"installID":4,"exitCode":7}"#,
    )
    .unwrap();

    assert!(os::recover(&setup.installer, &setup.log).is_none());
    for (install_id, exit_code) in [(1, None), (2, None), (3, None), (4, Some(7))] {
        assert_eq!(
            result_of(&setup, install_id),
            UpdateResult {
                install_id,
                exit_code
            }
        );
    }
    assert_eq!(names(&setup.staging()), Vec::<String>::new());
    assert_eq!(
        names(&setup.records()),
        ["1.result", "2.result", "3.result", "4.result", PREPARED]
    );
    stop(&stand_in);
}
