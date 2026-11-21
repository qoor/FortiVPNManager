//! What a Helper leaves for a later one: the run directory with the run record, the record of the
//! exception routes it added and the record of the network settings a revert left, whose formats
//! this module alone decides, and the session secrets whose format it decides too; cleaning up
//! after an earlier Helper; and the logout request for a session that could not log out itself.

mod leftovers;
pub(crate) mod logout;
pub(crate) mod run_directory;

pub use run_directory::{Failure, RecordSlot, create_private_file};

pub(crate) use leftovers::clean_up;
pub(crate) use run_directory::{write_bypass_record, write_unreverted_record};
