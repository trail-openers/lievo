// Tests for GetEntityTool basic behavior and non-Module tiers (issue #119).
//
// Covers: not found, found, Subsystem, File entity behavior.
// For Module-specific nested function tests, see tools_tests_get_entity_module.rs

use serde_json::json;

use crate::model::{Entity, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

// In the test module: super = tools_impl, super::super = tools
use super::super::GetEntityTool;
use super::tools_tests_helpers::{make_ctx, setup_storage, test_entity};

#[test]
fn test_get_entity_not_found() {
    let (storage, project_id) = setup_storage();
    let ctx = make_ctx(storage, project_id);
    let tool = GetEntityTool { ctx };

    let result = tool.call(json!({"entity_id": "nonexistent"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["error"], "entity not found");
}

#[test]
fn test_get_entity_found() {
    let (storage, project_id) = setup_storage();
    storage
        .upsert_entity(&test_entity(
            "e1",
            "my_module",
            Some("src/my.rs"),
            &project_id,
        ))
        .unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = GetEntityTool { ctx };

    let result = tool.call(json!({"entity_id": "e1"})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["name"], "my_module");
    assert_eq!(parsed["entity_id"], "e1");
}

// Test get_entity include_children for non-module (single-level behavior)
#[test]
fn test_get_entity_include_children_non_module() {
    let (storage, project_id) = setup_storage();

    // Create a Subsystem entity with File children
    let subsystem = Entity {
        id: "sub-1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Subsystem,
        parent_id: None,
        name: "core".to_string(),
        path: Some("src/core".to_string()),
        language: None,
        summary: Some("Core subsystem".to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&subsystem).unwrap();

    // Add File children
    let file1 = Entity {
        id: "file-1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("sub-1".to_string()),
        name: "auth.rs".to_string(),
        path: Some("src/core/auth.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&file1).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = GetEntityTool { ctx };

    let result = tool
        .call(json!({"entity_id": "sub-1", "include_children": true}))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["entity_id"], "sub-1");
    assert!(
        parsed.get("children").is_some(),
        "children field should exist"
    );

    let children = parsed["children"].as_array().unwrap();
    assert_eq!(children.len(), 1, "should have one child");
    assert_eq!(children[0]["id"], "file-1");
    assert_eq!(children[0]["tier"], "file");

    // For non-module, functions should NOT be nested (single-level behavior)
    assert!(
        children[0].get("functions").is_none(),
        "non-module should not have nested functions"
    );
}

// Issue #535: File-tier include_children returns functions (existing behavior unchanged)
#[test]
fn test_file_tier_include_children_unchanged() {
    let (storage, project_id) = setup_storage();

    // Create a File entity
    let file = Entity {
        id: "file-api".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "api.rs".to_string(),
        path: Some("src/api.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&file).unwrap();

    // Add Function children
    let func1 = Entity {
        id: "func-fetch".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-api".to_string()),
        name: "fetch_data".to_string(),
        path: Some("src/api.rs::fetch_data".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&func1).unwrap();

    let func2 = Entity {
        id: "func-parse".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-api".to_string()),
        name: "parse_response".to_string(),
        path: Some("src/api.rs::parse_response".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&func2).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = GetEntityTool { ctx };

    let result = tool
        .call(json!({"entity_id": "file-api", "include_children": true}))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["entity_id"], "file-api");
    assert!(
        parsed.get("children").is_some(),
        "children field should exist"
    );

    let children = parsed["children"].as_array().unwrap();
    assert_eq!(children.len(), 2, "should have two function children");

    // Functions should be direct children (not nested), existing behavior
    let fetch_func = children
        .iter()
        .find(|f| f["id"] == "func-fetch")
        .expect("fetch function should exist");
    assert_eq!(fetch_func["name"], "fetch_data");
    assert_eq!(fetch_func["tier"], "function");

    let parse_func = children
        .iter()
        .find(|f| f["id"] == "func-parse")
        .expect("parse function should exist");
    assert_eq!(parse_func["name"], "parse_response");
    assert_eq!(parse_func["tier"], "function");
}

// Issue #535: Subsystem-tier include_children single-level only (no function recursion)
#[test]
fn test_subsystem_include_children_single_level_only() {
    let (storage, project_id) = setup_storage();

    // Create a Subsystem entity
    let subsystem = Entity {
        id: "sub-core".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Subsystem,
        parent_id: None,
        name: "core".to_string(),
        path: Some("src/core".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&subsystem).unwrap();

    // Add Module child
    let module = Entity {
        id: "mod-auth".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: Some("sub-core".to_string()),
        name: "auth".to_string(),
        path: Some("src/core/auth".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&module).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = GetEntityTool { ctx };

    let result = tool
        .call(json!({"entity_id": "sub-core", "include_children": true}))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["entity_id"], "sub-core");
    assert!(
        parsed.get("children").is_some(),
        "children field should exist"
    );

    let children = parsed["children"].as_array().unwrap();
    assert_eq!(children.len(), 1, "should have one module child");

    // Module should be a direct child (single-level only)
    assert_eq!(children[0]["id"], "mod-auth");
    assert_eq!(children[0]["name"], "auth");
    assert_eq!(children[0]["tier"], "module");

    // No functions field since this is a Module, not a File
    assert!(
        children[0].get("functions").is_none(),
        "Module child should not have nested functions (single-level only)"
    );
}
