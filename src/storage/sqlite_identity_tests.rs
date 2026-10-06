// Issue #31: Storage identity-method tests — set_repo_git_url,
// find_repos_by_git_url, and update_repo_local_path.
//
// Extracted from sqlite_tests.rs so the main test file stays within the
// 800-line test-file budget (AGENTS.md §5). These are the storage-level
// identity tests; the MCP-level cross-project lookup lives in
// src/mcp/repo_resolution_tests.rs.

use super::*;

/// Roundtrip: set_repo_git_url then find_repos_by_git_url returns the repo.
#[test]
fn test_set_repo_git_url_roundtrip() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();
    assert_eq!(repo.git_url, None, "git_url starts as None");

    storage
        .set_repo_git_url(&repo.id, "github.com:foo/bar")
        .unwrap();

    let fetched = storage.get_repo(&repo.id).unwrap().unwrap();
    assert_eq!(
        fetched.git_url.as_deref(),
        Some("github.com:foo/bar"),
        "git_url must be set after set_repo_git_url"
    );

    // find_repos_by_git_url returns the repo
    let found = storage.find_repos_by_git_url("github.com:foo/bar").unwrap();
    assert_eq!(found.len(), 1, "find_repos_by_git_url must return the repo");
    assert_eq!(found[0].id, repo.id);
}

/// find_repos_by_git_url returns empty Vec when no repo matches (incl. all-NULL git_url).
#[test]
fn test_find_repos_by_git_url_empty_when_no_match() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();
    let found = storage.find_repos_by_git_url("github.com:foo/bar").unwrap();
    assert!(found.is_empty());
}

/// Multiple repos sharing the same git_url: find_repos_by_git_url returns all of them.
#[test]
fn test_find_repos_by_git_url_returns_multiple_repos() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();
    let repo_a = storage
        .add_repo(&project.id, "checkout-a", "/path/to/checkout-a")
        .unwrap();
    let repo_b = storage
        .add_repo(&project.id, "checkout-b", "/path/to/checkout-b")
        .unwrap();
    let key = "github.com:org/repo";
    storage.set_repo_git_url(&repo_a.id, key).unwrap();
    storage.set_repo_git_url(&repo_b.id, key).unwrap();

    let found = storage.find_repos_by_git_url(key).unwrap();
    assert_eq!(
        found.len(),
        2,
        "find_repos_by_git_url must return all repos sharing the same git_url"
    );
    let ids: Vec<String> = found.iter().map(|r| r.id.clone()).collect();
    assert!(ids.contains(&repo_a.id.clone()));
    assert!(ids.contains(&repo_b.id.clone()));
}

/// update_repo_local_path changes both local_path and index_path atomically.
#[test]
fn test_update_repo_local_path_updates_both_columns() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();
    // Set an initial index_path so we can verify it changes
    storage
        .update_repo_index_path(&repo.id, "/path/to/repo1/.lievo/index")
        .unwrap();

    storage
        .update_repo_local_path(
            &repo.id,
            "/new/path/to/repo1",
            Some("/new/path/to/repo1/.lievo/index"),
        )
        .unwrap();

    let fetched = storage.get_repo(&repo.id).unwrap().unwrap();
    assert_eq!(fetched.local_path, "/new/path/to/repo1");
    assert_eq!(
        fetched.index_path.as_deref(),
        Some("/new/path/to/repo1/.lievo/index")
    );
}

/// All three identity methods return RepoNotFound for a missing repo_id.
#[test]
fn test_identity_methods_return_repo_not_found_for_missing_repo() {
    use crate::LievoError;
    let storage = SqliteStorage::open_in_memory().unwrap();

    let missing_id = "nonexistent-repo-id";

    let err = storage
        .set_repo_git_url(missing_id, "github.com:foo/bar")
        .unwrap_err();
    assert!(
        matches!(err, LievoError::RepoNotFound(ref id) if id == missing_id),
        "set_repo_git_url must return RepoNotFound for a missing repo"
    );

    let err = storage
        .update_repo_local_path(missing_id, "/new/path", Some("/new/index"))
        .unwrap_err();
    assert!(
        matches!(err, LievoError::RepoNotFound(ref id) if id == missing_id),
        "update_repo_local_path must return RepoNotFound for a missing repo"
    );

    // find_repos_by_git_url for an unknown key returns an empty Vec (not an error)
    let found = storage.find_repos_by_git_url("github.com:foo/bar").unwrap();
    assert!(found.is_empty());
}

/// update_repo_local_path propagates UNIQUE constraint violations.
#[test]
fn test_update_repo_local_path_unique_constraint_error() {
    use crate::LievoError;
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();
    let repo_a = storage
        .add_repo(&project.id, "repo-a", "/path/to/repo-a")
        .unwrap();
    // repo_b owns /path/to/repo-b — the collision target for the move below.
    let _repo_b = storage
        .add_repo(&project.id, "repo-b", "/path/to/repo-b")
        .unwrap();

    // Try to move repo_a onto repo_b's local_path — must fail with a
    // constraint error, not silently corrupt the identity.
    let result = storage.update_repo_local_path(
        &repo_a.id,
        "/path/to/repo-b",
        Some("/path/to/repo-b/.lievo/index"),
    );
    assert!(
        result.is_err(),
        "update_repo_local_path must fail when the new local_path is already held"
    );
    // The error should be a database/SQLite error, not RepoNotFound
    assert!(
        !matches!(result.unwrap_err(), LievoError::RepoNotFound(_)),
        "error must be a constraint violation, not RepoNotFound"
    );
    // repo_a's local_path is unchanged
    let fetched = storage.get_repo(&repo_a.id).unwrap().unwrap();
    assert_eq!(fetched.local_path, "/path/to/repo-a");
}

/// update_repo_local_path accepts `None` for `new_index_path`, writing NULL
/// to preserve the NULL state through the move (a fresh repo whose index
/// has not been built yet can be relocated without fabricating a path).
#[test]
fn test_update_repo_local_path_none_index_path_writes_null() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();
    // Fresh repo: index_path is NULL.
    assert_eq!(repo.index_path, None, "fresh repo has no index_path");

    // Relocate with None: index_path stays NULL, local_path moves.
    storage
        .update_repo_local_path(&repo.id, "/new/path", None)
        .unwrap();

    let fetched = storage.get_repo(&repo.id).unwrap().unwrap();
    assert_eq!(fetched.local_path, "/new/path");
    assert_eq!(
        fetched.index_path, None,
        "None new_index_path must preserve the NULL state"
    );
}
