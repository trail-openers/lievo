//! Serve-startup automatic index (issue #864).
//!
//! `lievo mcp` calls `start_index_if_needed` once after the repo is
//! resolved: a never-indexed repo gets a full first index in the
//! background, a stale repo gets an incremental refresh, a fresh index
//! does nothing. The work runs on a background thread holding the
//! per-repo cross-process lock (see `lock`), which the OS releases if the
//! process dies.
//!
//! The automatic path builds ONLY the core index (entities, relationships,
//! file hashes) — it skips the semantic/vector index and any embedding-model
//! download, which `lievo refresh` still builds (issue #864 decision: the
//! model serves only the gated `search_entities` tool and
//! `lievo query entities --semantic`, and a first MCP start must not
//! download model files from HuggingFace).

use std::path::Path;

use crate::refresh::lock;
use crate::refresh::{RefreshOptions, ensure_fresh};
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

/// Decision made at serve start (issue #864 binding decision 1): decide
/// whether a background index is needed for a freshly resolved repo.
/// - never indexed (no stored `last_analyzed_commit`) → first full index
/// - stale (new commits since last index) → incremental refresh
/// - fresh → nothing
///
/// The decision is made in the caller's process (fast, <10ms, using the
/// server's already-open storage) — the background thread re-acquires the
/// cross-process lock and re-runs the full pipeline. A lock acquisition
/// failure (another live session indexing) is reported via tracing and
/// returns `None`; the caller can then surface "indexing in progress".
pub fn start_index_if_needed(project_id: &str, repo_path: &Path) -> Option<lock::IndexTrigger> {
    if std::env::var("LIEVO_NO_REFRESH").is_ok_and(|v| v == "1") {
        return None;
    }

    let trigger = check_trigger(project_id)?;

    // Acquire the lock synchronously BEFORE spawning: this is the only
    // moment the caller can know "we won the index" vs "a live session is
    // already indexing". The File handle is moved into the thread and held
    // for the duration of the refresh; the OS releases it when the
    // process/thread dies (binding decision 2).
    let data_dir = match crate::extraction::lievo_data_dir() {
        Ok(d) => d,
        Err(_) => return None, // No data dir: cannot hold the cross-process lock.
    };
    let lock_file = match lock::acquire(repo_path, trigger, &data_dir) {
        Ok(f) => f,
        Err(_) => {
            // A live session holds the lock — it will do the work. Do not
            // start a second one. The `lievo_explore` in-progress response
            // picks this up via `indexing_status`.
            tracing::debug!("another lievo mcp session is indexing; skipping");
            return None;
        }
    };

    // In-process guard: prevent double-spawn within the same process
    // (the cross-process lock is held by the background thread, so a
    // second serve() call in the same process must not spawn again).
    let guard = super::refresh_guard::RefreshGuard::try_acquire()?;

    let pid = project_id.to_string();
    let _handle = std::thread::spawn(move || {
        let _guard = guard;
        eprintln!(
            "[auto-index-bg] starting {} for '{}'",
            trigger.as_str(),
            pid
        );
        let storage = match SqliteStorage::open() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[auto-index-bg] failed to open storage: {e}");
                return;
            }
        };
        // Core index only — no summarization and no semantic/vector index
        // (or embedding-model download): both are left to `lievo refresh`
        // (issue #872 binding decision 5: only the serve-start automatic
        // path changes; manual refresh keeps its summary-aware behaviour).
        let opts = RefreshOptions {
            no_summarize: true,
            skip_semantic_index: true,
            ..Default::default()
        };
        match ensure_fresh(&storage, &pid, &opts) {
            Ok(stats) => eprintln!(
                "[auto-index-bg] completed: {}s, {} repos refreshed",
                stats.elapsed_secs, stats.repos_refreshed
            ),
            Err(e) => eprintln!("[auto-index-bg] refresh failed: {e}"),
        }
        // lock_file (and guard) drop here; OS lock released on drop.
        let _ = lock_file;
    });

    Some(trigger)
}

