// Batch processing operations for the summarization pipeline.
//
// Extracted from pipeline.rs to keep that file under the 500-line limit
// (AGENTS.md §6). Each function here handles one path of batch result
// processing: full batches (all positions have summaries) and partial
// batches (some positions missing, requiring salvage + retry).
//
// This file is an `include!`d file (see `mod batch_ops { include!(...) }` in
// pipeline.rs), so `super` refers to the pipeline module's scope.

use crate::model::CodeUnit;
use crate::storage::Storage;
use crate::summarization::apfel::{summarize_code, BackendTransport};
use indicatif::ProgressBar;

use super::pipeline_ops::{UpsertCounters, apply_upsert_outcome, upsert_function_summary};
use super::is_overflow_error;

/// Context for batch processing, bundling the shared parameters.
pub(crate) struct BatchCtx<'a> {
    pub(crate) storage: &'a dyn Storage,
    pub(crate) repo_id: &'a str,
    pub(crate) batch: &'a [(String, String, String)],
    pub(crate) results: &'a [Option<String>],
    pub(crate) counters: &'a mut UpsertCounters,
    pub(crate) pb: &'a ProgressBar,
    /// Per-backend input budget (chars) resolved once for this run (issue
    /// #792) — the retry guard must use the same value `batch_summarize`
    /// guarded with, or an input between the backend budgets would be
    /// summarized in-batch but re-skipped on retry.
    pub(crate) input_char_budget: usize,
    /// The explicit HTTP transport for this run (issue #783), or `None` for
    /// the one-shot CLI fallback.
    pub(crate) transport: Option<&'a BackendTransport>,
}

/// Build a minimal `CodeUnit` from batch tuple data (file, name, code).
fn make_unit(file: &str, name: &str, code: &str) -> CodeUnit {
    CodeUnit {
        name: name.to_string(),
        unit_type: "function".to_string(),
        file: file.to_string(),
        line: 0,
        end_line: 0,
        language: "Rust".to_string(),
        signature: None,
        code: Some(code.to_string()),
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
        qualified_name: name.to_string(),
        docstring: None,
        parent_class: None,
    }
}

/// Process a partial batch: upsert present summaries, retry missing positions.
pub(crate) fn process_partial_batch(
    ctx: &mut BatchCtx<'_>,
    present_positions: &[usize],
    missing_positions: &[usize],
) {
    // Upsert present summaries
    for pos in present_positions {
        let summary = match &ctx.results[*pos] {
            Some(s) => s,
            None => {
                ctx.pb.inc(1);
                continue;
            }
        };

        let (file, name, code) = &ctx.batch[*pos];
        let unit = make_unit(file, name, code);
        let outcome = upsert_function_summary(ctx.storage, ctx.repo_id, &unit, summary);
        apply_upsert_outcome(outcome, ctx.counters);
        ctx.pb.inc(1);
    }

    // Retry missing functions individually
    for pos in missing_positions {
        let (file, name, code) = &ctx.batch[*pos];

        // Budget guard: if the code exceeds the resolved per-backend input
        // budget, skip the subprocess call entirely — it would be guaranteed
        // to overflow (issue #649). The batch path already rejected this via
        // batch_summarize's defensive check; the retry must not spawn a
        // subprocess that will hang for APFEL_TIMEOUT_SECS before failing.
        // (issue #792: the guard reads the same resolved value as the batch
        // guard, not the apfel global.)
        if code.len() > ctx.input_char_budget {
            ctx.counters.skipped_oversized += 1;
            ctx.pb.inc(1);
            continue;
        }

        let unit = make_unit(file, name, code);

        match summarize_code(code, ctx.transport) {
            Ok(result) => {
                let outcome =
                    upsert_function_summary(ctx.storage, ctx.repo_id, &unit, &result.content);
                apply_upsert_outcome(outcome, ctx.counters);
            }
            Err(e) => {
                eprintln!(
                    "warning: failed to summarize function {} individually: {e}",
                    name
                );
                // Only context overflow (input too large) is counted as
                // `skipped_oversized` — distinct from IO/timeout failures,
                // which remain untracked individual failures. A non-overflow
                // parse failure (a response line that lacks a parsable
                // `#<n>:` entry) is counted as `parse_failures` (issue #776)
                // rather than silently degrading into the salvage warning.
                if is_overflow_error(&e) {
                    ctx.counters.skipped_oversized += 1;
                } else if crate::summarization::pipeline::is_parse_failure_error(&e) {
                    ctx.counters.parse_failures += 1;
                }
                // Leave summary NULL — a single function failure shouldn't abort the run
            }
        }

        ctx.pb.inc(1);
    }
}

/// Process a full batch (all positions have summaries).
pub(crate) fn process_full_batch(ctx: &mut BatchCtx<'_>, tty: bool) {
    for (i, result) in ctx.results.iter().enumerate() {
        let summary = match result {
            Some(s) => s,
            None => {
                ctx.pb.inc(1);
                continue;
            }
        };

        let (file, name, code) = &ctx.batch[i];
        let unit = make_unit(file, name, code);
        let outcome = upsert_function_summary(ctx.storage, ctx.repo_id, &unit, summary);
        apply_upsert_outcome(outcome, ctx.counters);
        ctx.pb.inc(1);

        if !tty {
            eprint!(".");
        }
    }
}
