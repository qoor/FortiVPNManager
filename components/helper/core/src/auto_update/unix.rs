use std::env;
use std::ffi::{CString, OsStr};
use std::fs::{DirBuilder, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use fvm_contract::app_helper::{UPDATE_RECORDS_DIRECTORY_MACOS, UPDATE_RESULT_SUFFIX, Version};
use nix::libc;
use nix::spawn::{PosixSpawnAttr, PosixSpawnFileActions, PosixSpawnFlags, posix_spawn};
use nix::sys::signal::{SigSet, Signal as SpawnSignal};
use rustix::process::{Pid, WaitOptions};

use super::{DRAFT_SUFFIX, Installer, Log, Staged};

pub const RECORDS_DIRECTORY: &str = UPDATE_RECORDS_DIRECTORY_MACOS;
pub(super) const INSTALLER_EXTENSION: &str = "pkg";
const SHELL: &str = "/bin/sh";
const RUNNER: &str = r#"/usr/sbin/installer -pkg "$4" -target /
code=$?
printf '{"installID":%s,"exitCode":%s}' "$3" "$code" > "$2" && /bin/mv -f "$2" "$1"
shift 3
/bin/rm -f "$@"
"#;

pub(super) const RUNNER_IS_INSTALLER: bool = false;

pub(super) struct Runner(pub(super) Pid);

impl Runner {
    pub(super) fn id(&self) -> u32 {
        self.0.as_raw_nonzero().get().unsigned_abs()
    }

    pub(super) fn running(&mut self) -> bool {
        rustix::process::waitpid(Some(self.0), WaitOptions::NOHANG)
            .expect("checking on the unreaped update runner")
            .is_none()
    }
}

pub(super) fn open_source(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
}

pub(super) fn create_records_directory(directory: &Path) -> io::Result<()> {
    match DirBuilder::new().mode(0o755).create(directory) {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => Err(error),
        _ => Ok(()),
    }
}

pub(super) fn recover(_installer: &Installer, _log: &Log) -> Option<Runner> {
    None
}

pub(super) fn launch(
    installer: &Installer,
    staged: &Staged,
    install_id: u64,
    _version: Version,
    _log: &Log,
) -> io::Result<Runner> {
    let result = installer
        .records
        .join(format!("{install_id}{UPDATE_RESULT_SUFFIX}"));
    let draft = installer
        .records
        .join(format!("{install_id}{UPDATE_RESULT_SUFFIX}{DRAFT_SUFFIX}"));
    let null = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;
    let id = install_id.to_string();
    let argv: Vec<CString> = [
        OsStr::new(SHELL),
        OsStr::new("-c"),
        OsStr::new(RUNNER),
        OsStr::new("sh"),
        result.as_os_str(),
        draft.as_os_str(),
        OsStr::new(&id),
        staged.installer.as_os_str(),
        staged.checksums.as_os_str(),
        staged.signature.as_os_str(),
    ]
    .into_iter()
    .map(|arg| CString::new(arg.as_bytes()).expect("the runner's paths hold no NUL"))
    .collect();
    let env: Vec<CString> = env::vars_os()
        .map(|(name, value)| {
            let mut entry = name.into_vec();
            entry.push(b'=');
            entry.extend(value.into_vec());
            CString::new(entry).expect("the environment holds no NUL")
        })
        .collect();

    let mut actions = PosixSpawnFileActions::init()?;
    for fd in 0..3 {
        actions.add_dup2(null.as_raw_fd(), fd)?;
    }
    let mut attributes = PosixSpawnAttr::init()?;
    attributes.set_sigmask(&SigSet::empty())?;
    let mut every_signal = SigSet::all();
    every_signal.remove(SpawnSignal::SIGKILL);
    every_signal.remove(SpawnSignal::SIGSTOP);
    attributes.set_sigdefault(&every_signal)?;
    attributes.set_pgroup(nix::unistd::Pid::from_raw(0))?;
    attributes.set_flags(
        PosixSpawnFlags::POSIX_SPAWN_SETSIGMASK
            | PosixSpawnFlags::POSIX_SPAWN_SETSIGDEF
            | PosixSpawnFlags::POSIX_SPAWN_SETPGROUP
            | PosixSpawnFlags::from_bits_retain(libc::POSIX_SPAWN_CLOEXEC_DEFAULT),
    )?;
    let pid = posix_spawn(Path::new(SHELL), &actions, &attributes, &argv, &env)?;
    Ok(Runner(
        Pid::from_raw(pid.as_raw()).expect("posix_spawn returns a positive pid"),
    ))
}
