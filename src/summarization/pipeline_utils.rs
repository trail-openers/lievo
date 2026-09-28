// Utility functions for the summarization pipeline (issue #788: extracted to
// keep pipeline.rs within the 500-line budget).

use crate::error::LievoError;
use crate::summarization::pipeline::{
    BUDGET_EXCEEDED_MARKER, CONTEXT_OVERFLOW_MARKER, PARSE_FAILURE_MARKER,
};

/// True if a code unit's name follows the Rust test-function naming
/// convention (`test_` prefix). Shared by the pipeline, the entity filter
/// (`function_preservation.rs`), the SQL count (`COUNT_MISSING_SUMMARIES`),
/// and the `refresh` command — all must agree (issue #649). `pub` because
/// the `refresh` handler is in the bin crate.
pub fn is_test_entity(name: &str) -> bool {
    name.to_lowercase().starts_with("test_")
}

/// Classify a context-overflow failure (input too large for apfel's context
/// window). Matches the two known overflow signatures (`[context overflow]`
/// stderr and the budget-exceeded `SummarizationFailed`). Deliberately NOT
/// matched: timeouts, JSON parse failures, spawn/wait IO failures, and
/// generic "apfel failed with status" wrappers.
pub(crate) fn is_overflow_error(err: &LievoError) -> bool {
    let LievoError::SummarizationFailed(msg) = err else {
        return false;
    };
    msg.contains(CONTEXT_OVERFLOW_MARKER) || msg.contains(BUDGET_EXCEEDED_MARKER)
}

/// Classify a demux parse failure: the backend returned a response body but
/// no line carried a valid `#<n>:` entry (issue #776). Distinct from context
/// overflow and from IO/timeout failures.
pub(crate) fn is_parse_failure_error(err: &LievoError) -> bool {
    let LievoError::SummarizationFailed(msg) = err else {
        return false;
    };
    msg.contains(PARSE_FAILURE_MARKER)
}

/// Extract present and missing positions from a partial batch result.
pub(crate) fn salvage_partial_batch(results: &[Option<String>]) -> (Vec<usize>, Vec<usize>) {
    let mut present = Vec::new();
    let mut missing = Vec::new();

    for (i, result) in results.iter().enumerate() {
        if result.is_some() {
            present.push(i);
        } else {
            missing.push(i);
        }
    }

    (present, missing)
}
