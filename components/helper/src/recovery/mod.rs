//! What a Helper leaves for a later one: the run directory with the run record and the record of the
//! exception routes it added, whose formats this module alone decides; cleaning up after an earlier
//! Helper; and the route repair a run asks for. The cleanup gets the backend adapters of every
//! client the OS can run from `process`.

mod leftovers;
pub(crate) mod run_directory;

use std::net::Ipv4Addr;
use std::thread;

use fvm_contract::backend::RouteRepairOutcome;

pub use run_directory::Failure;

pub(crate) use leftovers::clean_up;
pub(crate) use run_directory::write_bypass_record;

/// Finishes the route repair a run asked for. The run waits for it, so dropping it unfinished
/// panics.
#[must_use = "the run reports its exit only after the route repair is finished"]
pub struct RouteRepair {
    /// The gateway the client connected to, if its IPv4 address is known.
    pub gateway: Option<Ipv4Addr>,
    /// Gives the outcome to the run that asked, which says how when it asks.
    answer: Option<Box<dyn FnOnce(RouteRepairOutcome) + Send>>,
}

impl RouteRepair {
    pub(crate) fn new(
        gateway: Option<Ipv4Addr>,
        answer: impl FnOnce(RouteRepairOutcome) + Send + 'static,
    ) -> Self {
        Self {
            gateway,
            answer: Some(Box::new(answer)),
        }
    }

    pub fn finish(mut self, outcome: RouteRepairOutcome) {
        let answer = self.answer.take().expect("a route repair is finished once");
        answer(outcome);
    }
}

impl Drop for RouteRepair {
    fn drop(&mut self) {
        if self.answer.is_some() && !thread::panicking() {
            panic!("a route repair was dropped unfinished; the run would never report its exit");
        }
    }
}
