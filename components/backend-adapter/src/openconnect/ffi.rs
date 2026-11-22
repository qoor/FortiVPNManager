#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
#[cfg(unix)]
use std::fs::File;
#[cfg(target_os = "macos")]
use std::io;
#[cfg(unix)]
use std::io::Write;
use std::marker::PhantomData;
#[cfg(unix)]
use std::mem::ManuallyDrop;
#[cfg(unix)]
use std::os::fd::FromRawFd;
#[cfg(target_os = "macos")]
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::{self, NonNull};
#[cfg(target_os = "macos")]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, Ordering};

use fvm_helper_core::session::backend::Command;
#[cfg(target_os = "macos")]
use nix::sys::socket::{
    AddressFamily, SockFlag, SockProtocol, SockType, SysControlAddr, connect, socket,
};
use zeroize::Zeroizing;

use super::network::IpInfo;
use super::session::Context;

pub(super) const PRG_ERR: c_int = 0;
pub(super) const PRG_INFO: c_int = 1;
pub(super) const OC_FORM_OPT_TEXT: c_int = 1;
pub(super) const OC_FORM_OPT_PASSWORD: c_int = 2;
pub(super) const OC_FORM_OPT_IGNORE: c_uint = 0x0001;
pub(super) const OC_FORM_RESULT_ERR: c_int = -1;
pub(super) const OC_FORM_RESULT_OK: c_int = 0;
pub(super) const OC_FORM_RESULT_CANCELLED: c_int = 1;
pub(super) const RECONNECT_INTERVAL_MIN: c_int = 10;
pub(super) const EINTR: c_int = 4;
pub(super) const EIO: c_int = 5;
const EINVAL: c_int = 22;
const OC_CMD_CANCEL: u8 = b'x';
const OC_CMD_DETACH: u8 = b'd';
const USER_AGENT: &CStr = c"AnyConnect-compatible OpenConnect VPN Agent";
#[cfg(target_os = "macos")]
const UTUN_CONTROL_NAME: &str = "com.apple.net.utun_control";

#[cfg(unix)]
type CmdSocket = c_int;
#[cfg(windows)]
type CmdSocket = usize;
#[cfg(unix)]
const CMD_PIPE_ERR: CmdSocket = -1;
#[cfg(windows)]
const CMD_PIPE_ERR: CmdSocket = usize::MAX;

#[repr(C)]
struct OpenConnectInfo {
    _opaque: [u8; 0],
}

#[repr(C)]
struct OcFormOpt {
    next: *mut OcFormOpt,
    kind: c_int,
    name: *mut c_char,
    _label: *mut c_char,
    _value: *mut c_char,
    flags: c_uint,
    _reserved: *mut c_void,
}

#[repr(C)]
struct OcAuthForm {
    banner: *mut c_char,
    message: *mut c_char,
    error: *mut c_char,
    auth_id: *mut c_char,
    _method: *mut c_char,
    _action: *mut c_char,
    opts: *mut OcFormOpt,
    _authgroup_opt: *mut c_void,
    _authgroup_selection: c_int,
}

#[repr(C)]
struct OcSplitInclude {
    route: *const c_char,
    next: *const OcSplitInclude,
}

#[repr(C)]
struct OcIpInfo {
    addr: *const c_char,
    netmask: *const c_char,
    addr6: *const c_char,
    netmask6: *const c_char,
    dns: [*const c_char; 3],
    nbns: [*const c_char; 3],
    domain: *const c_char,
    _proxy_pac: *const c_char,
    mtu: c_int,
    split_dns: *const OcSplitInclude,
    split_includes: *const OcSplitInclude,
    split_excludes: *const OcSplitInclude,
    gateway_addr: *const c_char,
}

type ValidatePeerCert = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type ProcessAuthForm = unsafe extern "C" fn(*mut c_void, *mut OcAuthForm) -> c_int;
type Progress = unsafe extern "C" fn(*mut c_void, c_int, *const c_char, ...);
type Notify = unsafe extern "C" fn(*mut c_void);
type Line = unsafe extern "C" fn(*mut c_void, c_int, *const c_char);

