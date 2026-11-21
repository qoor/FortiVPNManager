//! The NSXPC side: accepts connections from the FortiVPNManager app (the listener pins its signing
//! identity) and exports a service bound to each one, which answers the version check itself and
//! passes every other request, unread, to the Helper entry.

use std::ptr::NonNull;
use std::sync::Arc;

use block2::{Block, RcBlock};
use fvm_contract::app_helper::{HelperError, HelperErrorCode};
use fvm_contract::helper_internal::{AppRequest, LogCategory};
use fvm_os_api_macos::ipc::{self, DataReply, FlagReply, OptionalDataReply, Peer};
use fvm_os_api_macos::log::Level;
use objc2::rc::Retained;
use objc2::runtime::{AnyProtocol, Bool, NSObject, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class};
use objc2_foundation::{
    NSData, NSObjectProtocol, NSString, NSXPCConnection, NSXPCListener, NSXPCListenerDelegate,
};
use serde_json::json;

use crate::relay::{ClientLink, Relay, Reply};
use crate::self_update::Watcher;
use crate::{APP_ID, BUILD, HELPER_ID, log};

pub(crate) struct ServiceIvars {
    relay: Arc<Relay>,
    client: Arc<ClientLink>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "FVMHelperService"]
    #[ivars = ServiceIvars]
    struct Service;

    unsafe impl NSObjectProtocol for Service {}

    impl Service {
        #[unsafe(method(getVersionWithReply:))]
        fn get_version(&self, reply: &Block<dyn Fn(NonNull<NSData>)>) {
            let version = serde_json::to_vec(&json!({"value": BUILD}))
                .expect("a build ID encodes as JSON");
            DataReply::new(reply).send(&version);
        }

        #[unsafe(method(connect:reply:))]
        fn connect(&self, request: &NSData, reply: &Block<dyn Fn(*mut NSData)>) {
            let reply = OptionalDataReply::new(reply);
            let Ok(json) = String::from_utf8(request.to_vec()) else {
                let error = HelperError {
                    code: HelperErrorCode::InvalidRequest,
                    message: "could not decode the connect request".to_owned(),
                };
                let error = serde_json::to_vec(&error).expect("an error encodes as JSON");
                reply.send(Some(&error));
                return;
            };
            self.forward(AppRequest::Connect { json }, Reply::OptionalData(reply));
        }

        #[unsafe(method(submitOTPWithAttemptID:code:reply:))]
        fn submit_otp(
            &self,
            attempt: &NSString,
            code: &NSString,
            reply: &Block<dyn Fn(*mut NSData)>,
        ) {
            self.forward(
                AppRequest::SubmitOtp {
                    attempt: attempt.to_string(),
                    code: code.to_string(),
                },
                Reply::OptionalData(OptionalDataReply::new(reply)),
            );
        }

        #[unsafe(method(disconnectWithAttemptID:reason:reply:))]
        fn disconnect(
            &self,
            attempt: &NSString,
            reason: &NSString,
            reply: &Block<dyn Fn(*mut NSData)>,
        ) {
            self.forward(
                AppRequest::Disconnect {
                    attempt: attempt.to_string(),
                    reason: reason.to_string(),
                },
                Reply::OptionalData(OptionalDataReply::new(reply)),
            );
        }

        #[unsafe(method(statusWithReply:))]
        fn status(&self, reply: &Block<dyn Fn(NonNull<NSData>)>) {
            self.forward(AppRequest::Status {}, Reply::Data(DataReply::new(reply)));
        }

        #[unsafe(method(exitWhenIdleWithReply:))]
        fn exit_when_idle(&self, reply: &Block<dyn Fn(Bool)>) {
            self.forward(AppRequest::ExitWhenIdle {}, Reply::Flag(FlagReply::new(reply)));
        }

        #[unsafe(method(bypassRoutesWithReply:))]
        fn bypass_routes(&self, reply: &Block<dyn Fn(NonNull<NSData>)>) {
            self.forward(
                AppRequest::ReadBypassRoutes {},
                Reply::Data(DataReply::new(reply)),
            );
        }

        /// Bytes that are not UTF-8 reach the entry replaced, and it refuses what it cannot read.
        #[unsafe(method(setBypassRoutes:reply:))]
        fn set_bypass_routes(&self, request: &NSData, reply: &Block<dyn Fn(NonNull<NSData>)>) {
            self.forward(
                AppRequest::WriteBypassRoutes { json: text(request) },
                Reply::Data(DataReply::new(reply)),
            );
        }

        #[unsafe(method(prepareUpdate:reply:))]
        fn prepare_update(&self, request: &NSData, reply: &Block<dyn Fn(NonNull<NSData>)>) {
            self.forward(
                AppRequest::PrepareUpdate { json: text(request) },
                Reply::Data(DataReply::new(reply)),
            );
        }

        #[unsafe(method(installUpdate:reply:))]
        fn install_update(&self, request: &NSData, reply: &Block<dyn Fn(NonNull<NSData>)>) {
            self.forward(
                AppRequest::InstallUpdate { json: text(request) },
                Reply::Data(DataReply::new(reply)),
            );
        }

        #[unsafe(method(preparedUpdateWithReply:))]
        fn prepared_update(&self, reply: &Block<dyn Fn(NonNull<NSData>)>) {
            self.forward(
                AppRequest::ReadPreparedUpdate {},
                Reply::Data(DataReply::new(reply)),
            );
        }

        #[unsafe(method(discardPreparedUpdateWithReply:))]
        fn discard_prepared_update(&self, reply: &Block<dyn Fn(NonNull<NSData>)>) {
            self.forward(
                AppRequest::DiscardPreparedUpdate {},
                Reply::Data(DataReply::new(reply)),
            );
        }
    }
);

