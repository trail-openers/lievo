// Summarization pipeline — bottom-up summarization: Functions → Files → Modules → Subsystems.
use crate::config::{RepoConfig, SummarizerBackend};
use crate::error::Result;
use crate::extraction::function_preservation::is_function_unit;
use crate::model::{CodeUnit, EntityTier};
use crate::storage::Storage;
use crate::summarization::apfel::{BackendTransport, batch_summarize, pack_by_char_budget};
use indicatif::{ProgressBar, ProgressDrawTarget};
use sha2::{Digest, Sha256};
use std::io::IsTerminal;

mod batch_ops {
    include!("batch_ops.rs");
}

mod pipeline_ops {
    include!("pipeline_ops.rs");
}

mod pipeline_rollup {
    include!("pipeline_rollup.rs");
}

mod pipeline_outcome {
    include!("pipeline_outcome.rs");
}

pub(crate) mod server_dispatch {
    include!("server_dispatch.rs");
}

// Re-exports so `use super::*` in pipeline_tests.rs keeps resolving the
// upsert machinery and helpers (issues #702, #786).
pub use crate::extraction::function_preservation::{function_id, is_test_file_path};
pub use pipeline_ops::{SummaryUpsertOutcome, apply_upsert_outcome, upsert_function_summary};
// Rollup skip outcome types (issue #793) live in pipeline_outcome.rs.
pub(crate) use pipeline_outcome::{RollupOutcome, classify_rollup_skip};
pub use pipeline_outcome::{RollupSkips, SkippedRollup, SummaryOutcome};
// Re-export helpers moved to pipeline_utils.rs (issue #788) so that include!d
// files and external callers that reference them at the `pipeline` module level
// keep working.
pub use crate::summarization::pipeline_utils::is_test_entity;
pub(crate) use crate::summarization::pipeline_utils::{
    is_overflow_error, is_parse_failure_error, salvage_partial_batch,
};
use server_dispatch::remote_endpoint_warning;
/// Marker shared with `apfel.rs` so the classifier and the producers
/// cannot drift silently (issue #649); `PARSE_FAILURE_MARKER` (#776) marks
/// a batch whose response body had no parsable `#<n>:` line.
pub(crate) const CONTEXT_OVERFLOW_MARKER: &str = "[context overflow]";
pub(crate) const BUDGET_EXCEEDED_MARKER: &str = "exceeds budget";
pub(crate) const PARSE_FAILURE_MARKER: &str = "[parse failure]";

/// Configuration for summarization.
#[derive(Debug, Clone)]
pub struct SummarizationConfig {
    pub enabled: bool,
    /// The `--no-summarize` flag value this config was derived from (issue
    /// #788: threaded so `run`'s unconfigured check uses the same inputs as
    /// the gate that produced `enabled`).
    pub no_summarize: bool,
    /// Pre-detected apfel availability (issue #788: threaded so `run` does
    /// not re-probe with values that could drift from the gate's probe).
    pub apfel_available: bool,
}

/// The summarization enablement decision, reflecting the CONFIGURED backend
/// rather than the presence of one specific vendor binary (issue #786).
/// Produced by [`summarize_decision`]; each disabled variant covers one state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummarizeDecision {
    /// Summarization runs.
    Enabled,
    /// Disabled by the `--no-summarize` CLI flag (wins over everything).
    DisabledCliFlag,
    /// Disabled by `summarize: false` in the repository config.
    DisabledByConfig,
    /// Non-apfel backend configured, but no endpoint to call.
    DisabledNoEndpoint,
    /// Apfel backend (configured or default), and apfel is not available.
    DisabledApfelMissing,
}

impl SummarizeDecision {
    /// The user-facing reason summarization is off, naming the CONFIGURED
    /// backend (issue #786). Pure `match` — cannot drift from the rule
    /// [`summarize_decision`] encodes.
    pub fn disabled_reason(&self, repo_config: &RepoConfig) -> Option<String> {
        let backend = crate::summarization::backend_profile::display_name(
            repo_config.effective_summarizer_backend(),
        );
        Some(match self {
            SummarizeDecision::Enabled => return None,
            SummarizeDecision::DisabledCliFlag => {
                format!("the --no-summarize flag disables {backend} summarization")
            }
            SummarizeDecision::DisabledByConfig => {
                format!("{backend} summarization is disabled in the repository config")
            }
            SummarizeDecision::DisabledNoEndpoint => {
                format!("the {backend} backend has no endpoint configured")
            }
            SummarizeDecision::DisabledApfelMissing => {
                format!("the {backend} binary is not on PATH")
            }
        })
    }
}