#[cfg_attr(windows, link(name = "libopenconnect-5", kind = "raw-dylib"))]
unsafe extern "C" {
    fn openconnect_init_ssl() -> c_int;
    fn openconnect_get_version() -> *const c_char;
    fn openconnect_vpninfo_new(
        useragent: *const c_char,
        validate_peer_cert: Option<ValidatePeerCert>,
        write_new_config: *const c_void,
        process_auth_form: Option<ProcessAuthForm>,
        progress: Option<Progress>,
        privdata: *mut c_void,
    ) -> *mut OpenConnectInfo;
    fn openconnect_vpninfo_free(vpninfo: *mut OpenConnectInfo);
    fn openconnect_set_loglevel(vpninfo: *mut OpenConnectInfo, level: c_int);
    fn openconnect_set_protocol(vpninfo: *mut OpenConnectInfo, protocol: *const c_char) -> c_int;
    fn openconnect_set_system_trust(vpninfo: *mut OpenConnectInfo, val: c_uint);
    fn openconnect_parse_url(vpninfo: *mut OpenConnectInfo, url: *const c_char) -> c_int;
    fn openconnect_setup_cmd_pipe(vpninfo: *mut OpenConnectInfo) -> CmdSocket;
    fn openconnect_obtain_cookie(vpninfo: *mut OpenConnectInfo) -> c_int;
    fn openconnect_get_cookie(vpninfo: *mut OpenConnectInfo) -> *const c_char;
    fn openconnect_get_peer_cert_hash(vpninfo: *mut OpenConnectInfo) -> *const c_char;
    fn openconnect_set_option_value(opt: *mut OcFormOpt, value: *const c_char) -> c_int;
    fn openconnect_make_cstp_connection(vpninfo: *mut OpenConnectInfo) -> c_int;
    fn openconnect_setup_dtls(vpninfo: *mut OpenConnectInfo, attempt_period: c_int) -> c_int;
    fn openconnect_disable_dtls(vpninfo: *mut OpenConnectInfo) -> c_int;
    fn openconnect_set_setup_tun_handler(vpninfo: *mut OpenConnectInfo, handler: Option<Notify>);
    fn openconnect_set_reconnected_handler(vpninfo: *mut OpenConnectInfo, handler: Option<Notify>);
    #[cfg(target_os = "macos")]
    fn openconnect_setup_tun_fd(vpninfo: *mut OpenConnectInfo, tun_fd: c_int) -> c_int;
    #[cfg(windows)]
    fn openconnect_setup_tun_device(
        vpninfo: *mut OpenConnectInfo,
        vpnc_script: *const c_char,
        ifname: *const c_char,
    ) -> c_int;
    #[cfg(windows)]
    fn openconnect_get_ifname(vpninfo: *mut OpenConnectInfo) -> *const c_char;
    fn openconnect_get_ip_info(
        vpninfo: *mut OpenConnectInfo,
        info: *mut *const OcIpInfo,
        cstp_options: *mut *const c_void,
        dtls_options: *mut *const c_void,
    ) -> c_int;
    fn openconnect_mainloop(
        vpninfo: *mut OpenConnectInfo,
        reconnect_timeout: c_int,
        reconnect_interval: c_int,
    ) -> c_int;
}

unsafe extern "C" {
    fn fvm_oc_progress(privdata: *mut c_void, level: c_int, fmt: *const c_char, ...);
    fn fvm_oc_set_line(line: Line);
}

pub(super) fn init() {
    // SAFETY: The call takes nothing.
    let code = unsafe { openconnect_init_ssl() };
    assert_eq!(code, 0, "openconnect_init_ssl failed");
    // SAFETY: `line` has the shim's line type and lives as long as the process.
    unsafe { fvm_oc_set_line(line) };
}

pub(super) fn version() -> String {
    // SAFETY: The call takes nothing and returns the library's static, NUL-terminated version.
    unsafe { CStr::from_ptr(openconnect_get_version()) }
        .to_string_lossy()
        .into_owned()
}

pub(super) struct Callbacks<'a> {
    context: &'a Context<'a>,
    vpninfo: AtomicPtr<OpenConnectInfo>,
    #[cfg(target_os = "macos")]
    tun: OnceLock<OwnedFd>,
}

impl<'a> Callbacks<'a> {
    pub(super) fn new(context: &'a Context<'a>) -> Self {
        Self {
            context,
            vpninfo: AtomicPtr::default(),
            #[cfg(target_os = "macos")]
            tun: OnceLock::new(),
        }
    }
}

pub(super) struct VpnInfo<'a> {
    callbacks: &'a Callbacks<'a>,
    raw: NonNull<OpenConnectInfo>,
}

