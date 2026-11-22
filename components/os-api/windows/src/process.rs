use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::time::Duration;
use std::{iter, mem, ptr};

use windows_sys::Win32::Foundation::{
    HANDLE_FLAG_INHERIT, SetHandleInformation, TRUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::{Error, last_error};

pub struct Process(OwnedHandle);

impl Process {
    /// Ends the process with exit code 1.
    ///
    /// # Errors
    ///
    /// Returns the Windows error code when the process cannot be ended.
    pub fn kill(&self) -> Result<(), Error> {
        // SAFETY: The handle is the process's own and open.
        if unsafe { TerminateProcess(self.0.as_raw_handle(), 1) } == 0 {
            return Err(last_error("TerminateProcess"));
        }
        Ok(())
    }

    /// Waits up to `limit` for the process to end; `true` when it ended.
    ///
    /// # Errors
    ///
    /// Returns the Windows error code when the wait fails.
    ///
    /// # Panics
    ///
    /// Panics if `limit` is 49 days or longer.
    pub fn ended_within(&self, limit: Duration) -> Result<bool, Error> {
        let milliseconds =
            u32::try_from(limit.as_millis()).expect("a wait limit is shorter than 49 days");
        // SAFETY: As in `kill`.
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), milliseconds) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(last_error("WaitForSingleObject")),
        }
    }

    /// Waits for the process to end and returns its exit code.
    ///
    /// # Errors
    ///
    /// Returns the Windows error code when the wait or the exit code query fails.
    pub fn wait(&self) -> Result<u32, Error> {
        // SAFETY: As in `kill`.
        if unsafe { WaitForSingleObject(self.0.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(last_error("WaitForSingleObject"));
        }
        let mut code = 0;
        // SAFETY: As in `kill`; `code` is live for the call.
        if unsafe { GetExitCodeProcess(self.0.as_raw_handle(), &raw mut code) } == 0 {
            return Err(last_error("GetExitCodeProcess"));
        }
        Ok(code)
    }
}

/// Starts `program` with `args`, `environment` added to this process's, and `stdio` as its standard
/// input, output and error, without a console window.
///
/// # Errors
///
/// Returns the Windows error code when the handles cannot be passed on or the process cannot be
/// started.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the parent's copies of the handles close when spawning returns"
)]
pub fn spawn(
    program: &str,
    args: &[&str],
    environment: &[(&str, OsString)],
    stdio: [OwnedHandle; 3],
) -> Result<Process, Error> {
    for handle in &stdio {
        // SAFETY: The handle is open and this process's own.
        let marked = unsafe {
            SetHandleInformation(
                handle.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        };
        if marked == 0 {
            return Err(last_error("SetHandleInformation"));
        }
    }
    let handles = stdio.each_ref().map(AsRawHandle::as_raw_handle);
    let mut size = 0;
    // SAFETY: Without a list the call only writes the size a list for one attribute needs.
    unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &raw mut size) };
    let mut list = vec![0_usize; size.div_ceil(mem::size_of::<usize>())];
    let attributes: LPPROC_THREAD_ATTRIBUTE_LIST = list.as_mut_ptr().cast();
    // SAFETY: `list` holds `size` bytes, aligned for the pointers it keeps.
    if unsafe { InitializeProcThreadAttributeList(attributes, 1, 0, &raw mut size) } == 0 {
        return Err(last_error("InitializeProcThreadAttributeList"));
    }
    // SAFETY: The list was initialized above for one attribute and is deleted only after this.
    let started = unsafe { start(program, args, environment, &handles, attributes) };
    // SAFETY: The list was initialized above, and no call uses it after this.
    unsafe { DeleteProcThreadAttributeList(attributes) };
    started
}

/// # Safety
///
/// `attributes` points to a list that `InitializeProcThreadAttributeList` initialized for one
/// attribute, and the list is not deleted before this returns.
unsafe fn start(
    program: &str,
    args: &[&str],
    environment: &[(&str, OsString)],
    handles: &[RawHandle; 3],
    attributes: LPPROC_THREAD_ATTRIBUTE_LIST,
) -> Result<Process, Error> {
    // SAFETY: The caller passes a list initialized for one attribute, and `handles` outlives its
    // use by CreateProcessW.
    let listed = unsafe {
        UpdateProcThreadAttribute(
            attributes,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr().cast(),
            mem::size_of_val(handles),
            ptr::null_mut(),
            ptr::null(),
        )
    };
    if listed == 0 {
        return Err(last_error("UpdateProcThreadAttribute"));
    }
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb =
        u32::try_from(mem::size_of::<STARTUPINFOEXW>()).expect("the structure size fits u32");
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    [
        startup.StartupInfo.hStdInput,
        startup.StartupInfo.hStdOutput,
        startup.StartupInfo.hStdError,
    ] = *handles;
    startup.lpAttributeList = attributes;
    let application: Vec<u16> = program.encode_utf16().chain([0]).collect();
    let mut command_line = command_line(program, args);
    let environment = environment_block(environment);
    let mut information = PROCESS_INFORMATION::default();
    // SAFETY: The application name and command line are NUL-terminated and the command line
    // writable, the environment is a UTF-16 block ending in two NULs, and `startup` and
    // `information` are live for the call.
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            TRUE,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            ptr::null(),
            ptr::from_ref(&startup).cast(),
            &raw mut information,
        )
    };
    if created == 0 {
        return Err(last_error("CreateProcessW"));
    }
    // SAFETY: CreateProcessW opened both handles for this process, and only these close them.
    let (process, _thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(information.hProcess),
            OwnedHandle::from_raw_handle(information.hThread),
        )
    };
    Ok(Process(process))
}

