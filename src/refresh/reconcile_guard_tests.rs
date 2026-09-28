//! Issue #872: data-loss regression tests for the reconcile guard in
//! `ensure_fresh`.
//!
//! The guard lives in the step-3 reconcile loop: `reconcile_entities` deletes
//! every entity whose on-disk path no longer exists, so a repo whose
//! `local_path` vanished (deleted, moved, unmounted drive) would have its
//! ENTIRE index wiped. The guard (head_commit succeeds => reconcile;
//! otherwise skip with a warning) must keep the index intact.

use std::path::Path;

use crate::refresh::{RefreshOptions, ensure_fresh};
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

/// Isolate `HOME` + `LIEVO_DB` in temp dirs for the test's duration
/// (crate-shared EnvGuard, issue #872).
type EnvGuard = crate::test_env_support::EnvGuard;

fn git_run(dir: &Path, args: &[&str]) {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .expect("git run")
        .success()
        .then_some(())
        .expect("git command failed");
}

fn init_git_repo(dir: &Path) {
    std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
    git_run(dir, &["init", "-q"]);
    git_run(dir, &["add", "a.rs"]);
    git_run(dir, &["commit", "-q", "-m", "one"]);
}

/// Register + really index a temp git repo, then make the path unavailable by
/// renaming the directory. The stored `last_analyzed_commit` stays equal to
/// HEAD (the rename did not change the commit), so `is_stale` is false and
/// `ensure_fresh` returns early without reaching step 3 — exactly the real
/// first-launch race window. The wipe happened in the prior call: with the
/// guard removed from that call, `reconcile_entities` deleted every entity.
///
/// Falsification: with the guard removed, `ensure_fresh` (step 3, force)
/// reconciles against the vanished path and deletes all file-tier entities —
/// the assertions below fail.
#[test]
fn renamed_repo_path_is_skipped_by_reconcile_and_index_stays_intact() {
    let base = tempfile::tempdir().unwrap();
    let repo_src = base.path().join("repo");
    std::fs::create_dir(&repo_src).unwrap();
    let repo_path = repo_src.canonicalize().unwrap();
    init_git_repo(&repo_path);

    let db = tempfile::tempdir().unwrap();
    let db_path = db.path().join("reconcile-guard.db");
    let _guard = EnvGuard::new(&db_path);

    // Initial real index: entities to lose.
    {
        let storage = SqliteStorage::open().expect("open storage");
        let project = storage.create_project("reconcile-proj", None).unwrap();
        storage
            .add_repo(&project.id, "repo1", repo_path.to_str().unwrap())
            .expect("add_repo");
        let opts = RefreshOptions {
            force: true,
            no_summarize: true,
            ..Default::default()
        };
        let stats = ensure_fresh(&storage, &project.id, &opts).expect("first index ran");
        assert!(stats.was_stale, "first index must run");
        let entities = storage
            .entities_by_repo(&storage.list_repos(&project.id).unwrap()[0].id, None)
            .expect("list entities");
        assert!(
            !entities.is_empty(),
            "indexed repo must have entities (nothing to lose otherwise)"
        );
    }

    // Make the repo path unavailable.
    let repo_moved = base.path().join("repo-moved");
    std::fs::rename(&repo_src, &repo_moved).unwrap();

    // Record the index state (the entities to potentially lose).
    let (before_entity_count, before_rel_count, repo_id, project_id) = {
        let storage = SqliteStorage::open().unwrap();
        let project = storage
            .list_projects()
            .unwrap()
            .into_iter()
            .find(|p| p.name == "reconcile-proj")
            .expect("project present");
        let repos = storage.list_repos(&project.id).unwrap();
        let repo = &repos[0];
        (
            storage.entities_by_repo(&repo.id, None).unwrap().len(),
            storage.list_all_relationships(&project.id).unwrap().len(),
            repo.id.clone(),
            project.id,
        )
    };

    {
        let storage = SqliteStorage::open().unwrap();
        let opts = RefreshOptions {
            force: true,
            no_summarize: true,
            ..Default::default()
        };
        ensure_fresh(&storage, &project_id, &opts).expect("refresh ran");
    }

    // The index must be untouched: same entities, no relationships deleted.
    {
        let storage = SqliteStorage::open().unwrap();
        let after_entities = storage.entities_by_repo(&repo_id, None).unwrap().len();
        let after_rels = storage.list_all_relationships(&project_id).unwrap().len();
        assert_eq!(
            after_entities, before_entity_count,
            "reconcile guard must not delete entities of an unreachable repo"
        );
        assert_eq!(
            after_rels, before_rel_count,
            "reconcile guard must not delete relationships of an unreachable repo"
        );
    }

    // Restore the path and assert the index is still usable: a normal query
    // returns the entities.
    std::fs::rename(&repo_moved, &repo_src).unwrap();
    {
        let storage = SqliteStorage::open().unwrap();
        let entities = storage
            .entities_by_repo(&repo_id, None)
            .expect("entities still queryable");
        assert_eq!(
            entities.len(),
            before_entity_count,
            "index intact after restore"
        );
        assert!(
            entities.iter().any(|e| e.name == "main"),
            "normal entity query still works after restore"
        );
    }
}

