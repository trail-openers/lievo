// The ONE classification of a repo's summarization state (issue #788).
//
// The enabled-but-unconfigured state (gate enabled by config or flag, but no
// usable backend) was previously detected by separate rules at each call
// site — the summarize pipeline's early return, the refresh staleness
// marker, and the refresh command's coverage gate — creating drift risk
// between the three. This enum is the single source of truth:
// `classify_enabled_state` is the one decision point, and every call site
// matches on its result instead of re-encoding the rule.
//
// The classification itself composes the two existing building blocks
// (issue #786's `summarize_decision` ladder and `is_enabled_but_unconfigured`
// from the `unconfigured` module); this module adds the unconfigured-state
// marker bookkeeping (fingerprint persistence in `refresh.rs`) without
// duplicating any detection logic.

use crate::config::RepoConfig;
use crate::summarization::pipeline::{SummarizeDecision, summarize_decision};
use crate::summarization::unconfigured::is_enabled_but_unconfigured;

/// The repo's summarization state for this invocation (issue #788).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnabledState {
    /// Summarization will run for this repo (a usable backend is present).
    Enabled,
    /// The gate is enabled (explicit `summarize: true` or a configured
    /// backend) but nothing is usable — no endpoint, no apfel binary.
    /// Re-running the pipeline cannot help; the caller persists the
    /// unconfigured marker and reports the state (issue #788).
    EnabledButUnconfigured,
    /// Summarization is off for this repo (flag, config, or no backend at
    /// all); nothing runs and nothing should be reported.
    Disabled,
}

/// The one decision point (issue #788): classify a repo's summarization
/// state from the same inputs the gate uses (`--no-summarize` flag, repo
/// config, pre-probed apfel availability). Call sites match on the result
/// instead of re-encoding the rule.
///
/// - `summarize_decision` says the gate is off → `Disabled`;
/// - gate on + no usable backend (`is_enabled_but_unconfigured`) →
///   `EnabledButUnconfigured`;
/// - gate on + usable backend → `Enabled`.
pub fn classify_enabled_state(
    no_summarize: bool,
    repo_config: &RepoConfig,
    apfel_available: bool,
) -> EnabledState {
    match summarize_decision(no_summarize, repo_config, apfel_available) {
        SummarizeDecision::Enabled => {
            if is_enabled_but_unconfigured(no_summarize, repo_config, apfel_available) {
                EnabledState::EnabledButUnconfigured
            } else {
                EnabledState::Enabled
            }
        }
        _ => EnabledState::Disabled,
    }
}