/// The ONE enablement branch ladder (issue #786). Pure and side-effect-free;
/// `apfel_available` is pre-computed (reachability is `server_dispatch`'s job).
///
/// `no_summarize` always wins; `summarize: false`/`true` override everything;
/// unset + non-apfel + endpoint → enabled (unreachable → #780 degradation);
/// unset + non-apfel + no endpoint → disabled; unset + apfel → enabled iff
/// apfel is spawnable.
pub fn summarize_decision(
    no_summarize: bool,
    repo_config: &RepoConfig,
    apfel_available: bool,
) -> SummarizeDecision {
    if no_summarize {
        return SummarizeDecision::DisabledCliFlag;
    }
    match repo_config.summarize {
        Some(false) => SummarizeDecision::DisabledByConfig,
        Some(true) => SummarizeDecision::Enabled,
        None => {
            let backend = repo_config.effective_summarizer_backend();
            if backend == SummarizerBackend::Apfel {
                if apfel_available {
                    SummarizeDecision::Enabled
                } else {
                    SummarizeDecision::DisabledApfelMissing
                }
            } else if repo_config.effective_apfel_endpoint().is_some() {
                SummarizeDecision::Enabled
            } else {
                SummarizeDecision::DisabledNoEndpoint
            }
        }
    }
}

/// Decide whether summarization runs at all (issue #786). Thin wrapper over
/// [`summarize_decision`].
pub fn summarization_enabled(
    no_summarize: bool,
    repo_config: &RepoConfig,
    apfel_available: bool,
) -> bool {
    matches!(
        summarize_decision(no_summarize, repo_config, apfel_available),
        SummarizeDecision::Enabled
    )
}

impl SummarizationConfig {
    /// Create config from CLI flag, repo config, and pre-detected apfel availability.
    pub fn new(no_summarize: bool, repo_config: &RepoConfig, apfel_available: bool) -> Self {
        Self {
            enabled: summarization_enabled(no_summarize, repo_config, apfel_available),
            no_summarize,
            apfel_available,
        }
    }
}

/// Outcome of the function summarization pass.
pub(crate) struct SummarizationOutcome {
    pub(crate) summarized: usize,
    pub(crate) counters: pipeline_ops::UpsertCounters,
}

/// Main summarization pipeline.
pub struct SummarizationPipeline;

impl SummarizationPipeline {
    /// Run bottom-up summarization for a repository. Acquires the server and
    /// threads the explicit HTTP transport down the call path (issue #783).
    /// Returns total summarized count plus functions skipped (issue #649).
    pub fn run(
        storage: &dyn Storage,
        repo_id: &str,
        _project_id: &str,
        code_units: &[CodeUnit],
        config: &SummarizationConfig,
        repo_config: &RepoConfig,
    ) -> Result<SummaryOutcome> {
        if !config.enabled {
            return Ok(SummaryOutcome::default());
        }

        // Enabled-but-unconfigured (issue #788): gate enabled but no endpoint
        // and no apfel binary — return early with one accurate message. The
        // shared classifier uses the same flag and availability values the
        // caller used to enable the gate, so the check cannot drift from it.
        if crate::summarization::enabled_state::classify_enabled_state(
            config.no_summarize,
            repo_config,
            config.apfel_available,
        ) == crate::summarization::enabled_state::EnabledState::EnabledButUnconfigured
        {
            eprintln!(
                "  [{}] {}",
                repo_id,
                crate::summarization::unconfigured::unconfigured_message(repo_config)
            );
            return Ok(SummaryOutcome::default());
        }

        // Acquire the server handle; the transport is returned as a value
        // (issue #783: no ambient state). Acquisition never hard-errors:
        // a configured non-apfel backend that cannot be reached degrades to
        // the no-summarizer path, never to the apfel CLI (issue #780).
        let (handle, transport, degradation) = server_dispatch::acquire_for_run(repo_config);
        // Remote-endpoint notice (PR #787 review, security): a usable HTTP
        // transport on a NON-loopback host means source code is leaving the
        // machine. The #780 degradation message only fires on failure, so
        // without this the success path stays silent. Loopback stays silent
        // so the normal local setup is not nagged.
        if degradation.is_none()
            && let Some(warning) = remote_endpoint_warning(repo_config)
        {
            eprintln!("{warning}");
        }
        if let Some(degradation) = degradation.as_ref() {
            eprintln!("{}", degradation.message());
            // No silent cross-backend fallback (issue #780).
            if degradation.backend != crate::config::SummarizerBackend::Apfel {
                return Ok(SummaryOutcome::default());
            }
        }

        // Resolve the budget once so the config override reaches every consumer.
        let input_char_budget = crate::summarization::apfel::resolve_input_char_budget(
            transport.as_ref(),
            repo_config.summarizer_input_char_budget,
        );
        let outcome = Self::run_internal(
            storage,
            repo_id,
            code_units,
            input_char_budget,
            transport.as_ref(),
        );

        // The handle (if any) is dropped here, shutting down a spawned server.
        drop(handle);

        outcome
    }

