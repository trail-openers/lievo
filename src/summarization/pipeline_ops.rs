// Function-summary upsert machinery for the summarization pipeline.
//
// Extracted from pipeline.rs to keep that file under the 500-line limit
// (AGENTS.md §6). This is an `include!`d file (see
// `mod pipeline_ops { include!(...) }` in pipeline.rs), so `super` refers to
// the pipeline module's scope: the moved items use the pipeline's
// `SummarizationPipeline::hash_code` helper, and `batch_ops` (a sibling
// include!d mod) reaches them via `super::pipeline_ops::*`.

use crate::extraction::function_preservation::function_id;
use crate::model::CodeUnit;
use crate::storage::Storage;

use super::SummarizationPipeline;

/// Counters for upsert outcomes during function summarization.
///
/// Fields are `pub(super)` so the sibling `batch_ops` include! (a distinct
/// module scope) can increment them while updating progress.
#[derive(Debug, Default)]
pub struct UpsertCounters {
    pub summarized: u64,
    pub skipped_file: u64,
    pub skipped_fn_entity: u64,
    pub skipped_no_code: u64,
    pub skipped_oversized: u64,
    /// Functions whose batch response had a line but no parsable `#<n>:`
    /// entry for them, so no summary was written. Distinct from
    /// `skipped_oversized` (input too large) and from batch-level IO failures
    /// (where the model never produced output). Surfacing this separately is
    /// the first-class outcome #776 requires: an arbitrary GGUF model may drift
    /// from the `#<n>:` format as batch size grows, and the drift must be
    /// visible in production rather than hidden in a salvage warning.
    pub parse_failures: u64,
}

/// Outcome of attempting to upsert a function summary.
///
/// This typed enum distinguishes between file-not-found, function-not-found,
/// code-unavailable, and storage write errors without relying on string matching in error messages.
#[derive(Debug)]
#[doc(hidden)]
pub enum SummaryUpsertOutcome {
    /// Summary was upserted successfully.
    Ok(usize),
    /// File entity was not found (summary cannot be anchored).
    FileMissing,
    /// Function entity was not found (summary has nowhere to go).
    FunctionMissing,
    /// Function code was unavailable (CodeUnit had no code).
    CodeUnavailable,
    /// Storage write failed (entity found, but upsert failed).
    UpsertFailed,
}

/// Apply an upsert outcome to the counters, incrementing the appropriate counter.
pub fn apply_upsert_outcome(
    outcome: SummaryUpsertOutcome,
    counters: &mut UpsertCounters,
) -> usize {
    match outcome {
        SummaryUpsertOutcome::Ok(count) => {
            counters.summarized += count as u64;
            count
        }
        SummaryUpsertOutcome::FileMissing => {
            counters.skipped_file += 1;
            0
        }
        SummaryUpsertOutcome::FunctionMissing => {
            counters.skipped_fn_entity += 1;
            0
        }
        SummaryUpsertOutcome::CodeUnavailable => {
            counters.skipped_no_code += 1;
            0
        }
        SummaryUpsertOutcome::UpsertFailed => {
            // Storage write error — already warned, don't increment skip counters
            0
        }
    }
}

/// Upsert a function summary by looking up the file and function entities.
///
/// This helper is shared across three paths: present partial batch,
/// missing individual retry, and main successful batch. It returns
/// a typed outcome indicating success or the specific skip reason.
///
/// Caller is responsible for locating the CodeUnit and verifying it has code.
///
/// Returns:
/// - `SummaryUpsertOutcome::Ok(count)` on success
/// - `SummaryUpsertOutcome::FileMissing` if the file entity is not found
/// - `SummaryUpsertOutcome::FunctionMissing` if the function entity is not found
/// - `SummaryUpsertOutcome::CodeUnavailable` if the function code is unavailable
///
/// Genuine storage errors EntityNotFound errors for both file and function entities.
/// All other errors are propagated as the real LievoError.
pub fn upsert_function_summary(
    storage: &dyn Storage,
    repo_id: &str,
    unit: &CodeUnit,
    summary: &str,
) -> SummaryUpsertOutcome {
    let code = match unit.code.as_ref() {
        Some(code) => code,
        None => {
            eprintln!("warning: no code available for function: {}", unit.name);
            return SummaryUpsertOutcome::CodeUnavailable;
        }
    };
    let code_hash = SummarizationPipeline::hash_code(code);

    // Look up the FILE entity to get its stable ID, then derive the FUNCTION entity ID.
    let file_entity = match storage.entity_by_path(repo_id, &unit.file) {
        Ok(Some(entity)) => entity,
        Ok(None) => return SummaryUpsertOutcome::FileMissing,
        Err(e) => {
            eprintln!(
                "warning: failed to look up file entity for {}: {e}",
                unit.name
            );
            return SummaryUpsertOutcome::FileMissing;
        }
    };
    let fn_entity_id = function_id(&file_entity.id, unit.name.as_str());

    // Function entity must exist — otherwise the summary has nowhere to go.
    let target_entity = match storage.get_entity(&fn_entity_id) {
        Ok(Some(entity)) => entity,
        Ok(None) => return SummaryUpsertOutcome::FunctionMissing,
        Err(e) => {
            eprintln!(
                "warning: failed to look up function entity for {}: {e}",
                unit.name
            );
            return SummaryUpsertOutcome::FunctionMissing;
        }
    };

    let mut updated = target_entity;
    updated.summary = Some(summary.to_string());
    updated.summary_commit = Some(code_hash);
    if let Err(e) = storage.upsert_entity(&updated) {
        eprintln!("warning: failed to upsert summary for {}: {e}", unit.name);
        return SummaryUpsertOutcome::UpsertFailed;
    }

    SummaryUpsertOutcome::Ok(1)
}
