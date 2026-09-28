// Auto-refresh: staleness detection and cheap-layer refresh.
//
// The staleness/refresh core lives in this file (grandfathered above the
// 500-line budget — see scripts/file_size_baseline.txt); the cross-process
// lock + status logic (issue #864 binding decision 6) lives in `lock`, and
// the serve-startup automatic index (issue #864) lives in `auto`.

pub mod auto;

pub mod lock;

mod lock_holder;

mod refresh_guard;
pub(crate) use refresh_guard::RefreshGuard;

#[cfg(test)]
#[path = "reconcile_guard_tests.rs"]
mod reconcile_guard_tests;

pub use auto::start_index_if_needed;
pub use lock::{IndexTrigger, IndexingStatus, acquire as acquire_index_lock, indexing_status};

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::Result;

use crate::analysis::insights::InsightDetector;
use crate::analysis::pipeline::{AnalysisPipeline, PipelineConfig, ReindexMode};
use crate::refresh_resume::resume_incomplete_summarization;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

/// Statistics produced by a single `ensure_fresh()` call.
#[derive(Debug, Clone, Default)]
pub struct RefreshStats {
    /// Whether any repository was found to have a new HEAD commit.
    pub was_stale: bool,
    /// Time elapsed running the refresh, in whole seconds.
    pub elapsed_secs: u64,
    /// Number of repositories whose analysis ran (skipped-up-to-date repos excluded).
    pub repos_refreshed: usize,
}

/// Options to configure refresh behavior.
#[derive(Default)]
pub struct RefreshOptions {
    pub force: bool,
    pub full: bool,
    pub no_ignore: bool,
    pub no_summarize: bool,
    /// When true, the pipeline skips the semantic/vector index build (and
    /// any embedding-model download). The serve-startup automatic index
    /// (issue #864) sets this so a first `lievo mcp` start never downloads
    /// model files and never holds the index lock through the ~minutes-long
    /// model download; a manual `lievo refresh` still builds the semantic
    /// index.
    pub skip_semantic_index: bool,
}

/// Apply (or confirm) the enabled-but-unconfigured marker for a repo whose
/// summarization gate is enabled but has no usable backend (issue #788).
///
/// The marker is keyed to the config fingerprint so it survives across
/// invocations but is invalidated the moment the summarization-relevant
/// config changes. It only suppresses the retry loop for the SAME commit:
/// `is_stale` returns stale earlier when a new commit arrives and clears
/// the marker there, so reaching here with a matching fingerprint means the
/// prior (unconfigured) run on this same commit produced these missing
/// summaries — stay silent (debug). Otherwise the marker is (re)persisted
/// and the state is warned so the user sees why it repeats (issue #788).
fn apply_unconfigured_marker(
    storage: &dyn Storage,
    repo: &crate::model::Repository,
    repo_config: &crate::config::RepoConfig,
) {
    let fingerprint = crate::summarization::unconfigured::unconfigured_fingerprint(repo_config);
    let marker_matches = repo.summarization_unconfigured.as_deref() == Some(fingerprint.as_str());
    if marker_matches {
        // Marker already set for this exact config on this same commit: the
        // previous invocation already reported the state, so stay silent
        // (debug) and do not re-trigger the full pipeline.
        tracing::debug!(
            "repo '{}' summarization is enabled but unconfigured (marker pending); skipping stale trigger",
            repo.name
        );
        return;
    }
    // First observation of this unconfigured state (or the config changed since
    // the marker was set): persist the marker and warn.
    if let Err(e) = storage.update_repository_unconfigured_marker(&repo.id, Some(&fingerprint)) {
        tracing::warn!(
            "repo '{}': failed to persist unconfigured marker: {e}",
            repo.name
        );
    }
    tracing::warn!(
        "repo '{}' has missing summaries (structural up-to-date, summarization enabled but unconfigured — configure a backend or remove `summarize: true`)",
        repo.name
    );
}