    /// Core summarization logic (functions + rollups).
    fn run_internal(
        storage: &dyn Storage,
        repo_id: &str,
        code_units: &[CodeUnit],
        input_char_budget: usize,
        transport: Option<&BackendTransport>,
    ) -> Result<SummaryOutcome> {
        let SummarizationOutcome {
            summarized: functions_summarized,
            counters: fn_counters,
        } = Self::summarize_functions(storage, repo_id, code_units, input_char_budget, transport)?;

        if fn_counters.skipped_oversized > 0 {
            eprintln!(
                "warning: {} functions too large to summarize (context window exceeded).",
                fn_counters.skipped_oversized
            );
        }
        if fn_counters.parse_failures > 0 {
            eprintln!(
                "warning: {} functions could not be summarized — model output did not honour the #<n>: format (parse failure).",
                fn_counters.parse_failures
            );
        }
        // Roll up to files, modules, subsystems (issue #793: skips are a
        // first-class outcome, not a log line).
        let mut rollup_skipped = RollupSkips::default();
        let mut rollup_summarized = 0usize;
        for tier in [EntityTier::File, EntityTier::Module, EntityTier::Subsystem] {
            let RollupOutcome { updated, skipped } =
                Self::rollup_to_tier(storage, repo_id, tier, input_char_budget, transport)?;
            rollup_summarized += updated;
            if skipped.total() > 0 {
                // Bounded output: one aggregate line per tier, never per entity.
                eprintln!(
                    "warning: {} tier: {} of {} entities skipped rollup — {} policy, {} child-summary missing, {} without children.",
                    tier,
                    skipped.total(),
                    skipped.total() + updated as u64,
                    skipped.policy,
                    skipped.child_summary_missing,
                    skipped.no_children,
                );
            }
            rollup_skipped.record(tier, skipped);
        }

        Ok(SummaryOutcome {
            summarized: functions_summarized + rollup_summarized,
            skipped_oversized: fn_counters.skipped_oversized,
            parse_failures: fn_counters.parse_failures,
            rollup_skipped,
        })
    }

