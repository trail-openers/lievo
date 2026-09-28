// Enabled-but-unconfigured detection (issue #788).
//
// The enabled-but-nothing-configured state: the gate is Enabled (explicit
// `summarize: true` or a configured backend) but there is no endpoint to call
// and no apfel binary to fall back to. The caller uses this to emit the
// unconfigured message and return early instead of proceeding to per-batch
// apfel CLI failures.

use crate::config::{RepoConfig, SummarizerBackend};
use crate::summarization::pipeline::SummarizeDecision;
use crate::summarization::pipeline::summarize_decision;

/// One accurate message naming the real cause when summarization is enabled
/// but nothing is configured or available (issue #788). The enabled-but-
/// unconfigured state previously fell through to per-batch `run_apfel` CLI
/// failures naming a backend the user never configured, and the refresh report
/// then advised re-running — advice that cannot help, because re-running
/// reproduces the state exactly. This message replaces that chain: it names
/// the cause, and the caller returns early so no per-batch apfel spawn occurs.
pub fn unconfigured_message(repo_config: &RepoConfig) -> String {
    let backend = crate::summarization::backend_profile::display_name(
        repo_config.effective_summarizer_backend(),
    );
    format!(
        "warning: summarization is enabled but no {backend} backend is configured or available; no summaries will be produced. Configure `summarizer_backend` and `apfel_endpoint` in .lievo/config.yaml, or install the backend binary."
    )
}

/// Fingerprint of the config subset that determines the enabled-but-
/// unconfigured state (issue #788). Compares only the summarization-relevant
/// fields (`summarize` and the effective backend/endpoint) so unrelated config
/// edits (e.g. `exclude` globs) do not invalidate a pending marker.
pub fn unconfigured_fingerprint(repo_config: &RepoConfig) -> String {
    format!(
        "{}|{}|{}",
        repo_config
            .summarize
            .map(|s| s.to_string())
            .unwrap_or_default(),
        repo_config.effective_summarizer_backend().as_str(),
        repo_config
            .effective_apfel_endpoint()
            .as_deref()
            .unwrap_or_default()
    )
}

/// The enabled-but-nothing-configured decision: the gate is Enabled (explicit
/// `summarize: true` or a configured backend) but there is no endpoint to call
/// and no apfel binary to fall back to. The caller uses this to emit the
/// unconfigured message and return early instead of proceeding to per-batch
/// apfel CLI failures (issue #788).
///
/// "No usable backend" means:
/// - No endpoint configured, AND
/// - For the apfel backend: the apfel binary is not available (the one-shot
///   CLI path is the only fallback for apfel, and it requires the binary).
/// - For non-apfel backends: there is no fallback at all (no CLI path), so an
///   endpoint is the only route — and `summarize_decision` only enables a
///   non-apfel backend when an endpoint is present (an explicit
///   `summarize: true` is the lone exception, and it defaults to the apfel
///   backend).
pub fn is_enabled_but_unconfigured(
    no_summarize: bool,
    repo_config: &RepoConfig,
    apfel_available: bool,
) -> bool {
    match summarize_decision(no_summarize, repo_config, apfel_available) {
        SummarizeDecision::Enabled => {
            let backend = repo_config.effective_summarizer_backend();
            let has_endpoint = repo_config.effective_apfel_endpoint().is_some();
            if has_endpoint {
                return false;
            }
            // No endpoint: only the apfel backend has a fallback (the one-shot
            // CLI path, which requires the binary). A non-apfel backend that
            // reaches Enabled without an endpoint is unreachable, because
            // `summarize_decision` only enables non-apfel backends when an
            // endpoint is present (an explicit `summarize: true` is the lone
            // exception, and it defaults to the apfel backend). For apfel, the
            // CLI fallback needs the binary.
            backend == SummarizerBackend::Apfel && !apfel_available
        }
        _ => false,
    }
}
