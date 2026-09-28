// Issue #856 end-to-end: the repo-wide unresolved-import counts must reach
// the query layer on a REAL index — no hand-seeded `path == repo.name`
// marker entity. A JS repo with one bad relative import (internal) and one
// bad bare npm-style import (external) is indexed through the full
// AnalysisPipeline, and `get_unresolved_counts` / `get_impact`'s honesty
// signal must report the recorded values (and a fully-resolved fresh index
// must read as recorded zeros, never null).

use lievo::analysis::pipeline::{AnalysisPipeline, PipelineConfig, ReindexMode};
use lievo::model::Repository;
use lievo::query::dependency::{ResolutionSignal, impact_resolution_for_repo};
use lievo::retrieval::tools::tools_impl::tools_query::resolution_coverage_value;
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;
use std::path::Path;
use tempfile::Builder;

/// Temp dir that is NOT auto-cleaned: the extractor persists its file-hash
/// store at `<repo>/.lievo`, which must outlive the test so a later no-op
/// incremental refresh sees the same known file set (a dropped dir looks
/// like "every file deleted" to the incremental bookkeeping).
fn fresh_repo_dir() -> std::path::PathBuf {
    Builder::new()
        .prefix("lievo-856-")
        .tempdir_in("/tmp")
        .unwrap()
        .keep()
}

fn init_git_repo(dir: &Path) {
    let git_repo = git2::Repository::init(dir).unwrap();
    let mut config = git_repo.config().unwrap();
    config.set_str("user.name", "T").unwrap();
    config.set_str("user.email", "t@t.com").unwrap();
}

fn commit_all(git_repo: &git2::Repository, msg: &str) {
    let mut index = git_repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = git_repo.find_tree(tree_id).unwrap();
    let sig = git_repo.signature().unwrap();
    git_repo
        .commit(Some("HEAD"), &sig, &sig, msg, &tree, &[])
        .unwrap();
}

fn run_full_pipeline(storage: &SqliteStorage, repo: &Repository) {
    let config = PipelineConfig {
        respect_ignore: true,
        reindex: ReindexMode::Full,
        no_summarize: true,
        skip_semantic_index: false,
    };
    AnalysisPipeline::run_repo(storage, repo, &config)
        .expect("full pipeline run must succeed")
        .expect("repo must be analyzed, not skipped");
}

// ── 1. Core e2e: real pipeline, no hand-seeding — signal reaches the query ──

#[test]
fn test_pipeline_unresolved_counts_reach_get_unresolved_counts() {
    let root = fresh_repo_dir();
    init_git_repo(&root);
    // One bad relative import (internal) + one bad bare npm-style import
    // (external — not a local directory, not a declared dependency).
    std::fs::write(
        root.join("broken.js"),
        "import { helper } from './does-not-exist';\n\
         import { widget } from 'left-pad-zz';\n\
         export function main() { return helper; }\n",
    )
    .unwrap();
    commit_all(&git2::Repository::open(&root).unwrap(), "init");

    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("e2e-unresolved", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "broken-repo", root.to_str().unwrap())
        .unwrap();

    // Pre-index: the counts must be unrecorded (null), not a fabricated zero.
    assert_eq!(
        storage.get_unresolved_counts(&repo.id),
        None,
        "before indexing the counts must be null (not recorded)"
    );

    run_full_pipeline(&storage, &repo);

    let counts = storage
        .get_unresolved_counts(&repo.id)
        .expect("real index must record the counts (Some, not null)");
    assert_eq!(
        counts,
        (1, 1),
        "one bad relative (internal) + one bad bare npm (external) import"
    );
    let signal = impact_resolution_for_repo(&storage, &repo.id);
    assert_eq!(signal.unresolved_internal, Some(1));
    assert_eq!(signal.unresolved_external, Some(1));
    assert!(signal.caveat_active());
}

// ── 2. Fresh index with zero unresolved → recorded 0, not null ─────────────

#[test]
fn test_pipeline_zero_unresolved_writes_recorded_zeros() {
    let root = fresh_repo_dir();
    init_git_repo(&root);
    std::fs::write(root.join("ok.js"), "export function main() { return 1; }\n").unwrap();
    std::fs::write(
        root.join("other.js"),
        "export function helper() { return 2; }\n",
    )
    .unwrap();
    commit_all(&git2::Repository::open(&root).unwrap(), "init");

    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("e2e-clean", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "clean-repo", root.to_str().unwrap())
        .unwrap();

    run_full_pipeline(&storage, &repo);

    // Recorded zeros must read as Some((0, 0)) — the null-vs-zero distinction
    // (#856 decision 2: the two states must never look alike).
    assert_eq!(storage.get_unresolved_counts(&repo.id), Some((0, 0)));
    let signal = impact_resolution_for_repo(&storage, &repo.id);
    assert_eq!(signal, ResolutionSignal::from_counts(Some((0, 0))));
    // A recorded-zero repo must surface as full coverage (1.0), never null.
    assert_eq!(resolution_coverage_value(&signal), serde_json::json!(1.0));
}

// ── 3. Incremental: a no-op refresh must not flip recorded ↔ null ──────────

#[test]
fn test_incremental_noop_refresh_preserves_recorded_counts() {
    let root = fresh_repo_dir();
    init_git_repo(&root);
    std::fs::write(
        root.join("broken.js"),
        "import { missing } from './nope';\nexport function main() {}\n",
    )
    .unwrap();
    let git_repo = git2::Repository::open(&root).unwrap();
    commit_all(&git_repo, "init");
    let head = git_repo
        .head()
        .unwrap()
        .peel(git2::ObjectType::Commit)
        .unwrap()
        .id()
        .to_string();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("e2e-incremental", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "incr-repo", root.to_str().unwrap())
        .unwrap();

    // Full analysis records (1, 0) and stamps last_analyzed_commit on the row.
    run_full_pipeline(&storage, &repo);
    assert_eq!(storage.get_unresolved_counts(&repo.id), Some((1, 0)));
    // Re-read the repo: the skip check compares the persisted
    // last_analyzed_commit against HEAD, which the handle from before
    // indexing does not carry.
    let repo = storage
        .get_repo(&repo.id)
        .unwrap()
        .expect("repo must exist");
    assert_eq!(repo.last_analyzed_commit.as_deref(), Some(head.as_str()));

    // No-op incremental refresh (same HEAD) → skip path.
    let config = PipelineConfig {
        respect_ignore: true,
        reindex: ReindexMode::Incremental,
        no_summarize: true,
        skip_semantic_index: false,
    };
    let run =
        AnalysisPipeline::run_repo(&storage, &repo, &config).expect("incremental run must succeed");
    assert!(run.is_none(), "same HEAD must take the no-op skip path");

    // The recorded counts must survive the no-op refresh unchanged — never
    // flipped to null.
    assert_eq!(
        storage.get_unresolved_counts(&repo.id),
        Some((1, 0)),
        "no-op incremental refresh must not clear the recorded counts"
    );
}
