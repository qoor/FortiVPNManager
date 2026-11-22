use std::ffi::OsStr;
use std::fs::{self, DirBuilder, File};
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, Prefix};
use std::process;
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};

use fvm_contract::app_helper::{
    UPDATE_RECORDS_DIRECTORY_WINDOWS, UPDATE_RESULT_SUFFIX, UpdateResult, Version,
};
use fvm_contract::helper_internal::LogLevel;
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, Process, ProcessRefreshKind, ProcessesToUpdate, System, Uid, UpdateKind};

use super::{DRAFT_SUFFIX, Installer, Log, RUN_SUFFIX, Staged};
use crate::recovery::run_directory;

pub const RECORDS_DIRECTORY: &str = UPDATE_RECORDS_DIRECTORY_WINDOWS;
pub(super) const INSTALLER_EXTENSION: &str = "msi";
const MSIEXEC: &str = r"C:\Windows\System32\msiexec.exe";
const MSIEXEC_LOG: &str = "update.log";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;

pub(super) const RUNNER_IS_INSTALLER: bool = true;

pub(super) struct Runner {
    pub(super) pid: u32,
    pub(super) waiter: JoinHandle<()>,
}

impl Runner {
    pub(super) fn id(&self) -> u32 {
        self.pid
    }

    pub(super) fn running(&mut self) -> bool {
        !self.waiter.is_finished()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProcessIdentity {
    pub(super) pid: u32,
    pub(super) start_seconds: u64,
    pub(super) path: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RunRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) process: Option<ProcessIdentity>,
    pub(super) installer: String,
    pub(super) version: Version,
}

pub(super) fn open_source(path: &Path) -> io::Result<File> {
    match path.components().next() {
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
        {
            File::open(path)
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a path on a drive",
        )),
    }
}

pub(super) fn create_records_directory(directory: &Path) -> io::Result<()> {
    match DirBuilder::new().create(directory) {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => Err(error),
        _ => Ok(()),
    }
}

pub(super) fn launch(
    installer: &Installer,
    staged: &Staged,
    install_id: u64,
    version: Version,
    log: &Log,
) -> io::Result<Runner> {
    let mut msiexec = Command::new(MSIEXEC);
    msiexec
        .raw_arg(format!(
            r#"/i "{}" /qn /norestart /l*v "{}""#,
            staged.installer.display(),
            installer.records.join(MSIEXEC_LOG).display()
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = msiexec
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()?;
    let pid = child.id();
    let run = installer.records.join(format!("{install_id}{RUN_SUFFIX}"));
    let record = RunRecord {
        process: self::process(pid),
        installer: staged
            .installer
            .file_name()
            .and_then(OsStr::to_str)
            .expect("a prepared installer is named FortiVPNManager-<version>.msi")
            .to_owned(),
        version,
    };
    let json = serde_json::to_vec(&record).expect("a run record serializes");
    if let Err(error) = write_record(&run, &json) {
        log(
            LogLevel::Error,
            format!(
                "update install {install_id}: could not write {}: {error}",
                run.display()
            ),
        );
    }
    let waiter = wait(installer, staged.clone(), install_id, log, move || {
        child.wait().ok().and_then(|status| status.code())
    });
    Ok(Runner { pid, waiter })
}

pub(super) fn recover(installer: &Installer, log: &Log) -> Option<Runner> {
    recover_runs(installer, Path::new(MSIEXEC), log)
}

pub(super) fn recover_runs(installer: &Installer, image: &Path, log: &Log) -> Option<Runner> {
    let entries = match fs::read_dir(&installer.records) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            log(
                LogLevel::Error,
                format!("could not read {}: {error}", installer.records.display()),
            );
            return None;
        }
    };
    let mut runner = None;
    for path in entries.flatten().map(|entry| entry.path()) {
        let Some(install_id) = path
            .file_name()
            .and_then(OsStr::to_str)
            .and_then(|name| name.strip_suffix(RUN_SUFFIX))
            .and_then(|id| id.parse::<u64>().ok())
        else {
            continue;
        };
        let Some(record) = fs::read(&path)
            .ok()
            .and_then(|json| serde_json::from_slice::<RunRecord>(&json).ok())
        else {
            log(
                LogLevel::Error,
                format!(
                    "update install {install_id}: could not read {}",
                    path.display()
                ),
            );
            continue;
        };
        let staged = Staged::new(&installer.staging, &record.installer);
        let result = installer
            .records
            .join(format!("{install_id}{UPDATE_RESULT_SUFFIX}"));
        if result.exists() {
            remove(&installer.records, &staged, install_id, log);
            continue;
        }
        let live = record
            .process
            .as_ref()
            .filter(|recorded| {
                image
                    .to_str()
                    .is_some_and(|image| recorded.path.eq_ignore_ascii_case(image))
            })
            .and_then(|recorded| {
                let system = processes(ProcessesToUpdate::Some(&[Pid::from_u32(recorded.pid)]));
                let identity = system
                    .process(Pid::from_u32(recorded.pid))
                    .and_then(|process| identity(recorded.pid, process));
                (identity.as_ref() == Some(recorded)).then_some((recorded.pid, system))
            });
        if let Some((pid, system)) = live {
            log(
                LogLevel::Notice,
                format!(
                    "update install {install_id}: waiting for msiexec {pid}, started before the Helper restarted"
                ),
            );
            let waiter = wait(installer, staged, install_id, log, move || {
                system
                    .process(Pid::from_u32(pid))
                    .and_then(Process::wait)
                    .and_then(|status| status.code())
            });
            runner = Some(Runner { pid, waiter });
        } else {
            log(
                LogLevel::Notice,
                format!(
                    "update install {install_id}: msiexec ended while no Helper waited for it; the Helper is {}, the update {}",
                    installer.current, record.version
                ),
            );
            finish(&installer.records, &staged, install_id, None, log);
        }
    }
    runner
}