    /// Summarize function entities with budget-bounded batching. Functions
    /// without a stored entity (e.g. `preserve_function_entities=false`) are
    /// skipped; a single oversized function is still summarized alone.
    pub(crate) fn summarize_functions(
        storage: &dyn Storage,
        repo_id: &str,
        code_units: &[CodeUnit],
        input_char_budget: usize,
        transport: Option<&BackendTransport>,
    ) -> Result<SummarizationOutcome> {
        let tty = std::io::stdout().is_terminal();

        // Collect all functions (skip test_ prefix and those without code).
        let all_functions: Vec<(String, String, String)> = code_units
            .iter()
            .filter(|u| is_function_unit(&u.unit_type))
            .filter(|u| !is_test_entity(&u.name))
            .filter_map(|u| {
                u.code
                    .as_ref()
                    .map(|code| (u.file.clone(), u.name.clone(), code.clone()))
            })
            .collect();

        if all_functions.is_empty() {
            return Ok(SummarizationOutcome {
                summarized: 0,
                counters: pipeline_ops::UpsertCounters::default(),
            });
        }

        let mut counters = pipeline_ops::UpsertCounters::default();

        eprintln!(
            "  Starting on-device summarization ({} functions, use --no-summarize to skip)...",
            all_functions.len()
        );

        // Pack functions by char budget and process each batch.
        let batches = pack_by_char_budget(&all_functions, input_char_budget);
        let total_batches = batches.len();
        let pb = ProgressBar::new(all_functions.len() as u64);
        if !tty {
            pb.set_draw_target(ProgressDrawTarget::hidden());
        }
        pb.set_style(
            indicatif::ProgressStyle::default_bar()
                .template("{prefix:.bold} [{elapsed_precise}] {bar:40} {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("=> "),
        );
        pb.set_prefix("Summarizing");

        for (batch_idx, batch) in batches.into_iter().enumerate() {
            if batch.is_empty() {
                continue;
            }

            let msg = if batch.len() == 1 {
                format!(
                    "Summarizing {} (batch {}/{})",
                    batch[0].1,
                    batch_idx + 1,
                    total_batches
                )
            } else {
                format!(
                    "Summarizing batch {}/{} ({} functions)",
                    batch_idx + 1,
                    total_batches,
                    batch.len()
                )
            };
            pb.set_message(msg);

            let results = match batch_summarize(&batch, transport) {
                Ok(r) => r,
                Err(e) if is_overflow_error(&e) => {
                    // Every function in the batch is oversized; skip individual
                    // retries — each would hit the same budget guard (issue #649).
                    for (_, name, _) in batch.iter() {
                        eprintln!(
                            "warning: {} exceeds the summarization budget ({} chars), skipped",
                            name, input_char_budget
                        );
                    }
                    eprintln!(
                        "warning: batch summarization failed for {} functions: {e}",
                        batch.len()
                    );
                    counters.skipped_oversized += batch.len() as u64;
                    for _ in batch {
                        pb.inc(1);
                    }
                    continue;
                }
                Err(e) => {
                    eprintln!(
                        "warning: batch summarization failed for {} functions: {e}",
                        batch.len()
                    );
                    vec![None; batch.len()]
                }
            };

            let present_count = results.iter().filter(|r| r.is_some()).count();

            let mut ctx = batch_ops::BatchCtx {
                storage,
                repo_id,
                batch: &batch,
                results: &results,
                counters: &mut counters,
                pb: &pb,
                input_char_budget,
                transport,
            };

            if present_count != batch.len() {
                eprintln!(
                    "warning: batch summarization returned {} results for {} functions — salvaging partial batch",
                    present_count,
                    batch.len()
                );
                let (present_positions, missing_positions) = salvage_partial_batch(&results);
                batch_ops::process_partial_batch(&mut ctx, &present_positions, &missing_positions);
                continue;
            }

            batch_ops::process_full_batch(&mut ctx, tty);
        }

        pb.finish_and_clear();

        if counters.skipped_fn_entity > 0 {
            eprintln!(
                "warning: {} function entities not found in storage — summaries skipped.\nRun `lievo refresh --force <project>` to re-index.",
                counters.skipped_fn_entity
            );
        }
        if counters.skipped_file > 0 {
            eprintln!(
                "warning: {} summaries skipped — file entities not found in storage.",
                counters.skipped_file
            );
        }
        if counters.skipped_no_code > 0 {
            eprintln!(
                "warning: {} summaries skipped — function code unavailable.",
                counters.skipped_no_code
            );
        }

        if !tty && counters.summarized > 0 {
            eprintln!();
        }
        Ok(SummarizationOutcome {
            summarized: counters.summarized as usize,
            counters,
        })
    }
    /// Compute SHA-256 hash of code content, truncated to 16 hex chars.
    fn hash_code(code: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(code.as_bytes());
        let result = hasher.finalize();
        let hex: String = result.iter().map(|b| format!("{:02x}", b)).collect();
        // SHA-256 always produces 64 hex chars; take first 16 for a compact fingerprint.
        hex[..16].to_string()
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
