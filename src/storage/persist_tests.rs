// Tests for persist_analysis_batch operation

use super::db_test_helpers::{
    create_in_memory_db, seed_repo_unresolved_counts, setup_analysis_batch,
};
use crate::model::*;
use crate::storage::sqlite::sqlite_ops;

#[test]
fn test_get_unresolved_counts_reads_repo_columns() {
    let conn = create_in_memory_db();
    seed_repo_unresolved_counts(&conn, "repo-test", 5, 3);

    let counts = sqlite_ops::get_unresolved_counts(&conn, "repo-test").unwrap();
    assert_eq!(counts, Some((5, 3)));
}

#[test]
fn test_get_unresolved_counts_zero_counts_are_not_null() {
    // Recorded zeros (fully-resolved fresh index) must read as Some((0, 0)),
    // never None — the two states must never look alike (#856 decision 2).
    let conn = create_in_memory_db();
    seed_repo_unresolved_counts(&conn, "repo-test", 0, 0);

    let counts = sqlite_ops::get_unresolved_counts(&conn, "repo-test").unwrap();
    assert_eq!(counts, Some((0, 0)));
}

#[test]
fn test_get_unresolved_counts_unrecorded_repo_is_none() {
    let conn = create_in_memory_db();
    // repo-test has no recorded counts (columns NULL) -> None, never a
    // fabricated zero.
    let counts = sqlite_ops::get_unresolved_counts(&conn, "repo-test").unwrap();
    assert_eq!(counts, None);
}

#[test]
fn test_get_unresolved_counts_unknown_repo_is_none() {
    let conn = create_in_memory_db();
    let counts = sqlite_ops::get_unresolved_counts(&conn, "nonexistent").unwrap();
    assert_eq!(counts, None);
}

#[test]
fn test_persist_analysis_batch_filters_dangling_relationships() {
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";
    let last_commit = "abc123";

    let (entity, run) = setup_analysis_batch(&conn);

    // Create two relationships:
    // 1. Valid: both endpoints exist in the entity set
    // 2. Dangling: target_id references an external entity not in the batch
    let valid_rel = Relationship {
        source_id: entity.id.clone(),
        target_id: entity.id.clone(), // Self-reference for simplicity
        rel_type: RelType::Contains,
        weight: 1.0,
        evidence_json: Some("{}".to_string()),
        provenance: EdgeProvenance::Resolved,
    };

    let dangling_rel = Relationship {
        source_id: entity.id.clone(),
        target_id: "external-lib-std-io".to_string(), // Not in entity set
        rel_type: RelType::DependsOn,
        weight: 1.0,
        evidence_json: Some("{}".to_string()),
        provenance: EdgeProvenance::Heuristic,
    };

    let entities = vec![&entity];
    let relationships = vec![valid_rel, dangling_rel];

    // Persist batch - should succeed without FK error
    let result = sqlite_ops::persist_analysis_batch(
        &conn,
        &entities,
        &relationships,
        &run,
        repo_id,
        last_commit,
        now,
    );

    assert!(
        result.is_ok(),
        "persist_analysis_batch should succeed without FK error"
    );

    let (entities_upserted, rels_upserted) = result.unwrap();
    assert_eq!(entities_upserted, 1, "should upsert 1 entity");
    assert_eq!(
        rels_upserted, 1,
        "should upsert only 1 relationship (the valid one)"
    );

    // Verify only the valid relationship was inserted
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1, "database should contain exactly 1 relationship");

    // Verify the valid relationship exists
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM relationships WHERE source_id = ? AND target_id = ?",
            [&entity.id, &entity.id],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(exists, "self-reference relationship should be in database");

    // Verify the dangling relationship does NOT exist
    let dangling_exists: bool = conn
        .query_row(
            "SELECT 1 FROM relationships WHERE target_id = 'external-lib-std-io'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(
        !dangling_exists,
        "dangling relationship should not be in database"
    );
}

