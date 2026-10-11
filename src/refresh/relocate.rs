//! Re-key a moved or re-cloned checkout to its new path (issue #30).
//!
//! When a repo with a stored git identity has vanished and the same identity
//! appears at a new path, the existing repo row is reused: the index
//! directory and status file move from the old path's hash to the new one's,
//! and the row is updated afterwards. Nothing is reindexed.
//!
//! Ordering: check the identity's provenance, take the old and new locks (in
//! hash order), re-read the row under them, check the project, rename the
//! filesystem state, and only then update the row. A failed row update is
//! judged by re-reading the row: if it already names the new path the move is
//! committed; rollback happens only when it still names the old path. Other
//! failures roll back completed renames, leave the row untouched and return a
//! [`SkipReason`], so the caller registers fresh. Rollback runs while the locks
//! are still held, so the files are in their final state before anything else
//! can touch them. If the row cannot be confirmed as either path, the renames
//! are kept and [`SkipReason::RowStateUnknown`] is returned: the caller must
//! not register fresh. This module prints nothing; the caller decides how to
//! report a skip.
//!
//! A rollback that itself fails leaves some files at the new hash while the
//! row still names the old path. Nothing finishes this automatically, and
//! `register()` refuses to register fresh at the new path (it returns
//! `LievoError::RepoStateUnknown`), so nothing indexes over the split files.
//! That refusal is sticky: a later `register()` still sees the leftover state
//! at the new hash (see [`leftover_index_state`]) and keeps refusing. The
//! leftover check runs after every skip except `NotOriginIdentity`, which covers
//! `identity:` overrides and returns before anything is touched. Transient
//! skips, such as a busy or failed lock, a storage read error or a data-dir
//! error, are checked too, so they cannot register fresh over split state left
//! by an earlier run. Recovery is manual: the error says which files to move
//! back.
//!
//! The lock files are never removed. Unlinking one after release lets a
//! second process lock an orphaned inode while a third creates a new file at
//! the same path, so two writers could run at once. A leftover empty lock
//! file is harmless.
//!
//! Known limitation (provenance): the origin check looks only at the identity
//! a row's `git_url` was stored under. The `git_url` is not tagged with where
//! its key came from. A key derived from a repo-controlled `identity:`
//! override can therefore be stored on a row by path-match backfill, and a
//! later checkout whose origin remote matches that key could then move the
//! row. Recording provenance needs a schema change, which is out of scope. See
//! `docs/data-storage.md` (Known limitations).

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use super::lock::try_acquire_hash;
use crate::identity::{Identity, IdentitySource};

#[path = "relocate/rename.rs"]
mod rename;
use crate::layout::{CanonicalPath, hash_canonical_path, index_dir_for_hash, status_path_for_hash};
use crate::model::{Project, Repository};
use crate::storage::Storage;
#[cfg(test)]
pub(crate) use rename::BETWEEN_RENAMES;
pub use rename::{FailedUndo, Rename};
use rename::{between_renames, exists, move_if_present, roll_back};

/// Why relocation was not done. Unless stated otherwise, the row and files are
/// left as they were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The lievo data dir is unknown or could not be created.
    DataDir(String),
    /// A stored or new path cannot be hashed: it is not absolute and
    /// normalised, is not UTF-8, or does not resolve. The message says which.
    Unusable(String),
    /// The old or new lock could not be opened or taken, for a reason other
    /// than contention.
    LockFailed(io::ErrorKind, String),
    /// A live process holds the old or the new index lock.
    IndexInUse,
    /// The identity is not from the origin remote, or it does not match the
    /// row's stored `git_url` (case-insensitively). Only an origin identity
    /// may move a row. Nothing is touched.
    NotOriginIdentity,
    /// The row changed while relocation was starting.
    RowChanged,
    /// The row's project is missing, so a moved row would have no owner.
    ProjectMissing,
    /// The stored index path is not under the old index directory.
    IndexOutsideIndexDir,
    /// The repo row or its project could not be read. Nothing was changed.
    StorageRead(String),
    /// A rename or the row update failed. `split` lists the renames that could
    /// not be undone, each with its error: those files stay at `to`.
    Move {
        cause: String,
        split: Vec<FailedUndo>,
    },
    /// The row update failed and the row names neither path (or could not be
    /// read), so its state is unknown. `renamed` lists the files left at
    /// their new location; they are not rolled back. The caller must not
    /// register fresh at the old or the new path.
    RowStateUnknown { renamed: Vec<Rename> },
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataDir(e) => write!(f, "the lievo data directory is unavailable: {e}"),
            Self::Unusable(e) => f.write_str(e),
            Self::LockFailed(_, e) => write!(f, "its lock could not be taken: {e}"),
            Self::IndexInUse => f.write_str("its index is in use by another process"),
            Self::NotOriginIdentity => {
                f.write_str("its identity is not the origin remote's identity")
            }
            Self::RowChanged => f.write_str("its row changed while relocation was starting"),
            Self::ProjectMissing => f.write_str("its project is missing"),
            Self::IndexOutsideIndexDir => {
                f.write_str("its stored index path is outside its index directory")
            }
            Self::StorageRead(e) => write!(f, "its row could not be read: {e}"),
            Self::Move { cause, .. } => write!(f, "the move failed: {cause}"),
            Self::RowStateUnknown { .. } => f.write_str("the repo row state is unknown"),
        }
    }
}

