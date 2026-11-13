//! The Windows service: what the service control manager starts and stops. Its start callback
//! never reads its arguments, so no unsafe code is needed; the user who may connect comes from the
//! executable's own command line, which the install script writes into the service's settings.

use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use fvm_contract::helper_internal::{LogCategory, LogLevel};
use fvm_contract::helper_pipe::PIPE_NAME;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;

use crate::Log;
use crate::pipe::{Exit, Helper, Options};

/// The service's name, which the install script registers.
pub const SERVICE_NAME: &str = "FortiVPNManagerHelper";
/// The event log source the Helper registers and writes to.
const LOG_SOURCE: &str = "FortiVPNManager Helper";
/// The system account's own folder; only the system and administrators can reach it.
const RUN_DIRECTORY: &str =
    r"C:\Windows\System32\config\systemprofile\AppData\Local\FortiVPNManager";
/// Names the user who may connect, as a SID: `--allowed-user S-1-5-21-…`.
const ALLOWED_USER_ARGUMENT: &str = "--allowed-user";
/// How long the drain before exiting waits for apps that stop reading.
const DRAIN_LIMIT: Duration = Duration::from_secs(1);
/// What the service control manager is told to wait for while the Helper starts (it first cleans
/// up after an earlier Helper) and while it stops (it first ends and reports the run).
const START_WAIT: Duration = Duration::from_secs(60);
const STOP_WAIT: Duration = Duration::from_secs(30);

/// Runs the service until it stops. Fails when the service control manager did not start the
/// process.
pub fn run() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, service_main)
}

