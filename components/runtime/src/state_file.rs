use std::io;
use std::path::{Path, PathBuf};

use fvm_core::state::{self, MAX_FILE_BYTES, State};

use crate::files;

pub(crate) struct StateFile {
    path: PathBuf,
    state: State,
}

impl StateFile {
    /// Reads the file; a missing or unreadable one starts empty and is replaced on the next save.
    pub(crate) fn open(path: PathBuf) -> (Self, Option<String>) {
        let (state, problem) = match files::read_regular(&path, MAX_FILE_BYTES) {
            Ok(Some(bytes)) => match state::read(&bytes) {
                Ok(state) => (state, None),
                Err(error) => (
                    State::default(),
                    Some(format!("ignoring unreadable state.json: {error}")),
                ),
            },
            Ok(None) => (State::default(), None),
            Err(error) => (
                State::default(),
                Some(format!("ignoring unreadable state.json: {error}")),
            ),
        };
        (Self { path, state }, problem)
    }

    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    /// Applies `change` and writes the file when the state changed; the error describes a failed
    /// write, after which the state in memory still holds the change.
    pub(crate) fn update(&mut self, change: impl FnOnce(&mut State)) -> Result<(), String> {
        let mut next = self.state.clone();
        change(&mut next);
        if next == self.state {
            return Ok(());
        }
        self.state = next;
        write(&self.path, &state::to_bytes(&self.state))
            .map_err(|error| format!("saving state.json failed: {error}"))
    }
}

fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        files::make_directory(directory)?;
    }
    files::write_atomically(path, bytes)
}
