//! Blast-radius and call-path internals for `lievo_explore`.
//!
//! `core` holds the entry shape, the hop-2 reverse traversal, and the
//! owning-file normalization (extracted from a single module, issue #834,
//! to keep each file under the 500-line source budget). `outer` holds the
//! edge-set constants, the seed/entry helpers, the lean dependent-count
//! hint, and the public `call_paths_and_blast_radius` entry point.
//!
//! The test modules live alongside this directory's source files and are
//! declared with plain relative module paths.

mod core;
mod outer;
pub(crate) use core::{OwningFileResult, owning_file};

#[cfg(test)]
pub(crate) mod tools_explore_blast_dedup_tests;
#[cfg(test)]
pub(crate) mod tools_explore_blast_emit_tests;
#[cfg(test)]
pub(crate) mod tools_explore_blast_hop_tests;
#[cfg(test)]
pub(crate) mod tools_explore_blast_tests;
#[cfg(test)]
pub(crate) mod tools_explore_lean_hint_tests;
pub(crate) use outer::{
    call_paths_and_blast_radius, file_function_entity_id, is_dependency_edge, lean_dependents_hint,
};

#[cfg(test)]
pub(crate) use outer::MAX_BLAST_ENTRIES;

#[cfg(test)]
pub(crate) use outer::{count_direct_dependents, direct_seeds};
