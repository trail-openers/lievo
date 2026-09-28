/// Tests for entity_by_path_projectwide function separated to keep sqlite_ops.rs under 500 lines
use rusqlite::Connection;

use crate::model::EntityTier;
use crate::storage::queries as q;
use crate::storage::schema;
use crate::storage::sqlite::sqlite_ops;

#[test]
fn test_entity_by_path_projectwide_multi_repo_collision_returns_all() {
    let conn = Connection::open_in_memory().unwrap();
    schema::migrate(&conn).unwrap();

    let tier_str = EntityTier::File.to_string();
    // Create project first to satisfy foreign key constraint
    conn.execute(
        "INSERT INTO projects (id, name) VALUES (?1, ?2)",
        ("proj1", "Project 1"),
    )
    .unwrap();
    // Create repos
    conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path) VALUES (?1, ?2, ?3, ?4)",
        ("repo1", "proj1", "repo1", "/tmp/repo1"),
    )
    .unwrap();
    conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path) VALUES (?1, ?2, ?3, ?4)",
        ("repo2", "proj1", "repo2", "/tmp/repo2"),
    )
    .unwrap();

    // Create entity in repo1
    conn.execute(
        q::UPSERT_ENTITY,
        (
            "e1",
            "proj1",
            "repo1",
            tier_str.as_str(),
            None::<&str>,
            "main.rs",
            "src/main.rs",
            Some("rust"),
            None::<&str>,
            None::<&str>,
            None::<&str>,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        ),
    )
    .unwrap();
    // Create entity in repo2 with same path
    conn.execute(
        q::UPSERT_ENTITY,
        (
            "e2",
            "proj1",
            "repo2",
            tier_str.as_str(),
            None::<&str>,
            "main.rs",
            "src/main.rs",
            Some("rust"),
            None::<&str>,
            None::<&str>,
            None::<&str>,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
        ),
    )
    .unwrap();

    let result = sqlite_ops::entity_by_path_projectwide(&conn, "proj1", "src/main.rs").unwrap();
    assert_eq!(
        result.len(),
        2,
        "should return both entities with same path in different repos"
    );

    // Verify both entities are returned with different repo_ids
    let repo_ids: Vec<_> = result.iter().filter_map(|e| e.repo_id.as_ref()).collect();
    assert!(repo_ids.contains(&&"repo1".to_string()));
    assert!(repo_ids.contains(&&"repo2".to_string()));
}
