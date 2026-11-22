use std::ffi::{CString, c_int, c_uint};
use std::sync::{Mutex, Once};

use fvm_helper_core::session::backend::{
    FormKind, LineLevel, Link, SessionEnd, SessionEvent, SessionRequest,
};

#[cfg(target_os = "macos")]
use super::ffi::open_utun;
use super::ffi::{
    Callbacks, Form, OC_FORM_OPT_IGNORE, OC_FORM_OPT_PASSWORD, OC_FORM_OPT_TEXT,
    OC_FORM_RESULT_CANCELLED, OC_FORM_RESULT_ERR, OC_FORM_RESULT_OK, PRG_ERR, PRG_INFO,
    RECONNECT_INTERVAL_MIN, Tunnel, VpnInfo,
};
use super::network::{self, IpInfo};

const DTLS_ATTEMPT_PERIOD: c_int = 60;

#[expect(
    clippy::needless_pass_by_value,
    reason = "the request and its secrets are dropped when the session ends"
)]
pub(super) fn run(request: SessionRequest, link: &Link) -> SessionEnd {
    let context = Context {
        link,
        request: &request,
        forms: Mutex::default(),
        gateway_reached: Once::new(),
    };
    let callbacks = Callbacks::new(&context);
    let vpninfo = VpnInfo::new(&callbacks);
    vpninfo.set_loglevel(PRG_INFO);
    assert_eq!(
        vpninfo.set_protocol(c"fortinet"),
        0,
        "openconnect has the fortinet protocol"
    );
    vpninfo.set_system_trust(false);
    if !request.dtls {
        vpninfo.disable_dtls();
    }
    let url = format!(
        "https://{}:{}/{}",
        request.host,
        request.port,
        request.realm.as_deref().unwrap_or_default()
    );
    let url = CString::new(url).expect("the request was checked");
    assert_eq!(vpninfo.parse_url(&url), 0, "the request was checked");
    let armed = vpninfo.arm_cancel();
    if armed != 0 {
        return SessionEnd { code: armed };
    }
    let code = vpninfo.obtain_cookie();
    if code != 0 {
        return SessionEnd { code };
    }
    link.opened(vpninfo.cookie());
    link.report(SessionEvent::Authenticated);
    let code = vpninfo.make_cstp_connection();
    if code != 0 {
        return SessionEnd { code };
    }
    if request.dtls && vpninfo.setup_dtls(DTLS_ATTEMPT_PERIOD) != 0 {
        vpninfo.disable_dtls();
    }
    SessionEnd {
        code: vpninfo.mainloop(0, RECONNECT_INTERVAL_MIN),
    }
}

pub(super) struct Context<'a> {
    pub(super) link: &'a Link,
    request: &'a SessionRequest,
    forms: Mutex<Vec<FormKind>>,
    gateway_reached: Once,
}