/// Determine which trigger (if any) applies — commits only (issue #872
/// binding decision 1): never indexed (no repo with a stored
/// `last_analyzed_commit`) → first full index; any repo with a stored
/// `last_analyzed_commit` different from its current HEAD → incremental
/// refresh; otherwise nothing. Missing summaries never trigger the
/// automatic index (that branch of `is_stale` is reserved for the manual
/// `lievo refresh` path). Repos whose path is unreachable or not a git
/// repository are skipped with a warning and never make the project stale.
/// The caller (serve start) uses this to decide whether to spawn a
/// background thread.
fn check_trigger(project_id: &str) -> Option<lock::IndexTrigger> {
    let storage = SqliteStorage::open().ok()?;
    let repos = storage.list_repos(project_id).ok()?;
    if repos.is_empty() {
        return None;
    }
    let has_any_indexed = repos.iter().any(|r| r.last_analyzed_commit.is_some());
    if !has_any_indexed {
        return Some(lock::IndexTrigger::FirstIndex);
    }
    // Commit-only staleness (OR across repos): the first repo whose stored
    // commit differs from its current HEAD makes the project stale.
    for repo in &repos {
        let Some(stored) = repo.last_analyzed_commit.as_deref() else {
            continue;
        };
        // Shared helper (issue #872): an unborn branch (no commits) or an
        // unreachable path yields None (the helper warns on the latter); an
        // unreachable repo never makes the project stale (issue #872 binding
        // decision 1).
        if let Some(head) =
            super::reachable_head_commit(&repo.name, std::path::Path::new(&repo.local_path))
            && head != stored
        {
            return Some(lock::IndexTrigger::IncrementalRefresh);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refresh::lock::lock_path;

    /// Isolate the lievo data dir + database in a temp home / temp DB for the
    /// test's duration (crate-shared EnvGuard, issue #872).
    type EnvGuard = crate::test_env_support::EnvGuard;

    fn init_git_repo(dir: &std::path::Path) {
        // A real top-level function (not just a data file): tree-sitter
        // extracts it as a function-tier entity, so after indexing the repo
        // has real entities — hence missing summaries.
        std::fs::write(
            dir.join("src.rs"),
            "pub fn compute() -> u32 {\n    42 + 1\n}\n",
        )
        .unwrap();
        git(&["-C", dir.to_str().unwrap(), "init", "-q"]);
        git(&["-C", dir.to_str().unwrap(), "add", "src.rs"]);
        git(&["-C", dir.to_str().unwrap(), "commit", "-q", "-m", "one"]);
    }

    /// Run git with the test committer identity (committed to the repo in
    /// `dir`), asserting a successful exit.
    fn git(args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .expect("git run");
        assert!(status.success(), "git {args:?} failed");
    }

    /// The automatic index (serve-startup path) must build the core index
    /// only — no vector index file and no embedding-model download
    /// (issue #864). A manual `lievo refresh` still builds the semantic
    /// index; `tests/analysis_pipeline_test.rs` covers the default
    /// (semantic-on) pipeline path with `skip_semantic_index: false`.
    #[test]
    fn test_auto_index_skips_vector_index() {
        let repo = tempfile::tempdir().unwrap();
        let repo_path = repo.path().canonicalize().unwrap();
        init_git_repo(&repo_path);

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("auto.db");
        let _guard = EnvGuard::new(&db_path);

        let storage = SqliteStorage::open().expect("open storage");
        let project = storage.create_project("auto-proj", None).unwrap();
        storage
            .add_repo(&project.id, "repo1", repo_path.to_str().unwrap())
            .expect("add_repo");

        // Serve-startup automatic index path: core index only.
        let opts = RefreshOptions {
            skip_semantic_index: true,
            ..Default::default()
        };
        let stats = ensure_fresh(&storage, &project.id, &opts).expect("auto index ran");
        assert!(stats.was_stale, "first index must run");
        drop(storage);

        // Core index was written: the repo has a stored analyzed commit.
        let storage = SqliteStorage::open().unwrap();
        let repos = storage.list_repos(&project.id).unwrap();
        assert!(
            repos[0]
                .last_analyzed_commit
                .as_ref()
                .is_some_and(|c| !c.is_empty()),
            "core index must record the analyzed commit"
        );

        // Vector index must NOT exist (no model download ran).
        let vector_index = crate::extraction::ts_index_dir_for_repo(&repo_path)
            .unwrap()
            .join("vectors.usearch");
        assert!(
            !vector_index.exists(),
            "automatic index must not build the vector index at {vector_index:?}"
        );
        drop(storage);
    }

    /// A stale-lock holder that dies (SIGKILL) must release the lock: the
    /// next `start_index_if_needed` call wins the lock and spawns the
    /// background index. This is an integration test for `auto` — it needs
    /// the real `acquire` path via the spawned lock holder and real env
    /// isolation.
    #[test]
    fn test_auto_index_acquires_lock_after_dead_holder() {
        // The real lock-release semantics are covered by
        // `lock_tests::sigkilled_holder_releases_lock_within_bounded_wait`
        // and the spawn-side by `start_index_if_needed`'s acquire call; this
        // test pins the auto-path-specific behaviour: a fresh never-indexed
        // repo with no env override must return `FirstIndex` from
        // `start_index_if_needed` (i.e. it acquired the lock and spawned).
        let repo = tempfile::tempdir().unwrap();
        let repo_path = repo.path().canonicalize().unwrap();
        init_git_repo(&repo_path);

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("auto-lock.db");
        let _guard = EnvGuard::new(&db_path);

        let storage = SqliteStorage::open().expect("open storage");
        let project = storage.create_project("lock-proj", None).unwrap();
        storage
            .add_repo(&project.id, "repo1", repo_path.to_str().unwrap())
            .expect("add_repo");

        let trigger = start_index_if_needed(&project.id, &repo_path);
        assert_eq!(trigger, Some(lock::IndexTrigger::FirstIndex));

        // The background thread is holding the lock; a second
        // start_index_if_needed in the same process must be denied by the
        // in-process RefreshGuard (not the cross-process lock, which is
        // held by our own thread). The dead-holder variant is covered by
        // `refresh::lock_holder::lock_holder` via the lock release test in
        // `lock_tests::sigkilled_holder_releases_lock_within_bounded_wait`.
        let second = start_index_if_needed(&project.id, &repo_path);
        assert_eq!(
            second, None,
            "second spawn in the same process must be denied by the guard"
        );

        // Wait for the background index to complete so the guard and the
        // lock are released before the EnvGuard drops (test teardown).
        wait_for_background_index(&repo_path);
    }

    /// Poll the cross-process lock file until the background thread has
    /// released it (lock file unheld or gone), bounding the wait at 120s.
    fn wait_for_background_index(repo_path: &std::path::Path) {
        let lock_file = lock_path(repo_path).expect("lock path");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            match std::fs::File::open(&lock_file) {
                Ok(f) => {
                    if f.try_lock().is_ok() {
                        return;
                    }
                }
                Err(_) => return,
            }
            if std::time::Instant::now() > deadline {
                panic!("background index did not finish within 120s");
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
    }

    /// A current repo with missing summaries must NOT start a background
    /// index on serve start: the automatic trigger decides staleness by
    /// commits only, and the missing-summaries branch of `is_stale` is
    /// reserved for the manual `lievo refresh` path (issue #872 acceptance
    /// criterion: "missing summaries never trigger an automatic index").
    #[test]
    fn test_current_repo_with_missing_summaries_does_not_trigger() {
        let repo = tempfile::tempdir().unwrap();
        let repo_path = repo.path().canonicalize().unwrap();
        init_git_repo(&repo_path);

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("no-summaries.db");
        let _guard = EnvGuard::new(&db_path);

        let storage = SqliteStorage::open().expect("open storage");
        let project = storage.create_project("sum-proj", None).unwrap();
        storage
            .add_repo(&project.id, "repo1", repo_path.to_str().unwrap())
            .expect("add_repo");

        // A real core index: the repo gets a stored analyzed commit and real
        // entities, which (with `no_summarize`) is a repo with missing
        // summaries.
        let opts = RefreshOptions {
            no_summarize: true,
            skip_semantic_index: true,
            ..Default::default()
        };
        ensure_fresh(&storage, &project.id, &opts).expect("first index ran");
        use crate::model::EntityTier;
        let entities = storage
            .list_entities(&project.id, Some(EntityTier::Function))
            .unwrap();
        assert!(
            !entities.is_empty(),
            "a function-tier entity must exist (hence summaries missing) for the test to mean anything"
        );
        drop(storage);

        // Commit-stale but summaries missing: no trigger. (Missing summaries
        // would have triggered under the old summary-aware `is_stale` path
        // whenever a healthy apfel backend was enabled for the repo.)
        assert_eq!(
            check_trigger(&project.id),
            None,
            "a current repo with missing summaries must not start a background index"
        );
    }

    /// The automatic trigger skips repos whose path is unreachable (or not a
    /// git repository) with a warning instead of making the project stale
    /// (issue #872 binding decision 1: the same OR semantics as `is_stale`,
    /// minus the missing-summaries branch, and unreachable repos never make
    /// the project stale).
    #[test]
    fn test_unreachable_repo_is_skipped_not_stale() {
        let repo = tempfile::tempdir().unwrap();
        let repo_path = repo.path().canonicalize().unwrap();
        init_git_repo(&repo_path);

        let db = tempfile::tempdir().unwrap();
        let db_path = db.path().join("unreachable.db");
        let _guard = EnvGuard::new(&db_path);

        let storage = SqliteStorage::open().expect("open storage");
        let project = storage.create_project("unreach-proj", None).unwrap();
        storage
            .add_repo(&project.id, "repo1", repo_path.to_str().unwrap())
            .expect("add_repo");
        ensure_fresh(
            &storage,
            &project.id,
            &RefreshOptions {
                no_summarize: true,
                skip_semantic_index: true,
                ..Default::default()
            },
        )
        .expect("first index ran");
        drop(storage);

        // Rename the repo away: the stored commit no longer matches any
        // reachable HEAD. The trigger must skip the repo (warning) rather
        // than report stale — the index of an unreachable repo must never be
        // refreshed (and the reconcile step must never wipe it).
        let renamed = repo_path.with_file_name("renamed-away");
        std::fs::rename(&repo_path, &renamed).unwrap();
        assert_eq!(
            check_trigger(&project.id),
            None,
            "an unreachable repo must not trigger an automatic refresh"
        );
        std::fs::rename(&renamed, &repo_path).unwrap();
    }
}