/// The result of [`relocate_checkout`].
#[derive(Debug)]
pub enum RelocateOutcome {
    /// The row now points at the new path. The move is committed, so this is
    /// a success even when `notice` is set.
    Moved {
        /// The row as stored after the move. If reading it back failed, this
        /// is the row as written, and `notice` says so.
        repo: Box<Repository>,
        /// The row's project, checked before the commit point.
        project: Box<Project>,
        /// A secondary problem after the commit, for the caller to report.
        notice: Option<String>,
    },
    /// Relocation was not done. See [`SkipReason`] for what the caller must do.
    Skipped(SkipReason),
}

/// The hashes and paths a relocation derives from the old and new checkouts.
struct Plan {
    new_path: String,
    old_hash: String,
    new_hash: String,
    old_dir: PathBuf,
    new_dir: PathBuf,
}

impl Plan {
    fn new(repo: &Repository, new_root: &Path, data_dir: &Path) -> Result<Plan, SkipReason> {
        let old = CanonicalPath::new(&repo.local_path).ok_or_else(|| {
            SkipReason::Unusable("its stored path is not an absolute, normalised path".to_string())
        })?;
        let canon = new_root.canonicalize().map_err(|e| {
            SkipReason::Unusable(format!("its new path could not be resolved: {e}"))
        })?;
        let new = canon.to_str().and_then(CanonicalPath::new).ok_or_else(|| {
            SkipReason::Unusable("its new path is not UTF-8 and normalised".to_string())
        })?;
        let old_hash = hash_canonical_path(&old);
        let new_hash = hash_canonical_path(&new);
        Ok(Plan {
            old_dir: index_dir_for_hash(data_dir, &old_hash),
            new_dir: index_dir_for_hash(data_dir, &new_hash),
            new_path: new.as_str().to_string(),
            old_hash,
            new_hash,
        })
    }
}

/// Index state already present at the new checkout's hash: its index
/// directory and its status file, whichever exist. A failed rollback leaves
/// such state behind, and a fresh registration at `new_root` would share it.
/// Fails closed: an error (no data dir, an unhashable path, an unreadable
/// path) is returned, never read as "no state". The caller must refuse
/// registration on `Err` as well as on a non-empty list.
pub fn leftover_index_state(repo: &Repository, new_root: &Path) -> io::Result<Vec<PathBuf>> {
    let data_dir = crate::layout::lievo_data_dir().map_err(|e| io::Error::other(e.to_string()))?;
    let plan = Plan::new(repo, new_root, &data_dir)
        .map_err(|reason| io::Error::other(reason.to_string()))?;
    let mut present = Vec::new();
    for path in [
        plan.new_dir,
        status_path_for_hash(&data_dir, &plan.new_hash),
    ] {
        if exists(&path)? {
            present.push(path);
        }
    }
    Ok(present)
}

/// Move the index directory and the status file from the old hash to the new.
fn move_state(data_dir: &Path, plan: &Plan, done: &mut Vec<Rename>) -> io::Result<()> {
    move_if_present(done, plan.old_dir.clone(), plan.new_dir.clone())?;
    between_renames();
    move_if_present(
        done,
        status_path_for_hash(data_dir, &plan.old_hash),
        status_path_for_hash(data_dir, &plan.new_hash),
    )
}

/// The index path after the move: the same path relative to the new index
/// directory. A repo with no index keeps NULL. A path outside the old index
/// directory, or one whose relative part is not plain (`..`, root), has no
/// safe home, so the move is skipped rather than writing NULL over it.
fn rebase_index_path(index_path: Option<&str>, plan: &Plan) -> Result<Option<String>, SkipReason> {
    let Some(stored) = index_path else {
        return Ok(None);
    };
    let rel = Path::new(stored)
        .strip_prefix(&plan.old_dir)
        .map_err(|_| SkipReason::IndexOutsideIndexDir)?;
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(SkipReason::IndexOutsideIndexDir);
    }
    plan.new_dir
        .join(rel)
        .to_str()
        .map(|s| Some(s.to_string()))
        .ok_or_else(|| SkipReason::Unusable("its moved index path is not UTF-8".to_string()))
}