impl<'a> VpnInfo<'a> {
    pub(super) fn new(callbacks: &'a Callbacks<'a>) -> Self {
        let privdata = ptr::from_ref(callbacks).cast_mut().cast();
        // SAFETY: The user agent is NUL-terminated, the callbacks have openconnect's types, and
        // `privdata` points to `callbacks`, which outlives this VpnInfo and so
        // openconnect_vpninfo_free.
        let raw = unsafe {
            openconnect_vpninfo_new(
                USER_AGENT.as_ptr(),
                Some(validate_peer_cert),
                ptr::null(),
                Some(process_auth_form),
                Some(fvm_oc_progress),
                privdata,
            )
        };
        let raw = NonNull::new(raw).expect("openconnect_vpninfo_new allocates");
        callbacks.vpninfo.store(raw.as_ptr(), Ordering::Relaxed);
        // SAFETY: `raw` is live, and the handlers have openconnect's type.
        unsafe {
            openconnect_set_setup_tun_handler(raw.as_ptr(), Some(setup_tun));
            openconnect_set_reconnected_handler(raw.as_ptr(), Some(reconnected));
        }
        Self { callbacks, raw }
    }

    pub(super) fn set_loglevel(&self, level: c_int) {
        // SAFETY: The session is live.
        unsafe { openconnect_set_loglevel(self.raw.as_ptr(), level) };
    }

    pub(super) fn set_protocol(&self, protocol: &CStr) -> c_int {
        // SAFETY: The session is live and the name NUL-terminated.
        unsafe { openconnect_set_protocol(self.raw.as_ptr(), protocol.as_ptr()) }
    }

    pub(super) fn set_system_trust(&self, trust: bool) {
        // SAFETY: The session is live.
        unsafe { openconnect_set_system_trust(self.raw.as_ptr(), c_uint::from(trust)) };
    }

    pub(super) fn parse_url(&self, url: &CStr) -> c_int {
        // SAFETY: The session is live and the URL NUL-terminated; openconnect copies its parts.
        unsafe { openconnect_parse_url(self.raw.as_ptr(), url.as_ptr()) }
    }

    pub(super) fn arm_cancel(&self) -> c_int {
        // SAFETY: The session is live.
        let pipe = unsafe { openconnect_setup_cmd_pipe(self.raw.as_ptr()) };
        if pipe == CMD_PIPE_ERR {
            return -EIO;
        }
        if self
            .callbacks
            .context
            .link
            .arm_cancel(move |command| send_command(pipe, command))
        {
            0
        } else {
            -EINTR
        }
    }

    pub(super) fn obtain_cookie(&self) -> c_int {
        // SAFETY: The session is live.
        unsafe { openconnect_obtain_cookie(self.raw.as_ptr()) }
    }

    pub(super) fn cookie(&self) -> Zeroizing<String> {
        // SAFETY: The session is live; the cookie is null or NUL-terminated.
        let cookie = unsafe { text(openconnect_get_cookie(self.raw.as_ptr())) };
        Zeroizing::new(cookie.expect("a login that succeeded leaves a cookie"))
    }

    pub(super) fn make_cstp_connection(&self) -> c_int {
        // SAFETY: The session is live.
        unsafe { openconnect_make_cstp_connection(self.raw.as_ptr()) }
    }

    pub(super) fn setup_dtls(&self, attempt_period: c_int) -> c_int {
        // SAFETY: The session is live.
        unsafe { openconnect_setup_dtls(self.raw.as_ptr(), attempt_period) }
    }

    pub(super) fn disable_dtls(&self) {
        // SAFETY: The session is live.
        unsafe { openconnect_disable_dtls(self.raw.as_ptr()) };
    }

    pub(super) fn mainloop(&self, reconnect_timeout: c_int, reconnect_interval: c_int) -> c_int {
        // SAFETY: The session is live.
        unsafe { openconnect_mainloop(self.raw.as_ptr(), reconnect_timeout, reconnect_interval) }
    }
}

impl Drop for VpnInfo<'_> {
    fn drop(&mut self) {
        self.callbacks.context.link.disarm_cancel();
        // SAFETY: The session is live and freed once; its command pipe is no longer written to,
        // and no callback runs after this.
        unsafe { openconnect_vpninfo_free(self.raw.as_ptr()) };
    }
}

#[cfg(unix)]
fn send_command(pipe: CmdSocket, command: Command) {
    // SAFETY: The link writes only while armed, and the session disarms it before
    // openconnect_vpninfo_free closes the pipe; ManuallyDrop leaves it open.
    let pipe = ManuallyDrop::new(unsafe { File::from_raw_fd(pipe) });
    let _ = (&*pipe).write(&[command_byte(command)]);
}

