//! Re-key a moved or re-cloned checkout to its new path (issue #30).
//!
//! When a repo with a stored git identity has vanished and the same identity
//! appears at a new path, the existing repo row is reused: the index
//! directory and status file move from the old path's hash to the new one's,
//! and the row is updated afterwards. Nothing is reindexed.
//!
//! Ordering: take the old lock, re-read the row under it, check the project,
//! rename the filesystem state, and only then update the row. A failure at any
//! step rolls back completed renames, leaves the row untouched and returns a
//! [`SkipReason`], so the caller registers fresh. This module prints nothing;
//! the caller decides how to report a skip.
//!
//! A rollback that itself fails leaves some files at the new hash while the
//! row still names the old path. The next relocation of the same checkout
//! finishes the job: a source that is already gone is skipped, and the row's
//! index path is rebased onto the new directory.
//!
//! The old lock file is never removed. Unlinking it after release lets a
//! second process lock an orphaned inode while a third creates a new file at
//! the same path, so two writers could run at once. A leftover empty lock
//! file is harmless.

use std::fmt;
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

use super::lock::try_acquire_hash;
use crate::layout::{CanonicalPath, hash_canonical_path, index_dir_for_hash, status_path_for_hash};
use crate::model::{Project, Repository};
use crate::storage::Storage;

/// Why relocation was not done. Unless stated otherwise, the row and files are
/// left as they were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The lievo data dir is unknown or could not be created.
    DataDir(String),
    /// A stored or new path cannot be hashed: it is not absolute and
    /// normalised, is not UTF-8, or does not resolve. The message says which.
    Unusable(String),
    /// The old lock could not be opened or taken, for a reason other than contention.
    LockFailed(io::ErrorKind, String),
    /// A live process holds the old index lock.
    IndexInUse,
    /// The row changed while relocation was starting.
    RowChanged,
    /// The row's project is missing, so a moved row would have no owner.
    ProjectMissing,
    /// The stored index path is not under the old index directory.
    IndexOutsideIndexDir,
    /// A rename or the row update failed. `unrestored` lists renames that
    /// could not be undone, so those files are left at the new location.
    Move {
        cause: String,
        unrestored: Vec<String>,
    },
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataDir(e) => write!(f, "the lievo data directory is unavailable: {e}"),
            Self::Unusable(e) => f.write_str(e),
            Self::LockFailed(_, e) => write!(f, "its lock could not be taken: {e}"),
            Self::IndexInUse => f.write_str("its index is in use by another process"),
            Self::RowChanged => f.write_str("its row changed while relocation was starting"),
            Self::ProjectMissing => f.write_str("its project is missing"),
            Self::IndexOutsideIndexDir => {
                f.write_str("its stored index path is outside its index directory")
            }
            Self::Move { cause, unrestored } if unrestored.is_empty() => {
                write!(f, "the move failed: {cause}")
            }
            Self::Move { cause, unrestored } => write!(
                f,
                "the move failed: {cause}; could not restore, files left at the new location: {}",
                unrestored.join(", ")
            ),
        }
    }
}

/// The result of [`relocate_checkout`].
#[derive(Debug)]
pub enum RelocateOutcome {
    /// The row now points at the new path.
    Moved {
        /// The row as written. Its `updated_at` is the value read before the
        /// update, so reload the row if the stored timestamp matters.
        repo: Box<Repository>,
        /// The row's project, checked before the commit point.
        project: Box<Project>,
    },
    /// Relocation was not done; the caller registers fresh.
    Skipped(SkipReason),
}

/// A completed rename, kept so it can be undone if a later step fails.
struct Rename {
    from: PathBuf,
    to: PathBuf,
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

/// Whether `path` exists (without following a symlink).
fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Rename `from` to `to` when `from` exists. A missing source is not an error:
/// a repo that was never indexed has no index directory or status file. An
/// existing destination is an error, never overwritten: it may be stale state
/// that must not be destroyed silently.
fn move_if_present(done: &mut Vec<Rename>, from: PathBuf, to: PathBuf) -> io::Result<()> {
    if !exists(&from)? {
        return Ok(());
    }
    if exists(&to)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("destination '{}' already exists", to.display()),
        ));
    }
    fs::rename(&from, &to)?;
    done.push(Rename { from, to });
    Ok(())
}

