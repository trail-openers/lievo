// Tests for SearchEntitiesTool (issue #119).
//
// Split from tools_tests.rs for file size management.

use serde_json::json;

use crate::model::{Entity, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

// In the test module: super = tools_impl, super::super = tools
use super::super::SearchEntitiesTool;
use super::tools_tests_helpers::{make_ctx, setup_storage, test_entity};

#[test]
fn test_search_entities_finds_by_name() {
    let (storage, project_id) = setup_storage();
    storage
        .upsert_entity(&test_entity(
            "e1",
            "auth_module",
            Some("src/auth.rs"),
            &project_id,
        ))
        .unwrap();
    storage
        .upsert_entity(&test_entity(
            "e2",
            "database_layer",
            Some("src/db.rs"),
            &project_id,
        ))
        .unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = SearchEntitiesTool { ctx };

    let result = tool.call(json!({"query": "auth"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "auth_module");
    assert_eq!(results[0]["entity_id"], "e1");
}

#[test]
fn test_search_entities_finds_by_path() {
    let (storage, project_id) = setup_storage();
    storage
        .upsert_entity(&test_entity(
            "e1",
            "module_a",
            Some("src/storage/mod.rs"),
            &project_id,
        ))
        .unwrap();
    storage
        .upsert_entity(&test_entity(
            "e2",
            "module_b",
            Some("src/analysis.rs"),
            &project_id,
        ))
        .unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = SearchEntitiesTool { ctx };

    let result = tool.call(json!({"query": "storage"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["entity_id"], "e1");
}

#[test]
fn test_search_entities_limit_respected() {
    let (storage, project_id) = setup_storage();
    for i in 0..5 {
        storage
            .upsert_entity(&test_entity(
                &format!("e{i}"),
                &format!("target_{i}"),
                None,
                &project_id,
            ))
            .unwrap();
    }

    let ctx = make_ctx(storage, project_id);
    let tool = SearchEntitiesTool { ctx };

    let result = tool.call(json!({"query": "target", "limit": 2})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["results"].as_array().unwrap().len(), 2);
}

#[test]
fn test_search_entities_missing_query_returns_error() {
    let (storage, project_id) = setup_storage();
    let ctx = make_ctx(storage, project_id);
    let tool = SearchEntitiesTool { ctx };

    assert!(tool.call(json!({})).is_err());
}

#[test]
fn test_search_entities_finds_by_name_and_path_only() {
    // Issue #408 removed summary from search. Search now only matches by name or path.
    // Verify that summary content does NOT trigger a match when name/path don't match.
    let (storage, project_id) = setup_storage();
    let entity = Entity {
        id: "e1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "foo_module".to_string(),
        path: Some("src/foo.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: Some("Handles user authentication and session management".to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&entity).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = SearchEntitiesTool { ctx };

    // Query matches summary but NOT name or path - should return empty
    let result = tool.call(json!({"query": "authentication"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(
        results.len(),
        0,
        "expected no match since summary is not searched"
    );
}

#[test]
fn test_search_entities_non_semantic_returns_summary_snippet() {
    // Issue #523: verify that non-semantic search returns summary in snippet field
    let (storage, project_id) = setup_storage();
    let entity = Entity {
        id: "e1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "auth_module".to_string(),
        path: Some("src/auth.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: Some("Handles user authentication and session management".to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&entity).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = SearchEntitiesTool { ctx };

    // Query matches name (non-semantic path)
    let result = tool.call(json!({"query": "auth"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    // Verify snippet contains summary, not just path
    assert_eq!(
        results[0]["snippet"], "Handles user authentication and session management",
        "snippet should contain summary content"
    );
}

#[test]
fn test_search_entities_snippet_cascades_to_path_when_summary_none() {
    // Verify that when summary is None, snippet falls back to path
    let (storage, project_id) = setup_storage();
    let entity = Entity {
        id: "e1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "auth_module".to_string(),
        path: Some("src/auth.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None, // No summary
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&entity).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = SearchEntitiesTool { ctx };

    let result = tool.call(json!({"query": "auth"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["snippet"], "src/auth.rs",
        "snippet should fall back to path when summary is None"
    );
}

#[test]
fn test_search_entities_snippet_cascades_to_name_when_summary_and_path_none() {
    // Verify that when both summary and path are None, snippet falls back to name
    let (storage, project_id) = setup_storage();
    let entity = Entity {
        id: "e1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "auth_module".to_string(),
        path: None, // No path
        language: Some("Rust".to_string()),
        summary: None, // No summary
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&entity).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = SearchEntitiesTool { ctx };

    let result = tool.call(json!({"query": "auth"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["snippet"], "auth_module",
        "snippet should fall back to name when both summary and path are None"
    );
}

#[test]
fn test_search_entities_snippet_cascades_to_path_when_summary_empty() {
    // Verify that when summary is Some(""), snippet falls back to path (not empty string)
    let (storage, project_id) = setup_storage();
    let entity = Entity {
        id: "e1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "auth_module".to_string(),
        path: Some("src/auth.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: Some("".to_string()), // Empty summary
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&entity).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = SearchEntitiesTool { ctx };

    let result = tool.call(json!({"query": "auth"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let results = parsed["results"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["snippet"], "src/auth.rs",
        "snippet should fall back to path when summary is empty string"
    );
}