#[cfg(windows)]
fn send_command(pipe: CmdSocket, command: Command) {
    let byte = command_byte(command);
    // SAFETY: The link writes only while armed, and the session disarms it before
    // openconnect_vpninfo_free closes the socket; the byte is live for the call.
    unsafe { windows_sys::Win32::Networking::WinSock::send(pipe, &raw const byte, 1, 0) };
}

fn command_byte(command: Command) -> u8 {
    match command {
        Command::Cancel => OC_CMD_CANCEL,
        Command::Detach => OC_CMD_DETACH,
    }
}

pub(super) struct Form<'f>(&'f mut OcAuthForm);

impl Form<'_> {
    pub(super) fn auth_id(&self) -> Option<String> {
        // SAFETY: openconnect's form strings are null or NUL-terminated for the callback.
        unsafe { text(self.0.auth_id) }
    }

    pub(super) fn banner(&self) -> Option<String> {
        // SAFETY: As in `auth_id`.
        unsafe { text(self.0.banner) }
    }

    pub(super) fn message(&self) -> Option<String> {
        // SAFETY: As in `auth_id`.
        unsafe { text(self.0.message) }
    }

    pub(super) fn error(&self) -> Option<String> {
        // SAFETY: As in `auth_id`.
        unsafe { text(self.0.error) }
    }

    pub(super) fn fields(&mut self) -> Vec<Field<'_>> {
        let mut fields = Vec::new();
        let mut next = self.0.opts;
        while let Some(mut opt) = NonNull::new(next) {
            // SAFETY: The options form a null-terminated list that openconnect leaves to the
            // callback, each one distinct.
            let opt = unsafe { opt.as_mut() };
            next = opt.next;
            fields.push(Field(opt));
        }
        fields
    }
}

pub(super) struct Field<'f>(&'f mut OcFormOpt);

impl Field<'_> {
    pub(super) fn kind(&self) -> c_int {
        self.0.kind
    }

    pub(super) fn name(&self) -> Option<String> {
        // SAFETY: As in `Form::auth_id`.
        unsafe { text(self.0.name) }
    }

    pub(super) fn flags(&self) -> c_uint {
        self.0.flags
    }

    pub(super) fn set(&mut self, value: &str) -> bool {
        let mut text = Zeroizing::new(Vec::with_capacity(value.len() + 1));
        text.extend_from_slice(value.as_bytes());
        text.push(0);
        // SAFETY: The option is live for the callback and `text` is NUL-terminated; openconnect
        // copies it.
        unsafe { openconnect_set_option_value(self.0, text.as_ptr().cast()) == 0 }
    }
}

pub(super) struct Tunnel<'t> {
    raw: NonNull<OpenConnectInfo>,
    #[cfg(target_os = "macos")]
    tun: &'t OnceLock<OwnedFd>,
    _callback: PhantomData<&'t ()>,
}

