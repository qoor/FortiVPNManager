//! openconnect 9.21 with its fortinet protocol, run in the Helper's process through libopenconnect;
//! on Windows with the Wintun driver.

mod ffi;
mod network;
mod request;
mod session;

use std::sync::OnceLock;

use fvm_contract::app_helper::RunRequest;
use fvm_helper_core::session::backend::{Backend, Link, SessionEnd, SessionRequest};

#[derive(Default)]
pub struct OpenConnect {
    version: OnceLock<String>,
}

impl Backend for OpenConnect {
    fn init(&self) {
        ffi::init();
    }

    fn name(&self) -> &'static str {
        "openconnect"
    }

    fn version(&self) -> &str {
        self.version.get_or_init(|| {
            let version = ffi::version();
            version.strip_prefix('v').unwrap_or(&version).to_owned()
        })
    }

    fn check(&self, request: &RunRequest) -> Result<(), String> {
        request::check(request)
    }

    fn run(&self, request: SessionRequest, link: &Link) -> SessionEnd {
        session::run(&request, link)
    }
}
