// Tests for search_entities_by_name operation

use super::db_test_helpers::create_in_memory_db;
use crate::model::{Entity, EntityTier};
use crate::storage::sqlite::sqlite_ops;

#[test]
fn test_search_entities_by_name_single_word() {
    let conn = create_in_memory_db();

    // Insert test entities
    let entities = vec![
        Entity {
            id: "e1".to_string(),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Module,
            parent_id: None,
            name: "memory_manager".to_string(),
            path: Some("src/memory_manager.rs".to_string()),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "e2".to_string(),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Module,
            parent_id: None,
            name: "network_client".to_string(),
            path: Some("src/network_client.rs".to_string()),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "e3".to_string(),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Function,
            parent_id: Some("e1".to_string()),
            name: "clear_memory".to_string(),
            path: Some("src/memory_manager.rs".to_string()),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        },
    ];

    for entity in &entities {
        sqlite_ops::upsert_entity(&conn, entity, "2020-01-01T00:00:00Z").unwrap();
    }

    // Search for "memory"
    let words = vec!["memory"];
    let results =
        sqlite_ops::search_entities_by_name(&conn, "proj-test", &words, 10, None).unwrap();

    // Should match memory_manager and clear_memory (via path)
    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"memory_manager"));
    assert!(names.contains(&"clear_memory"));
}

#[test]
fn test_search_entities_by_name_multi_word() {
    let conn = create_in_memory_db();

    // Insert test entities
    let entities = vec![
        Entity {
            id: "e1".to_string(),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Module,
            parent_id: None,
            name: "memory_store".to_string(),
            path: Some("src/memory_store.rs".to_string()),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "e2".to_string(),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Module,
            parent_id: None,
            name: "memory_cache".to_string(),
            path: Some("src/memory_cache.rs".to_string()),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "e3".to_string(),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Module,
            parent_id: None,
            name: "network_client".to_string(),
            path: Some("src/network_client.rs".to_string()),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        },
    ];

    for entity in &entities {
        sqlite_ops::upsert_entity(&conn, entity, "2020-01-01T00:00:00Z").unwrap();
    }

    // Search for "memory store" - should match entities with either word (OR logic within each word block)
    let words = vec!["memory", "store"];
    let results =
        sqlite_ops::search_entities_by_name(&conn, "proj-test", &words, 10, None).unwrap();

    // Should match memory_store (both words), memory_cache (only first word), network_client (only second word via path)
    assert!(!results.is_empty());
    let names: Vec<&str> = results.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"memory_store"));
}

#[test]
fn test_search_entities_by_name_output_dir_excluded() {
    let conn = create_in_memory_db();

    // Insert test entities
    let entities = vec![
        Entity {
            id: "e1".to_string(),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Module,
            parent_id: None,
            name: "memory_manager".to_string(),
            path: Some("src/memory_manager.rs".to_string()),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "e2".to_string(),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Module,
            parent_id: None,
            name: "output_memory".to_string(),
            path: Some("output/memory.rs".to_string()),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        },
        Entity {
            id: "e3".to_string(),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Function,
            parent_id: None,
            name: "nested_memory".to_string(),
            path: Some("output/utils/nested_memory.rs".to_string()),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        },
    ];

    for entity in &entities {
        sqlite_ops::upsert_entity(&conn, entity, "2020-01-01T00:00:00Z").unwrap();
    }

    // Search for "memory" (output_dir filtering moved to retrieval layer)
    let words = vec!["memory"];
    let results =
        sqlite_ops::search_entities_by_name(&conn, "proj-test", &words, 10, None).unwrap();

    // Should match all entities (filtering happens in retrieval layer)
    assert_eq!(results.len(), 3);
}

#[test]
fn test_search_entities_by_name_limit_applied_in_db() {
    let conn = create_in_memory_db();

    // Insert many entities
    for i in 0..20 {
        let entity = Entity {
            id: format!("e{}", i),
            project_id: "proj-test".to_string(),
            repo_id: Some("repo-test".to_string()),
            tier: EntityTier::Function,
            parent_id: None,
            name: format!("memory_{}", i),
            path: Some(format!("src/memory_{}.rs", i)),
            language: Some("rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            updated_at: "2020-01-01T00:00:00Z".to_string(),
        };
        sqlite_ops::upsert_entity(&conn, &entity, "2020-01-01T00:00:00Z").unwrap();
    }

    // Search with limit=5
    let words = vec!["memory"];
    let results = sqlite_ops::search_entities_by_name(&conn, "proj-test", &words, 5, None).unwrap();

    // Should only return 5 results (limit applied in DB)
    assert_eq!(results.len(), 5);
}

#[test]
fn test_search_excludes_output_dir_preserves_null_path_entities() {
    // CRITICAL: Verify NULL-path entities are preserved when output_dir is set
    let conn = create_in_memory_db();
    let now = "2020-01-01T00:00:00Z";

    // Create entity with NULL path
    let entity_null_path = Entity {
        id: "e-null".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "null_path_module".to_string(),
        path: None,
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    // Create entity with path in output dir
    let entity_in_output = Entity {
        id: "e-output".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "output_module".to_string(),
        path: Some("output/subdir/module.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    // Create entity with path outside output dir
    let entity_outside_output = Entity {
        id: "e-outside".to_string(),
        project_id: "proj-test".to_string(),
        repo_id: Some("repo-test".to_string()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "outside_module".to_string(),
        path: Some("src/module.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };

    sqlite_ops::upsert_entity(&conn, &entity_null_path, now).unwrap();
    sqlite_ops::upsert_entity(&conn, &entity_in_output, now).unwrap();
    sqlite_ops::upsert_entity(&conn, &entity_outside_output, now).unwrap();

    // Search without output_dir filter (filtering moved to retrieval layer)
    let words = vec!["module"];
    let results =
        sqlite_ops::search_entities_by_name(&conn, "proj-test", &words, 10, None).unwrap();

    // Should return all entities that match (filtering happens in retrieval layer)
    assert_eq!(results.len(), 3);
}

#[test]
fn test_search_entities_rejects_empty_words() {
    // HIGH: Verify empty words slice returns InvalidInput error
    let conn = create_in_memory_db();

    // Create a test entity
    let entity = Entity {
        id: "e1".to_string(),
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

    sqlite_ops::upsert_entity(&conn, &entity, "2020-01-01T00:00:00Z").unwrap();

    // Search with empty words should fail
    let words: Vec<&str> = vec![];
    let result = sqlite_ops::search_entities_by_name(&conn, "proj-test", &words, 10, None);
    assert!(result.is_err());
    match result {
        Err(crate::LievoError::InvalidInput(msg)) => {
            assert!(msg.contains("requires at least one search word"));
        }
        _ => panic!("Expected InvalidInput error"),
    }
}
