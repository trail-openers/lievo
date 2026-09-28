// Extraction pipeline - code analysis
// Includes: Grouping, Metrics, tree-sitter based extraction, etc.

mod call_extraction;
pub mod code_extractor;
pub mod detectors;
pub mod entity_id;
pub mod framework;
pub(crate) mod framework_profiles;
pub(crate) mod framework_readers;
pub mod function_preservation;
mod go_imports;
pub mod grouping;
pub mod grouping_filter;
mod grouping_helpers;
mod grouping_impl;
pub mod tree_sitter_extractor;
mod tree_sitter_utils;
pub use tree_sitter_utils::{lievo_data_dir, repo_hash, ts_index_dir_for_repo};
mod ts_metrics;