/// Take one hash's lock. A busy lock means a live indexer is writing there;
/// any other failure is reported as itself.
fn lock_hash(data_dir: &Path, hash: &str) -> Result<File, SkipReason> {
    match try_acquire_hash(data_dir, hash) {
        Ok(Some(file)) => Ok(file),
        Ok(None) => Err(SkipReason::IndexInUse),
        Err(e) => Err(SkipReason::LockFailed(e.kind(), e.to_string())),
    }
}

/// Take the old and the new hash's locks before any rename, so a live indexer
/// on either path blocks the move. Taken in lexical hash order, so two
/// relocations over the same pair of paths cannot deadlock. The guards are held
/// until dropped; a failure drops any guard already taken.
fn lock_pair(data_dir: &Path, plan: &Plan) -> Result<Vec<File>, SkipReason> {
    let mut hashes = vec![plan.old_hash.as_str(), plan.new_hash.as_str()];
    hashes.sort_unstable();
    hashes.dedup();
    hashes
        .into_iter()
        .map(|hash| lock_hash(data_dir, hash))
        .collect()
}

/// What the row says after a failed update, judged by a re-read of the row.
enum RowAfterError {
    /// The row names the new path: the update committed despite the error.
    Committed(Box<Repository>),
    /// The row still names the old path: the update did not land.
    OldPath,
    /// The row names neither path, or could not be read: it cannot be trusted
    /// either way, so the renames must not be undone.
    Unknown,
}

/// Judge a failed row update from a re-read of the row. Only a row that names
/// the old path permits a rollback.
fn row_after_error(
    reread: crate::Result<Option<Repository>>,
    old_path: &str,
    new_path: &str,
) -> RowAfterError {
    match reread {
        Ok(Some(row)) if row.local_path == new_path => RowAfterError::Committed(Box::new(row)),
        Ok(Some(row)) if row.local_path == old_path => RowAfterError::OldPath,
        _ => RowAfterError::Unknown,
    }
}

/// The outcome of a failed row update, given what the row now says. `done`
/// are the renames made before the update; they are undone only for
/// [`RowAfterError::OldPath`] and kept otherwise.
fn failed_update_outcome(
    cause: String,
    state: RowAfterError,
    done: &[Rename],
    project: Project,
) -> RelocateOutcome {
    match state {
        RowAfterError::Committed(repo) => RelocateOutcome::Moved {
            repo,
            project: Box::new(project),
            notice: Some(format!(
                "the row update reported an error ({cause}), but the row already names the new path"
            )),
        },
        RowAfterError::OldPath => RelocateOutcome::Skipped(SkipReason::Move {
            cause,
            split: roll_back(done),
        }),
        RowAfterError::Unknown => RelocateOutcome::Skipped(SkipReason::RowStateUnknown {
            renamed: done.to_vec(),
        }),
    }
}

/// Whether `identity` may move `repo`: it must come from the origin remote
/// (a repo-controlled `identity:` override never moves a row) and name the
/// same identity the row was stored under. A row with no stored `git_url`
/// never moves.
fn may_move(repo: &Repository, identity: &Identity) -> bool {
    identity.source() == IdentitySource::Origin
        && repo
            .git_url
            .as_deref()
            .is_some_and(|url| url.eq_ignore_ascii_case(identity.key()))
}

/// Re-key `repo` (a vanished checkout) to `new_root`, using the lievo data dir.
///
/// Never returns an error. Every failure is a [`RelocateOutcome::Skipped`],
/// including a failed storage read ([`SkipReason::StorageRead`]). A move is
/// made only if `identity` passes the provenance check (see [`may_move`]);
/// otherwise the result is `Skipped(NotOriginIdentity)` with nothing touched.
/// Only [`SkipReason::RowStateUnknown`] needs the caller to act beyond
/// reporting: it must not register fresh.
pub fn relocate_checkout(
    storage: &dyn Storage,
    repo: &Repository,
    new_root: &Path,
    identity: &Identity,
) -> RelocateOutcome {
    match crate::layout::lievo_data_dir() {
        Ok(data_dir) => relocate_in(storage, repo, new_root, identity, &data_dir),
        Err(e) => RelocateOutcome::Skipped(SkipReason::DataDir(e.to_string())),
    }
}

/// The row and its project as read under the locks.
struct Locked {
    row: Repository,
    project: Project,
    /// Another relocation already moved the row to the new path.
    moved_by_other: bool,
}

