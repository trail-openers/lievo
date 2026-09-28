// refresh command handler — re-analyze repositories whose HEAD has moved.
//
// Runs cheap layers (extraction → relationships → insights → semantic index).

use lievo::output::OutputFormat;
use lievo::project_resolution::resolve_project_id;
use lievo::refresh::{RefreshOptions, ensure_fresh};
use lievo::storage::Storage;
use lievo::summarization::{EnabledState, classify_enabled_state};
use serde_json::json;

#[path = "refresh_report.rs"]
mod refresh_report_impl;

use refresh_report_impl::{TierCoverage, build_report, compute_tier_coverage};

/// Run the refresh pipeline for a project.
///
/// Prints a brief report of what was refreshed, plus summary coverage
/// so partial summarization is distinguishable from a full success.
pub fn refresh(
    storage: &dyn Storage,
    project_name: Option<&str>,
    full: bool,
    force: bool,
    no_summarize: bool,
    no_ignore: bool,
    fmt: OutputFormat,
) -> lievo::Result<()> {
    let project_id = resolve_project_id(storage, project_name)?;

    let opts = RefreshOptions {
        force,
        full,
        no_ignore,
        no_summarize,
        skip_semantic_index: false,
    };
    let stats = ensure_fresh(storage, &project_id, &opts)?;

    if !stats.was_stale && !force {
        let result = json!({
            "was_stale": false,
            "message": "Project is already up to date — nothing to refresh."
        })
        .to_string();
        if fmt == OutputFormat::Json {
            println!("{result}");
        } else {
            println!("Project is already up to date — nothing to refresh.");
        }
        return Ok(());
    }

    // Evaluate the enablement gate PER REPO (issue #788) — the previous
    // implementation sampled it from `list_repos().next()` (the FIRST repo's
    // config), so a project with repo A (apfel, no binary) and repo B
    // (`summarize: true`) reported A's outcome regardless of B's. A config
    // that fails to parse or validate warns via `load_or_default` (issue #788)
    // and falls back to defaults; a missing config is silent.
    let repos = storage.list_repos(&project_id)?;
    let mut enabled_repos: Vec<&lievo::model::Repository> = Vec::new();
    let mut unconfigured_repos: Vec<&lievo::model::Repository> = Vec::new();
    let apfel_available = lievo::summarization::apfel::is_apfel_available();
    for repo in &repos {
        let repo_config =
            lievo::config::RepoConfig::load_or_default(std::path::Path::new(&repo.local_path));
        // The shared classifier (issue #788) distinguishes repos with a usable
        // backend from those enabled-by-intent but with nothing configured or
        // available, and disabled repos (contribute to neither list).
        match classify_enabled_state(no_summarize, &repo_config, apfel_available) {
            EnabledState::Enabled => enabled_repos.push(repo),
            EnabledState::EnabledButUnconfigured => unconfigured_repos.push(repo),
            EnabledState::Disabled => {}
        }
    }

    // Per-tier coverage is only reported when summarization is enabled for at
    // least one repo; a storage failure must not block the refresh report when
    // there are no summaries by design (issue #648).
    let tiers = if !enabled_repos.is_empty() {
        compute_tier_coverage(storage, &project_id).ok()
    } else {
        eprintln!("Summarization disabled — skipping summary coverage computation.");
        None
    };

    // Coverage only matters when summarization is on; when disabled, the
    // report carries `summarization_disabled: true` without implying
    // incompleteness.
    let tiers = tiers.unwrap_or_else(|| {
        (
            TierCoverage::default(),
            TierCoverage::default(),
            TierCoverage::default(),
            TierCoverage::default(),
        )
    });
    let summarization_enabled = !enabled_repos.is_empty();
    let report = build_report(&stats, tiers, summarization_enabled, fmt);
    print!("{report}");
    Ok(())
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod tests;
