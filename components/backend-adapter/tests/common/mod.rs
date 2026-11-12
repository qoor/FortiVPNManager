//! Feeds output to an interpreter the way the backend contract says the watchdog does: complete
//! lines to `line`, decoded as the watchdog does, and after each read that leaves an unfinished
//! line, that line's bytes to `prompt`.

#![allow(dead_code)] // Each test file uses a different part.

use std::mem;

#[cfg(unix)]
use fvm_backend_adapter::openfortivpn::{Interpreter, OpenFortiVpn};
use fvm_contract::app_helper::OutputSource;
#[cfg(unix)]
use fvm_contract::backend::Backend;
use fvm_contract::backend::{Finding, OutputInterpreter, Prompt};

pub struct Run<I> {
    pub interpreter: I,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    pub findings: Vec<Finding>,
}

#[cfg(unix)]
impl Run<Interpreter> {
    pub fn new() -> Self {
        Self::with(OpenFortiVpn.interpreter())
    }
}

impl<I: OutputInterpreter> Run<I> {
    pub fn with(interpreter: I) -> Self {
        Self {
            interpreter,
            stdout: Vec::new(),
            stderr: Vec::new(),
            findings: Vec::new(),
        }
    }

    /// One read of `source`; returns the prompt reported for it.
    pub fn read(&mut self, source: OutputSource, mut bytes: &[u8]) -> Option<Prompt> {
        let Self {
            interpreter,
            stdout,
            stderr,
            findings,
        } = self;
        let partial = match source {
            OutputSource::Stdout => stdout,
            OutputSource::Stderr => stderr,
            OutputSource::Helper => panic!("the Helper's own lines are not read"),
        };
        while let Some(end) = bytes.iter().position(|&byte| byte == b'\n') {
            partial.extend_from_slice(&bytes[..end]);
            let text = String::from_utf8_lossy(&mem::take(partial)).into_owned();
            findings.extend(interpreter.line(source, &text));
            bytes = &bytes[end + 1..];
        }
        if bytes.is_empty() {
            return None;
        }
        partial.extend_from_slice(bytes);
        interpreter.prompt(source, partial)
    }
}