/// The current HEAD commit of a repo, with unified error handling shared by
/// the staleness/trigger checks and the reconcile guard (issue #872 review):
/// an `UnbornBranch` (a repo with no commits yet) is not an error worth a
/// warning and counts as not stale / nothing to reconcile; any other failure
/// to open the repo (missing path, not a git repo, …) emits ONE warning line
/// naming the repo, the path and the error, and is treated as
/// "unreachable" by the callers.
pub(crate) fn reachable_head_commit(
    repo_name: &str,
    local_path: &std::path::Path,
) -> Option<String> {
    match crate::analysis::incremental::head_commit(local_path) {
        Ok(head) => Some(head),
        Err(crate::LievoError::Git(e)) if e.code() == git2::ErrorCode::UnbornBranch => {
            // A repo with no commits yet: silent, not stale, nothing to
            // reconcile.
            None
        }
        Err(e) => {
            eprintln!(
                "[auto-refresh] repo '{repo_name}' at path '{}' could not be opened as a git repository ({e}); skipped",
                local_path.display()
            );
            None
        }
    }
}

/// Check whether any repository HEAD has moved past `last_analyzed_commit`.
/// Also returns `true` if structural analysis is current but summarization is incomplete
/// and summarization is enabled (not gated by `no_summarize` flag).
///
/// Returns `true` as soon as the first stale repo is found.
/// Repos whose `local_path` cannot be opened as a git repo are skipped with a
/// warning and never make the project stale; a repo with no commits yet is
/// skipped silently.
pub fn is_stale(storage: &dyn Storage, project_id: &str, no_summarize: bool) -> Result<bool> {
    let repos = storage.list_repos(project_id)?;

    for repo in &repos {
        let Some(head) = reachable_head_commit(&repo.name, std::path::Path::new(&repo.local_path))
        else {
            // Unborn branch (no commits yet) — not stale — or an unreachable
            // path (warning already emitted by the helper).
            continue;
        };
        let last = repo.last_analyzed_commit.as_deref().unwrap_or("");
        if head != last {
            // A new commit arrived: the unconfigured marker, if any, was set
            // for the previous commit and no longer applies — clear it so the
            // re-evaluation on the next pipeline run is loud again (issue
            // #788). Best-effort: a failure to clear does not change staleness.
            if repo.summarization_unconfigured.is_some()
                && let Err(e) = storage.update_repository_unconfigured_marker(&repo.id, None)
            {
                tracing::warn!(
                    "repo '{}': failed to clear unconfigured marker for new commit: {e}",
                    repo.name
                );
            }
            return Ok(true);
        }

        // Check if summarization is incomplete (structural analysis is current but
        // missing summaries). Only trigger resume when the shared enablement rule
        // (issue #786) says summarization runs for this repo — the same rule the
        // summarize and refresh commands apply, so staleness agrees with what
        // actually ran. apfel availability is probed lazily here so the
        // cheap path (apfel backend) is not forced when a non-apfel backend
        // (or an explicit `summarize: true`) already enables it.
        if head == last {
            let repo_path = std::path::Path::new(&repo.local_path);
            // A config that fails to parse or validate warns here (issue #788) and
            // falls back to defaults; a missing config is silent.
            let repo_config = crate::config::RepoConfig::load_or_default(repo_path);
            let apfel_available = crate::summarization::apfel::is_apfel_available();
            // The shared classifier (issue #788) is the one decision point for
            // this repo's state: enabled, enabled-but-unconfigured, or disabled.
            let state = crate::summarization::enabled_state::classify_enabled_state(
                no_summarize,
                &repo_config,
                apfel_available,
            );
            if state == crate::summarization::enabled_state::EnabledState::Disabled {
                continue;
            }
            // Structural analysis is current. Check if summaries are incomplete.
            match storage.count_missing_summaries(&repo.id) {
                Ok(0) => {}
                Ok(missing) => {
                    if state
                        == crate::summarization::enabled_state::EnabledState::EnabledButUnconfigured
                    {
                        // Enabled-but-unconfigured (issue #788): the gate is on
                        // but no backend is usable, so re-running the pipeline
                        // cannot help. A new commit arriving before this point
                        // is handled above (the `head != last` branch returns
                        // stale and clears the marker), so reaching here means
                        // either the marker was already set for this commit
                        // (silent — prior invocation reported) or the config
                        // changed / the marker was just cleared (loud re-report).
                        apply_unconfigured_marker(storage, repo, &repo_config);
                    } else {
                        // Healthy enabled backend: clear any stale marker
                        // (the config now has a usable backend) so a future
                        // failure re-warns, and report the true incomplete
                        // state.
                        if repo.summarization_unconfigured.is_some()
                            && let Err(e) =
                                storage.update_repository_unconfigured_marker(&repo.id, None)
                        {
                            tracing::warn!(
                                "repo '{}': failed to clear unconfigured marker: {e}",
                                repo.name
                            );
                        }
                        // `warn!` (not `debug!`): the gate is enabled but the prior
                        // run left summaries incomplete, so the user must see the
                        // state and the reason it repeats (issue #788).
                        tracing::warn!(
                            "repo '{}' has {} missing summaries (structural up-to-date, summarization needed)",
                            repo.name,
                            missing
                        );
                        return Ok(true);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "failed to count missing summaries for repo '{}': {}",
                        repo.name,
                        e
                    );
                }
            }
        }
    }
    Ok(false)
}

