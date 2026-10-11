//! Tests for moved-checkout relocation (issue #30).

use super::*;
use crate::extraction::lievo_data_dir;
use crate::layout::{lock_path_for_hash, repo_hash, status_path_for_hash};
use crate::refresh::lock::open_lock_file;
use crate::registration::{RegistrationRequest, register};
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use crate::test_env_support::EnvGuard;
use tempfile::TempDir;

const GIT_URL: &str = "github.com/acme/tool";
const INDEX_FILE: &str = "ts-index/vectors.usearch";

/// A vanished checkout registered with an index, under `data`.
struct Fixture {
    storage: SqliteStorage,
    project_id: String,
    repo: Repository,
    old_path: String,
    old_hash: String,
    new_root: PathBuf,
    data: PathBuf,
}

/// Build the fixture under `base` (canonical). The old checkout path does not
/// exist; `new_root` does. Seeds `indices/<old>/ts-index/vectors.usearch`, a
/// status file, and a last-analyzed commit.
fn fixture(base: &Path, data: PathBuf, with_git_url: bool) -> Fixture {
    let old_path = base.join("old-checkout").to_str().unwrap().to_string();
    let new_root = base.join("new-checkout");
    fs::create_dir_all(&new_root).unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("tool", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "old-checkout", &old_path)
        .unwrap();
    if with_git_url {
        storage.set_repo_git_url(&repo.id, GIT_URL).unwrap();
    }
    storage.update_repo_last_commit(&repo.id, "abc123").unwrap();

    let old_hash = hash_canonical_path(&old_path);
    let index_file = data.join("indices").join(&old_hash).join(INDEX_FILE);
    fs::create_dir_all(index_file.parent().unwrap()).unwrap();
    fs::write(&index_file, b"idx").unwrap();
    fs::write(status_path_for_hash(&data, &old_hash), b"{}").unwrap();
    storage
        .update_repo_index_path(&repo.id, index_file.to_str().unwrap())
        .unwrap();
    let repo = storage.get_repo(&repo.id).unwrap().unwrap();

    Fixture {
        storage,
        project_id: project.id,
        repo,
        old_path,
        old_hash,
        new_root,
        data,
    }
}

fn new_canon(f: &Fixture) -> PathBuf {
    f.new_root.canonicalize().unwrap()
}

/// Open (creating if needed) a lock file under `data` for a hash.
fn open_lock(data: &Path, hash: &str) -> File {
    fs::create_dir_all(data).unwrap();
    open_lock_file(&lock_path_for_hash(data, hash)).unwrap()
}

/// The moved row, panicking with the skip reason otherwise.
fn moved(outcome: RelocateOutcome) -> Repository {
    match outcome {
        RelocateOutcome::Moved(repo) => *repo,
        RelocateOutcome::Skipped(reason) => panic!("expected a move, skipped: {reason}"),
    }
}

/// The skip reason, panicking if the move happened.
fn skipped(outcome: RelocateOutcome) -> SkipReason {
    match outcome {
        RelocateOutcome::Skipped(reason) => reason,
        RelocateOutcome::Moved(_) => panic!("expected a skip, got a move"),
    }
}

#[test]
fn successful_move_rekeys_row_and_index_without_reindex() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);

    let outcome = relocate_in(&f.storage, &f.repo, &f.new_root, &f.data).unwrap();
    let moved = moved(outcome);

    let new_hash = repo_hash(&new_canon(&f)).unwrap();
    let new_dir = f.data.join("indices").join(&new_hash);
    assert_eq!(moved.id, f.repo.id, "the same repo row is reused");
    assert_eq!(moved.project_id, f.project_id);
    assert_eq!(moved.local_path, new_canon(&f).to_str().unwrap());
    // Single source of truth: the index path lives under the new-path hash dir.
    assert_eq!(
        moved.index_path.as_deref(),
        Some(new_dir.join(INDEX_FILE).to_str().unwrap())
    );
    assert!(new_dir.join(INDEX_FILE).exists());
    assert!(!f.data.join("indices").join(&f.old_hash).exists());
    assert!(status_path_for_hash(&f.data, &new_hash).exists());
    assert!(!status_path_for_hash(&f.data, &f.old_hash).exists());
    // No reindex: the analyzed commit and project survive the move.
    assert_eq!(moved.last_analyzed_commit.as_deref(), Some("abc123"));
    assert_eq!(f.storage.list_projects().unwrap().len(), 1);
    // The new path's lock is free right away.
    let new_lock = open_lock(&f.data, &new_hash);
    assert!(new_lock.try_lock().is_ok());
}

