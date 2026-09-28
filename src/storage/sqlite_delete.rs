/// Delete operation for projects and all associated data.
/// Extracted from `sqlite_ops` to keep individual files under 500 lines.
use rusqlite::Connection;
use std::fs;

use crate::Result;
use crate::storage::queries as q;

/// Resolve and canonicalize ~/.lievo/indices/. Returns None on infrastructure failure
/// (logs via tracing::warn). Caller treats None as "all paths unsafe".
fn canonical_indices_root() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    let data_dir = match crate::extraction::lievo_data_dir() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to resolve lievo data dir");
            return None;
        }
    };
    let indices_root: PathBuf = data_dir.join("indices");
    match indices_root.canonicalize() {
        Ok(p) => Some(p),
        Err(e) => {
            tracing::warn!(error = %e, indices_root = %indices_root.display(), "Failed to canonicalize indices root");
            None
        }
    }
}

/// Validate that an index path is safe to remove.
///
/// Verifies the path canonicalizes to a location under the provided canonical root.
/// This prevents accidental deletion of arbitrary paths if the DB is tampered with.
///
/// If canonicalization fails (e.g., the path was already deleted), returns false
/// (this is the normal "path already removed" case).
fn is_safe_index_path(path: &str, canonical_root: &std::path::Path) -> bool {
    use std::path::Path;

    let candidate = Path::new(path);

    // Canonicalize the candidate path. If it doesn't exist or can't be canonicalized,
    // return false (normal case: path already removed, no logging needed).
    let canonical_path = match candidate.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };

    // Verify canonical_path is under canonical_root
    canonical_path.starts_with(canonical_root)
}

/// Remove index directories for a collection of paths.
///
/// Iterates through optional paths, validates each against the canonical root,
/// and attempts removal. Returns count of successfully removed directories.
/// Logs warnings for safety-check failures and removal errors; logs infrastructure
/// failures separately via `canonical_indices_root()` when root is None.
///
/// # Arguments
/// - `paths`: Iterator over optional path strings to attempt removal
///
/// # Returns
/// Count of successfully removed directories (0 if root resolves to None or all paths fail)
fn remove_safe_index_dirs<'a>(paths: impl IntoIterator<Item = &'a Option<String>>) -> u64 {
    let canonical_root = match canonical_indices_root() {
        Some(r) => r,
        None => {
            // Infrastructure failure already logged in canonical_indices_root.
            // Treat all paths as unsafe, skip all removals.
            return 0;
        }
    };

    paths
        .into_iter()
        .map(|path| match path.as_ref() {
            Some(p) => {
                if !is_safe_index_path(p, &canonical_root) {
                    tracing::warn!(path = %p, "index_path fails safety check — skipping removal");
                    return 0;
                }
                match fs::remove_dir_all(std::path::Path::new(p)) {
                    Ok(()) => 1,
                    Err(e) => {
                        tracing::warn!(path = %p, error = %e, "failed to remove index directory");
                        0
                    }
                }
            }
            None => 0,
        })
        .sum()
}

