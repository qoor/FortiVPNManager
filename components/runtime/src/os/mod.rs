use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use fvm_contract::app_internal::port::SecretStoreFailure;
use fvm_contract::app_internal::{FromRuntime, SecretText};
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

/// A secret the app keeps in the OS's secret storage.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum SecretItem {
    VpnPassword {
        username: String,
        host: String,
    },
    GmailAppPassword {
        address: String,
    },
    /// Matches the address without regard to case.
    GoogleRefreshToken {
        address: String,
    },
    GoogleClientSecret {
        client_id: String,
    },
}

impl SecretItem {
    /// The service the item is stored under. Changing it loses the items already saved.
    pub(crate) fn service(&self) -> &'static str {
        match self {
            Self::VpnPassword { .. } => "com.qoor.FortiVPNManager.vpn",
            Self::GmailAppPassword { .. } => "com.qoor.FortiVPNManager.gmail",
            Self::GoogleRefreshToken { .. } | Self::GoogleClientSecret { .. } => {
                "com.qoor.FortiVPNManager.google"
            }
        }
    }

    /// The account the item is stored under, kept like `service`.
    pub(crate) fn account(&self) -> String {
        match self {
            Self::VpnPassword { username, host } => format!("{username}@{host}"),
            Self::GmailAppPassword { address } => address.clone(),
            Self::GoogleRefreshToken { address } => {
                format!("refresh-token:{}", address.to_lowercase())
            }
            Self::GoogleClientSecret { client_id } => format!("client-secret:{client_id}"),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum OsRequest {
    /// Answered by `Secret`, or `SecretStoreFailed`.
    ReadSecret {
        item: SecretItem,
    },
    /// Answered by `HasSecret`, or `SecretStoreFailed`. Never reads the secret, so the OS asks the
    /// user for nothing.
    HasSecret {
        item: SecretItem,
    },
    /// Answered by `SecretChanged`, or `SecretStoreFailed`.
    WriteSecret {
        item: SecretItem,
        value: SecretText,
    },
    /// Answered by `SecretChanged`, or `SecretStoreFailed`; deleting a missing item succeeds.
    DeleteSecret {
        item: SecretItem,
    },
    InterfaceAddresses,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum OsResponse {
    Secret { value: Option<SecretText> },
    HasSecret { exists: bool },
    SecretChanged,
    SecretStoreFailed { failure: SecretStoreFailure },
    InterfaceAddresses { addresses: Vec<InterfaceAddress> },
}

pub(crate) fn describe(failure: &SecretStoreFailure) -> String {
    failure
        .description
        .clone()
        .unwrap_or_else(|| platform::unknown_error(failure.code))
}

pub(crate) fn connect(app: Sender<FromRuntime>, inputs: Sender<Input>) -> Sender<Outgoing> {
    let (requests, received) = mpsc::channel();
    let os_inputs = inputs.clone();
    thread::Builder::new()
        .name("fvm-runtime-os".into())
        .spawn(move || serve(&received, &os_inputs))
        .expect("starting the runtime's OS thread");
    let (outgoing, messages) = mpsc::channel();
    thread::Builder::new()
        .name("fvm-runtime-out".into())
        .spawn(move || pass(&messages, &app, &requests, &inputs))
        .expect("starting the runtime's outgoing thread");
    outgoing
}

fn pass(
    messages: &Receiver<Outgoing>,
    app: &Sender<FromRuntime>,
    requests: &Sender<(u64, OsRequest)>,
    inputs: &Sender<Input>,
) {
    for message in messages {
        match message {
            Outgoing::App(message) => {
                let _ = app.send(message);
            }
            Outgoing::Os { id, request } => {
                let _ = requests.send((id, request));
            }
            Outgoing::WatchNetworkSettings => platform::watch_network_settings(inputs.clone()),
        }
    }
}

fn serve(requests: &Receiver<(u64, OsRequest)>, inputs: &Sender<Input>) {
    for (id, request) in requests {
        let _ = inputs.send(Input::Os {
            id,
            response: answer(request),
        });
    }
}

fn answer(request: OsRequest) -> OsResponse {
    let changed = |result: Result<(), SecretStoreFailure>| match result {
        Ok(()) => OsResponse::SecretChanged,
        Err(failure) => OsResponse::SecretStoreFailed { failure },
    };
    match request {
        OsRequest::ReadSecret { item } => {
            match platform::read_secret(item.service(), &item.account()) {
                Ok(value) => OsResponse::Secret {
                    value: value.map(SecretText),
                },
                Err(failure) => OsResponse::SecretStoreFailed { failure },
            }
        }
        OsRequest::HasSecret { item } => {
            match platform::has_secret(item.service(), &item.account()) {
                Ok(exists) => OsResponse::HasSecret { exists },
                Err(failure) => OsResponse::SecretStoreFailed { failure },
            }
        }
        OsRequest::WriteSecret { item, value } => changed(platform::write_secret(
            item.service(),
            &item.account(),
            &value.0,
        )),
        OsRequest::DeleteSecret { item } => {
            changed(platform::delete_secret(item.service(), &item.account()))
        }
        OsRequest::InterfaceAddresses => OsResponse::InterfaceAddresses {
            addresses: platform::interface_addresses(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_keep_their_stored_names() {
        let items = [
            SecretItem::VpnPassword {
                username: "alice".into(),
                host: "vpn.example.com".into(),
            },
            SecretItem::GmailAppPassword {
                address: "Alice@example.com".into(),
            },
            SecretItem::GoogleRefreshToken {
                address: "Alice@Example.com".into(),
            },
            SecretItem::GoogleClientSecret {
                client_id: "987-xyz.apps.googleusercontent.com".into(),
            },
        ];
        assert_eq!(
            items.iter().map(SecretItem::service).collect::<Vec<_>>(),
            [
                "com.qoor.FortiVPNManager.vpn",
                "com.qoor.FortiVPNManager.gmail",
                "com.qoor.FortiVPNManager.google",
                "com.qoor.FortiVPNManager.google",
            ]
        );
        assert_eq!(
            items.iter().map(SecretItem::account).collect::<Vec<_>>(),
            [
                "alice@vpn.example.com",
                "Alice@example.com",
                "refresh-token:alice@example.com",
                "client-secret:987-xyz.apps.googleusercontent.com",
            ]
        );
    }
}