/// Undo completed renames, newest first. Returns the undo failures; those
/// files stay at the new location.
fn roll_back(done: &[Rename]) -> Vec<String> {
    done.iter()
        .rev()
        .filter_map(|r| {
            fs::rename(&r.to, &r.from)
                .err()
                .map(|e| format!("'{}' ({e})", r.to.display()))
        })
        .collect()
}

/// Move the index directory and the status file from the old hash to the new.
fn move_state(data_dir: &Path, plan: &Plan, done: &mut Vec<Rename>) -> io::Result<()> {
    move_if_present(done, plan.old_dir.clone(), plan.new_dir.clone())?;
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

/// Take the old path's lock. A busy lock means a live indexer is writing
/// there; any other failure is reported as itself.
fn lock_old(data_dir: &Path, old_hash: &str) -> Result<File, SkipReason> {
    match try_acquire_hash(data_dir, old_hash) {
        Ok(Some(file)) => Ok(file),
        Ok(None) => Err(SkipReason::IndexInUse),
        Err(e) => Err(SkipReason::LockFailed(e.kind(), e.to_string())),
    }
}

/// Re-key `repo` (a vanished checkout with a stored identity) to `new_root`,
/// using the lievo data dir.
///
/// Only a storage read failure is an error; every other failure is a
/// [`RelocateOutcome::Skipped`].
pub fn relocate_checkout(
    storage: &dyn Storage,
    repo: &Repository,
    new_root: &Path,
) -> crate::Result<RelocateOutcome> {
    match crate::layout::lievo_data_dir() {
        Ok(data_dir) => relocate_in(storage, repo, new_root, &data_dir),
        Err(e) => Ok(RelocateOutcome::Skipped(SkipReason::DataDir(e.to_string()))),
    }
}

/// [`relocate_checkout`] with an explicit data dir.
fn relocate_in(
    storage: &dyn Storage,
    repo: &Repository,
    new_root: &Path,
    data_dir: &Path,
) -> crate::Result<RelocateOutcome> {
    let plan = match Plan::new(repo, new_root, data_dir) {
        Ok(plan) => plan,
        Err(reason) => return Ok(RelocateOutcome::Skipped(reason)),
    };
    if let Err(e) = fs::create_dir_all(data_dir) {
        return Ok(RelocateOutcome::Skipped(SkipReason::DataDir(e.to_string())));
    }
    let lock = match lock_old(data_dir, &plan.old_hash) {
        Ok(lock) => lock,
        Err(reason) => return Ok(RelocateOutcome::Skipped(reason)),
    };

    // Re-read the row under the lock. A concurrent registration may have moved
    // it after our snapshot was taken, in which case `repo` is stale.
    let current = match storage.get_repo(&repo.id)? {
        Some(row) if row.local_path == repo.local_path => row,
        _ => return Ok(RelocateOutcome::Skipped(SkipReason::RowChanged)),
    };
    // Check the project before the commit point: once the row is updated, a
    // missing project could only be reported as a failure of a finished move.
    let project = match storage.get_project_by_id(&current.project_id)? {
        Some(project) => project,
        None => return Ok(RelocateOutcome::Skipped(SkipReason::ProjectMissing)),
    };
    let index_path = match rebase_index_path(current.index_path.as_deref(), &plan) {
        Ok(path) => path,
        Err(reason) => return Ok(RelocateOutcome::Skipped(reason)),
    };

    // Move the filesystem state, recording each rename for rollback. The row
    // is updated only after the files are in place.
    let mut done = Vec::new();
    let result = move_state(data_dir, &plan, &mut done)
        .map_err(|e| e.to_string())
        .and_then(|()| {
            storage
                .update_repo_local_path(&repo.id, &plan.new_path, index_path.as_deref())
                .map_err(|e| e.to_string())
        });

    // Release the lock. It stays on disk (see module docs).
    drop(lock);
    match result {
        Ok(()) => Ok(RelocateOutcome::Moved {
            repo: Box::new(Repository {
                local_path: plan.new_path,
                index_path,
                ..current
            }),
            project: Box::new(project),
        }),
        Err(cause) => Ok(RelocateOutcome::Skipped(SkipReason::Move {
            cause,
            unrestored: roll_back(&done),
        })),
    }
}

#[cfg(test)]
#[path = "relocate_tests.rs"]
mod tests;