fn wait(
    installer: &Installer,
    staged: Staged,
    install_id: u64,
    log: &Log,
    exited: impl FnOnce() -> Option<i32> + Send + 'static,
) -> JoinHandle<()> {
    let records = installer.records.clone();
    let log = Arc::clone(log);
    thread::Builder::new()
        .name("fvm-update-wait".into())
        .spawn(move || finish(&records, &staged, install_id, exited(), &log))
        .expect("starting the update wait thread")
}

fn finish(records: &Path, staged: &Staged, install_id: u64, exit_code: Option<i32>, log: &Log) {
    let result = records.join(format!("{install_id}{UPDATE_RESULT_SUFFIX}"));
    let json = serde_json::to_vec(&UpdateResult {
        install_id,
        exit_code,
    })
    .expect("an update result serializes");
    match write_record(&result, &json) {
        Ok(()) => log(
            LogLevel::Notice,
            match exit_code {
                Some(code) => format!("update install {install_id}: exit code {code} recorded"),
                None => format!("update install {install_id}: exit code not observed"),
            },
        ),
        Err(error) => log(
            LogLevel::Error,
            format!(
                "update install {install_id}: could not write {}: {error}",
                result.display()
            ),
        ),
    }
    remove(records, staged, install_id, log);
}

fn remove(records: &Path, staged: &Staged, install_id: u64, log: &Log) {
    let run = records.join(format!("{install_id}{RUN_SUFFIX}"));
    for path in [
        &staged.installer,
        &staged.checksums,
        &staged.signature,
        &run,
    ] {
        if let Err(line) = run_directory::remove(path) {
            log(LogLevel::Error, line);
        }
    }
}

fn write_record(path: &Path, json: &[u8]) -> io::Result<()> {
    let mut draft = path.as_os_str().to_owned();
    draft.push(DRAFT_SUFFIX);
    fs::write(&draft, json)?;
    fs::rename(&draft, path)
}

pub(super) fn process(pid: u32) -> Option<ProcessIdentity> {
    let processes = processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]));
    identity(pid, processes.process(Pid::from_u32(pid))?)
}

fn identity(pid: u32, process: &Process) -> Option<ProcessIdentity> {
    if process.user_id() != Some(own_user()) {
        return None;
    }
    Some(ProcessIdentity {
        pid,
        start_seconds: process.start_time(),
        path: process.exe()?.to_str()?.to_owned(),
    })
}

fn own_user() -> &'static Uid {
    static USER: OnceLock<Uid> = OnceLock::new();
    USER.get_or_init(|| {
        let own = Pid::from_u32(process::id());
        processes(ProcessesToUpdate::Some(&[own]))
            .process(own)
            .and_then(Process::user_id)
            .cloned()
            .expect("the Helper can read its own user")
    })
}

fn processes(which: ProcessesToUpdate<'_>) -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        which,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_user(UpdateKind::Always),
    );
    system
}
