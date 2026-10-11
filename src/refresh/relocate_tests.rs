//! Tests for moved-checkout relocation (issue #30).

use super::*;
use crate::extraction::lievo_data_dir;
use crate::layout::{lock_path_for_hash, repo_hash, status_path_for_hash};
use crate::model::{Entity, EntityTier};
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

    let old_hash = hash_canonical_path(&CanonicalPath::new(&old_path).unwrap());
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

/// The identity a fixture's row was stored under: the origin remote.
fn origin() -> Identity {
    Identity {
        key: GIT_URL.to_string(),
        source: IdentitySource::Origin,
    }
}

/// The moved row, panicking with the skip reason otherwise.
fn moved(outcome: RelocateOutcome) -> Repository {
    match outcome {
        RelocateOutcome::Moved { repo, .. } => *repo,
        RelocateOutcome::Skipped(reason) => panic!("expected a move, skipped: {reason}"),
    }
}

/// The skip reason, panicking if the move happened.
fn skipped(outcome: RelocateOutcome) -> SkipReason {
    match outcome {
        RelocateOutcome::Skipped(reason) => reason,
        RelocateOutcome::Moved { .. } => panic!("expected a skip, got a move"),
    }
}

#[test]
fn successful_move_rekeys_row_and_index_without_reindex() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let entity = Entity {
        id: format!("{}:old-checkout:file:src/lib.rs", f.project_id),
        project_id: f.project_id.clone(),
        repo_id: Some(f.repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "lib.rs".to_string(),
        path: Some("src/lib.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    f.storage.upsert_entity(&entity).unwrap();

    let outcome = relocate_in(&f.storage, &f.repo, &f.new_root, &origin(), &f.data);
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
    // Entity ids are not rewritten: the same entity still belongs to the repo.
    let kept = f
        .storage
        .get_entity(&entity.id)
        .unwrap()
        .expect("entity kept");
    assert_eq!(kept.repo_id, entity.repo_id);
    assert_eq!(kept.path, entity.path);
    assert_eq!(f.storage.count_entities(&f.repo.id).unwrap(), 1);
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

    let reason = skipped(relocate_in(
        &f.storage,
        &f.repo,
        &f.new_root,
        &origin(),
        &f.data,
    ));

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
fn busy_new_lock_skips_move_with_no_renames() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let new_hash = repo_hash(&new_canon(&f)).unwrap();
    let holder = open_lock(&f.data, &new_hash);
    holder.try_lock().unwrap();

    let reason = skipped(relocate_in(
        &f.storage,
        &f.repo,
        &f.new_root,
        &origin(),
        &f.data,
    ));

    assert_eq!(
        reason,
        SkipReason::IndexInUse,
        "a live indexer on the destination blocks the move"
    );
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists(),
        "nothing is renamed"
    );
    assert!(!f.data.join("indices").join(&new_hash).exists());
    let row = f.storage.get_repo(&f.repo.id).unwrap().unwrap();
    assert_eq!(row.local_path, f.old_path);
}

#[test]
fn row_already_at_new_path_is_a_finished_move_not_a_conflict() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let new_path = new_canon(&f).to_str().unwrap().to_string();
    // Another process moved the row after our snapshot (`f.repo`) was taken.
    f.storage
        .update_repo_local_path(&f.repo.id, &new_path, None)
        .unwrap();

    match relocate_in(&f.storage, &f.repo, &f.new_root, &origin(), &f.data) {
        RelocateOutcome::Moved { repo, notice, .. } => {
            assert!(
                notice.is_none(),
                "no notice for a move someone else finished"
            );
            assert_eq!(repo.local_path, new_path);
        }
        RelocateOutcome::Skipped(reason) => panic!("expected a move, skipped: {reason}"),
    }
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists(),
        "no renames are made"
    );
}