/// A structural re-persist through persist_analysis_batch (summary = None) must NOT
/// wipe a summary that was stored by a prior pass. Issue #674: every existing persist
/// fixture used summary: None, which is exactly why the unconditional-overwrite wipe
/// was never caught. This test fails on main.
#[test]
fn test_persist_analysis_batch_preserves_existing_summaries() {
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";
    let last_commit = "abc123";

    let (entity, run) = setup_analysis_batch(&conn);

    // First pass: persist the entity WITH a real summary (as a summarization pass would).
    let summarized = Entity {
        summary: Some("apfel summary".to_string()),
        summary_commit: Some("commit-1".to_string()),
        ..entity.clone()
    };
    let entities_first = vec![&summarized];
    sqlite_ops::persist_analysis_batch(
        &conn,
        &entities_first,
        &[],
        &run,
        repo_id,
        last_commit,
        now,
    )
    .unwrap();

    let stored_after_first: Option<String> = conn
        .query_row(
            "SELECT summary FROM entities WHERE id = ?",
            [entity.id.clone()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_after_first.as_deref(), Some("apfel summary"));

    // Second pass: structural re-persist with summary = None (preserve_functions shape).
    let re_persisted = Entity {
        name: "test_module_renamed".to_string(),
        summary: None,
        summary_commit: None,
        ..entity
    };
    let entities_second = vec![&re_persisted];
    sqlite_ops::persist_analysis_batch(
        &conn,
        &entities_second,
        &[],
        &run,
        repo_id,
        last_commit,
        now,
    )
    .unwrap();

    let (summary_after, commit_after, name_after) = conn
        .query_row(
            "SELECT summary, summary_commit, name FROM entities WHERE id = ?",
            [re_persisted.id.clone()],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();

    assert_eq!(
        summary_after.as_deref(),
        Some("apfel summary"),
        "persist_analysis_batch with a NULL incoming summary must not wipe a stored summary"
    );
    assert_eq!(
        commit_after.as_deref(),
        Some("commit-1"),
        "summary_commit must survive a NULL-summary re-persist"
    );
    assert_eq!(
        name_after, "test_module_renamed",
        "non-summary columns must still update on re-persist"
    );
}

#[test]
fn test_persist_analysis_batch_filters_source_dangling() {
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";
    let last_commit = "abc123";

    let (entity, run) = setup_analysis_batch(&conn);

    // Relationship with dangling source (source_id not in entity set)
    let rel = Relationship {
        source_id: "external-lib-core".to_string(), // Not in entity set
        target_id: entity.id.clone(),
        rel_type: RelType::Calls,
        weight: 1.0,
        evidence_json: Some("{}".to_string()),
        provenance: EdgeProvenance::Heuristic,
    };

    let entities = vec![&entity];
    let relationships = vec![rel];

    let result = sqlite_ops::persist_analysis_batch(
        &conn,
        &entities,
        &relationships,
        &run,
        repo_id,
        last_commit,
        now,
    );

    assert!(result.is_ok());
    let (_, rels_upserted) = result.unwrap();
    assert_eq!(
        rels_upserted, 0,
        "should upsert 0 relationships when source is missing"
    );

    // Verify no relationships exist
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "no relationships should be inserted");
}

#[test]
fn test_persist_analysis_batch_filters_both_endpoints_dangling() {
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";
    let last_commit = "abc123";

    let (entity, run) = setup_analysis_batch(&conn);

    // Relationship with both endpoints external
    let rel = Relationship {
        source_id: "external-lib-std".to_string(),
        target_id: "external-lib-core".to_string(),
        rel_type: RelType::DependsOn,
        weight: 1.0,
        evidence_json: Some("{}".to_string()),
        provenance: EdgeProvenance::Heuristic,
    };

    let entities = vec![&entity];
    let relationships = vec![rel];

    let result = sqlite_ops::persist_analysis_batch(
        &conn,
        &entities,
        &relationships,
        &run,
        repo_id,
        last_commit,
        now,
    );

    assert!(result.is_ok());
    let (_, rels_upserted) = result.unwrap();
    assert_eq!(
        rels_upserted, 0,
        "should upsert 0 relationships when both endpoints are missing"
    );
}

#[test]
fn test_persist_analysis_batch_valid_cross_entity_relationships() {
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";
    let last_commit = "abc123";

    let (entity1, run) = setup_analysis_batch(&conn);

    let entity2 = Entity {
        id: "entity-2".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "test_module_2".to_string(),
        path: Some("src/test_module_2.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };

    // Valid cross-entity relationship
    let rel = Relationship {
        source_id: entity1.id.clone(),
        target_id: entity2.id.clone(),
        rel_type: RelType::Imports,
        weight: 1.0,
        evidence_json: Some("{}".to_string()),
        provenance: EdgeProvenance::Resolved,
    };

    let entities = vec![&entity1, &entity2];
    let relationships = vec![rel];

    let result = sqlite_ops::persist_analysis_batch(
        &conn,
        &entities,
        &relationships,
        &run,
        repo_id,
        last_commit,
        now,
    );

    assert!(result.is_ok());
    let (entities_upserted, rels_upserted) = result.unwrap();
    assert_eq!(entities_upserted, 2, "should upsert both entities");
    assert_eq!(
        rels_upserted, 1,
        "should upsert 1 valid cross-entity relationship"
    );
}

#[test]
fn test_persist_analysis_batch_parent_id_fk_constraint() {
    // Regression test: entities with parent_id referencing other entities in the
    // same batch must be inserted in parent-first order (subsystem → module → file)
    // to satisfy the entities.parent_id REFERENCES entities(id) FK constraint.
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";
    let last_commit = "abc123";

    // Create analysis run
    conn.execute(
        "INSERT INTO analysis_runs (id, repo_id, commit_hash, status) VALUES ('run-parent', 'repo-test', 'abc123', 'pending')",
        [],
    ).unwrap();

    // Subsystem entity (no parent)
    let subsystem = Entity {
        id: "subsystem-src".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Subsystem,
        parent_id: None,
        name: "src".to_string(),
        path: Some("src".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };

    // Module entity (parent = subsystem)
    let module = Entity {
        id: "module-src-main".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Module,
        parent_id: Some("subsystem-src".to_string()),
        name: "main".to_string(),
        path: Some("src/main".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };

    // File entity (parent = module)
    let file = Entity {
        id: "file-src-main-rs".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::File,
        parent_id: Some("module-src-main".to_string()),
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };

    let run = AnalysisRun {
        id: "run-parent".to_string(),
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

    // Hierarchy relationship: subsystem CONTAINS module, module CONTAINS file
    let rel1 = Relationship {
        source_id: "subsystem-src".to_string(),
        target_id: "module-src-main".to_string(),
        rel_type: RelType::Contains,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    let rel2 = Relationship {
        source_id: "module-src-main".to_string(),
        target_id: "file-src-main-rs".to_string(),
        rel_type: RelType::Contains,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };

    // Order: subsystem → module → file (parents before children)
    let entities: Vec<&Entity> = vec![&subsystem, &module, &file];
    let relationships = vec![rel1, rel2];

    let result = sqlite_ops::persist_analysis_batch(
        &conn,
        &entities,
        &relationships,
        &run,
        repo_id,
        last_commit,
        now,
    );

    assert!(
        result.is_ok(),
        "persist_analysis_batch with parent_id hierarchy should succeed"
    );
    let (entities_upserted, rels_upserted) = result.unwrap();
    assert_eq!(entities_upserted, 3, "should upsert 3 entities");
    assert_eq!(rels_upserted, 2, "should upsert 2 relationships");

    // Verify parent_id values are stored correctly
    let module_parent: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM entities WHERE id = 'module-src-main'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        module_parent,
        Some("subsystem-src".to_string()),
        "module parent_id should reference subsystem"
    );

    let file_parent: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM entities WHERE id = 'file-src-main-rs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        file_parent,
        Some("module-src-main".to_string()),
        "file parent_id should reference module"
    );
}

#[test]
fn test_persist_analysis_batch_parent_id_reverse_order_fails() {
    // Verify that inserting entities in child-before-parent order causes FK
    // constraint failure on parent_id. This proves the insertion order matters.
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";
    let last_commit = "abc123";

    // Create analysis run
    conn.execute(
        "INSERT INTO analysis_runs (id, repo_id, commit_hash, status) VALUES ('run-reverse', 'repo-test', 'abc123', 'pending')",
        [],
    ).unwrap();

    let subsystem = Entity {
        id: "subsystem-app".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Subsystem,
        parent_id: None,
        name: "app".to_string(),
        path: Some("app".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };

    let module = Entity {
        id: "module-app-models".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Module,
        parent_id: Some("subsystem-app".to_string()),
        name: "models".to_string(),
        path: Some("app/models".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };

    let run = AnalysisRun {
        id: "run-reverse".to_string(),
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

    // Deliberately insert in WRONG order: child (module) before parent (subsystem)
    let entities: Vec<&Entity> = vec![&module, &subsystem];
    let relationships: Vec<Relationship> = vec![];

    let result = sqlite_ops::persist_analysis_batch(
        &conn,
        &entities,
        &relationships,
        &run,
        repo_id,
        last_commit,
        now,
    );

    // This MUST fail — module references subsystem via parent_id, but subsystem
    // hasn't been inserted yet at the point module is upserted.
    assert!(
        result.is_err(),
        "persist_analysis_batch with child-before-parent order must fail FK constraint"
    );
}
