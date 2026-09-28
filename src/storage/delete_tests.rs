// Tests for delete_entities_by_paths operation

use super::db_test_helpers::create_in_memory_db;
use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use crate::storage::sqlite::sqlite_ops;

#[test]
fn test_delete_entities_by_paths_removes_relationships_before_entities() {
    // Regression test: delete_entities_by_paths must delete relationships
    // referencing an entity BEFORE deleting the entity itself, otherwise
    // the FOREIGN KEY constraint on relationships.(source_id|target_id)
    // REFERENCES entities(id) causes a constraint violation.
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";
    let repo_id = "repo-test";

    // Module entity (parent) with path in excluded dir
    let module_entity = Entity {
        id: "mod-1".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "docs_module".to_string(),
        path: Some("docs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    // File entity (child) also in excluded dir
    let file_entity = Entity {
        id: "file-1".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::File,
        parent_id: Some("mod-1".to_string()),
        name: "index.md".to_string(),
        path: Some("docs/index.md".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    // Kept entity outside excluded paths
    let kept_entity = Entity {
        id: "kept-1".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    sqlite_ops::upsert_entity(&conn, &module_entity, now).unwrap();
    sqlite_ops::upsert_entity(&conn, &file_entity, now).unwrap();
    sqlite_ops::upsert_entity(&conn, &kept_entity, now).unwrap();

    // Create relationships: module CONTAINS file, file DEPENDS_ON kept
    let contains_rel = Relationship {
        source_id: "mod-1".to_string(),
        target_id: "file-1".to_string(),
        rel_type: RelType::Contains,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    let depends_rel = Relationship {
        source_id: "file-1".to_string(),
        target_id: "kept-1".to_string(),
        rel_type: RelType::DependsOn,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    sqlite_ops::upsert_relationship(&conn, &contains_rel).unwrap();
    sqlite_ops::upsert_relationship(&conn, &depends_rel).unwrap();

    // Verify setup: both relationships exist
    let from_mod = sqlite_ops::relationships_from(&conn, "mod-1").unwrap();
    assert_eq!(from_mod.len(), 1, "module should have 1 relationship");
    let from_file = sqlite_ops::relationships_from(&conn, "file-1").unwrap();
    assert_eq!(from_file.len(), 1, "file should have 1 relationship");

    // Delete excluded entities — must remove relationships first
    let exclude_paths = vec!["docs".to_string()];
    let deleted_count =
        sqlite_ops::delete_entities_by_paths(&conn, repo_id, &exclude_paths).unwrap();

    // Both the module and file in "docs" should be deleted
    assert_eq!(deleted_count, 2);
    assert!(sqlite_ops::get_entity(&conn, "mod-1").unwrap().is_none());
    assert!(sqlite_ops::get_entity(&conn, "file-1").unwrap().is_none());
    assert!(sqlite_ops::get_entity(&conn, "kept-1").unwrap().is_some());

    // The kept entity should still have no dangling relationships
    let to_kept = sqlite_ops::relationships_to(&conn, "kept-1").unwrap();
    assert!(
        to_kept.is_empty(),
        "kept entity should have no incoming relationships after docs entities deleted"
    );
}