/// Run the cheap analysis layers (extraction → relationships → insights → semantic index).
///
/// Only runs when `is_stale()` is true (unless force=true). Returns immediately with `was_stale=false`
/// if the project is already up to date.
///
/// Also resumes incomplete summarization for structurally-current repos (issue #638).
pub fn ensure_fresh(
    storage: &dyn Storage,
    project_id: &str,
    opts: &RefreshOptions,
) -> Result<RefreshStats> {
    // Skip staleness check if forced refresh is requested
    if !opts.force && !is_stale(storage, project_id, opts.no_summarize)? {
        return Ok(RefreshStats {
            was_stale: false,
            ..Default::default()
        });
    }

    let start = Instant::now();

    // Step 1: re-run analysis pipeline (extraction + relationships + metrics).
    // `respect_ignore=true` means default behavior: respect .gitignore
    let reindex_mode = if opts.full {
        ReindexMode::Full
    } else if opts.force {
        ReindexMode::Force
    } else {
        ReindexMode::Incremental
    };
    let config = PipelineConfig {
        respect_ignore: !opts.no_ignore,
        reindex: reindex_mode,
        no_summarize: opts.no_summarize,
        skip_semantic_index: opts.skip_semantic_index,
    };
    let result = AnalysisPipeline::run_project(storage, project_id, &config)?;
    let repos_refreshed = result.runs.len();
    let summarization_failed = result.summarization_failed;

    // Step 1.5: Resume incomplete summarization (if any repos were skipped but have missing summaries)
    if let Err(e) = resume_incomplete_summarization(
        storage,
        project_id,
        opts.no_summarize,
        summarization_failed,
    ) {
        eprintln!("[auto-refresh] warning: summarization resume failed: {}", e);
    }

    // Propagate per-repo errors as warnings to stderr but do not abort —
    // partial refresh is better than no refresh.
    for (repo_name, err) in &result.repo_errors {
        eprintln!("[auto-refresh] warning: repo '{repo_name}' failed: {err}");
    }

    // Step 2: recompute insights after analysis.
    // Fetch repos first to get repo_root for the detector.
    let repos = match storage.list_repos(project_id) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[auto-refresh] warning: failed to list repos for reconciliation: {e}");
            Vec::new()
        }
    };
    let repo_root = repos.first().map(|r| std::path::Path::new(&r.local_path));
    let detector = match repo_root {
        Some(root) => InsightDetector::new(storage, project_id).with_repo_root(root),
        None => InsightDetector::new(storage, project_id),
    };
    if let Err(e) = detector.detect() {
        eprintln!("[auto-refresh] warning: insight detection failed: {e}");
    }

    // Step 3: reconcile entities by removing orphans (files that no longer exist).
    // This is best-effort — log results but don't fail the refresh.
    //
    // Guard (issue #872): only reconcile repos whose path is a live git
    // repository. `reconcile_entities` deletes every entity whose on-disk path
    // no longer exists — for a repo whose path vanished (deleted, moved,
    // unmounted drive) that condition is true for ALL of its entities, so an
    // unguarded reconcile wipes the whole index. Once wiped, `is_stale` skips
    // the unreachable repo (head_commit errors) and the stored commit still
    // matches HEAD when the path returns, so the project reports current with
    // an empty index. A valid repo with deleted files still reconciles as
    // before: the guard only rejects paths that are not openable git repos.
    let repos = match storage.list_repos(project_id) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[auto-refresh] warning: failed to list repos for reconciliation: {e}");
            Vec::new()
        }
    };
    for repo in &repos {
        let repo_path = std::path::Path::new(&repo.local_path);
        // Guard (shared helper): an unreachable path or not-a-git-repo yields
        // None (with a single warning line) and the index is left intact;
        // an unborn branch (no commits) is openable, so it reconciles as
        // before (nothing to remove anyway).
        if reachable_head_commit(&repo.name, repo_path).is_none() {
            continue;
        }
        match storage.reconcile_entities(project_id, &repo.id, repo_path) {
            Ok(stats) => eprintln!(
                "[auto-refresh] reconciled: removed {} entities, {} relationships",
                stats.entities_removed, stats.relationships_removed
            ),
            Err(e) => eprintln!(
                "[auto-refresh] warning: reconciliation for repo '{}' failed: {e}",
                repo.name
            ),
        }
    }

    let elapsed_secs = start.elapsed().as_secs();
    Ok(RefreshStats {
        was_stale: true,
        elapsed_secs,
        repos_refreshed,
    })
}

