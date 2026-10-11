//! Lookups for registration: the cross-project path match, identity backfill
//! for path-matched rows, and checkout liveness (issue #29).

use std::path::Path;

use super::sanitize;
use crate::model::Repository;
use crate::storage::Storage;

/// Find a registered repo whose canonical `local_path` matches `path`,
/// across ALL projects (issue #863 decision 1). Compares both the raw and
/// the canonicalised form of the stored path against the raw and
/// canonicalised form of `path`, so a symlinked or otherwise
/// non-canonical lookup still matches.
pub fn find_repo_by_path(storage: &dyn Storage, path: &Path) -> crate::Result<Option<Repository>> {
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    for project in storage.list_projects()? {
        for repo in storage.list_repos(&project.id)? {
            let stored = Path::new(&repo.local_path);
            let canon = stored.canonicalize().ok();
            if stored == path
                || stored == target
                || canon.as_deref() == Some(path)
                || canon.as_deref() == Some(target.as_path())
            {
                return Ok(Some(repo));
            }
        }
    }
    Ok(None)
}

/// Backfill `git_url` on a path-matched row when it is NULL and a derivable
/// identity exists. A changed on-disk origin on a path-matched row does NOT
/// re-key: stored `git_url` governs identity, so a non-NULL stored value is
/// left untouched. The identity key is passed in (derived once in
/// `register`) rather than re-derived. The caller re-reads the row afterwards.
pub(super) fn backfill_git_url(
    storage: &dyn Storage,
    repo: &Repository,
    identity: Option<&str>,
) -> crate::Result<()> {
    if repo.git_url.is_some() {
        return Ok(());
    }
    let Some(key) = identity else {
        return Ok(());
    };
    if let Some(other) = find_other_repo_by_git_url(storage, key, &repo.id)? {
        // Derived key already belongs to another repo. Log the overlap and
        // let path match win: no merge, no move, no backfill.
        eprintln!(
            "identity overlap for path-matched repo '{}' ({}): derived identity '{}' is already owned by repo '{}' in project '{}'; path match wins, not merging",
            sanitize(&repo.name),
            sanitize(&repo.id),
            sanitize(key),
            sanitize(&other.id),
            sanitize(&other.project_id),
        );
        return Ok(());
    }
    storage.set_repo_git_url(&repo.id, key)?;
    Ok(())
}

/// Find a repo OTHER than `except_id` that owns the given identity key.
/// Named to not shadow `Storage::find_repos_by_git_url` (the full-vec
/// cross-project lookup).
fn find_other_repo_by_git_url(
    storage: &dyn Storage,
    key: &str,
    except_id: &str,
) -> crate::Result<Option<Repository>> {
    for r in storage.find_repos_by_git_url(key)? {
        if r.id != except_id {
            return Ok(Some(r));
        }
    }
    Ok(None)
}

/// Whether the stored checkout at `local_path` still exists. Only a definite
/// `Ok(false)` (not found) counts as vanished; any other stat error is treated
/// as live, so an I/O failure never re-registers a checkout as fresh or moved.
pub(super) fn path_exists(local_path: &str) -> bool {
    Path::new(local_path).try_exists().unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn path_exists_missing_path_counts_as_vanished() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("gone");
        assert!(!path_exists(missing.to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn path_exists_io_error_counts_as_live() {
        // Stat through a regular file fails with ENOTDIR, not NotFound: an I/O
        // error, which must not be read as "vanished".
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("plain");
        std::fs::write(&file, b"x").unwrap();
        let under_file = file.join("child");
        assert!(path_exists(under_file.to_str().unwrap()));
    }
}
