use super::*;
use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;

fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::schema::migrate(&conn).unwrap();
    conn
}

fn insert_project(conn: &Connection, id: &str, name: &str) {
    conn.execute(
        "INSERT INTO projects (id, name, description, created_at, updated_at) VALUES (?1, ?2, ?3, datetime('now'), datetime('now'))",
        [id, name, ""],
    ).unwrap();
}

fn insert_repo(
    conn: &Connection,
    id: &str,
    project_id: &str,
    name: &str,
    index_path: Option<&str>,
) {
    let local_path = format!("/path/{}", name);
    conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path, git_url, default_branch, index_path, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'https://github.com/test/repo', 'main', ?5, datetime('now'), datetime('now'))",
        [id, project_id, name, &local_path, index_path.unwrap_or("")],
    ).unwrap();
}

fn insert_entity(conn: &Connection, id: &str, project_id: &str, repo_id: &str) {
    conn.execute(
        "INSERT INTO entities (id, project_id, repo_id, tier, name, created_at, updated_at) VALUES (?1, ?2, ?3, 'file', 'test.file', datetime('now'), datetime('now'))",
        [id, project_id, repo_id],
    ).unwrap();
}

fn insert_relationship(conn: &Connection, source_id: &str, target_id: &str) {
    conn.execute(
        "INSERT INTO relationships (source_id, target_id, rel_type, weight) VALUES (?1, ?2, 'depends_on', 1.0)",
        [source_id, target_id],
    ).unwrap();
}

fn insert_analysis_run(conn: &Connection, id: &str, repo_id: &str) {
    conn.execute(
        "INSERT INTO analysis_runs (id, repo_id, commit_hash, status) VALUES (?1, ?2, 'abc123', 'completed')",
        [id, repo_id],
    ).unwrap();
}

fn insert_file_hash(conn: &Connection, repo_id: &str, file_path: &str) {
    conn.execute(
        "INSERT INTO file_hashes (repo_id, file_path, content_hash, last_analyzed) VALUES (?1, ?2, 'hash123', datetime('now'))",
        [repo_id, file_path],
    ).unwrap();
}

/// Create a test index directory under ~/.lievo/indices/ and return its path.
fn create_test_index_dir(subdir: &str) -> PathBuf {
    let data_dir = crate::extraction::lievo_data_dir().expect("failed to get lievo data dir");
    let indices_dir = data_dir.join("indices");
    fs::create_dir_all(&indices_dir).expect("failed to create indices dir");
    let test_dir = indices_dir.join(subdir);
    fs::create_dir_all(&test_dir).expect("failed to create test index dir");
    test_dir
}

/// Clean up test index directory.
fn cleanup_test_index_dir(path: &PathBuf) {
    if path.exists() {
        fs::remove_dir_all(path).expect("failed to remove test index dir");
    }
}

#[test]
fn test_delete_repo_removes_all_data() {
    let conn = setup_db();

    // Setup: project, repo, 2 entities, 1 relationship, 1 analysis_run, 1 file_hash
    insert_project(&conn, "proj1", "project1");
    insert_repo(&conn, "repo1", "proj1", "repo1", None);
    insert_entity(&conn, "entity1", "proj1", "repo1");
    insert_entity(&conn, "entity2", "proj1", "repo1");
    insert_relationship(&conn, "entity1", "entity2");
    insert_analysis_run(&conn, "run1", "repo1");
    insert_file_hash(&conn, "repo1", "file.rs");

    // Verify data exists before deletion
    let entity_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE repo_id = 'repo1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(entity_count, 2);

    let rel_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rel_count, 1);

    // Delete the repo
    let result = delete_repo(&conn, "repo1");
    assert!(result.is_ok());

    let stats = result.unwrap();
    assert_eq!(stats.repos_deleted, 1);
    assert_eq!(stats.entities_deleted, 2);
    assert_eq!(stats.relationships_deleted, 1);
    assert_eq!(stats.index_dirs_removed, 0);

    // Verify all data is gone
    let entity_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE repo_id = 'repo1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(entity_count, 0);

    let rel_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rel_count, 0);

    let run_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM analysis_runs WHERE repo_id = 'repo1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(run_count, 0);

    let hash_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_hashes WHERE repo_id = 'repo1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hash_count, 0);

    let repo_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM repositories WHERE id = 'repo1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repo_count, 0);
}