fn command_line(program: &str, args: &[&str]) -> Vec<u16> {
    let mut line = String::new();
    for (index, argument) in iter::once(program).chain(args.iter().copied()).enumerate() {
        if index > 0 {
            line.push(' ');
        }
        let quoted = argument.is_empty() || argument.contains([' ', '\t']);
        if quoted {
            line.push('"');
        }
        let mut backslashes = 0;
        for character in argument.chars() {
            if character == '\\' {
                backslashes += 1;
            } else {
                if character == '"' {
                    line.extend(iter::repeat_n('\\', backslashes + 1));
                }
                backslashes = 0;
            }
            line.push(character);
        }
        if quoted {
            line.extend(iter::repeat_n('\\', backslashes));
            line.push('"');
        }
    }
    line.encode_utf16().chain([0]).collect()
}

fn environment_block(environment: &[(&str, OsString)]) -> Vec<u16> {
    let mut sorted: Vec<_> = environment.iter().collect();
    sorted.sort_by_key(|(name, _)| name.to_uppercase());
    let mut block = Vec::new();
    for (name, value) in sorted {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    if environment.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

pub fn terminate_this_process(exit_code: u32) -> ! {
    // SAFETY: The call takes no pointers, and the pseudo handle stands for this process.
    unsafe { TerminateProcess(GetCurrentProcess(), exit_code) };
    unreachable!("TerminateProcess does not return when a process terminates itself")
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs::File;
    use std::io::{self, Read, Write};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn a_spawned_process_inherits_only_its_standard_handles() {
        let (stdin_read, mut stdin) = io::pipe().unwrap();
        let (mut stdout, stdout_write) = io::pipe().unwrap();
        let stderr = stdout_write.try_clone().unwrap();
        let (mut other, other_write) = io::pipe().unwrap();
        // SAFETY: The handle is open and this process's own.
        let marked = unsafe {
            SetHandleInformation(
                other_write.as_raw_handle(),
                HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT,
            )
        };
        assert_ne!(marked, 0);
        let process = spawn(
            r"C:\Windows\System32\sort.exe",
            &[],
            &[("SystemRoot", env::var_os("SystemRoot").unwrap())],
            [stdin_read.into(), stdout_write.into(), stderr.into()],
        )
        .unwrap();
        drop(other_write);
        let (sender, other_ended) = mpsc::channel();
        thread::spawn(move || {
            let mut rest = Vec::new();
            let _ = other.read_to_end(&mut rest);
            let _ = sender.send(rest);
        });
        let other_ended = other_ended.recv_timeout(Duration::from_secs(5));
        stdin.write_all(b"b\r\na\r\n").unwrap();
        drop(stdin);
        let mut output = String::new();
        stdout.read_to_string(&mut output).unwrap();
        assert_eq!(process.wait(), Ok(0));
        assert_eq!(output.lines().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(
            other_ended,
            Ok(Vec::new()),
            "the child held the other pipe open"
        );
    }

    #[test]
    fn a_process_still_running_at_the_wait_limit_is_ended_by_a_kill() {
        let null = File::options().read(true).write(true).open("NUL").unwrap();
        let process = spawn(
            r"C:\Windows\System32\ping.exe",
            &["-n", "30", "127.0.0.1"],
            &[("SystemRoot", env::var_os("SystemRoot").unwrap())],
            [
                null.try_clone().unwrap().into(),
                null.try_clone().unwrap().into(),
                null.into(),
            ],
        )
        .unwrap();
        let started = Instant::now();
        assert_eq!(process.ended_within(Duration::from_millis(300)), Ok(false));
        let waited = started.elapsed();
        assert!(waited < Duration::from_secs(5), "{waited:?}");
        process.kill().unwrap();
        assert_eq!(process.ended_within(Duration::from_secs(5)), Ok(true));
        assert_eq!(process.wait(), Ok(1));
    }
}
