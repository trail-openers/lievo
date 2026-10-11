//! Re-key a moved or re-cloned checkout to its new path (issue #30).
//!
//! When a repo with a stored git identity has vanished and the same identity
//! appears at a new path, the existing repo row is reused: the index
//! directory and status file move from the old path's hash to the new one's,
//! and the row is updated afterwards. Nothing is reindexed.
//!
//! Ordering: take the old lock, re-read the row under it, rename the
//! filesystem state, and only then update the row. A failure at any step rolls
//! back completed renames, leaves the row untouched and returns `None`, so the
//! caller registers fresh.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use super::lock::{lock_path_for_hash, status_path_for_hash};
use crate::LievoError;
use crate::extraction::hash_canonical_path;
use crate::model::Repository;
use crate::registration::sanitize;
use crate::storage::Storage;

/// A completed rename, kept so it can be undone if a later step fails.
struct Rename {
    from: PathBuf,
    to: PathBuf,
}

/// Rename `from` to `to` when `from` exists. A missing source is not an error:
/// a repo that was never indexed has no index directory or status file.
fn move_if_present(done: &mut Vec<Rename>, from: PathBuf, to: PathBuf) -> io::Result<()> {
    match fs::rename(&from, &to) {
        Ok(()) => {
            done.push(Rename { from, to });
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Undo completed renames, newest first. A failed undo is reported on stderr
/// rather than hidden, since the files are then left at the new location.
fn roll_back(done: &[Rename]) {
    for r in done.iter().rev() {
        if let Err(e) = fs::rename(&r.to, &r.from) {
            eprintln!(
                "could not restore '{}' to '{}': {}",
                sanitize(&r.to.display().to_string()),
                sanitize(&r.from.display().to_string()),
                sanitize(&e.to_string()),
            );
        }
    }
}

/// Move the index directory and the status file from `old_hash` to `new_hash`.
fn move_state(
    data_dir: &Path,
    old_hash: &str,
    new_hash: &str,
    done: &mut Vec<Rename>,
) -> io::Result<()> {
    let indices = data_dir.join("indices");
    move_if_present(done, indices.join(old_hash), indices.join(new_hash))?;
    move_if_present(
        done,
        status_path_for_hash(data_dir, old_hash),
        status_path_for_hash(data_dir, new_hash),
    )
}

/// Map an index path under the old index directory to the same relative path
/// under the new one. A path outside it has no known home, so it yields `None`
/// and the next refresh rebuilds the index.
fn rebase_index_path(index_path: &str, old_dir: &Path, new_dir: &Path) -> Option<String> {
    let rel = Path::new(index_path).strip_prefix(old_dir).ok()?;
    new_dir.join(rel).to_str().map(str::to_string)
}

/// Notice printed when relocation is abandoned. The caller then registers the
/// new path as a fresh project.
pub(crate) fn fall_back(
    old_path: &str,
    new_path: &str,
    reason: &str,
) -> crate::Result<Option<Repository>> {
    eprintln!(
        "checkout '{}' appears to have moved to '{}', but it could not be re-keyed ({}); registering '{}' as a new project",
        sanitize(old_path),
        sanitize(new_path),
        sanitize(reason),
        sanitize(new_path),
    );
    Ok(None)
}

/// Re-key `repo` (a vanished checkout with a stored identity) to `new_root`.
///
/// Returns the updated row on success. Returns `Ok(None)` when relocation is
/// not possible (old index in use, a filesystem or row update failed, or the
/// row changed under us), after a notice has been printed. Only a storage
/// read failure is an error. `data_dir` is the lievo data dir (`~/.lievo`).
pub fn relocate_checkout(
    storage: &dyn Storage,
    repo: &Repository,
    new_root: &Path,
    data_dir: &Path,
) -> crate::Result<Option<Repository>> {
    let new_canon = match new_root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return fall_back(
                &repo.local_path,
                &new_root.display().to_string(),
                &format!("its new path could not be resolved: {e}"),
            );
        }
    };
    let Some(new_path) = new_canon.to_str() else {
        return fall_back(
            &repo.local_path,
            &new_canon.display().to_string(),
            "its new path is not valid UTF-8",
        );
    };
    let old_hash = hash_canonical_path(&repo.local_path);
    let new_hash = hash_canonical_path(new_path);

    // Step 1: the old path's lock must be free. A busy lock means a live
    // indexer is writing there, so the move is abandoned.
    if let Err(e) = fs::create_dir_all(data_dir) {
        return fall_back(
            &repo.local_path,
            new_path,
            &format!("the lievo data directory could not be created: {e}"),
        );
    }
    let lock_path = lock_path_for_hash(data_dir, &old_hash);
    let lock = match File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) => {
            return fall_back(
                &repo.local_path,
                new_path,
                &format!("its lock could not be opened: {e}"),
            );
        }
    };
    if lock.try_lock().is_err() {
        return fall_back(
            &repo.local_path,
            new_path,
            "its index is in use by another process",
        );
    }

    // Re-read the row under the lock. A concurrent registration may have moved
    // it after our snapshot was taken, in which case `repo` is stale.
    let current = match storage.get_repo(&repo.id)? {
        Some(r) if r.local_path == repo.local_path => r,
        _ => {
            return fall_back(
                &repo.local_path,
                new_path,
                "its row changed while relocation was starting",
            );
        }
    };

    // Step 2: move the filesystem state, recording each rename for rollback.
    let old_dir = data_dir.join("indices").join(&old_hash);
    let new_dir = data_dir.join("indices").join(&new_hash);
    let mut done = Vec::new();
    let index_path = current
        .index_path
        .as_deref()
        .and_then(|p| rebase_index_path(p, &old_dir, &new_dir));

    // Step 3: only after the files are in place, update the row in one statement.
    let result = move_state(data_dir, &old_hash, &new_hash, &mut done)
        .map_err(LievoError::Io)
        .and_then(|()| storage.update_repo_local_path(&repo.id, new_path, index_path.as_deref()));

    // Step 4: release and remove the old lock. The new path's lock was never
    // created here, so it is immediately acquirable.
    drop(lock);
    match result {
        Ok(()) => {
            let _ = fs::remove_file(&lock_path);
            let moved = storage.get_repo(&repo.id)?.ok_or_else(|| {
                LievoError::InvalidInput(format!("repo {} missing after move", repo.id))
            })?;
            Ok(Some(moved))
        }
        Err(e) => {
            roll_back(&done);
            fall_back(&repo.local_path, new_path, &e.to_string())
        }
    }
}

#[cfg(test)]
#[path = "relocate_tests.rs"]
mod tests;
