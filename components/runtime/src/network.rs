//! Looking at the network for the core: the gateway's address, the
//! interface the route to it uses, the interfaces' addresses, and whether the VPN client still
//! runs.

use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};

use fvm_contract::app_helper::HelperStatus;
use fvm_contract::app_internal::port::{HelperCall, PortRequest, PortResponse};
use fvm_core::connection::{self, NetworkProbe, ProbeResult};

use crate::orchestration::Runtime;

/// The first IPv4 address of `host`, through the system resolver. The tunnel is IPv4 only.
fn resolve(host: &str) -> Option<Ipv4Addr> {
    (host, 0)
        .to_socket_addrs()
        .ok()?
        .find_map(|address| match address {
            SocketAddr::V4(address) => Some(*address.ip()),
            SocketAddr::V6(_) => None,
        })
}

impl Runtime {
    pub(crate) fn probe_network(&mut self, probe: NetworkProbe) {
        let queue = self.queue();
        let gateway = probe.gateway.clone();
        self.tasks.spawn_blocking(move || {
            let gateway_ip =
                gateway.and_then(|gateway| gateway.known_ip.or_else(|| resolve(&gateway.host)));
            queue.run(move |runtime| runtime.probe_route(probe, gateway_ip));
        });
    }

    fn probe_route(&mut self, probe: NetworkProbe, gateway_ip: Option<Ipv4Addr>) {
        let Some(destination) = gateway_ip else {
            self.probe_addresses(probe, None, None);
            return;
        };
        self.request(
            PortRequest::RouteInterface { destination },
            move |runtime, response| {
                let PortResponse::RouteInterface { interface } = response else {
                    panic!("a route answer")
                };
                runtime.probe_addresses(probe, gateway_ip, interface);
            },
        );
    }

    fn probe_addresses(
        &mut self,
        probe: NetworkProbe,
        gateway_ip: Option<Ipv4Addr>,
        route_interface: Option<String>,
    ) {
        self.request(
            PortRequest::InterfaceAddresses {},
            move |runtime, response| {
                let PortResponse::InterfaceAddresses { addresses } = response else {
                    panic!("an addresses answer")
                };
                let mut result = ProbeResult {
                    id: probe.id,
                    gateway_ip,
                    route_interface,
                    addresses,
                    client_running: None,
                };
                if !probe.client_running {
                    runtime.feed(connection::Input::NetworkProbed(result));
                    return;
                }
                runtime.call_helper(HelperCall::Status {}, move |runtime, response| {
                    result.client_running = match response {
                        PortResponse::HelperReply { json: Some(json) } => {
                            serde_json::from_str::<HelperStatus>(&json)
                                .ok()
                                .map(|status| status.client_running)
                        }
                        _ => None,
                    };
                    runtime.feed(connection::Input::NetworkProbed(result));
                });
            },
        );
    }
}