/// Delete a project and all its associated data in a single atomic transaction.
///
/// Relationships are deleted with a single SQL statement across ALL entity IDs
/// for the project, ensuring an accurate `relationships_deleted` count regardless
/// of how many repos the project contains.
///
/// Returns counts of each type of record removed, including index directories removed.
///
/// # Errors
///
/// Returns an error if the project disappears between the caller's existence check
/// and the actual deletion (concurrent modification), or if any SQL operation fails.
pub(super) fn delete_project(
    conn: &Connection,
    project_id: &str,
) -> Result<crate::storage::DeleteStats> {
    // SAFETY: unchecked_transaction is safe here for the same reason as persist_analysis_batch —
    // SqliteStorage has no reentrant entry points, and this method holds exclusive access.
    let tx = conn.unchecked_transaction()?;

    // Delete all relationships that touch any entity in this project in one shot.
    // A single statement gives an accurate total count regardless of how many repos
    // the project has (per-repo iteration would undercount cross-repo relationships).
    let relationships_deleted =
        tx.execute(q::DELETE_RELATIONSHIPS_BY_PROJECT, [project_id])? as u64;

    // First, capture index_path for each repo before deleting them.
    let index_paths: Vec<Option<String>> = {
        let mut stmt = tx.prepare("SELECT index_path FROM repositories WHERE project_id = ?1")?;
        stmt.query_map([project_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    // Delete entities, analysis runs, and file hashes per-repo (no cross-repo issue here).
    let repo_ids: Vec<String> = {
        let mut stmt = tx.prepare(q::LIST_REPO_IDS_FOR_PROJECT)?;
        stmt.query_map([project_id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };

    let mut entities_deleted: u64 = 0;
    for repo_id in &repo_ids {
        let ents = tx.execute(q::DELETE_ENTITIES_BY_REPO, [repo_id])?;
        entities_deleted += ents as u64;
        tx.execute(q::DELETE_ANALYSIS_RUNS_BY_REPO, [repo_id])?;
        tx.execute(q::DELETE_FILE_HASHES_BY_REPO, [repo_id])?;
    }

    // Delete insights and conventions scoped to the project.
    let insights_deleted = tx.execute(q::DELETE_INSIGHTS_BY_PROJECT, [project_id])? as u64;
    let conventions_deleted = tx.execute(q::DELETE_CONVENTIONS_BY_PROJECT, [project_id])? as u64;

    // Delete repositories, then the project itself.
    let repos_deleted = tx.execute(q::DELETE_REPOS_BY_PROJECT, [project_id])? as u64;

    // Guard against TOCTOU: the project must still exist at this point.
    let project_rows = tx.execute(q::DELETE_PROJECT_BY_ID, [project_id])?;
    if project_rows == 0 {
        return Err(crate::LievoError::InvalidInput(
            "project disappeared during deletion (concurrent modification)".to_string(),
        ));
    }

    tx.commit()?;

    // Remove index directories (best-effort, outside transaction)
    let index_dirs_removed = remove_safe_index_dirs(index_paths.iter());

    Ok(crate::storage::DeleteStats {
        repos_deleted,
        entities_deleted,
        relationships_deleted,
        insights_deleted,
        conventions_deleted,
        index_dirs_removed,
    })
}

/// Delete a repository and all its associated data in a single atomic transaction.
///
/// Returns counts of each type of record removed, including whether the index
/// directory was removed.
///
/// # Errors
///
/// Returns an error if the repository disappears between the caller's existence check
/// and the actual deletion (concurrent modification), or if any SQL operation fails.
pub(super) fn delete_repo(conn: &Connection, repo_id: &str) -> Result<crate::storage::DeleteStats> {
    // SAFETY: unchecked_transaction is safe here for the same reason as persist_analysis_batch.
    let tx = conn.unchecked_transaction()?;

    // Capture index_path inside the transaction to prevent TOCTOU
    let index_path: Option<String> = {
        let mut stmt = tx.prepare("SELECT index_path FROM repositories WHERE id = ?1")?;
        stmt.query_row([repo_id], |row| row.get(0)).ok()
    };

    // Delete all relationships that touch any entity in this repo.
    let relationships_deleted = tx.execute(q::DELETE_RELATIONSHIPS_BY_REPO, [repo_id])? as u64;

    // Delete entities, analysis runs, and file hashes.
    let entities_deleted = tx.execute(q::DELETE_ENTITIES_BY_REPO, [repo_id])? as u64;
    tx.execute(q::DELETE_ANALYSIS_RUNS_BY_REPO, [repo_id])?;
    tx.execute(q::DELETE_FILE_HASHES_BY_REPO, [repo_id])?;

    // Delete the repository.
    let repos_deleted = tx.execute(q::DELETE_REPO, [repo_id])? as u64;

    // Guard against TOCTOU: the repo must still exist at this point.
    if repos_deleted == 0 {
        return Err(crate::LievoError::RepoNotFound(repo_id.to_string()));
    }

    tx.commit()?;

    // Remove index directory (best-effort, outside transaction)
    let index_dirs_removed = remove_safe_index_dirs(std::iter::once(&index_path));

    Ok(crate::storage::DeleteStats {
        repos_deleted,
        entities_deleted,
        relationships_deleted,
        insights_deleted: 0,
        conventions_deleted: 0,
        index_dirs_removed,
    })
}

#[cfg(test)]
#[path = "sqlite_delete_tests.rs"]
mod sqlite_delete_tests;
