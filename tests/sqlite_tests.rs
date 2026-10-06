// Integration tests for SqliteStorage that require the full storage stack.
use lievo::model::{AnalysisRun, AnalysisStatus, Entity, EntityTier, RelType, Relationship};
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;

/// `persist_analysis_batch` must atomically persist entities, update the analysis
/// run record, and record the last analyzed commit in a single transaction.
#[test]
fn test_persist_analysis_batch_atomic() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();

    let entity = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let run = AnalysisRun {
        id: "run-1".to_string(),
        repo_id: repo.id.clone(),
        commit_hash: "abc123".to_string(),
        files_analyzed: 1,
        files_changed: 1,
        entities_upserted: 1,
        relationships_upserted: 0,
        duration_ms: Some(42),
        status: AnalysisStatus::Completed,
        completed_at: Some("2024-01-01T00:00:01Z".to_string()),
    };
    storage.create_analysis_run(&repo.id, "abc123").unwrap();

    let (entities_upserted, rels_upserted) = storage
        .persist_analysis_batch(&[&entity], &[], &run, &repo.id, "abc123")
        .unwrap();

    assert_eq!(entities_upserted, 1);
    assert_eq!(rels_upserted, 0);

    // Verify entity was persisted
    let fetched = storage.get_entity("e1").unwrap().unwrap();
    assert_eq!(fetched.name, "main.rs");

    // Verify last commit was updated atomically with the entity upsert
    let fetched_repo = storage.get_repo(&repo.id).unwrap().unwrap();
    assert_eq!(fetched_repo.last_analyzed_commit.as_deref(), Some("abc123"));
}

/// `delete_entities_by_paths` must also delete relationships referencing deleted entities
/// to prevent orphaned relationship rows.
#[test]
fn test_delete_entities_by_paths_clears_relationships() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();

    // Create two entities: one to be deleted, one to keep
    let entity_to_delete = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "delete_me.rs".to_string(),
        path: Some("src/delete_me.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let entity_to_keep = Entity {
        id: "e2".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "keep_me.rs".to_string(),
        path: Some("src/keep_me.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let run = AnalysisRun {
        id: "run-1".to_string(),
        repo_id: repo.id.clone(),
        commit_hash: "abc123".to_string(),
        files_analyzed: 2,
        files_changed: 0,
        entities_upserted: 2,
        relationships_upserted: 0,
        duration_ms: Some(42),
        status: AnalysisStatus::Completed,
        completed_at: Some("2024-01-01T00:00:01Z".to_string()),
    };

    // Persist entities and create a relationship between them
    storage
        .persist_analysis_batch(
            &[&entity_to_delete, &entity_to_keep],
            &[Relationship {
                source_id: "e1".to_string(),
                target_id: "e2".to_string(),
                rel_type: RelType::Imports,
                weight: 1.0,
                evidence_json: None,
                provenance: lievo::model::EdgeProvenance::Heuristic,
            }],
            &run,
            &repo.id,
            "abc123",
        )
        .unwrap();

    // Verify relationship exists
    let rels_from_e1 = storage.relationships_from("e1").unwrap();
    assert_eq!(rels_from_e1.len(), 1);

    // Delete entity_to_delete by path
    let deleted = storage
        .delete_entities_by_paths(
            &repo.id,
            &["src/delete_me.rs".to_string(), "delete_me.rs".to_string()],
        )
        .unwrap();
    assert_eq!(deleted, 1);

    // Verify entity was deleted
    assert!(storage.get_entity("e1").unwrap().is_none());
    assert!(storage.get_entity("e2").unwrap().is_some());

    // Verify relationship was also deleted (no orphaned relationships)
    let rels_from_e1 = storage.relationships_from("e1").unwrap();
    assert_eq!(
        rels_from_e1.len(),
        0,
        "orphaned relationships should be deleted"
    );

    // Verify no relationships reference the deleted entity
    let all_rels = storage.list_all_relationships(&project.id).unwrap();
    for rel in all_rels {
        assert_ne!(
            rel.source_id, "e1",
            "source_id should not reference deleted entity"
        );
        assert_ne!(
            rel.target_id, "e1",
            "target_id should not reference deleted entity"
        );
    }
}