/// The service's start callback. Its arguments are the ones a service start may add, which the
/// Helper does not use.
extern "system" fn service_main(_argument_count: u32, _arguments: *mut *mut u16) {
    let log = event_log();
    let helper: Arc<OnceLock<Helper>> = Arc::new(OnceLock::new());
    let control_side = Arc::clone(&helper);
    let status: Arc<OnceLock<service_control_handler::ServiceStatusHandle>> =
        Arc::new(OnceLock::new());
    let status_side = Arc::clone(&status);
    // When Windows shuts down, it first tells the services that accept preshutdown, while other
    // services and the network still run, and waits for them up to the limit the install script
    // sets; the shutdown notice that follows leaves only a few seconds (5 by default), too few for
    // the logout. A second notice is ignored by the entry.
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Preshutdown | ServiceControl::Shutdown => {
            if let Some(status) = status_side.get() {
                report(status, ServiceState::StopPending, ServiceExitCode::Win32(0));
            }
            // Stop is accepted only once the Helper runs.
            if let Some(helper) = control_side.get() {
                helper.terminate();
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let handle = match service_control_handler::register(SERVICE_NAME, handler) {
        Ok(handle) => handle,
        Err(error) => {
            log(
                LogLevel::Error,
                LogCategory::Helper,
                &format!("cannot register the service control handler: {error}"),
            );
            return;
        }
    };
    let _ = status.set(handle);
    report(
        &handle,
        ServiceState::StartPending,
        ServiceExitCode::Win32(0),
    );
    let Some(user) = allowed_user(env::args_os()) else {
        log(
            LogLevel::Error,
            LogCategory::Helper,
            &format!("not starting: the command line has no {ALLOWED_USER_ARGUMENT} <SID>"),
        );
        report(
            &handle,
            ServiceState::Stopped,
            ServiceExitCode::ServiceSpecific(1),
        );
        return;
    };
    let options = Options {
        pipe_name: PIPE_NAME.to_owned(),
        // The system account owns the pipe, so the user cannot change who may do what with it.
        security: format!("O:SYG:SY{}", pipe_access(&user)),
        run_directory: PathBuf::from(RUN_DIRECTORY),
    };
    let started = match Helper::start(options, Arc::clone(&log)) {
        Ok(started) => started,
        Err(error) => {
            log(
                LogLevel::Error,
                LogCategory::Helper,
                &format!("cannot open the pipe {PIPE_NAME}: {error}"),
            );
            report(
                &handle,
                ServiceState::Stopped,
                ServiceExitCode::ServiceSpecific(2),
            );
            return;
        }
    };
    let helper = helper.get_or_init(|| started);
    report(&handle, ServiceState::Running, ServiceExitCode::Win32(0));
    log(
        LogLevel::Notice,
        LogCategory::Helper,
        &format!("helper {} running for {user}", crate::BUILD),
    );
    match helper.wait_for_exit() {
        Exit::AfterDraining => helper.drain(DRAIN_LIMIT),
        Exit::Now => {}
    }
    report(&handle, ServiceState::Stopped, ServiceExitCode::Win32(0));
    // The other threads would keep the process alive.
    std::process::exit(0);
}

/// The SID after `--allowed-user`, if it looks like one: it goes into the pipe's security
/// descriptor as text.
fn allowed_user(arguments: impl Iterator<Item = OsString>) -> Option<String> {
    let mut arguments = arguments.skip(1);
    while let Some(argument) = arguments.next() {
        if argument == ALLOWED_USER_ARGUMENT {
            let sid = arguments.next()?.into_string().ok()?;
            let parts = sid.strip_prefix("S-1-")?;
            let well_formed = parts.split('-').all(|part| {
                !part.is_empty()
                    && part.len() <= 10
                    && part.bytes().all(|byte| byte.is_ascii_digit())
            });
            return well_formed.then_some(sid);
        }
    }
    None
}

/// Who may do what with the pipe, in SDDL. The system account may do anything; `user` may read and
/// write it, but not create instances of it (FILE_GENERIC_READ | FILE_GENERIC_WRITE without
/// FILE_CREATE_PIPE_INSTANCE), so it cannot open a pipe of the same name next to this one.
fn pipe_access(user: &str) -> String {
    format!("D:P(A;;GA;;;SY)(A;;0x12019b;;;{user})")
}

/// The event log, with the source registered first; registering needs the system account's rights.
/// A line that cannot be written is lost.
fn event_log() -> Log {
    let _ = eventlog::register(LOG_SOURCE);
    let _ = eventlog::init(LOG_SOURCE, log::Level::Info);
    Arc::new(|level, category, message| {
        let level = match level {
            LogLevel::Info | LogLevel::Notice => log::Level::Info,
            LogLevel::Error => log::Level::Error,
        };
        match category {
            LogCategory::Helper => log::log!(level, "{message}"),
            LogCategory::Routes => log::log!(level, "routes: {message}"),
        }
    })
}

/// Tells the service control manager the service's state. Only the running service takes stop and
/// shutdown notices, the pending states say how long to wait, and `exit_code` tells how a stopped
/// service ended.
fn report(
    handle: &service_control_handler::ServiceStatusHandle,
    state: ServiceState,
    exit_code: ServiceExitCode,
) {
    let (controls_accepted, wait_hint) = match state {
        ServiceState::Running => (
            ServiceControlAccept::STOP
                | ServiceControlAccept::PRESHUTDOWN
                | ServiceControlAccept::SHUTDOWN,
            Duration::ZERO,
        ),
        ServiceState::StartPending => (ServiceControlAccept::empty(), START_WAIT),
        ServiceState::StopPending => (ServiceControlAccept::empty(), STOP_WAIT),
        _ => (ServiceControlAccept::empty(), Duration::ZERO),
    };
    let _ = handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        exit_code,
        checkpoint: 0,
        wait_hint,
        process_id: None,
    });
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use interprocess::os::windows::named_pipe::{
        PipeListener, PipeListenerOptions, PipeMode, pipe_mode,
    };
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use serde::Deserialize;
    use widestring::U16CString;

    use super::*;
    use crate::{command, routes};

    fn arguments(list: &[&str]) -> impl Iterator<Item = OsString> {
        list.iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn the_allowed_user_is_a_sid() {
        let sid = "S-1-5-21-1004336348-1177238915-682003330-512";
        assert_eq!(
            allowed_user(arguments(&["fvm-helper.exe", "--allowed-user", sid])),
            Some(sid.to_owned())
        );
        for bad in [
            "S-1-5-21-",
            "S-1-5-21-12)(A;;GA;;;WD",
            "alice",
            "S-1-5--1",
            "",
        ] {
            assert_eq!(
                allowed_user(arguments(&["fvm-helper.exe", "--allowed-user", bad])),
                None,
                "{bad}"
            );
        }
        assert_eq!(allowed_user(arguments(&["fvm-helper.exe"])), None);
        assert_eq!(
            allowed_user(arguments(&["fvm-helper.exe", "--allowed-user"])),
            None
        );
    }

    /// The SID of the user running the tests, as `whoami` prints it.
    fn current_user() -> String {
        let output = Command::new(r"C:\Windows\System32\whoami.exe")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        let sid = text.trim().rsplit(',').next().unwrap().trim_matches('"');
        assert!(sid.starts_with("S-1-"), "{text}");
        sid.to_owned()
    }

    /// As the user running the tests, connects to the pipe named on stdin the way the app's .NET
    /// side does and writes to it, then tries to open another pipe of that name the way another
    /// program would. A constructor's error arrives wrapped, so the innermost one is named.
    const TRY_THE_PIPE: &str = "$ErrorActionPreference = 'Stop'
$name = [Console]::In.ReadToEnd().Trim()
$connected = 'yes'
try {
  $client = New-Object System.IO.Pipes.NamedPipeClientStream('.', $name, [System.IO.Pipes.PipeDirection]::InOut)
  $client.Connect(5000)
  $client.Write([byte[]](0, 0, 0, 0), 0, 4)
  $client.Dispose()
} catch { $connected = $_.Exception.GetBaseException().GetType().Name }
$opened = 'yes'
try {
  $server = New-Object System.IO.Pipes.NamedPipeServerStream($name, [System.IO.Pipes.PipeDirection]::InOut, [System.IO.Pipes.NamedPipeServerStream]::MaxAllowedServerInstances)
  $server.Dispose()
} catch { $opened = $_.Exception.GetBaseException().GetType().Name }
@{ connected = $connected; opened = $opened } | ConvertTo-Json -Compress
";

    #[derive(Deserialize, Debug)]
    struct Tried {
        connected: String,
        opened: String,
    }

    /// Creates a pipe with `access` (the test is not the system account, so it cannot make that
    /// account the owner) and tries it as the user running the tests.
    fn try_pipe(label: &str, access: &str) -> Tried {
        let name = format!("fvm-service-test-{}-{label}", std::process::id());
        let security = U16CString::from_str(access).unwrap();
        let _listener: PipeListener<pipe_mode::Bytes, pipe_mode::Bytes> =
            PipeListenerOptions::new()
                .path(format!(r"\\.\pipe\{name}").as_str())
                .mode(PipeMode::Bytes)
                .security_descriptor(Some(SecurityDescriptor::deserialize(&security).unwrap()))
                .create_duplex::<pipe_mode::Bytes>()
                .unwrap();
        let output = command::run(
            routes::POWERSHELL,
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                TRY_THE_PIPE,
            ],
            name.into_bytes(),
            Duration::from_secs(120),
        )
        .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(stdout.trim())
            .unwrap_or_else(|_| panic!("{stdout} {}", String::from_utf8_lossy(&output.stderr)))
    }

    #[test]
    fn the_allowed_user_may_use_the_pipe_but_not_open_one_of_its_name() {
        let user = current_user();
        let tried = try_pipe("install-user", &pipe_access(&user));
        assert_eq!(tried.connected, "yes", "{tried:?}");
        assert_eq!(tried.opened, "UnauthorizedAccessException", "{tried:?}");
        // With the right to create instances, the same attempt opens one: the refusal above comes
        // from leaving that right out.
        let with_create_right = format!("D:P(A;;GA;;;SY)(A;;0x12019f;;;{user})");
        let tried = try_pipe("create-right", &with_create_right);
        assert_eq!(tried.connected, "yes", "{tried:?}");
        assert_eq!(tried.opened, "yes", "{tried:?}");
    }
}