/// Check staleness by opening storage and evaluating `is_stale()`.
///
/// Returns `Ok(true)` if any repo is stale, `Ok(false)` if none are.
/// Returns `Err` if storage cannot be opened OR if staleness check fails.
/// Preserves distinct warning messages:
/// - Storage-open failure: "Background refresh: failed to open storage: {e}"
/// - Staleness-check failure: "Background refresh: staleness check failed: {e}"
fn check_staleness(project_id: &str) -> Result<bool> {
    let storage = match SqliteStorage::open() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Background refresh: failed to open storage: {}", e);
            return Err(e);
        }
    };
    match is_stale(&storage, project_id, false) {
        Ok(stale) => Ok(stale),
        Err(e) => {
            tracing::warn!("Background refresh: staleness check failed: {}", e);
            Err(e)
        }
    }
}

/// Spawn a background refresh if data is stale. Returns immediately.
///
/// Checks staleness synchronously (fast <10ms). If stale, spawns a std::thread
/// that runs the full refresh pipeline. Next call will see fresh data.
///
/// Respects `LIEVO_NO_REFRESH` env var — returns early if set to "1".
/// Logs refresh start/completion/errors to stderr.
///
/// Thread safety: Opens a new SqliteStorage connection in the spawned thread
/// to avoid cloning across thread boundaries. SQLite WAL mode allows concurrent
/// readers during writes.
/// Prevents concurrent refreshes with an atomic guard.
static REFRESH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