/// `clear_all_summaries` must be called in Full reindex mode to clear all entity summaries.
#[test]
fn test_reindex_full_clears_summaries() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();

    // Create entity with a summary
    let entity = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: Some("This is a test summary".to_string()),
        summary_commit: Some("abc123".to_string()),
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let run = AnalysisRun {
        id: "run-1".to_string(),
        repo_id: repo.id.clone(),
        commit_hash: "abc123".to_string(),
        files_analyzed: 1,
        files_changed: 0,
        entities_upserted: 1,
        relationships_upserted: 0,
        duration_ms: Some(42),
        status: AnalysisStatus::Completed,
        completed_at: Some("2024-01-01T00:00:01Z".to_string()),
    };

    storage
        .persist_analysis_batch(&[&entity], &[], &run, &repo.id, "abc123")
        .unwrap();

    // Verify entity has a summary
    let fetched = storage.get_entity("e1").unwrap().unwrap();
    assert!(fetched.summary.is_some());
    assert_eq!(fetched.summary.as_deref(), Some("This is a test summary"));

    // Clear all summaries (what Full reindex mode does)
    storage.clear_all_summaries(&project.id).unwrap();

    // Verify summary was cleared
    let fetched = storage.get_entity("e1").unwrap().unwrap();
    assert!(
        fetched.summary.is_none(),
        "summary should be cleared in Full mode"
    );
    assert!(
        fetched.summary_commit.is_none(),
        "summary_commit should be cleared in Full mode"
    );
}

/// `delete_entities_by_paths` must clean up relationships even when there's no CASCADE.
/// This regression test ensures relationships are explicitly deleted when entities are deleted.
#[test]
fn test_delete_entities_removes_relationships_without_cascade() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();

    // Create two entities with relationships
    let entity1 = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "file1.rs".to_string(),
        path: Some("src/file1.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let entity2 = Entity {
        id: "e2".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "file2.rs".to_string(),
        path: Some("src/file2.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let run = AnalysisRun {
        id: "run-1".to_string(),
        repo_id: repo.id.clone(),
        commit_hash: "abc123".to_string(),
        files_analyzed: 2,
        files_changed: 0,
        entities_upserted: 2,
        relationships_upserted: 0,
        duration_ms: Some(42),
        status: AnalysisStatus::Completed,
        completed_at: Some("2024-01-01T00:00:01Z".to_string()),
    };

    // Persist entities and create relationships
    storage
        .persist_analysis_batch(
            &[&entity1, &entity2],
            &[
                Relationship {
                    source_id: "e1".to_string(),
                    target_id: "e2".to_string(),
                    rel_type: RelType::Imports,
                    weight: 1.0,
                    evidence_json: None,
                    provenance: lievo::model::EdgeProvenance::Heuristic,
                },
                Relationship {
                    source_id: "e2".to_string(),
                    target_id: "e1".to_string(),
                    rel_type: RelType::DependsOn,
                    weight: 1.0,
                    evidence_json: None,
                    provenance: lievo::model::EdgeProvenance::Heuristic,
                },
            ],
            &run,
            &repo.id,
            "abc123",
        )
        .unwrap();

    // Verify relationships exist before deletion
    let rels_from_e1 = storage.relationships_from("e1").unwrap();
    assert_eq!(
        rels_from_e1.len(),
        1,
        "e1 should have 1 outgoing relationship"
    );

    let rels_to_e1 = storage.relationships_to("e1").unwrap();
    assert_eq!(
        rels_to_e1.len(),
        1,
        "e1 should have 1 incoming relationship"
    );

    // Delete entity1
    let deleted = storage
        .delete_entities_by_paths(&repo.id, &["src/file1.rs".to_string()])
        .unwrap();
    assert_eq!(deleted, 1);

    // Verify entity was deleted
    assert!(storage.get_entity("e1").unwrap().is_none());
    assert!(storage.get_entity("e2").unwrap().is_some());

    // Verify all relationships referencing deleted entity are gone
    let all_rels = storage.list_all_relationships(&project.id).unwrap();
    for rel in all_rels {
        assert_ne!(
            rel.source_id, "e1",
            "source_id should not reference deleted entity"
        );
        assert_ne!(
            rel.target_id, "e1",
            "target_id should not reference deleted entity"
        );
    }
}

/// `delete_entities_by_paths` with an empty list should be a no-op, not a SQL error.
#[test]
fn test_delete_entities_empty_list_is_noop() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();

    // Create an entity to verify empty delete doesn't affect anything
    let entity = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    storage.upsert_entity(&entity).unwrap();

    // Verify entity exists before
    assert!(storage.get_entity("e1").unwrap().is_some());

    // Call delete with empty list - should not panic or error
    let result = storage.delete_entities_by_paths(&repo.id, &[]);
    assert!(result.is_ok(), "empty delete should succeed");
    assert_eq!(result.unwrap(), 0, "should delete 0 entities");

    // Verify entity still exists (no changes)
    assert!(storage.get_entity("e1").unwrap().is_some());
}

