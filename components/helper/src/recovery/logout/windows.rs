use std::ffi::OsString;
use std::fs::File;
use std::io::{self, PipeReader, PipeWriter};
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;

use fvm_os_api_windows::os_api::{self, Process};

use super::NULL_DEVICE;

pub(super) struct Child(Process);

impl Child {
    pub(super) fn spawn(
        program: &str,
        args: &[&str],
        environment: &[(&str, OsString)],
    ) -> io::Result<(Self, PipeWriter, PipeReader)> {
        let (stdin_read, stdin) = io::pipe()?;
        let (stdout, stdout_write) = io::pipe()?;
        let null = File::options().write(true).open(NULL_DEVICE)?;
        let process = os_api::spawn(
            program,
            args,
            environment,
            [stdin_read.into(), stdout_write.into(), null.into()],
        )
        .map_err(io::Error::other)?;
        Ok((Self(process), stdin, stdout))
    }

    pub(super) fn kill(&self) -> io::Result<()> {
        self.0.kill().map_err(io::Error::other)
    }

    pub(super) fn wait(self) -> ExitStatus {
        ExitStatus::from_raw(
            self.0
                .wait()
                .expect("the child is this process's and its handle open"),
        )
    }
}
