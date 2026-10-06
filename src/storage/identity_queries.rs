// SQL query constants for repository identity operations (issue #31).
// Organizing concern: every statement here mutates the repositories table's
// external-identity columns (git_url, local_path, index_path). It lives in its
// own file (not queries.rs, which is at the 500-line cap) so identity writes
// have one home. `row_to_repo` is shared with the other repositories-table
// projections.

use crate::model::Repository;

/// Set the normalized git_url key on a repository row.
pub const SET_REPO_GIT_URL: &str = r#"
UPDATE repositories
SET git_url = ?1, updated_at = ?2
WHERE id = ?3
"#;

/// Find all repositories by their normalized git_url key.
///
/// This is one of three copies of the repositories 11-column projection —
/// the other two are GET_REPO and LIST_REPOS in queries.rs. Keep all three
/// column lists (and their order) in sync with `row_to_repo` below.
/// Returns multiple rows because the same normalized remote can be
/// registered at several local_paths (checkouts).
pub const FIND_REPOS_BY_GIT_URL: &str = r#"
SELECT id, project_id, name, git_url, local_path, default_branch, last_analyzed_commit, index_path, created_at, updated_at, summarization_unconfigured
FROM repositories
WHERE git_url = ?1
ORDER BY created_at ASC
"#;

/// Relocate a repository: update both local_path and index_path atomically
/// in a single statement, plus bump updated_at.
///
/// This is the write path that keeps `local_path` and `index_path` coupled:
/// any move to a new location MUST go through here. UPDATE_REPO_INDEX_PATH in
/// queries.rs is for the initial index setup only and never relocates a repo.
pub const UPDATE_REPO_LOCAL_PATH: &str = r#"
UPDATE repositories
SET local_path = ?1, index_path = ?2, updated_at = ?3
WHERE id = ?4
"#;

/// Map a single row from the repository SELECT projection (shared with
/// GET_REPO / LIST_REPOS in queries.rs) into a Repository struct.
pub(crate) fn row_to_repo(row: &rusqlite::Row) -> rusqlite::Result<Repository> {
    Ok(Repository {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        git_url: row.get(3)?,
        local_path: row.get(4)?,
        default_branch: row.get(5)?,
        last_analyzed_commit: row.get(6)?,
        index_path: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        summarization_unconfigured: row.get(10)?,
    })
}