/// `clear_repo_summaries` only clears summaries for the specified repo, not other repos.
#[test]
fn test_clear_repo_summaries_is_repo_scoped() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();

    // Create two repos in the same project
    let repo1 = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();
    let repo2 = storage
        .add_repo(&project.id, "repo2", "/path/to/repo2")
        .unwrap();

    // Create entities in both repos with summaries
    let entity1 = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo1.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "file1.rs".to_string(),
        path: Some("src/file1.rs".to_string()),
        language: Some("rust".to_string()),
        summary: Some("Summary for repo1".to_string()),
        summary_commit: Some("commit1".to_string()),
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let entity2 = Entity {
        id: "e2".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo2.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "file2.rs".to_string(),
        path: Some("src/file2.rs".to_string()),
        language: Some("rust".to_string()),
        summary: Some("Summary for repo2".to_string()),
        summary_commit: Some("commit2".to_string()),
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    storage.upsert_entity(&entity1).unwrap();
    storage.upsert_entity(&entity2).unwrap();

    // Verify both have summaries
    let fetched1 = storage.get_entity("e1").unwrap().unwrap();
    let fetched2 = storage.get_entity("e2").unwrap().unwrap();
    assert_eq!(fetched1.summary, Some("Summary for repo1".to_string()));
    assert_eq!(fetched2.summary, Some("Summary for repo2".to_string()));

    // Clear summaries only for repo1
    storage.clear_repo_summaries(&repo1.id).unwrap();

    // Verify repo1 entity has null summaries
    let fetched1_after = storage.get_entity("e1").unwrap().unwrap();
    assert_eq!(
        fetched1_after.summary, None,
        "repo1 summaries should be cleared"
    );
    assert_eq!(
        fetched1_after.summary_commit, None,
        "repo1 summary_commit should be cleared"
    );

    // Verify repo2 entity still has its summaries
    let fetched2_after = storage.get_entity("e2").unwrap().unwrap();
    assert_eq!(
        fetched2_after.summary,
        Some("Summary for repo2".to_string()),
        "repo2 summaries should not be affected"
    );
    assert_eq!(
        fetched2_after.summary_commit,
        Some("commit2".to_string()),
        "repo2 summary_commit should not be affected"
    );
}

// --- issue #31: repository identity round-trips via the public Storage API ---

#[test]
fn set_repo_git_url_then_find_repos_by_git_url_roundtrips() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();

    let key = "https://github.com/test/repo1.git";
    storage.set_repo_git_url(&repo.id, key).unwrap();

    // get_repo reflects the persisted key.
    let fetched = storage.get_repo(&repo.id).unwrap().unwrap();
    assert_eq!(fetched.git_url.as_deref(), Some(key));

    // find_repos_by_git_url returns the repo (cross-project Vec lookup).
    let found = storage.find_repos_by_git_url(key).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, repo.id);
    assert_eq!(found[0].git_url.as_deref(), Some(key));
}

// --- #764: list_all_relationships project-scoping regression ---

#[test]
fn list_all_relationships_does_not_return_cross_project_edges() {
    // Two projects in one database. Project A has a→b (Calls). Project B has
    // a cross-project edge foreign→b (foreign in projB, b in projA).
    // list_all_relationships(projA) must NOT return the cross-project edge;
    // list_all_relationships(projB) must not return the same-project a→b edge.
    use lievo::model::EdgeProvenance;

    let storage = SqliteStorage::open_in_memory().unwrap();
    let pa = storage.create_project("lar-a", None).unwrap();
    let pb = storage.create_project("lar-b", None).unwrap();

    // Project A entities.
    let a = Entity {
        id: "pa:a".into(),
        project_id: pa.id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: None,
        name: "a".into(),
        path: Some("a.rs".into()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "now".into(),
        updated_at: "now".into(),
    };
    let b = Entity {
        id: "pa:b".into(),
        project_id: pa.id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: None,
        name: "b".into(),
        path: Some("b.rs".into()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "now".into(),
        updated_at: "now".into(),
    };
    // Project B entity.
    let foreign = Entity {
        id: "pb:foreign".into(),
        project_id: pb.id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: None,
        name: "foreign".into(),
        path: Some("foreign.rs".into()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "now".into(),
        updated_at: "now".into(),
    };
    for e in [&a, &b, &foreign] {
        storage.upsert_entity(e).unwrap();
    }
    // Same-project edge: a → b (both in projA).
    storage
        .upsert_relationship(&Relationship {
            source_id: "pa:a".into(),
            target_id: "pa:b".into(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Resolved,
        })
        .unwrap();
    // Cross-project edge: foreign (projB) → b (projA).
    storage
        .upsert_relationship(&Relationship {
            source_id: "pb:foreign".into(),
            target_id: "pa:b".into(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        })
        .unwrap();

    // Project A sees only its own edge.
    let rels_a = storage.list_all_relationships(&pa.id).unwrap();
    assert_eq!(rels_a.len(), 1, "projA must see exactly 1 edge: {rels_a:?}");
    assert_eq!(rels_a[0].source_id, "pa:a");
    assert_eq!(rels_a[0].target_id, "pa:b");

    // Project B sees zero edges (the cross-project edge is not B's edge
    // — both endpoints must be in B for it to appear).
    let rels_b = storage.list_all_relationships(&pb.id).unwrap();
    assert_eq!(
        rels_b.len(),
        0,
        "projB must see 0 edges (the cross-project edge has one endpoint in projA): {rels_b:?}"
    );
}
