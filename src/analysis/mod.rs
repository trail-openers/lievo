// Analysis pipeline - relationships, insights, incremental updates
// Includes: AnalysisPipeline, RelationshipBuilder, InsightsDetector, ConventionDetector

pub mod convention_detector;
pub use coverage::{GateFailure, LanguageCoverage};
pub mod coverage;
pub mod file_hashing;
pub mod flow_tracer;
pub mod import_resolver;
pub mod incremental;
pub mod insights;
pub(crate) mod insights_circular;
pub(crate) mod insights_god;
pub(crate) mod insights_helpers;
#[cfg(test)]
pub mod insights_tests;
pub(crate) mod js_source_root;
pub mod metrics;
pub mod module_map;
pub mod pipeline;
mod pipeline_steps;
mod relationship_helpers;
/// Production `super::`/`self::` module-tree walk (issue #742 task-a),
/// re-exported for the selfcheck verifier's agreement test (issue #742
/// task-b): the verifier's `super::` arm in `selfcheck_metrics.rs` must
/// produce the identical target as this function, or the selfcheck gate
/// flags the new super:: edges as wrong and the CI threshold ratchet
/// regresses. See `relationship_helpers::resolve_rust_relative`.
#[doc(hidden)]
pub use relationship_helpers::resolve_rust_relative_for_agreement;
pub mod relationships;
pub mod rust_mod_parse;
#[cfg(test)]
mod test_helpers;
#[cfg(test)]
pub(crate) mod test_support;