#[test]
fn test_delete_repo_removes_cross_repo_relationship() {
    let conn = setup_db();

    // Setup: project, 2 repos, 2 entities (one per repo), cross-repo relationship
    insert_project(&conn, "proj1", "project1");
    insert_repo(&conn, "repo1", "proj1", "repo1", None);
    insert_repo(&conn, "repo2", "proj1", "repo2", None);
    insert_entity(&conn, "entity1", "proj1", "repo1");
    insert_entity(&conn, "entity2", "proj1", "repo2");
    insert_relationship(&conn, "entity1", "entity2"); // relationship crosses repos

    // Verify relationship exists
    let rel_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rel_count, 1);

    // Delete repo1
    let result = delete_repo(&conn, "repo1");
    assert!(result.is_ok());

    // Verify relationship is deleted (touched entity1 in repo1)
    let rel_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rel_count, 0);

    // Verify entity2 in repo2 still exists
    let entity2_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE repo_id = 'repo2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(entity2_count, 1);

    // Verify repo2 still exists
    let repo2_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM repositories WHERE id = 'repo2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repo2_count, 1);
}

#[test]
fn test_is_safe_index_path_inside_root() {
    let test_dir = create_test_index_dir("test_inside_root");
    let path_str = test_dir.to_str().unwrap();
    let canonical_root = canonical_indices_root().expect("failed to get canonical root");

    // Path under ~/.lievo/indices/ should pass
    assert!(is_safe_index_path(path_str, &canonical_root));

    cleanup_test_index_dir(&test_dir);
}

#[test]
fn test_is_safe_index_path_outside_root() {
    let canonical_root = canonical_indices_root().expect("failed to get canonical root");
    // Path in /tmp should fail validation (not under ~/.lievo/indices/)
    assert!(!is_safe_index_path(
        "/tmp/some_random_index_dir",
        &canonical_root
    ));
}

#[test]
fn test_is_safe_index_path_relative_path() {
    let canonical_root = canonical_indices_root().expect("failed to get canonical root");
    // Relative paths should fail canonicalization
    assert!(!is_safe_index_path("index/repo1", &canonical_root));
    assert!(!is_safe_index_path("./relative/path", &canonical_root));
}

#[test]
fn test_is_safe_index_path_traversal_with_dotdot() {
    let canonical_root = canonical_indices_root().expect("failed to get canonical root");
    // Path traversal attempts should be caught by canonicalization
    // Create a test dir and try to break out with ..
    let test_dir = create_test_index_dir("test_traversal_base");
    let data_dir = crate::extraction::lievo_data_dir().expect("failed to get lievo data dir");

    // Create a sibling directory outside the indices dir
    let evil_dir = data_dir.join("evil_sibling");
    fs::create_dir_all(&evil_dir).ok();

    // Try to use a path with .. to escape indices/
    let traversal_path = format!("{}/../evil_sibling", test_dir.to_str().unwrap());
    assert!(!is_safe_index_path(&traversal_path, &canonical_root));

    cleanup_test_index_dir(&test_dir);
    fs::remove_dir(&evil_dir).ok();
}

#[test]
fn test_delete_repo_index_cleanup() {
    let conn = setup_db();
    let index_dir = create_test_index_dir("test_delete_repo");
    let index_path = index_dir.to_str().unwrap().to_string();

    // Setup: project, repo with index_path
    insert_project(&conn, "proj1", "project1");
    insert_repo(&conn, "repo1", "proj1", "repo1", Some(&index_path));

    // Create a dummy file in the index directory
    let test_file = index_dir.join("test.file");
    fs::write(&test_file, "test data").unwrap();
    assert!(test_file.exists());

    // Delete the repo
    let result = delete_repo(&conn, "repo1");
    assert!(result.is_ok());

    let stats = result.unwrap();
    assert_eq!(stats.index_dirs_removed, 1);

    // Verify index directory no longer exists
    assert!(!index_dir.exists(), "index directory should be removed");
}

