//! Search tool execution paths and stable public entrypoints.

mod acquire;
mod budget;
pub(crate) use budget::finalize_indexed_response;
mod call;
#[cfg(test)]
mod cancel_tests;
mod docs;
mod gating;
mod hybrid;
mod lexical;
mod render;
mod semantic;
mod status;
#[cfg(test)]
mod test_support;
mod types;
mod wait;
#[cfg(test)]
pub(crate) use acquire::{try_acquire_engine, ACQUIRE_POLL};
pub(crate) use acquire::{AdmittedEngine, OwnerLockRefused, OwnerWait};
pub(crate) use call::search_call;
pub(crate) use render::WorkspaceFacts;
pub(crate) use types::{reference_search_output_schema, search_output_schema};
pub(crate) use wait::{await_reply, Withdrawn, REPLY_POLL};

pub use docs::{find_docs, search_docs};
#[allow(unused_imports)]
pub use hybrid::{hybrid_code, hybrid_code_cancellable};
pub use status::search_status;
pub(crate) use status::{
    append_workspace_changes, baseline_warming_not_ready, docs_not_ready, search_not_ready,
};

#[cfg(test)]
mod tests {
    #[test]
    fn all_search_modes_are_resident_only_under_held_lease() {
        super::hybrid::tests::assert_all_search_modes_are_resident_only_under_held_lease();
    }
}
