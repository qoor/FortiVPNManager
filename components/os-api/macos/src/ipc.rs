use std::ptr::{self, NonNull};

use block2::{Block, RcBlock};
use objc2::rc::{PartialInit, Retained, Weak};
use objc2::runtime::{AnyObject, AnyProtocol, Bool, MessageReceiver, NSObject, Sel};
use objc2::{ClassType, DefinedClass, msg_send};
use objc2_foundation::{NSData, NSError, NSXPCConnection, NSXPCInterface};

pub fn interface(protocol: &AnyProtocol) -> Retained<NSXPCInterface> {
    // SAFETY: a protocol the runtime knows, with methods NSXPC can describe.
    unsafe { NSXPCInterface::interfaceWithProtocol(protocol) }
}

pub fn set_exported_object(connection: &NSXPCConnection, object: &AnyObject) {
    // SAFETY: the caller set the exported interface to a protocol that `object` implements.
    unsafe { connection.setExportedObject(Some(object)) }
}

pub fn init<T>(this: PartialInit<T>) -> Retained<T>
where
    T: DefinedClass + ClassType<Super = NSObject>,
{
    // SAFETY: NSObject's init returns the same, now initialized, object.
    unsafe { msg_send![super(this), init] }
}

fn send_data(receiver: &AnyObject, selector: Sel, data: &NSData) {
    // SAFETY: the receiver's protocol declares `selector` with one object argument and no result.
    let () = unsafe { receiver.send_message(selector, (data,)) };
}

fn remote_proxy(
    connection: &NSXPCConnection,
    failed: impl Fn(String) + 'static,
) -> Retained<AnyObject> {
    let handler = RcBlock::new(move |error: NonNull<NSError>| {
        // SAFETY: NSXPC passes an error that lives through the call.
        let error = unsafe { error.as_ref() };
        failed(error.localizedDescription().to_string());
    });
    connection.remoteObjectProxyWithErrorHandler(&handler)
}

pub struct Peer(Weak<NSXPCConnection>);

// SAFETY: NSXPCConnection may be messaged from any thread, and the weak reference is loaded
// atomically.
unsafe impl Send for Peer {}
// SAFETY: as above.
unsafe impl Sync for Peer {}

impl Peer {
    pub fn new(connection: &NSXPCConnection) -> Self {
        Self(Weak::new(connection))
    }

    pub fn send(&self, selector: Sel, data: &[u8], failed: impl Fn(String) + Send + 'static) {
        let Some(connection) = self.0.load() else {
            return;
        };
        let proxy = remote_proxy(&connection, failed);
        let method = proxy
            .class()
            .instance_method(selector)
            .unwrap_or_else(|| panic!("the remote interface has no {selector:?}"));
        assert!(
            method.arguments_count() == 3
                && method.return_type().to_bytes() == b"v"
                && method
                    .argument_type(2)
                    .is_some_and(|argument| argument.to_bytes() == b"@"),
            "{selector:?} of the remote interface does not take one object and return nothing"
        );
        send_data(&proxy, selector, &NSData::with_bytes(data));
    }

    pub fn after_pending_sends(&self, block: impl Fn() + Send + 'static) {
        match self.0.load() {
            Some(connection) => connection.scheduleSendBarrierBlock(&RcBlock::new(block)),
            None => block(),
        }
    }
}

pub struct DataReply(RcBlock<dyn Fn(NonNull<NSData>)>);

// SAFETY: NSXPC reply blocks may be called from any thread.
unsafe impl Send for DataReply {}

impl DataReply {
    pub fn new(reply: &Block<dyn Fn(NonNull<NSData>)>) -> Self {
        Self(reply.copy())
    }

    pub fn send(self, data: &[u8]) {
        let data = NSData::with_bytes(data);
        self.0.call((NonNull::from(&*data),));
    }
}

pub struct OptionalDataReply(RcBlock<dyn Fn(*mut NSData)>);

// SAFETY: NSXPC reply blocks may be called from any thread.
unsafe impl Send for OptionalDataReply {}

impl OptionalDataReply {
    pub fn new(reply: &Block<dyn Fn(*mut NSData)>) -> Self {
        Self(reply.copy())
    }

    pub fn send(self, data: Option<&[u8]>) {
        match data {
            Some(data) => {
                let data = NSData::with_bytes(data);
                self.0.call((Retained::as_ptr(&data).cast_mut(),));
            }
            None => self.0.call((ptr::null_mut(),)),
        }
    }
}

pub struct FlagReply(RcBlock<dyn Fn(Bool)>);

// SAFETY: NSXPC reply blocks may be called from any thread.
unsafe impl Send for FlagReply {}

impl FlagReply {
    pub fn new(reply: &Block<dyn Fn(Bool)>) -> Self {
        Self(reply.copy())
    }

    pub fn send(self, value: bool) {
        self.0.call((Bool::new(value),));
    }
}
