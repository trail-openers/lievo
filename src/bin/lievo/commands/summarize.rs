// summarize command handler — re-summarize entities with cached descriptions.
//
// When `--file <path>` is provided, clears stale summaries for that file and its
// ancestors (module, subsystem), then re-runs summarization. Without `--file`,
// re-summarizes all entities in the project.

use lievo::config::RepoConfig;
use lievo::error::LievoError;
use lievo::extraction::code_extractor::CodeExtractor;
use lievo::extraction::entity_id::normalize_path;
use lievo::extraction::tree_sitter_extractor::TreeSitterExtractor;
use lievo::model::Entity;
use lievo::output::OutputFormat;
use lievo::storage::Storage;
use lievo::summarization::pipeline::{
    SummarizationConfig, SummarizationPipeline, summarize_decision,
};

/// Run the summarization pipeline for a project.
///
/// - Without `--file`: re-summarizes all entities in the project.
/// - With `--file`: clears the target file's summary and its ancestors, then re-summarizes.
///
/// Prints a brief report of what was summarized.
pub fn summarize(
    storage: &dyn Storage,
    project_name: Option<&str>,
    file_path: Option<&str>,
    fmt: OutputFormat,
) -> lievo::Result<()> {
    let project_id = lievo::project_resolution::resolve_project_id(storage, project_name)?;

    // If --file is provided, clear summaries for that file and its ancestors,
    // then run summarization directly (bypassing ensure_fresh's git-change check)
    if let Some(path) = file_path {
        summarize_file(storage, &project_id, path, fmt)?;
        return Ok(());
    }

    // Without --file: run the normal ensure_fresh path
    let opts = lievo::refresh::RefreshOptions {
        force: false,
        full: false,
        no_ignore: false,
        no_summarize: false,
        skip_semantic_index: false,
    };
    let stats = lievo::refresh::ensure_fresh(storage, &project_id, &opts)?;

    if !stats.was_stale {
        if fmt == OutputFormat::Json {
            println!(
                "{}",
                serde_json::json!({
                    "was_stale": false,
                    "message": "Project is already up to date — nothing to summarize."
                })
            );
        } else {
            println!("Project is already up to date — nothing to summarize.");
        }
        return Ok(());
    }

    if fmt == OutputFormat::Json {
        println!(
            "{}",
            serde_json::json!({
                "was_stale": true,
                "repos_refreshed": stats.repos_refreshed
            })
        );
    } else {
        println!(
            "Summarization complete: {} repo(s) re-analyzed.",
            stats.repos_refreshed
        );
    }

    Ok(())
}

/// Summarize a specific file and its ancestors by running the summarization pipeline directly.
///
/// This bypasses `ensure_fresh`'s git-change detection — after clearing the summaries,
/// the summarization pipeline will re-summarize all entities with `summary_commit = None`.
fn summarize_file(
    storage: &dyn Storage,
    project_id: &str,
    file_path: &str,
    fmt: OutputFormat,
) -> lievo::Result<()> {
    // Clear the file's summary and its ancestors
    clear_file_summaries(storage, project_id, file_path)?;
    eprintln!("Cleared stale summaries for file: {}", file_path);

    // Find the repo containing this file
    let normalized_path = normalize_path(file_path);

    // Security check: reject path traversal attempts
    if normalized_path.contains("..") {
        return Err(LievoError::InvalidInput(
            "Path cannot contain '..' sequences".to_string(),
        ));
    }

    let (target_repo, code_units) = {
        let (_file_entity, repo) = find_repo_for_file(storage, project_id, &normalized_path)?;

        // Use tree-sitter extractor to read code units for the target file only
        let repo_path = std::path::Path::new(&repo.local_path);
        let mut extractor = TreeSitterExtractor::new(repo_path, true)?;
        extractor.index(false)?;
        let units = extractor.units_for_file(&normalized_path)?;

        (repo, units)
    };

    // Load repo config and check apfel availability. A config that fails to
    // parse or validate warns via `load_or_default` (issue #788) and falls
    // back to defaults; a missing config is silent.
    let repo_path = std::path::Path::new(&target_repo.local_path);
    let repo_config = RepoConfig::load_or_default(repo_path);

    let apfel_available = lievo::summarization::apfel::is_apfel_available();

    // Build summarization config (respect CLI flag: summarization is always enabled here)
    let sum_config = SummarizationConfig::new(false, &repo_config, apfel_available);

    if !sum_config.enabled {
        // The reason derives from the SAME decision the gate used (issue
        // #786) — no second copy of the enablement rule in prose.
        let reason = summarize_decision(false, &repo_config, apfel_available)
            .disabled_reason(&repo_config)
            .expect("decision is disabled: the enabled gate says so");
        eprintln!("Summarization is disabled: {reason}");
        if fmt == OutputFormat::Json {
            println!(
                "{}",
                serde_json::json!({
                    "summarized": 0,
                    "message": "Summarization disabled"
                })
            );
        } else {
            println!("Summarization disabled for this repository.");
        }
        return Ok(());
    }

    // Run summarization pipeline directly
    eprintln!("Running summarization pipeline...");
    let outcome = SummarizationPipeline::run(
        storage,
        &target_repo.id,
        &target_repo.project_id,
        &code_units,
        &sum_config,
        &repo_config,
    )?;

    if fmt == OutputFormat::Json {
        println!(
            "{}",
            serde_json::json!({
                "summarized": outcome.summarized,
                "skipped_oversized": outcome.skipped_oversized,
                "parse_failures": outcome.parse_failures,
                "rollup_skipped": json_rollup_skips(&outcome.rollup_skipped),
                "repo": target_repo.name
            })
        );
    } else {
        println!(
            "Summarization complete: {} entities re-summarized in '{}'",
            outcome.summarized, target_repo.name
        );
        print_rollup_skips(&outcome.rollup_skipped);
    }

    Ok(())
}

