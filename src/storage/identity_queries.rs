// SQL query constants for repository identity operations (issue #31).
// Organizing concern: the identity-write statements (git_url, local_path,
// index_path mutations) have one home here, alongside `REPOS_COLUMNS` — the
// shared 11-column projection constant the repositories-table SELECTs
// interpolate. `row_to_repo` is a read-path mapper that is intentionally
// co-located here for cross-query reuse (GET_REPO / LIST_REPOS in queries.rs
// call it too), even though this module otherwise owns identity writes.
// The file lives in its own module (not queries.rs, which is at the 500-line
// cap) so identity writes and the shared projection constant have one home.

use crate::model::Repository;

/// The 11-column repositories-table projection, in the exact order
/// `row_to_repo` below maps positionally. Kept in sync manually with
/// GET_REPO / LIST_REPOS (queries.rs) and FIND_REPOS_BY_GIT_URL — the
/// sentinel test validates positional alignment against this constant.
pub const REPOS_COLUMNS: &str = "id, project_id, name, git_url, local_path, default_branch, last_analyzed_commit, index_path, created_at, updated_at, summarization_unconfigured";

/// Set the normalized git_url key on a repository row.
pub const SET_REPO_GIT_URL: &str = r#"
UPDATE repositories
SET git_url = ?1, updated_at = ?2
WHERE id = ?3
"#;

/// Find all repositories by their normalized git_url key.
///
/// Uses the same 11-column projection as GET_REPO / LIST_REPOS (see
/// REPOS_COLUMNS). The column list here must match REPOS_COLUMNS exactly —
/// the sentinel test `test_row_to_repo_positions_match_repos_columns_projection`
/// validates the positional mapping against REPOS_COLUMNS.
///
/// Returns multiple rows because the same normalized remote can be
/// registered at several local_paths (checkouts). Ordered by creation
/// time with `id` as a deterministic tiebreak for same-second rows.
pub const FIND_REPOS_BY_GIT_URL: &str = r#"
SELECT id, project_id, name, git_url, local_path, default_branch, last_analyzed_commit, index_path, created_at, updated_at, summarization_unconfigured
FROM repositories
WHERE git_url = ?1
ORDER BY created_at ASC, id ASC
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
/// GET_REPO / LIST_REPOS in queries.rs) into a Repository struct. The
/// positions here are the 0..10 order of REPOS_COLUMNS — keep them in sync
/// if a column is added or reordered.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::schema::migrate;

    /// The positional `row_to_repo` mapping must stay aligned with the
    /// 11-column projection every repositories-table SELECT interpolates:
    /// run the projection against a sentinel row (one distinct value per
    /// column, position 0..10) and assert each field lands in its own slot.
    /// Catches a column reorder in REPOS_COLUMNS (or in row_to_repo) that
    /// would otherwise swap two stringly-typed fields silently.
    #[test]
    fn test_row_to_repo_positions_match_repos_columns_projection() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let sentinel = |i: u32| format!("col{i}");

        // FK constraint: repositories.project_id references projects.id.
        conn.execute(
            "INSERT INTO projects (id, name) VALUES ('p1', 'test-project')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO repositories (id, project_id, name, git_url, local_path, default_branch, last_analyzed_commit, index_path, created_at, updated_at, summarization_unconfigured) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            (
                sentinel(0),
                "p1",
                sentinel(2),
                sentinel(3),
                sentinel(4),
                sentinel(5),
                sentinel(6),
                sentinel(7),
                sentinel(8),
                sentinel(9),
                sentinel(10),
            ),
        )
        .unwrap();

        let sql = format!("SELECT {REPOS_COLUMNS} FROM repositories WHERE id = ?1");
        let repo = conn
            .query_row(&sql, [sentinel(0)], row_to_repo)
            .expect("projection must map 11 columns to Repository");

        assert_eq!(repo.id, sentinel(0));
        assert_eq!(repo.project_id, "p1");
        assert_eq!(repo.name, sentinel(2));
        assert_eq!(repo.git_url, Some(sentinel(3)));
        assert_eq!(repo.local_path, sentinel(4));
        assert_eq!(repo.default_branch, sentinel(5));
        assert_eq!(repo.last_analyzed_commit, Some(sentinel(6)));
        assert_eq!(repo.index_path, Some(sentinel(7)));
        assert_eq!(repo.created_at, sentinel(8));
        assert_eq!(repo.updated_at, sentinel(9));
        assert_eq!(repo.summarization_unconfigured, Some(sentinel(10)));
    }
}