#[test]
fn busy_old_lock_skips_move_and_leaves_row_untouched() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let holder = open_lock(&f.data, &f.old_hash);
    holder.try_lock().unwrap();

    let reason = skipped(relocate_in(&f.storage, &f.repo, &f.new_root, &f.data).unwrap());

    assert_eq!(
        reason,
        SkipReason::IndexInUse,
        "a live indexer blocks the move"
    );
    let row = f.storage.get_repo(&f.repo.id).unwrap().unwrap();
    assert_eq!(row.local_path, f.old_path);
    assert_eq!(row.index_path, f.repo.index_path);
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists()
    );
    assert!(
        !f.data
            .join("indices")
            .join(repo_hash(&new_canon(&f)).unwrap())
            .exists()
    );
}

#[test]
fn failed_rename_rolls_back_completed_moves_and_keeps_row() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    // A non-empty directory where the status file must land: the index dir
    // rename succeeds first, then the status rename fails.
    let new_hash = repo_hash(&new_canon(&f)).unwrap();
    let blocker = status_path_for_hash(&f.data, &new_hash);
    fs::create_dir_all(&blocker).unwrap();
    fs::write(blocker.join("keep"), b"x").unwrap();

    let reason = skipped(relocate_in(&f.storage, &f.repo, &f.new_root, &f.data).unwrap());

    assert!(matches!(reason, SkipReason::Move { ref unrestored, .. } if unrestored.is_empty()));
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists(),
        "the completed index rename is rolled back"
    );
    assert!(!f.data.join("indices").join(&new_hash).exists());
    assert!(status_path_for_hash(&f.data, &f.old_hash).exists());
    assert!(
        blocker.join("keep").exists(),
        "the blocker is never overwritten"
    );
    let row = f.storage.get_repo(&f.repo.id).unwrap().unwrap();
    assert_eq!(row.local_path, f.old_path);
    assert_eq!(row.index_path, f.repo.index_path);
    // The old lock is released: it can be taken again.
    assert!(open_lock(&f.data, &f.old_hash).try_lock().is_ok());
}

#[test]
fn index_path_outside_index_dir_skips_without_moving() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let stray = base.join("elsewhere").join(INDEX_FILE);
    fs::create_dir_all(stray.parent().unwrap()).unwrap();
    fs::write(&stray, b"idx").unwrap();
    f.storage
        .update_repo_index_path(&f.repo.id, stray.to_str().unwrap())
        .unwrap();
    let repo = f.storage.get_repo(&f.repo.id).unwrap().unwrap();

    let reason = skipped(relocate_in(&f.storage, &repo, &f.new_root, &f.data).unwrap());

    assert_eq!(reason, SkipReason::IndexOutsideIndexDir);
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists(),
        "nothing is moved"
    );
    let row = f.storage.get_repo(&f.repo.id).unwrap().unwrap();
    assert_eq!(row.local_path, f.old_path);
}

#[test]
fn register_reuses_vanished_row_for_same_identity_at_new_path() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let _env = EnvGuard::new(&base.join("db.sqlite"));
    let data = lievo_data_dir().unwrap();
    let f = fixture(&base, data, true);
    let repo = git2::Repository::init(&f.new_root).unwrap();
    repo.config()
        .unwrap()
        .set_str("remote.origin.url", "git@github.com:acme/tool")
        .unwrap();

    let resolved = register(
        &f.storage,
        &RegistrationRequest {
            repo_root: &f.new_root,
            project_name: None,
        },
    )
    .unwrap();

    assert_eq!(resolved.project.id, f.project_id, "same project reused");
    assert_eq!(resolved.repo.id, f.repo.id, "same repo row reused");
    assert_eq!(resolved.repo.local_path, new_canon(&f).to_str().unwrap());
    assert_eq!(
        resolved.repo.last_analyzed_commit.as_deref(),
        Some("abc123")
    );
    assert_eq!(f.storage.list_projects().unwrap().len(), 1);
}