/// JSON shape of the rollup skip counters (issue #793): one object per tier
/// with the three mutually-exclusive skip buckets.
fn json_rollup_skips(skips: &lievo::summarization::RollupSkips) -> serde_json::Value {
    fn tier(v: &lievo::summarization::SkippedRollup) -> serde_json::Value {
        serde_json::json!({
            "policy": v.policy,
            "child_summary_missing": v.child_summary_missing,
            "no_children": v.no_children,
        })
    }
    serde_json::json!({
        "file": tier(&skips.file),
        "module": tier(&skips.module),
        "subsystem": tier(&skips.subsystem),
    })
}

/// One human line per rollup tier with skips; silent when nothing was
/// skipped (bounded output — no per-entity lines, issue #793).
fn print_rollup_skips(skips: &lievo::summarization::RollupSkips) {
    for (tier_name, bucket) in [
        ("file", &skips.file),
        ("module", &skips.module),
        ("subsystem", &skips.subsystem),
    ] {
        if bucket.total() > 0 {
            eprintln!(
                "{} tier: {} entities skipped rollup — {} policy, {} child-summary missing, {} without children",
                tier_name,
                bucket.total(),
                bucket.policy,
                bucket.child_summary_missing,
                bucket.no_children,
            );
        }
    }
}

/// Find the repository containing a file entity by path.
fn find_repo_for_file(
    storage: &dyn Storage,
    project_id: &str,
    normalized_path: &str,
) -> lievo::Result<(Entity, lievo::model::Repository)> {
    let candidates = storage.entity_by_path_projectwide(project_id, normalized_path)?;

    let file_entity = match candidates.len() {
        0 => {
            return Err(LievoError::EntityNotFound(format!(
                "No entity found at path '{}'. Check the path is relative to the project root.",
                normalized_path
            )));
        }
        // SAFETY: match arm guarantees exactly one element
        1 => candidates.into_iter().next().unwrap(),
        _ => {
            return Err(LievoError::InvalidInput(format!(
                "Path '{}' exists in {} repos. Re-index a specific repo to disambiguate.",
                normalized_path,
                candidates.len()
            )));
        }
    };

    let file_repo_id = file_entity.repo_id.clone().ok_or_else(|| {
        LievoError::EntityNotFound(format!(
            "repository not found for file entity '{}'",
            file_entity.id
        ))
    })?;

    let target_repo = storage
        .list_repos(project_id)?
        .into_iter()
        .find(|r| r.id == file_repo_id)
        .ok_or_else(|| {
            LievoError::EntityNotFound(format!(
                "repository not found for file entity '{}'",
                file_entity.id
            ))
        })?;

    Ok((file_entity, target_repo))
}

/// Clear summaries for a file entity and its ancestor hierarchy.
///
/// Clears `summary` and `summary_commit` for:
/// - The target File entity by path
/// - Its parent Module entity (if any)
/// - Its grandparent Subsystem entity (if any)
///
/// This marks them as stale so the summarization pipeline will re-summarize them.
fn clear_file_summaries(
    storage: &dyn Storage,
    project_id: &str,
    file_path: &str,
) -> lievo::Result<()> {
    // Normalize path before lookup to match stored entity paths
    // normalize_path() already strips leading ./
    let normalized_path = normalize_path(file_path);

    // Look up the file entity project-wide (single query, no loop)
    let candidates = storage.entity_by_path_projectwide(project_id, &normalized_path)?;

    let file_entity = match candidates.len() {
        0 => {
            return Err(LievoError::EntityNotFound(format!(
                "file entity not found at path '{}' in project",
                normalized_path
            )));
        }
        // SAFETY: match arm guarantees exactly one element
        1 => candidates.into_iter().next().unwrap(),
        _ => {
            return Err(LievoError::InvalidInput(format!(
                "Path '{}' exists in {} repos. Re-index a specific repo to disambiguate.",
                normalized_path,
                candidates.len()
            )));
        }
    };

    // Verify it's a File tier entity — provide helpful error showing actual tier
    if file_entity.tier != lievo::model::EntityTier::File {
        return Err(LievoError::InvalidInput(format!(
            "Entity at '{}' is a {:?}, not a File. Pass a file path.",
            file_path, file_entity.tier
        )));
    }

    // Clear the file's summary. A dedicated UPDATE is required because
    // UPSERT_ENTITY now COALESCEs the summary columns: upserting a NULL summary would
    // mean "no summary to offer" and would NOT wipe a stored summary.
    storage.clear_entity_summary(&file_entity.id)?;

    // Walk up the hierarchy: Module -> Subsystem, clearing BOTH summary AND summary_commit
    if let Some(module_id) = &file_entity.parent_id
        && let Some(module) = storage.get_entity(module_id)?
    {
        storage.clear_entity_summary(module_id)?;

        // Continue to subsystem
        if let Some(subsystem_id) = &module.parent_id {
            storage.clear_entity_summary(subsystem_id)?;
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "summarize_tests.rs"]
mod tests;