#[test]
fn failed_row_update_is_judged_by_the_row_it_left_behind() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let (old, new) = (f.old_path.as_str(), "/new/checkout");
    let at_new = Repository {
        local_path: new.to_string(),
        ..f.repo.clone()
    };
    let elsewhere = Repository {
        local_path: "/elsewhere".to_string(),
        ..f.repo.clone()
    };
    let read_failed = crate::LievoError::InvalidInput("read failed".to_string());

    assert!(matches!(
        row_after_error(Ok(Some(at_new)), old, new),
        RowAfterError::Committed(row) if row.local_path == new
    ));
    assert!(matches!(
        row_after_error(Ok(Some(f.repo.clone())), old, new),
        RowAfterError::OldPath
    ));
    assert!(matches!(
        row_after_error(Ok(Some(elsewhere)), old, new),
        RowAfterError::Unknown
    ));
    assert!(matches!(
        row_after_error(Ok(None), old, new),
        RowAfterError::Unknown
    ));
    assert!(matches!(
        row_after_error(Err(read_failed), old, new),
        RowAfterError::Unknown
    ));
}

#[test]
fn unknown_row_state_keeps_renames_and_is_not_a_split_move() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let project = f.storage.get_project_by_id(&f.project_id).unwrap().unwrap();
    let done = vec![Rename {
        from: base.join("a"),
        to: base.join("a-new"),
    }];

    let outcome = failed_update_outcome("boom".to_string(), RowAfterError::Unknown, &done, project);

    match outcome {
        RelocateOutcome::Skipped(SkipReason::RowStateUnknown { renamed }) => {
            assert_eq!(renamed, done, "the renames are listed and not undone");
        }
        other => panic!("expected RowStateUnknown, got {other:?}"),
    }
}

#[test]
fn committed_or_rolled_back_update_outcomes_are_unchanged() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let project = || f.storage.get_project_by_id(&f.project_id).unwrap().unwrap();
    let new_path = "/new/checkout".to_string();
    let at_new = Repository {
        local_path: new_path.clone(),
        ..f.repo.clone()
    };

    match failed_update_outcome(
        "boom".to_string(),
        RowAfterError::Committed(Box::new(at_new)),
        &[],
        project(),
    ) {
        RelocateOutcome::Moved { repo, notice, .. } => {
            assert_eq!(repo.local_path, new_path);
            assert!(notice.unwrap().contains("boom"));
        }
        other => panic!("expected Moved, got {other:?}"),
    }
    assert!(matches!(
        failed_update_outcome("boom".to_string(), RowAfterError::OldPath, &[], project()),
        RelocateOutcome::Skipped(SkipReason::Move { ref split, .. }) if split.is_empty()
    ));
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

    let reason = skipped(relocate_in(
        &f.storage,
        &f.repo,
        &f.new_root,
        &origin(),
        &f.data,
    ));

    assert!(matches!(reason, SkipReason::Move { ref split, .. } if split.is_empty()));
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
fn roll_back_reports_only_renames_it_could_not_undo() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let undone_from = base.join("a");
    let undone_to = base.join("a-new");
    let stuck_from = base.join("b");
    let stuck_to = base.join("b-new");
    fs::write(&undone_to, b"x").unwrap();
    // `stuck_to` never existed, so its undo fails and it must be reported.
    let done = vec![
        Rename {
            from: undone_from.clone(),
            to: undone_to.clone(),
        },
        Rename {
            from: stuck_from.clone(),
            to: stuck_to.clone(),
        },
    ];

    let split = roll_back(&done);

    assert_eq!(split.len(), 1);
    assert_eq!(split[0].from, stuck_from);
    assert_eq!(split[0].to, stuck_to);
    assert!(undone_from.exists(), "the undoable rename is restored");
    assert!(!undone_to.exists());
}

#[test]
fn split_rollback_leaves_files_at_new_dir_and_register_never_overwrites_them() {
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
    // The status rename fails (blocked), and the rollback of the index dir
    // rename cannot land because the old dir is re-created non-empty between
    // the two steps. The index files are left at the new hash.
    let new_hash = repo_hash(&new_canon(&f)).unwrap();
    let blocker = status_path_for_hash(&f.data, &new_hash);
    fs::create_dir_all(&blocker).unwrap();
    fs::write(blocker.join("keep"), b"x").unwrap();
    let old_dir = f.data.join("indices").join(&f.old_hash);
    let recreate = old_dir.clone();
    BETWEEN_RENAMES.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            fs::create_dir_all(&recreate).unwrap();
            fs::write(recreate.join("keep"), b"x").unwrap();
        }));
    });
    let new_dir = f.data.join("indices").join(&new_hash);

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
        "the split falls back to fresh"
    );
    assert!(
        new_dir.join(INDEX_FILE).exists(),
        "the split index file stays where it is, not overwritten"
    );
    assert!(
        old_dir.join("keep").exists(),
        "the re-created old dir is untouched"
    );
    let old = f.storage.get_repo(&f.repo.id).unwrap().unwrap();
    assert_eq!(old.local_path, f.old_path, "the vanished row is untouched");
    assert_eq!(old.index_path, f.repo.index_path);
}

