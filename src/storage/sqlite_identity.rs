// SqliteStorage identity method implementations (issue #31).
// Lives in its own file because sqlite.rs is at the 500-line cap.
// Mirrors the extraction pattern used by sqlite_ops.rs / sqlite_project.rs.

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
/// Precondition (trust boundary): `new_path` MUST be a canonical absolute
/// path. `new_index_path` may be `None` (writes NULL) when the repo has no
/// index yet — the coupling invariant is "if index_path was NULL before the
/// move, it stays NULL after." `Some` moves the index to the new location.
/// This method persists them opaque, with no canonicality or containment
/// validation, so callers (CLI add_repo, MCP wiring, config overrides) must
/// canonicalize before the storage call.
///
/// Returns `LievoError::RepoNotFound` when the repo_id does not match
/// any row (0 rows affected), matching the `record_unresolved_counts`
/// and `update_repository_unconfigured_marker` precedent.
///
/// A UNIQUE-constraint violation on local_path (moving to a path already
/// held by another repo row) is surfaced as a `crate::LievoError` via the
/// `?` operator — the caller gets an error, not a panic.
pub fn update_repo_local_path(
    conn: &Connection,
    repo_id: &str,
    new_path: &str,
    new_index_path: Option<&str>,
    now: &str,
) -> Result<()> {
    let rows = conn.execute(
        q::UPDATE_REPO_LOCAL_PATH,
        (new_path, new_index_path, now, repo_id),
    )?;
    if rows == 0 {
        return Err(crate::LievoError::RepoNotFound(repo_id.to_string()));
    }
    Ok(())
}