#[test]
fn test_delete_project_removes_index_dirs() {
    let conn = setup_db();

    // Create two test index directories under ~/.lievo/indices/
    let index_dir1 = create_test_index_dir("test_delete_proj_1");
    let index_dir2 = create_test_index_dir("test_delete_proj_2");
    let index_path1 = index_dir1.to_str().unwrap().to_string();
    let index_path2 = index_dir2.to_str().unwrap().to_string();

    // Write dummy files so the directories really exist on disk
    fs::write(index_dir1.join("index.file"), "data1").unwrap();
    fs::write(index_dir2.join("index.file"), "data2").unwrap();
    assert!(index_dir1.exists());
    assert!(index_dir2.exists());

    // Setup: project with two repos, each having an index_path
    insert_project(&conn, "proj1", "project1");
    insert_repo(&conn, "repo1", "proj1", "repo1", Some(&index_path1));
    insert_repo(&conn, "repo2", "proj1", "repo2", Some(&index_path2));
    insert_entity(&conn, "entity1", "proj1", "repo1");
    insert_entity(&conn, "entity2", "proj1", "repo2");
    insert_relationship(&conn, "entity1", "entity2");

    // Delete entire project
    let result = delete_project(&conn, "proj1");
    assert!(result.is_ok());

    let stats = result.unwrap();
    assert_eq!(stats.repos_deleted, 2);
    assert_eq!(stats.entities_deleted, 2);
    assert_eq!(stats.relationships_deleted, 1);
    assert_eq!(stats.index_dirs_removed, 2);

    // Verify both index directories were removed
    assert!(
        !index_dir1.exists(),
        "first index directory should be removed"
    );
    assert!(
        !index_dir2.exists(),
        "second index directory should be removed"
    );
}

#[test]
fn test_delete_project_skips_missing_index_dir() {
    let conn = setup_db();

    // Create a path under ~/.lievo/indices/ that doesn't exist on disk
    let data_dir = crate::extraction::lievo_data_dir().expect("failed to get lievo data dir");
    let indices_dir = data_dir.join("indices");
    fs::create_dir_all(&indices_dir).ok();
    let ghost_path = indices_dir
        .join("nonexistent_index_dir")
        .to_string_lossy()
        .into_owned();

    // Verify path doesn't exist
    assert!(
        !std::path::Path::new(&ghost_path).exists(),
        "ghost path must not exist for this test"
    );

    insert_project(&conn, "proj1", "project1");
    insert_repo(&conn, "repo1", "proj1", "repo1", Some(&ghost_path));

    let result = delete_project(&conn, "proj1");
    assert!(result.is_ok());

    let stats = result.unwrap();
    // Directory didn't exist, so 0 dirs removed (not an error)
    assert_eq!(stats.index_dirs_removed, 0);
}

#[test]
fn test_delete_repo_nonexistent_returns_error() {
    let conn = setup_db();

    let result = delete_repo(&conn, "nonexistent-id");
    assert!(result.is_err());

    match result {
        Err(crate::LievoError::RepoNotFound(id)) => {
            assert_eq!(id, "nonexistent-id");
        }
        _ => panic!("Expected RepoNotFound error"),
    }
}

#[test]
fn test_delete_repo_skips_unsafe_index_path() {
    let conn = setup_db();

    insert_project(&conn, "proj1", "project1");
    // Insert repo with a path outside ~/.lievo/indices/ (unsafe)
    insert_repo(
        &conn,
        "repo1",
        "proj1",
        "repo1",
        Some("/tmp/totally_unsafe_dir"),
    );

    let result = delete_repo(&conn, "repo1");
    assert!(result.is_ok());

    let stats = result.unwrap();
    // Unsafe path should be skipped, so 0 dirs removed
    assert_eq!(stats.index_dirs_removed, 0);
}

#[test]
fn test_delete_project_skips_unsafe_index_path() {
    let conn = setup_db();

    let safe_dir = create_test_index_dir("test_delete_proj_safe");
    let safe_path = safe_dir.to_str().unwrap().to_string();

    insert_project(&conn, "proj1", "project1");
    // One repo with safe path, one with unsafe path
    insert_repo(&conn, "repo1", "proj1", "repo1", Some(&safe_path));
    insert_repo(
        &conn,
        "repo2",
        "proj1",
        "repo2",
        Some("/tmp/totally_unsafe_dir"),
    );

    let result = delete_project(&conn, "proj1");
    assert!(result.is_ok());

    let stats = result.unwrap();
    // Only the safe index dir should be removed
    assert_eq!(stats.index_dirs_removed, 1);
    assert!(!safe_dir.exists(), "safe index dir should be removed");

    cleanup_test_index_dir(&safe_dir);
}