impl Context<'_> {
    pub(super) fn certificate(&self, hash: &str) -> bool {
        let trusted = self.request.pins.iter().any(|pin| pin == hash);
        if trusted {
            self.gateway_reached
                .call_once(|| self.link.report(SessionEvent::GatewayTrusted));
        } else {
            self.link
                .report(SessionEvent::CertificateRejected(hash.to_owned()));
        }
        trusted
    }

    pub(super) fn auth_form(&self, form: &mut Form<'_>) -> c_int {
        for (level, text) in [
            (LineLevel::Info, form.banner()),
            (LineLevel::Info, form.message()),
            (LineLevel::Error, form.error()),
        ] {
            if let Some(text) = text {
                self.lines(level, &text);
            }
        }
        let Some(kind) = form_kind(form.auth_id().as_deref()) else {
            return OC_FORM_RESULT_CANCELLED;
        };
        let fields = form.fields();
        let Some(values) = fields
            .iter()
            .map(|field| field_value(field.kind(), field.name().as_deref(), field.flags()))
            .collect::<Option<Vec<_>>>()
        else {
            return OC_FORM_RESULT_CANCELLED;
        };
        {
            let mut forms = self
                .forms
                .lock()
                .expect("the form list is never held across a panic");
            if forms.contains(&kind) {
                drop(forms);
                self.link.report(SessionEvent::FormRepeated(kind));
                return OC_FORM_RESULT_CANCELLED;
            }
            forms.push(kind);
        }
        let otp;
        let secret = match kind {
            FormKind::Login => self.request.password.as_str(),
            FormKind::Otp => {
                self.link.report(SessionEvent::OtpPrompted);
                let Some(code) = self.link.otp() else {
                    return OC_FORM_RESULT_CANCELLED;
                };
                otp = code;
                otp.as_str()
            }
        };
        for (mut field, value) in fields.into_iter().zip(values) {
            let value = match value {
                FieldValue::Unchanged => continue,
                FieldValue::Username => self.request.username.as_str(),
                FieldValue::Secret => secret,
            };
            if !field.set(value) {
                return OC_FORM_RESULT_ERR;
            }
        }
        OC_FORM_RESULT_OK
    }

    pub(super) fn line(&self, level: c_int, text: &str) {
        let level = if level == PRG_ERR {
            LineLevel::Error
        } else {
            LineLevel::Info
        };
        self.lines(level, text);
    }

    fn lines(&self, level: LineLevel, text: &str) {
        for line in text.lines() {
            self.link.report(SessionEvent::Line(level, line.to_owned()));
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn setup_tun(&self, tunnel: &Tunnel<'_>) {
        let ip = tunnel.ip_info();
        let name = match open_utun() {
            Ok((fd, name)) => {
                if tunnel.setup_tun_fd(fd) == 0 {
                    Ok(name)
                } else {
                    Err(None)
                }
            }
            Err(error) => Err(Some(error.to_string())),
        };
        self.tunnel_set_up(&ip, name);
    }

    #[cfg(windows)]
    pub(super) fn setup_tun(&self, tunnel: &Tunnel<'_>) {
        let ip = tunnel.ip_info();
        let name = if tunnel.setup_tun_device() == 0 {
            tunnel.ifname()
        } else {
            None
        };
        self.tunnel_set_up(&ip, name.ok_or(None));
    }

    fn tunnel_set_up(&self, ip: &IpInfo, name: Result<String, Option<String>>) {
        let name = match name {
            Ok(name) => name,
            Err(reason) => {
                let address = ip.addr.as_deref().and_then(|address| address.parse().ok());
                self.link
                    .report(SessionEvent::TunnelFailed(address, reason));
                self.link.cancel();
                return;
            }
        };
        let settings = interface_index(&name)
            .and_then(|index| network::settings(ip, name, index))
            .map(Box::new);
        self.link.report(SessionEvent::TunnelReady(settings));
    }

    pub(super) fn reconnected(&self) {
        self.link.report(SessionEvent::Reconnected);
    }
}

#[cfg(windows)]
fn interface_index(name: &str) -> Result<Option<u32>, String> {
    use fvm_os_api_windows::network::Luid;

    Luid::from_alias(name)
        .and_then(Luid::index)
        .map(Some)
        .map_err(|error| format!("the tunnel device {name:?} has no interface index: {error}"))
}

#[cfg(not(windows))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the Windows version, which can fail"
)]
fn interface_index(_: &str) -> Result<Option<u32>, String> {
    Ok(None)
}

fn form_kind(auth_id: Option<&str>) -> Option<FormKind> {
    let kind = if auth_id? == "_login" {
        FormKind::Login
    } else {
        FormKind::Otp
    };
    Some(kind)
}

#[derive(PartialEq, Debug)]
enum FieldValue {
    Unchanged,
    Username,
    Secret,
}

