// Summarization module — on-device code summarization via apfel.

pub mod apfel;
#[cfg(test)]
mod apfel_batch_http_tests;
#[cfg(test)]
mod apfel_server_tests;
pub mod backend_profile;
pub mod enabled_state;
pub mod pipeline;
pub mod pipeline_utils;
pub mod summarizer_ab;
pub mod summarizer_backend;
pub mod unconfigured;

#[cfg(test)]
#[path = "pipeline_tests_fixtures.rs"]
pub(crate) mod pipeline_tests_fixtures;

#[cfg(test)]
pub(crate) mod summarizer_fullpath_tests;

pub use apfel::{ApfelSummaryResult, summarize_code};
pub use enabled_state::{EnabledState, classify_enabled_state};
pub use pipeline::{
    RollupSkips, SkippedRollup, SummarizationConfig, SummarizationPipeline, SummarizeDecision,
    SummaryOutcome, summarize_decision,
};
pub use summarizer_backend::{ApfelServerHandle, acquire_server, apfel_server_available};