/// A valid repo whose files were deleted inside it must STILL be reconciled
/// (orphan removal must keep working — the guard only rejects paths that are
/// not openable git repositories).
#[test]
fn deleted_files_in_live_repo_still_reconciled() {
    let base = tempfile::tempdir().unwrap();
    let repo_src = base.path().join("repo");
    std::fs::create_dir(&repo_src).unwrap();
    std::fs::write(repo_src.join("a.rs"), "fn main() {}\n").unwrap();
    std::fs::write(repo_src.join("b.rs"), "fn helper() {}\n").unwrap();
    let repo_path = repo_src.canonicalize().unwrap();
    git_run(&repo_path, &["init", "-q"]);
    git_run(&repo_path, &["add", "a.rs"]);
    git_run(&repo_path, &["commit", "-q", "-m", "one"]);
    git_run(&repo_path, &["add", "b.rs"]);
    git_run(&repo_path, &["commit", "-q", "-m", "two"]);

    let db = tempfile::tempdir().unwrap();
    let db_path = db.path().join("reconcile-live.db");
    let _guard = EnvGuard::new(&db_path);

    // Index both files.
    let (repo_id, project_id) = {
        let storage = SqliteStorage::open().expect("open storage");
        let project = storage.create_project("reconcile-live", None).unwrap();
        storage
            .add_repo(&project.id, "repo1", repo_path.to_str().unwrap())
            .expect("add_repo");
        let repo_id = storage.list_repos(&project.id).unwrap()[0].id.clone();
        let opts = RefreshOptions {
            force: true,
            no_summarize: true,
            ..Default::default()
        };
        ensure_fresh(&storage, &project.id, &opts).expect("index ran");
        let before = storage.entities_by_repo(&repo_id, None).unwrap().len();
        assert!(before > 0, "indexed repo must have entities");
        (repo_id, project.id)
    };

    // Delete one tracked file from the working tree, then force refresh so
    // step 3 reconciles against the live repo.
    std::fs::remove_file(repo_path.join("b.rs")).unwrap();
    {
        let storage = SqliteStorage::open().unwrap();
        let opts = RefreshOptions {
            force: true,
            no_summarize: true,
            ..Default::default()
        };
        ensure_fresh(&storage, &project_id, &opts).expect("refresh ran");
    }

    // The file-tier entity for the deleted file must be gone — orphan removal
    // on a live repo is untouched by the guard.
    {
        let storage = SqliteStorage::open().unwrap();
        let entities = storage.entities_by_repo(&repo_id, None).unwrap();
        let b_entity = entities
            .iter()
            .filter(|e| e.path.as_deref().is_some_and(|p| p.ends_with("b.rs")))
            .count();
        assert_eq!(
            b_entity, 0,
            "orphan removal must still delete the entity for a deleted file in a live repo"
        );
        assert!(
            entities.iter().any(|e| e.name == "main"),
            "entities for surviving files must remain"
        );
    }
}