fn field_value(kind: c_int, name: Option<&str>, flags: c_uint) -> Option<FieldValue> {
    let name = name?;
    if flags & OC_FORM_OPT_IGNORE != 0 {
        return Some(FieldValue::Unchanged);
    }
    let starts_with = |prefix: &str| {
        name.get(..prefix.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    };
    let value = match kind {
        OC_FORM_OPT_TEXT if starts_with("user") || starts_with("uname") => FieldValue::Username,
        OC_FORM_OPT_PASSWORD => FieldValue::Secret,
        _ => FieldValue::Unchanged,
    };
    Some(value)
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use fvm_helper_core::session::backend::Command;
    use zeroize::Zeroizing;

    use super::*;

    const PIN: &str = "pin-sha256:VU1WXtWqa2dQMFah3Sd01wcu+T+caKs4wthkccwXx3A=";
    const OTHER_PIN: &str = "pin-sha256:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
    const OC_FORM_OPT_HIDDEN: c_int = 4;
    const OC_FORM_OPT_SELECT: c_int = 3;

    fn reported<T>(pins: &[&str], run: impl FnOnce(&Context<'_>) -> T) -> (T, Vec<SessionEvent>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let link = Link::new(
            {
                let events = Arc::clone(&events);
                move |event| events.lock().unwrap().push(event)
            },
            |_| {},
        );
        let request = SessionRequest {
            host: "203.0.113.10".into(),
            port: 443,
            realm: None,
            username: "alice".into(),
            password: Zeroizing::new("password".into()),
            pins: pins.iter().map(|&pin| pin.to_owned()).collect(),
            dtls: false,
        };
        let context = Context {
            link: &link,
            request: &request,
            forms: Mutex::default(),
            gateway_reached: Once::new(),
        };
        let result = run(&context);
        let events = events.lock().unwrap().clone();
        (result, events)
    }

    fn tunnel_set_up(name: Result<&str, Option<&str>>) -> (Vec<SessionEvent>, usize) {
        let cancels = Arc::new(AtomicUsize::new(0));
        let ip = IpInfo {
            addr: Some("10.10.0.5".into()),
            netmask: None,
            addr6: None,
            netmask6: None,
            dns: Vec::new(),
            nbns: Vec::new(),
            domain: None,
            mtu: 1351,
            split_dns: Vec::new(),
            split_includes: Vec::new(),
            split_excludes: Vec::new(),
            gateway_addr: Some("203.0.113.10".into()),
        };
        let ((), events) = reported(&[PIN], |context| {
            assert!(context.link.arm_cancel({
                let cancels = Arc::clone(&cancels);
                move |command| {
                    assert_eq!(command, Command::Cancel);
                    cancels.fetch_add(1, Ordering::SeqCst);
                }
            }));
            context.tunnel_set_up(
                &ip,
                name.map(str::to_owned)
                    .map_err(|reason| reason.map(str::to_owned)),
            );
        });
        (events, cancels.load(Ordering::SeqCst))
    }

    #[test]
    fn a_tunnel_device_that_failed_or_has_no_name_ends_the_session() {
        let address = Ipv4Addr::new(10, 10, 0, 5);
        let (events, cancels) = tunnel_set_up(Err(None));
        assert_eq!(events, [SessionEvent::TunnelFailed(Some(address), None)]);
        assert_eq!(cancels, 1);

        let reason = "Operation not permitted (os error 1)";
        let (events, cancels) = tunnel_set_up(Err(Some(reason)));
        assert_eq!(
            events,
            [SessionEvent::TunnelFailed(
                Some(address),
                Some(reason.to_owned())
            )]
        );
        assert_eq!(cancels, 1);

        let (events, cancels) = tunnel_set_up(Ok("utun5"));
        assert!(
            matches!(events[..], [SessionEvent::TunnelReady(_)]),
            "{events:?}"
        );
        assert_eq!(cancels, 0);
    }

    #[test]
    fn only_a_pinned_certificate_is_trusted() {
        let lowercase = PIN.to_lowercase();
        for pins in [&[OTHER_PIN][..], &[], &[lowercase.as_str()]] {
            assert_eq!(
                reported(pins, |context| context.certificate(PIN)),
                (
                    false,
                    vec![SessionEvent::CertificateRejected(PIN.to_owned())]
                ),
                "{pins:?}"
            );
        }
    }

    #[test]
    fn the_first_pinned_certificate_reaches_the_gateway() {
        assert_eq!(
            reported(&[OTHER_PIN, PIN], |context| {
                [context.certificate(PIN), context.certificate(PIN)]
            }),
            ([true, true], vec![SessionEvent::GatewayTrusted])
        );
    }

    #[test]
    fn only_fortinets_login_form_is_the_login() {
        assert_eq!(form_kind(Some("_login")), Some(FormKind::Login));
        for auth_id in ["_challenge", "loginForm", "unknown", ""] {
            assert_eq!(form_kind(Some(auth_id)), Some(FormKind::Otp), "{auth_id}");
        }
    }

    #[test]
    fn a_form_or_field_without_its_name_cancels_the_form() {
        assert_eq!(form_kind(None), None);
        for kind in [
            OC_FORM_OPT_TEXT,
            OC_FORM_OPT_PASSWORD,
            OC_FORM_OPT_SELECT,
            OC_FORM_OPT_HIDDEN,
        ] {
            assert_eq!(field_value(kind, None, 0), None, "{kind}");
            assert_eq!(field_value(kind, None, OC_FORM_OPT_IGNORE), None, "{kind}");
        }
    }

    #[test]
    fn the_username_and_secret_fields_are_filled() {
        assert_eq!(
            field_value(OC_FORM_OPT_TEXT, Some("username"), 0),
            Some(FieldValue::Username)
        );
        assert_eq!(
            field_value(OC_FORM_OPT_PASSWORD, Some("credential"), 0),
            Some(FieldValue::Secret)
        );
        assert_eq!(
            field_value(OC_FORM_OPT_PASSWORD, Some("code"), 0),
            Some(FieldValue::Secret)
        );
        for name in ["USER", "uname", "Username_2"] {
            assert_eq!(
                field_value(OC_FORM_OPT_TEXT, Some(name), 0),
                Some(FieldValue::Username),
                "{name}"
            );
        }
        for name in ["use", "domain", "", "us€r"] {
            assert_eq!(
                field_value(OC_FORM_OPT_TEXT, Some(name), 0),
                Some(FieldValue::Unchanged),
                "{name}"
            );
        }
    }

    #[test]
    fn hidden_select_and_ignored_fields_are_left_alone() {
        assert_eq!(
            field_value(OC_FORM_OPT_HIDDEN, Some("username"), 0),
            Some(FieldValue::Unchanged)
        );
        assert_eq!(
            field_value(OC_FORM_OPT_HIDDEN, Some("magic"), 0),
            Some(FieldValue::Unchanged)
        );
        assert_eq!(
            field_value(OC_FORM_OPT_SELECT, Some("group"), 0),
            Some(FieldValue::Unchanged)
        );
        for kind in [OC_FORM_OPT_TEXT, OC_FORM_OPT_PASSWORD] {
            assert_eq!(
                field_value(kind, Some("username"), OC_FORM_OPT_IGNORE),
                Some(FieldValue::Unchanged)
            );
        }
    }
}
