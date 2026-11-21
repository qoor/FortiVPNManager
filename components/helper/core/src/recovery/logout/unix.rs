use std::ffi::{CString, OsString};
use std::fs::File;
use std::io::{self, PipeReader, PipeWriter};
use std::iter;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use nix::libc;
use nix::spawn::{PosixSpawnAttr, PosixSpawnFileActions, PosixSpawnFlags, posix_spawn};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, WaitOptions};

use super::NULL_DEVICE;

pub(super) struct Child(Pid);

impl Child {
    pub(super) fn spawn(
        program: &str,
        args: &[&str],
        environment: &[(&str, OsString)],
    ) -> io::Result<(Self, PipeWriter, PipeReader)> {
        let (stdin_read, stdin) = io::pipe()?;
        let (stdout, stdout_write) = io::pipe()?;
        let null = File::options().write(true).open(NULL_DEVICE)?;
        let argv: Vec<CString> = iter::once(program)
            .chain(args.iter().copied())
            .map(|arg| CString::new(arg).expect("the arguments hold no NUL"))
            .collect();
        let envp: Vec<CString> = environment
            .iter()
            .map(|(name, value)| {
                let mut entry = OsString::from(format!("{name}="));
                entry.push(value);
                CString::new(entry.into_vec()).expect("an environment variable holds no NUL")
            })
            .collect();
        let mut actions = PosixSpawnFileActions::init()?;
        actions.add_dup2(stdin_read.as_raw_fd(), 0)?;
        actions.add_dup2(stdout_write.as_raw_fd(), 1)?;
        actions.add_dup2(null.as_raw_fd(), 2)?;
        let mut attributes = PosixSpawnAttr::init()?;
        attributes.set_flags(PosixSpawnFlags::from_bits_retain(
            libc::POSIX_SPAWN_CLOEXEC_DEFAULT,
        ))?;
        let pid = posix_spawn(program, &actions, &attributes, &argv, &envp)?;
        let pid = Pid::from_raw(pid.as_raw()).expect("posix_spawn returns a positive pid");
        Ok((Self(pid), stdin, stdout))
    }

    pub(super) fn kill(&self) -> io::Result<()> {
        Ok(rustix::process::kill_process(self.0, Signal::KILL)?)
    }

    pub(super) fn wait(self) -> ExitStatus {
        loop {
            match rustix::process::waitpid(Some(self.0), WaitOptions::empty()) {
                Err(Errno::INTR) => {}
                waited => {
                    let (_, status) = waited
                        .ok()
                        .flatten()
                        .expect("the child is this process's and not yet reaped");
                    return ExitStatus::from_raw(status.as_raw());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn only_the_three_pipes_are_inherited() {
        let leaked = rustix::io::dup(File::open("/dev/null").unwrap()).unwrap();
        let fd = leaked.as_raw_fd();
        let script = format!(
            "for fd in 0 1 2 {fd}; do if [ -e /dev/fd/$fd ]; then echo \"$fd open\"; else echo \"$fd closed\"; fi; done"
        );
        let (child, stdin, mut stdout) = Child::spawn("/bin/sh", &["-c", &script], &[]).unwrap();
        drop(stdin);
        let mut output = String::new();
        stdout.read_to_string(&mut output).unwrap();
        assert!(child.wait().success());
        assert_eq!(output, format!("0 open\n1 open\n2 open\n{fd} closed\n"));
        drop(leaked);
    }
}
