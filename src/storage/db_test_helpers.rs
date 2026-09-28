// Test helpers for sqlite operations

use crate::model::*;
use rusqlite::Connection;

pub fn create_in_memory_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::storage::schema::migrate(&conn).unwrap();

    // Setup required project and repo
    let project_id = "proj-test";
    let repo_id = "repo-test";

    conn.execute(
        "INSERT INTO projects (id, name) VALUES (?, ?)",
        [project_id, "test-project"],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO repositories (id, project_id, name, local_path) VALUES (?, ?, ?, ?)",
        [repo_id, project_id, "test-repo", "/tmp/test"],
    )
    .unwrap();

    conn
}

/// Seed the repo-wide unresolved-import counter on the repository row's
/// `unresolved_internal`/`unresolved_external` columns (#856) so
/// `get_unresolved_counts` returns them.
pub fn seed_repo_unresolved_counts(conn: &Connection, repo_id: &str, internal: u64, external: u64) {
    conn.execute(
        "UPDATE repositories SET unresolved_internal = ?1, unresolved_external = ?2 WHERE id = ?3",
        (internal as i64, external as i64, repo_id),
    )
    .unwrap();
}

// Helper to insert entity and create analysis run
pub fn setup_analysis_batch(conn: &Connection) -> (Entity, AnalysisRun) {
    let entity = Entity {
        id: "entity-1".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "test_module".to_string(),
        path: Some("src/test_module.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };

    let run = AnalysisRun {
        id: "run-1".to_string(),
        repo_id: "repo-test".to_string(),
        commit_hash: "abc123".to_string(),
        files_analyzed: 0,
        files_changed: 0,
        entities_upserted: 0,
        relationships_upserted: 0,
        duration_ms: None,
        status: AnalysisStatus::Pending,
        completed_at: None,
    };

    // Create the analysis run first
    conn.execute(
        "INSERT INTO analysis_runs (id, repo_id, commit_hash, status) VALUES (?, ?, ?, ?)",
        [&run.id, &run.repo_id, &run.commit_hash, "pending"],
    )
    .unwrap();

    (entity, run)
}