/// Stage 1, under the locks: re-read the row and its project. A concurrent
/// relocation may have moved the row after the caller's snapshot was taken:
/// then the move is already done and there is nothing left to rename.
fn read_under_locks(
    storage: &dyn Storage,
    repo: &Repository,
    new_path: &str,
) -> Result<Locked, SkipReason> {
    let row = match storage.get_repo(&repo.id) {
        Ok(Some(row)) => row,
        Ok(None) => return Err(SkipReason::RowChanged),
        Err(e) => return Err(SkipReason::StorageRead(e.to_string())),
    };
    let moved_by_other = row.local_path == new_path;
    if !moved_by_other && row.local_path != repo.local_path {
        return Err(SkipReason::RowChanged);
    }
    // Check the project before the commit point: once the row is updated, a
    // missing project could only be reported as a failure of a finished move.
    let project = match storage.get_project_by_id(&row.project_id) {
        Ok(Some(project)) => project,
        Ok(None) => return Err(SkipReason::ProjectMissing),
        Err(e) => return Err(SkipReason::StorageRead(e.to_string())),
    };
    Ok(Locked {
        row,
        project,
        moved_by_other,
    })
}

/// Stage 2, under the locks: rename the filesystem state, update the row, and
/// map the outcome. Any rollback happens here, before the caller releases the
/// locks, so the files are in their final state when the lock is released.
fn commit_move(
    storage: &dyn Storage,
    repo: &Repository,
    plan: &Plan,
    data_dir: &Path,
    locked: Locked,
) -> RelocateOutcome {
    let Locked {
        row,
        project,
        moved_by_other,
    } = locked;
    if moved_by_other {
        return RelocateOutcome::Moved {
            repo: Box::new(row),
            project: Box::new(project),
            notice: None,
        };
    }
    let index_path = match rebase_index_path(row.index_path.as_deref(), plan) {
        Ok(path) => path,
        Err(reason) => return RelocateOutcome::Skipped(reason),
    };

    // Move the filesystem state, recording each rename for rollback. The row
    // is updated only after the files are in place.
    let mut done = Vec::new();
    let committed = match move_state(data_dir, plan, &mut done) {
        // Nothing reached the row, so it still names the old path.
        Err(e) => Err((e.to_string(), RowAfterError::OldPath)),
        Ok(()) => storage
            .update_repo_local_path(&repo.id, &plan.new_path, index_path.as_deref())
            .map_err(|e| {
                let state =
                    row_after_error(storage.get_repo(&repo.id), &repo.local_path, &plan.new_path);
                (e.to_string(), state)
            }),
    };
    match committed {
        Ok(()) => {
            let fallback = Repository {
                local_path: plan.new_path.clone(),
                index_path,
                ..row
            };
            let (row, notice) = after_commit(|| storage.get_repo(&repo.id), fallback);
            RelocateOutcome::Moved {
                repo: Box::new(row),
                project: Box::new(project),
                notice,
            }
        }
        Err((cause, state)) => failed_update_outcome(cause, state, &done, project),
    }
}

/// [`relocate_checkout`] with an explicit data dir. The provenance check runs
/// first, so a non-origin identity touches nothing, not even the data dir.
fn relocate_in(
    storage: &dyn Storage,
    repo: &Repository,
    new_root: &Path,
    identity: &Identity,
    data_dir: &Path,
) -> RelocateOutcome {
    if !may_move(repo, identity) {
        return RelocateOutcome::Skipped(SkipReason::NotOriginIdentity);
    }
    let plan = match Plan::new(repo, new_root, data_dir) {
        Ok(plan) => plan,
        Err(reason) => return RelocateOutcome::Skipped(reason),
    };
    if let Err(e) = fs::create_dir_all(data_dir) {
        return RelocateOutcome::Skipped(SkipReason::DataDir(e.to_string()));
    }
    let locks = match lock_pair(data_dir, &plan) {
        Ok(locks) => locks,
        Err(reason) => return RelocateOutcome::Skipped(reason),
    };

    // The locks cover the re-read, the renames, the row update and any
    // rollback. They are released only once the outcome is final.
    let outcome = match read_under_locks(storage, repo, &plan.new_path) {
        Ok(locked) => commit_move(storage, repo, &plan, data_dir, locked),
        Err(reason) => RelocateOutcome::Skipped(reason),
    };
    drop(locks);
    outcome
}

/// Resolve the row after the commit point. The move is already durable, so a
/// failed read-back cannot undo it: the row built in memory is returned and the
/// failure is reported as a notice.
fn after_commit(
    reload: impl FnOnce() -> crate::Result<Option<Repository>>,
    fallback: Repository,
) -> (Repository, Option<String>) {
    match reload() {
        Ok(Some(row)) => (row, None),
        Ok(None) => (
            fallback,
            Some("the repo row was not found after the move".to_string()),
        ),
        Err(e) => (fallback, Some(format!("reading the repo row failed: {e}"))),
    }
}

#[cfg(test)]
#[path = "relocate_tests.rs"]
mod tests;
