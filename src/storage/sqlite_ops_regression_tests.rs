// Regression tests for relationship cleanup and migration issues

use crate::model::*;
use crate::storage::sqlite::sqlite_ops;
use rusqlite::Connection;

fn create_in_memory_db() -> Connection {
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

#[test]
fn test_persist_analysis_batch_cleans_stale_relationship_edges() {
    // Regression test for issue #491: stale relationship edges persisting after re-index.
    // When a repo is re-indexed, old relationship edges pointing to the previous
    // entity IDs remain orphaned because ON CONFLICT DO UPDATE only replaces rows —
    // it does not delete edges that are no longer present.
    // persist_analysis_batch should delete ALL existing relationship edges for the repo
    // before inserting new ones.
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";
    let last_commit = "abc123";

    // Create two file entities
    let file_a = Entity {
        id: "file-a".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: "a.rs".to_string(),
        path: Some("src/a.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    let file_b = Entity {
        id: "file-b".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: "b.rs".to_string(),
        path: Some("src/b.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    // Insert entities
    sqlite_ops::upsert_entity(&conn, &file_a, now).unwrap();
    sqlite_ops::upsert_entity(&conn, &file_b, now).unwrap();

    // Simulate old relationship edge from a previous analysis run
    let old_rel = Relationship {
        source_id: "file-a".to_string(),
        target_id: "file-b".to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    sqlite_ops::upsert_relationship(&conn, &old_rel).unwrap();

    // Verify old relationship exists
    let from_a_before = sqlite_ops::relationships_from(&conn, "file-a").unwrap();
    assert_eq!(from_a_before.len(), 1);

    // Create a DIFFERENT relationship in a new analysis batch
    // (simulating a re-index that found different relationships)
    let new_rel = Relationship {
        source_id: "file-b".to_string(),
        target_id: "file-a".to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };

    let run = AnalysisRun {
        id: "run-1".to_string(),
        repo_id: "repo-test".to_string(),
        commit_hash: "abc123".to_string(),
        files_analyzed: 2,
        files_changed: 2,
        entities_upserted: 2,
        relationships_upserted: 1,
        duration_ms: None,
        status: AnalysisStatus::Completed,
        completed_at: None,
    };

    let entities: Vec<&Entity> = vec![&file_a, &file_b];
    let relationships: Vec<Relationship> = vec![new_rel];

    // Persist the batch - this should clean up the old relationship edge
    let _ = sqlite_ops::persist_analysis_batch(
        &conn,
        &entities,
        &relationships,
        &run,
        repo_id,
        last_commit,
        now,
    )
    .unwrap();

    // Verify: old relationship (file-a → file-b) is gone, new relationship (file-b → file-a) exists
    let from_a_after = sqlite_ops::relationships_from(&conn, "file-a").unwrap();
    assert!(
        from_a_after.is_empty(),
        "old relationship from file-a should be deleted, but found {} relationships",
        from_a_after.len()
    );

    let from_b_after = sqlite_ops::relationships_from(&conn, "file-b").unwrap();
    assert_eq!(
        from_b_after.len(),
        1,
        "new relationship from file-b should exist"
    );
    assert_eq!(from_b_after[0].0.target_id, "file-a");

    // Verify total relationship count is exactly 1 (only the new one)
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relationships r
             JOIN entities e1 ON r.source_id = e1.id
             WHERE e1.repo_id = ?1",
            [repo_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "only the new relationship should remain");
}

#[test]
fn test_persist_analysis_batch_fresh_db_noop() {
    // Test that persist_analysis_batch succeeds on a fresh database with no stale edges
    // and deletes 0 rows (tests the no-op path).
    // Unlike the stale edge cleanup test, here we persist relationships that exactly
    // match what was there before — so DELETE_RELATIONSHIPS_BY_REPO removes them
    // and UPSERT puts them back identically.
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";
    let last_commit = "abc123";

    // Create two file entities
    let file_a = Entity {
        id: "file-a".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: "a.rs".to_string(),
        path: Some("src/a.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    let file_b = Entity {
        id: "file-b".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: "b.rs".to_string(),
        path: Some("src/b.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    // Insert entities
    sqlite_ops::upsert_entity(&conn, &file_a, now).unwrap();
    sqlite_ops::upsert_entity(&conn, &file_b, now).unwrap();

    // Create "imports" edge (no stale edges exist)
    let imports = Relationship {
        source_id: "file-a".to_string(),
        target_id: "file-b".to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };

    let run = AnalysisRun {
        id: "run-1".to_string(),
        repo_id: "repo-test".to_string(),
        commit_hash: "abc123".to_string(),
        files_analyzed: 2,
        files_changed: 2,
        entities_upserted: 2,
        relationships_upserted: 1,
        duration_ms: None,
        status: AnalysisStatus::Completed,
        completed_at: None,
    };

    let entities: Vec<&Entity> = vec![&file_a, &file_b];
    let relationships: Vec<Relationship> = vec![imports];

    // Persist the batch - should succeed without error
    let _ = sqlite_ops::persist_analysis_batch(
        &conn,
        &entities,
        &relationships,
        &run,
        repo_id,
        last_commit,
        now,
    )
    .unwrap();

    // Verify that the "imports" edge exists after the batch
    let from_a = sqlite_ops::relationships_from(&conn, "file-a").unwrap();
    assert_eq!(from_a.len(), 1);
    assert_eq!(from_a[0].0.rel_type, RelType::Imports);
}

#[test]
fn test_delete_entities_by_paths_removes_entity_and_relationships() {
    // Regression: delete_entities_by_paths must delete relationships referencing
    // an entity BEFORE deleting the entity itself, otherwise the FOREIGN KEY
    // constraint causes a constraint violation.
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";

    let entity_a = Entity {
        id: "del-a".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: "to_delete.rs".to_string(),
        path: Some("docs/to_delete.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    let entity_b = Entity {
        id: "del-b".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: "kept.rs".to_string(),
        path: Some("src/kept.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    sqlite_ops::upsert_entity(&conn, &entity_a, now).unwrap();
    sqlite_ops::upsert_entity(&conn, &entity_b, now).unwrap();

    // Relationship from docs entity to kept entity
    let rel = Relationship {
        source_id: "del-a".to_string(),
        target_id: "del-b".to_string(),
        rel_type: RelType::DependsOn,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    sqlite_ops::upsert_relationship(&conn, &rel).unwrap();

    // Delete entities under "docs/" — must succeed without FK error
    let deleted =
        sqlite_ops::delete_entities_by_paths(&conn, repo_id, &["docs".to_string()]).unwrap();
    assert_eq!(deleted, 1, "should delete exactly 1 entity");

    // del-a gone, del-b survives
    assert!(sqlite_ops::get_entity(&conn, "del-a").unwrap().is_none());
    assert!(sqlite_ops::get_entity(&conn, "del-b").unwrap().is_some());

    // Relationship from del-a must be cleaned up
    let to_b = sqlite_ops::relationships_to(&conn, "del-b").unwrap();
    assert!(
        to_b.is_empty(),
        "no incoming rels from deleted entity should remain"
    );
}

#[test]
fn test_upsert_relationship_persists_and_round_trips_provenance() {
    // Issue #714: provenance must round-trip through upsert_relationship and
    // both relationships_from/relationships_to readers, not silently default
    // or get dropped by the SQL column list.
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";

    let entity_a = Entity {
        id: "prov-a".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: "a.rs".to_string(),
        path: Some("src/a.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    let entity_b = Entity {
        id: "prov-b".to_string(),
        name: "b.rs".to_string(),
        path: Some("src/b.rs".to_string()),
        ..entity_a.clone()
    };
    sqlite_ops::upsert_entity(&conn, &entity_a, now).unwrap();
    sqlite_ops::upsert_entity(&conn, &entity_b, now).unwrap();

    let resolved_rel = Relationship {
        source_id: "prov-a".to_string(),
        target_id: "prov-b".to_string(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Resolved,
    };
    sqlite_ops::upsert_relationship(&conn, &resolved_rel).unwrap();

    let from_a = sqlite_ops::relationships_from(&conn, "prov-a").unwrap();
    assert_eq!(from_a.len(), 1);
    assert_eq!(from_a[0].0.provenance, EdgeProvenance::Resolved);

    let to_b = sqlite_ops::relationships_to(&conn, "prov-b").unwrap();
    assert_eq!(to_b.len(), 1);
    assert_eq!(to_b[0].0.provenance, EdgeProvenance::Resolved);

    let all = sqlite_ops::list_all_relationships(&conn, "proj-test").unwrap();
    let found = all
        .iter()
        .find(|r| r.source_id == "prov-a" && r.target_id == "prov-b")
        .expect("relationship should be present in list_all_relationships");
    assert_eq!(found.provenance, EdgeProvenance::Resolved);
}

#[test]
fn test_upsert_relationship_on_conflict_updates_provenance() {
    // Issue #714: UPSERT_RELATIONSHIP's ON CONFLICT clause must update
    // provenance, not just weight/evidence_json — otherwise a re-analysis that
    // re-derives an edge via a different (better) creation path would leave a
    // stale classification behind.
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";

    let entity_a = Entity {
        id: "conflict-a".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: "a.rs".to_string(),
        path: Some("src/a.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    let entity_b = Entity {
        id: "conflict-b".to_string(),
        name: "b.rs".to_string(),
        path: Some("src/b.rs".to_string()),
        ..entity_a.clone()
    };
    sqlite_ops::upsert_entity(&conn, &entity_a, now).unwrap();
    sqlite_ops::upsert_entity(&conn, &entity_b, now).unwrap();

    // First pass: heuristic classification (e.g. fn_map match)
    let heuristic_rel = Relationship {
        source_id: "conflict-a".to_string(),
        target_id: "conflict-b".to_string(),
        rel_type: RelType::Calls,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    sqlite_ops::upsert_relationship(&conn, &heuristic_rel).unwrap();

    let before = sqlite_ops::relationships_from(&conn, "conflict-a").unwrap();
    assert_eq!(before[0].0.provenance, EdgeProvenance::Heuristic);

    // Second pass: same edge key re-derived as resolved (e.g. resolver fix)
    let resolved_rel = Relationship {
        source_id: "conflict-a".to_string(),
        target_id: "conflict-b".to_string(),
        rel_type: RelType::Calls,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Resolved,
    };
    sqlite_ops::upsert_relationship(&conn, &resolved_rel).unwrap();

    let after = sqlite_ops::relationships_from(&conn, "conflict-a").unwrap();
    assert_eq!(
        after.len(),
        1,
        "same edge key must not duplicate the relationships row"
    );
    assert_eq!(
        after[0].0.provenance,
        EdgeProvenance::Resolved,
        "ON CONFLICT must update provenance to the re-derived classification"
    );
}