impl Service {
    fn forward(&self, request: AppRequest, reply: Reply) {
        let ivars = self.ivars();
        ivars.relay.forward(request, &ivars.client, reply);
    }
}

fn text(request: &NSData) -> String {
    String::from_utf8_lossy(&request.to_vec()).into_owned()
}

pub(crate) struct DelegateIvars {
    relay: Arc<Relay>,
    watcher: Arc<Watcher>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "FVMHelperListenerDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl NSXPCListenerDelegate for Delegate {
        #[unsafe(method(listener:shouldAcceptNewConnection:))]
        fn should_accept(&self, _listener: &NSXPCListener, connection: &NSXPCConnection) -> Bool {
            let ivars = self.ivars();
            let pid = connection.processIdentifier();
            let Some(client) = ivars.relay.admit(
                Peer::new(connection),
                connection.effectiveUserIdentifier(),
                pid,
                ivars.watcher.is_replaced(),
            ) else {
                log(
                    LogCategory::Helper,
                    Level::Notice,
                    &format!("refusing a connection from pid {pid}: helper is exiting"),
                );
                return Bool::NO;
            };
            connection.setExportedInterface(Some(&ipc::interface(protocol(HELPER_PROTOCOL))));
            let service = ipc::init(Service::alloc().set_ivars(ServiceIvars {
                relay: Arc::clone(&ivars.relay),
                client: Arc::clone(&client),
            }));
            ipc::set_exported_object(connection, &service);
            connection.setRemoteObjectInterface(Some(&ipc::interface(protocol(CLIENT_PROTOCOL))));
            let relay = Arc::clone(&ivars.relay);
            let gone = Arc::clone(&client);
            connection.setInvalidationHandler(Some(&RcBlock::new(move || {
                relay.disconnected(&gone);
            })));
            connection.resume();
            log(
                LogCategory::Helper,
                Level::Notice,
                &format!("accepted client pid {}, uid {}", client.pid, client.uid),
            );
            Bool::YES
        }
    }
);

const HELPER_PROTOCOL: &std::ffi::CStr = c"HelperXPCProtocol";
const CLIENT_PROTOCOL: &std::ffi::CStr = c"HelperClientXPCProtocol";

fn protocol(name: &std::ffi::CStr) -> &'static AnyProtocol {
    AnyProtocol::get(name).unwrap_or_else(|| panic!("the protocol {name:?} is linked in"))
}

/// What keeps the listener running; the listener holds its delegate weakly.
pub(crate) struct Listener {
    _listener: Retained<NSXPCListener>,
    _delegate: Retained<Delegate>,
}

pub(crate) fn listen(relay: Arc<Relay>, watcher: Arc<Watcher>, team_id: &str) -> Listener {
    let delegate = ipc::init(Delegate::alloc().set_ivars(DelegateIvars { relay, watcher }));
    let listener = NSXPCListener::initWithMachServiceName(
        NSXPCListener::alloc(),
        &NSString::from_str(HELPER_ID),
    );
    let requirement = format!(
        "anchor apple generic and identifier \"{APP_ID}\" and certificate leaf[subject.OU] = \"{team_id}\""
    );
    listener.setConnectionCodeSigningRequirement(&NSString::from_str(&requirement));
    listener.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    listener.resume();
    Listener {
        _listener: listener,
        _delegate: delegate,
    }
}

#[cfg(test)]
mod tests {
    use objc2::runtime::Sel;
    use objc2::{ClassType, sel};

    use super::*;

    #[test]
    fn the_shared_protocols_are_registered_and_make_interfaces() {
        for name in [HELPER_PROTOCOL, CLIENT_PROTOCOL] {
            ipc::interface(protocol(name));
        }
    }

    fn app_side() -> Peer {
        let listener = NSXPCListener::anonymousListener();
        let connection = NSXPCConnection::initWithListenerEndpoint(
            NSXPCConnection::alloc(),
            &listener.endpoint(),
        );
        connection.setRemoteObjectInterface(Some(&ipc::interface(protocol(CLIENT_PROTOCOL))));
        connection.resume();
        let peer = Peer::new(&connection);
        std::mem::forget((listener, connection));
        peer
    }

    #[test]
    fn events_and_output_go_through_the_client_protocol() {
        let peer = app_side();
        peer.send(sel!(event:), b"{}", |_| {});
        peer.send(sel!(output:), b"{}", |_| {});
    }

    #[test]
    #[should_panic(expected = "the remote interface has no")]
    fn a_selector_outside_the_client_protocol_is_never_sent() {
        app_side().send(sel!(status:), b"{}", |_| {});
    }

    #[test]
    fn the_service_answers_every_request_of_the_shared_protocol() {
        let requests: [Sel; 12] = [
            sel!(getVersionWithReply:),
            sel!(connect:reply:),
            sel!(submitOTPWithAttemptID:code:reply:),
            sel!(disconnectWithAttemptID:reason:reply:),
            sel!(statusWithReply:),
            sel!(exitWhenIdleWithReply:),
            sel!(bypassRoutesWithReply:),
            sel!(setBypassRoutes:reply:),
            sel!(prepareUpdate:reply:),
            sel!(installUpdate:reply:),
            sel!(preparedUpdateWithReply:),
            sel!(discardPreparedUpdateWithReply:),
        ];
        for request in requests {
            assert!(
                Service::class().instance_method(request).is_some(),
                "{request:?}"
            );
        }
    }
}