pub fn spawn_background_refresh_if_stale(project_id: String) {
    // Respect the env var
    if std::env::var("LIEVO_NO_REFRESH").is_ok_and(|v| v == "1") {
        return;
    }

    // Check and set the guard — if already in progress, skip.
    // This guard will be moved into the spawned thread, ensuring the flag
    // is held continuously from parent acquire through child completion.
    let guard = match RefreshGuard::try_acquire() {
        Some(g) => g,
        None => {
            tracing::debug!("Background refresh already in progress, skipping");
            return;
        }
    };

    // Quick staleness check (preserves distinct warn messages for two error sources)
    let is_stale = match check_staleness(&project_id) {
        Ok(stale) => stale,
        Err(_) => return, // guard drops here, clearing flag
    };

    if !is_stale {
        return; // guard drops here, clearing flag
    }

    // Spawn thread to run the refresh pipeline.
    // Move the guard into the thread closure to transfer ownership atomically.
    // This ensures the flag stays true from acquire to thread completion.
    let project_id_clone = project_id.clone();
    let _handle = std::thread::spawn(move || {
        // The guard is moved here from the parent thread.
        // We take ownership and hold it for the duration of the thread.
        let _guard = guard;

        eprintln!(
            "[auto-refresh-bg] starting background refresh for '{}'",
            project_id_clone
        );

        // Open a fresh connection in the spawned thread
        let storage = match SqliteStorage::open() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[auto-refresh-bg] failed to open storage: {e}");
                return; // guard drops here, clearing flag
            }
        };

        // Run the refresh
        let opts = RefreshOptions::default();
        match ensure_fresh(&storage, &project_id_clone, &opts) {
            Ok(stats) => {
                eprintln!(
                    "[auto-refresh-bg] completed: {}s, {} repos refreshed",
                    stats.elapsed_secs, stats.repos_refreshed
                );
            }
            Err(e) => {
                eprintln!("[auto-refresh-bg] refresh failed: {e}");
            }
        }

        // Guard automatically drops here, clearing flag (even on panic)
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn storage() -> SqliteStorage {
        SqliteStorage::open_in_memory().unwrap()
    }

    #[test]
    fn test_is_stale_no_repos_returns_false() {
        let s = storage();
        let proj = s.create_project("proj", None).unwrap();
        let result = is_stale(&s, &proj.id, false).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_update_repository_unconfigured_marker_roundtrip() {
        // Issue #788: the marker can be set, read back via list_repos, and
        // cleared. This pins the column plumbing (schema + query + model)
        // that `is_stale` relies on to skip re-entering the full pipeline for
        // an enabled-but-unconfigured repo across invocations.
        let s = storage();
        let proj = s.create_project("proj", None).unwrap();
        let repo = s
            .add_repo(&proj.id, "repo1", "/nonexistent-but-allowed")
            .expect("add_repo");

        // No marker by default.
        let repos = s.list_repos(&proj.id).unwrap();
        assert_eq!(repos[0].summarization_unconfigured, None);

        // Set the marker.
        s.update_repository_unconfigured_marker(&repo.id, Some("true|apfel|"))
            .expect("set marker");
        let repos = s.list_repos(&proj.id).unwrap();
        assert_eq!(
            repos[0].summarization_unconfigured.as_deref(),
            Some("true|apfel|")
        );

        // Clear the marker.
        s.update_repository_unconfigured_marker(&repo.id, None)
            .expect("clear marker");
        let repos = s.list_repos(&proj.id).unwrap();
        assert_eq!(repos[0].summarization_unconfigured, None);
    }

    #[test]
    fn test_ensure_fresh_no_repos_returns_not_stale() {
        let s = storage();
        let proj = s.create_project("proj", None).unwrap();
        let opts = RefreshOptions::default();
        let stats = ensure_fresh(&s, &proj.id, &opts).unwrap();
        assert!(!stats.was_stale);
        assert_eq!(stats.repos_refreshed, 0);
    }

    #[test]
    fn test_refresh_stats_default() {
        let stats = RefreshStats::default();
        assert!(!stats.was_stale);
        assert_eq!(stats.elapsed_secs, 0);
        assert_eq!(stats.repos_refreshed, 0);
    }

    // Guard behaviour tests moved to refresh_guard::tests (issue #864 file
    // split).
}
