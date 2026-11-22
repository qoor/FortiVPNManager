use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use fvm_contract::app_internal::FromRuntime;
use fvm_core::connection::InterfaceAddress;

use crate::orchestration::{Input, Outgoing};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum OsRequest {
    InterfaceAddresses,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum OsResponse {
    InterfaceAddresses { addresses: Vec<InterfaceAddress> },
}

pub(crate) fn connect(app: Sender<FromRuntime>, inputs: Sender<Input>) -> Sender<Outgoing> {
    let (requests, received) = mpsc::channel();
    thread::Builder::new()
        .name("fvm-runtime-os".into())
        .spawn(move || serve(&received, &inputs))
        .expect("starting the runtime's OS thread");
    let (outgoing, messages) = mpsc::channel();
    thread::Builder::new()
        .name("fvm-runtime-out".into())
        .spawn(move || pass(&messages, &app, &requests))
        .expect("starting the runtime's outgoing thread");
    outgoing
}

fn pass(
    messages: &Receiver<Outgoing>,
    app: &Sender<FromRuntime>,
    requests: &Sender<(u64, OsRequest)>,
) {
    for message in messages {
        match message {
            Outgoing::App(message) => {
                let _ = app.send(message);
            }
            Outgoing::Os { id, request } => {
                let _ = requests.send((id, request));
            }
        }
    }
}

fn serve(requests: &Receiver<(u64, OsRequest)>, inputs: &Sender<Input>) {
    for (id, request) in requests {
        let response = match request {
            OsRequest::InterfaceAddresses => OsResponse::InterfaceAddresses {
                addresses: platform::interface_addresses(),
            },
        };
        let _ = inputs.send(Input::Os { id, response });
    }
}