#[test]
fn after_commit_read_failure_keeps_the_move_and_reports_a_notice() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);

    let (row, notice) = after_commit(
        || Err(crate::LievoError::InvalidInput("read failed".to_string())),
        f.repo.clone(),
    );

    assert_eq!(row.id, f.repo.id, "the in-memory row is returned");
    assert!(notice.unwrap().contains("read failed"));
    let (row, notice) = after_commit(|| Ok(Some(f.repo.clone())), f.repo.clone());
    assert_eq!(row.id, f.repo.id);
    assert!(notice.is_none());
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

    let reason = skipped(relocate_in(
        &f.storage,
        &repo,
        &f.new_root,
        &origin(),
        &f.data,
    ));

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
fn index_path_with_parent_component_under_old_dir_skips_without_moving() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    // Lexically under old_dir, but `..` would escape the index directory once joined.
    let escaping = f
        .data
        .join("indices")
        .join(&f.old_hash)
        .join("..")
        .join("..")
        .join("x");
    f.storage
        .update_repo_index_path(&f.repo.id, escaping.to_str().unwrap())
        .unwrap();
    let repo = f.storage.get_repo(&f.repo.id).unwrap().unwrap();

    let reason = skipped(relocate_in(
        &f.storage,
        &repo,
        &f.new_root,
        &origin(),
        &f.data,
    ));

    assert_eq!(reason, SkipReason::IndexOutsideIndexDir);
    assert!(
        f.data
            .join("indices")
            .join(&f.old_hash)
            .join(INDEX_FILE)
            .exists()
    );
}

#[test]
fn override_identity_is_refused_by_relocate_and_touches_nothing() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let by_override = Identity {
        key: GIT_URL.to_string(),
        source: IdentitySource::Override,
    };

    let reason = skipped(relocate_in(
        &f.storage,
        &f.repo,
        &f.new_root,
        &by_override,
        &f.data,
    ));

    assert_eq!(reason, SkipReason::NotOriginIdentity);
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
fn origin_identity_must_match_stored_git_url_case_insensitively() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    let other_repo = Identity {
        key: "github.com/other/tool".to_string(),
        source: IdentitySource::Origin,
    };
    assert_eq!(
        skipped(relocate_in(
            &f.storage,
            &f.repo,
            &f.new_root,
            &other_repo,
            &f.data,
        )),
        SkipReason::NotOriginIdentity
    );

    let shouted = Identity {
        key: "GitHub.com/Acme/Tool".to_string(),
        source: IdentitySource::Origin,
    };
    let moved = moved(relocate_in(
        &f.storage,
        &f.repo,
        &f.new_root,
        &shouted,
        &f.data,
    ));
    assert_eq!(moved.local_path, new_canon(&f).to_str().unwrap());
}

#[test]
fn row_without_stored_git_url_never_moves() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), false);

    let reason = skipped(relocate_in(
        &f.storage,
        &f.repo,
        &f.new_root,
        &origin(),
        &f.data,
    ));

    assert_eq!(reason, SkipReason::NotOriginIdentity);
}

#[test]
fn unusable_data_dir_falls_back_without_error_and_keeps_row() {
    let base = TempDir::new().unwrap();
    let base = base.path().canonicalize().unwrap();
    let f = fixture(&base, base.join("data"), true);
    // The data dir path is occupied by a regular file, so it cannot be created.
    let bad_data = base.join("not-a-dir");
    fs::write(&bad_data, b"x").unwrap();

    let reason = skipped(relocate_in(
        &f.storage,
        &f.repo,
        &f.new_root,
        &origin(),
        &bad_data,
    ));

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
