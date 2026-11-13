//! How the Windows app starts the runtime: C# calls `fvm_runtime_start` once and talks to the
//! runtime only through the two pipes it returns, with the messages in `fvm_contract::app_internal`.
//! Only handle values cross: C# wraps them in streams, so no pointer passes between the two.

use std::env;
use std::io::{self, PipeReader, PipeWriter};
use std::os::windows::io::IntoRawHandle;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use crate::link;
use crate::orchestration::{self, Input, Paths};

/// The app's ends of the runtime's two pipes, which the app then owns.
#[repr(C)]
pub struct AppPipes {
    /// The handle the app reads the runtime's messages from.
    pub read: usize,
    /// The handle the app writes its messages to.
    pub write: usize,
}

/// Starts the runtime and returns the app's ends of its pipes.
#[allow(unsafe_code)] // The C symbol C# calls; exporting a fixed name needs the unsafe attribute.
#[unsafe(no_mangle)]
pub extern "C" fn fvm_runtime_start() -> AppPipes {
    let (read, write) = start();
    AppPipes {
        read: read.into_raw_handle().addr(),
        write: write.into_raw_handle().addr(),
    }
}

/// Starts the runtime and returns the app's ends of its pipes: the one it reads and the one it
/// writes.
pub fn start() -> (PipeReader, PipeWriter) {
    let (runtime_reader, app_writer) = pipe();
    let (app_reader, runtime_writer) = pipe();
    let (inputs, received) = mpsc::channel();
    let from_app = inputs.clone();
    let out = link::connect(runtime_reader, runtime_writer, move |message| {
        // The runtime thread lives as long as the process.
        let _ = from_app.send(Input::App(message));
    });
    thread::Builder::new()
        .name("fvm-runtime".into())
        .spawn(move || orchestration::run(paths(), inputs, received, out))
        .expect("starting the runtime thread");
    (app_reader, app_writer)
}

/// Its handles are not inherited, so programs the app starts do not hold the runtime's pipes open.
fn pipe() -> (PipeReader, PipeWriter) {
    io::pipe().expect("creating a pipe to the app")
}

/// Where SPEC puts the app's files on Windows: config.yaml under the user's folder as on macOS,
/// the rest under the local application data.
fn paths() -> Paths {
    let home = env::home_dir().expect("a home folder");
    let local = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .expect("the local application data folder");
    Paths {
        config: home.join(r".config\FortiVPNManager\config.yaml"),
        state: local.join(r"FortiVPNManager\state.json"),
        log: local.join(r"FortiVPNManager\Logs\openconnect.log"),
        home,
    }
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;

    use fvm_contract::app_internal::command::Command;
    use fvm_contract::app_internal::port::PortAction;
    use fvm_contract::app_internal::{FromRuntime, ToRuntime};
    use fvm_contract::stream;
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn messages_cross_the_pipes_both_ways() {
        let (runtime_reader, mut app_writer) = pipe();
        let (app_reader, runtime_writer) = pipe();
        let (delivered, received) = mpsc::channel();
        let outgoing = link::connect(runtime_reader, runtime_writer, move |message| {
            delivered.send(message).unwrap()
        });

        stream::send(
            &mut app_writer,
            &json!({"command": {"_0": {"reconnectNow": {}}}}),
        )
        .unwrap();
        assert_eq!(
            received.recv().unwrap(),
            ToRuntime::Command {
                command: Command::ReconnectNow {}
            }
        );

        outgoing
            .send(FromRuntime::PortAction {
                action: PortAction::HoldActivity { hold: true },
            })
            .unwrap();
        let message: Value = stream::receive(&mut BufReader::new(app_reader)).unwrap();
        assert_eq!(
            message,
            json!({"portAction": {"_0": {"holdActivity": {"hold": true}}}})
        );
        // Closing the app's end would stop the reader thread with a panic, as it does in the app.
        std::mem::forget(app_writer);
    }

    #[test]
    fn the_files_are_where_spec_puts_them() {
        let paths = paths();
        assert!(
            paths
                .config
                .ends_with(r".config\FortiVPNManager\config.yaml")
        );
        assert!(paths.state.ends_with(r"FortiVPNManager\state.json"));
        assert!(paths.log.ends_with(r"FortiVPNManager\Logs\openconnect.log"));
        assert!(paths.config.starts_with(&paths.home));
    }
}
