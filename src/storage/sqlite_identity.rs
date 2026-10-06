// SqliteStorage identity method implementations (issue #31).
// Groups the identity-related SQLite methods (git_url, local_path,
// index_path mutations). Mirrors the extraction pattern used by
// sqlite_ops.rs / sqlite_project.rs.

use crate::Result;
use crate::model::Repository;
use crate::storage::identity_queries as q;
use rusqlite::Connection;

/// Set the normalized git_url identity key on a repository row.
///
/// Precondition (trust boundary): `key` MUST already be a normalized
/// git-remote identity (see issue #24 sub-issue 1). This method persists it
/// opaque, with no well-formedness validation, so callers must normalize
/// before the storage call or identity lookups can alias/miss.
///
/// Returns `LievoError::RepoNotFound` when the repo_id does not match
/// any row (0 rows affected), matching the `record_unresolved_counts`
/// and `update_repository_unconfigured_marker` precedent.
pub fn set_repo_git_url(conn: &Connection, repo_id: &str, key: &str, now: &str) -> Result<()> {
    let rows = conn.execute(q::SET_REPO_GIT_URL, (key, now, repo_id))?;
    if rows == 0 {
        return Err(crate::LievoError::RepoNotFound(repo_id.to_string()));
    }
    Ok(())
}

/// Find all repositories registered under a normalized git_url key.
///
/// Returns multiple repos because the same normalized remote can be
/// registered at several local_paths (checkouts) across projects.
pub fn find_repos_by_git_url(conn: &Connection, key: &str) -> Result<Vec<Repository>> {
    let mut stmt = conn.prepare_cached(q::FIND_REPOS_BY_GIT_URL)?;
    let rows = stmt.query_map([key], q::row_to_repo)?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

/// Relocate a repository: update both local_path and index_path in a
/// single UPDATE statement so the two columns never diverge.
///
/// `new_path` must be non-empty and absolute, and `new_index_path` (when
/// `Some`) must be non-empty and absolute; otherwise
/// `LievoError::InvalidInput` is returned before any SQL runs. No
/// canonicalization or filesystem access happens here.
///
/// `new_index_path` may be `None` (writes NULL) when the repo has no index
/// yet — the coupling invariant is "if index_path was NULL before the move,
/// it stays NULL after." `Some` moves the index to the new location.
///
/// Returns `LievoError::RepoNotFound` when the repo_id does not match
/// any row (0 rows affected), matching the `record_unresolved_counts`
/// and `update_repository_unconfigured_marker` precedent.
///
/// A UNIQUE-constraint violation on local_path (moving to a path already
/// held by another repo row) surfaces as `LievoError::Database`, wrapping
/// rusqlite's constraint-violation error — the caller gets an error, not a
/// panic.
pub fn update_repo_local_path(
    conn: &Connection,
    repo_id: &str,
    new_path: &str,
    new_index_path: Option<&str>,
    now: &str,
) -> Result<()> {
    if new_path.is_empty() || !std::path::Path::new(new_path).is_absolute() {
        return Err(crate::LievoError::InvalidInput(format!(
            "new_path must be an absolute path, got: {new_path:?}"
        )));
    }
    if let Some(index) = new_index_path
        && (index.is_empty() || !std::path::Path::new(index).is_absolute())
    {
        return Err(crate::LievoError::InvalidInput(format!(
            "new_index_path must be an absolute path, got: {index:?}"
        )));
    }
    let rows = conn.execute(
        q::UPDATE_REPO_LOCAL_PATH,
        (new_path, new_index_path, now, repo_id),
    )?;
    if rows == 0 {
        return Err(crate::LievoError::RepoNotFound(repo_id.to_string()));
    }
    Ok(())
}