impl Tunnel<'_> {
    pub(super) fn ip_info(&self) -> IpInfo {
        let mut info: *const OcIpInfo = ptr::null();
        // SAFETY: The session is live; openconnect points `info` at its own IP information.
        unsafe {
            openconnect_get_ip_info(
                self.raw.as_ptr(),
                &raw mut info,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        // SAFETY: The IP information and its strings and lists stay unchanged until the setup_tun
        // callback returns; each string is null or NUL-terminated and each list null-terminated.
        unsafe {
            let info = &*info;
            IpInfo {
                addr: text(info.addr),
                netmask: text(info.netmask),
                addr6: text(info.addr6),
                netmask6: text(info.netmask6),
                dns: info.dns.iter().filter_map(|&server| text(server)).collect(),
                nbns: info
                    .nbns
                    .iter()
                    .filter_map(|&server| text(server))
                    .collect(),
                domain: text(info.domain),
                mtu: info.mtu,
                split_dns: routes(info.split_dns),
                split_includes: routes(info.split_includes),
                split_excludes: routes(info.split_excludes),
                gateway_addr: text(info.gateway_addr),
            }
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn setup_tun_fd(&self, fd: OwnedFd) -> c_int {
        let raw = fd.as_raw_fd();
        self.tun
            .set(fd)
            .expect("openconnect sets up a session's tunnel once");
        // SAFETY: The session is live; the callbacks, which outlive its VpnInfo, keep the fd open
        // until after openconnect_vpninfo_free, and openconnect closes a tunnel fd only for a
        // vpnc-script, which the session never sets.
        unsafe { openconnect_setup_tun_fd(self.raw.as_ptr(), raw) }
    }

    #[cfg(windows)]
    pub(super) fn setup_tun_device(&self) -> c_int {
        // SAFETY: The session is live; without a script or name openconnect picks its own.
        unsafe { openconnect_setup_tun_device(self.raw.as_ptr(), ptr::null(), ptr::null()) }
    }

    #[cfg(windows)]
    pub(super) fn ifname(&self) -> Option<String> {
        // SAFETY: The session is live; the name is null or NUL-terminated.
        unsafe { text(openconnect_get_ifname(self.raw.as_ptr())) }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn open_utun() -> io::Result<(OwnedFd, String)> {
    let fd = socket(
        AddressFamily::System,
        SockType::Datagram,
        SockFlag::empty(),
        SockProtocol::KextControl,
    )?;
    let id = SysControlAddr::from_name(fd.as_raw_fd(), UTUN_CONTROL_NAME, 0)?.id();
    for unit in 0..255 {
        if connect(fd.as_raw_fd(), &SysControlAddr::new(id, unit + 1)).is_ok() {
            return Ok((fd, format!("utun{unit}")));
        }
    }
    Err(io::Error::last_os_error())
}

unsafe fn text(pointer: *const c_char) -> Option<String> {
    // SAFETY: The caller passes null or a NUL-terminated string.
    (!pointer.is_null()).then(|| {
        unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned()
    })
}

unsafe fn routes(mut entry: *const OcSplitInclude) -> Vec<String> {
    let mut routes = Vec::new();
    // SAFETY: The caller passes a null-terminated list of entries with NUL-terminated routes.
    while let Some(this) = unsafe { entry.as_ref() } {
        routes.extend(unsafe { text(this.route) });
        entry = this.next;
    }
    routes
}

unsafe fn callbacks<'a>(privdata: *mut c_void) -> &'a Callbacks<'a> {
    // SAFETY: openconnect passes back the `privdata` VpnInfo::new gave it, which outlives every
    // callback.
    unsafe { &*privdata.cast::<Callbacks<'a>>() }
}

unsafe extern "C" fn validate_peer_cert(privdata: *mut c_void, _reason: *const c_char) -> c_int {
    // SAFETY: Called by openconnect with its `privdata`.
    let callbacks = unsafe { callbacks(privdata) };
    let vpninfo = callbacks.vpninfo.load(Ordering::Relaxed);
    // SAFETY: The session is live during the callback; the hash is null or NUL-terminated.
    let hash = unsafe { text(openconnect_get_peer_cert_hash(vpninfo)) };
    if hash.is_some_and(|hash| callbacks.context.certificate(&hash)) {
        0
    } else {
        -EINVAL
    }
}

unsafe extern "C" fn process_auth_form(privdata: *mut c_void, form: *mut OcAuthForm) -> c_int {
    // SAFETY: Called by openconnect with its `privdata`.
    let callbacks = unsafe { callbacks(privdata) };
    // SAFETY: The form is live and the callback's own until it returns.
    let mut form = Form(unsafe { &mut *form });
    callbacks.context.auth_form(&mut form)
}

unsafe extern "C" fn line(privdata: *mut c_void, level: c_int, text: *const c_char) {
    // SAFETY: Called by the shim with openconnect's `privdata`.
    let callbacks = unsafe { callbacks(privdata) };
    // SAFETY: The shim passes null or a NUL-terminated string.
    if let Some(text) = unsafe { self::text(text) } {
        callbacks.context.line(level, &text);
    }
}

unsafe extern "C" fn setup_tun(privdata: *mut c_void) {
    // SAFETY: Called by openconnect with its `privdata`.
    let callbacks = unsafe { callbacks(privdata) };
    let raw = NonNull::new(callbacks.vpninfo.load(Ordering::Relaxed))
        .expect("the session is set before its main loop");
    callbacks.context.setup_tun(&Tunnel {
        raw,
        #[cfg(target_os = "macos")]
        tun: &callbacks.tun,
        _callback: PhantomData,
    });
}

unsafe extern "C" fn reconnected(privdata: *mut c_void) {
    // SAFETY: Called by openconnect with its `privdata`.
    let callbacks = unsafe { callbacks(privdata) };
    callbacks.context.reconnected();
}