#[test]
fn identity_override_never_drives_a_move() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let _env = EnvGuard::new(&base.join("db.sqlite"));
    let data = lievo_data_dir().unwrap();
    let f = fixture(&base, data, true);
    // The identity comes only from a repo-controlled override, not an origin.
    fs::create_dir_all(f.new_root.join(".lievo")).unwrap();
    fs::write(
        f.new_root.join(".lievo").join("config.yaml"),
        format!("identity: {GIT_URL}\n"),
    )
    .unwrap();

    let resolved = register(
        &f.storage,
        &RegistrationRequest {
            repo_root: &f.new_root,
            project_name: None,
        },
    )
    .unwrap();

    assert_ne!(
        resolved.project.id, f.project_id,
        "an override claiming a vanished identity registers fresh"
    );
    let old = f.storage.get_repo(&f.repo.id).unwrap().unwrap();
    assert_eq!(old.local_path, f.old_path, "the vanished row is untouched");
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists()
    );
}

#[test]
fn unusable_data_dir_falls_back_without_error_and_keeps_row() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    // The data dir path is occupied by a regular file, so it cannot be created.
    let bad_data = base.join("not-a-dir");
    fs::write(&bad_data, b"x").unwrap();

    let reason = skipped(relocate_in(&f.storage, &f.repo, &f.new_root, &bad_data).unwrap());

    assert!(
        matches!(reason, SkipReason::DataDir(_)),
        "a filesystem error falls back, not Err"
    );
    let row = f.storage.get_repo(&f.repo.id).unwrap().unwrap();
    assert_eq!(row.local_path, f.old_path);
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists()
    );
}

#[test]
fn register_falls_back_to_fresh_when_move_cannot_complete() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let _env = EnvGuard::new(&base.join("db.sqlite"));
    let data = lievo_data_dir().unwrap();
    let f = fixture(&base, data, true);
    let repo = git2::Repository::init(&f.new_root).unwrap();
    repo.config()
        .unwrap()
        .set_str("remote.origin.url", "git@github.com:acme/tool")
        .unwrap();
    // Block the status rename so the move fails after the index dir moved.
    let new_hash = repo_hash(&new_canon(&f)).unwrap();
    let blocker = status_path_for_hash(&f.data, &new_hash);
    fs::create_dir_all(&blocker).unwrap();
    fs::write(blocker.join("keep"), b"x").unwrap();

    let resolved = register(
        &f.storage,
        &RegistrationRequest {
            repo_root: &f.new_root,
            project_name: None,
        },
    )
    .unwrap();

    assert_ne!(
        resolved.project.id, f.project_id,
        "a failed move registers fresh"
    );
    assert_ne!(resolved.repo.id, f.repo.id);
    let old = f.storage.get_repo(&f.repo.id).unwrap().unwrap();
    assert_eq!(old.local_path, f.old_path, "the vanished row is untouched");
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists(),
        "the completed index rename is rolled back"
    );
}

#[test]
fn register_registers_fresh_when_vanished_repo_has_null_git_url() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let _env = EnvGuard::new(&base.join("db.sqlite"));
    let data = lievo_data_dir().unwrap();
    let f = fixture(&base, data, false);
    let repo = git2::Repository::init(&f.new_root).unwrap();
    repo.config()
        .unwrap()
        .set_str("remote.origin.url", "git@github.com:acme/tool")
        .unwrap();

    let resolved = register(
        &f.storage,
        &RegistrationRequest {
            repo_root: &f.new_root,
            project_name: None,
        },
    )
    .unwrap();

    assert_ne!(
        resolved.project.id, f.project_id,
        "no move: a fresh project"
    );
    let old = f.storage.get_repo(&f.repo.id).unwrap().unwrap();
    assert_eq!(old.local_path, f.old_path, "the vanished row is untouched");
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists()
    );
}
