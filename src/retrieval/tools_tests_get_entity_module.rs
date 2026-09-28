// Tests for GetEntityTool Module-tier behavior with nested functions (issue #119, #535).
//
// Covers: Module-specific behavior, nested function hierarchies, edge cases.
// For basic and non-Module tests, see tools_tests_get_entity_basic.rs

use serde_json::json;

use crate::model::{Entity, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

// In the test module: super = tools_impl, super::super = tools
use super::super::GetEntityTool;
use super::tools_tests_helpers::{make_ctx, setup_storage};

// Test get_entity include_children for module (nested functions behavior)
#[test]
fn test_get_entity_include_children_module_nested_functions() {
    let (storage, project_id) = setup_storage();

    // Create a Module entity
    let module = Entity {
        id: "mod-1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: None,
        name: "storage".to_string(),
        path: Some("src/storage".to_string()),
        language: None,
        summary: Some("Storage module".to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&module).unwrap();

    // Add File children
    let file1 = Entity {
        id: "file-1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-1".to_string()),
        name: "store.rs".to_string(),
        path: Some("src/storage/store.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&file1).unwrap();

    // Add Function grandchildren for the file
    let func1 = Entity {
        id: "func-1".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-1".to_string()),
        name: "new".to_string(),
        path: Some("src/storage/store.rs::new".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&func1).unwrap();

    let func2 = Entity {
        id: "func-2".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-1".to_string()),
        name: "save".to_string(),
        path: Some("src/storage/store.rs::save".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&func2).unwrap();

    // Add another file without functions
    let file2 = Entity {
        id: "file-2".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-1".to_string()),
        name: "config.rs".to_string(),
        path: Some("src/storage/config.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&file2).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = GetEntityTool { ctx };

    let result = tool
        .call(json!({"entity_id": "mod-1", "include_children": true}))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["entity_id"], "mod-1");
    assert!(
        parsed.get("children").is_some(),
        "children field should exist"
    );

    let children = parsed["children"].as_array().unwrap();
    assert_eq!(children.len(), 2, "should have two file children");

    // Find store.rs entry - should have nested functions
    let store_entry = children
        .iter()
        .find(|c| c["id"] == "file-1")
        .expect("store.rs entry should exist");
    assert_eq!(store_entry["name"], "store.rs");
    assert_eq!(store_entry["tier"], "file");

    // Verify nested functions
    let functions = store_entry["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 2, "should have two functions");
    assert_eq!(functions[0]["id"], "func-1");
    assert_eq!(functions[0]["name"], "new");
    assert_eq!(functions[0]["tier"], "function");
    assert_eq!(functions[1]["id"], "func-2");
    assert_eq!(functions[1]["name"], "save");
    assert_eq!(functions[1]["tier"], "function");

    // Find config.rs entry - should NOT have functions field (no functions)
    let config_entry = children
        .iter()
        .find(|c| c["id"] == "file-2")
        .expect("config.rs entry should exist");
    assert_eq!(config_entry["name"], "config.rs");
    assert!(
        config_entry.get("functions").is_none(),
        "file with no functions should not have functions field"
    );
}

// Issue #535: Module-tier entity with include_children returns nested functions under File children
#[test]
fn test_module_include_children_returns_nested_functions() {
    let (storage, project_id) = setup_storage();

    // Create a Module entity
    let module = Entity {
        id: "mod-auth".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: None,
        name: "auth".to_string(),
        path: Some("src/auth".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&module).unwrap();

    // Add File child
    let file = Entity {
        id: "file-login".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-auth".to_string()),
        name: "login.rs".to_string(),
        path: Some("src/auth/login.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&file).unwrap();

    // Add Function grandchildren
    let func1 = Entity {
        id: "func-validate".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-login".to_string()),
        name: "validate_credentials".to_string(),
        path: Some("src/auth/login.rs::validate_credentials".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&func1).unwrap();

    let func2 = Entity {
        id: "func-session".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-login".to_string()),
        name: "create_session".to_string(),
        path: Some("src/auth/login.rs::create_session".to_string()),
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
        .call(json!({"entity_id": "mod-auth", "include_children": true}))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["entity_id"], "mod-auth");
    assert!(
        parsed.get("children").is_some(),
        "children field should exist"
    );

    let children = parsed["children"].as_array().unwrap();
    assert_eq!(children.len(), 1, "should have one file child");

    let file_entry = &children[0];
    assert_eq!(file_entry["id"], "file-login");
    assert_eq!(file_entry["name"], "login.rs");
    assert_eq!(file_entry["tier"], "file");

    // Verify nested functions exist (order not guaranteed)
    let functions = file_entry["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 2, "should have two nested functions");

    // Find each function and verify properties
    let validate_func = functions
        .iter()
        .find(|f| f["id"] == "func-validate")
        .expect("validate function should exist");
    assert_eq!(validate_func["name"], "validate_credentials");
    assert_eq!(validate_func["tier"], "function");

    let session_func = functions
        .iter()
        .find(|f| f["id"] == "func-session")
        .expect("session function should exist");
    assert_eq!(session_func["name"], "create_session");
    assert_eq!(session_func["tier"], "function");
}

// Issue #535: Empty module returns empty children array
#[test]
fn test_module_include_children_empty_module_returns_empty() {
    let (storage, project_id) = setup_storage();

    // Create a Module entity with no children
    let module = Entity {
        id: "mod-empty".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: None,
        name: "empty".to_string(),
        path: Some("src/empty".to_string()),
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
        .call(json!({"entity_id": "mod-empty", "include_children": true}))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["entity_id"], "mod-empty");
    assert!(
        parsed.get("children").is_some(),
        "children field should exist"
    );

    let children = parsed["children"].as_array().unwrap();
    assert_eq!(children.len(), 0, "empty module should have no children");
    assert_eq!(parsed["child_count"], 0, "child_count should be 0");
}

// Issue #535: Files present but no functions returns children without functions key
#[test]
fn test_module_include_children_files_with_no_functions() {
    let (storage, project_id) = setup_storage();

    // Create a Module entity
    let module = Entity {
        id: "mod-utils".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::Module,
        parent_id: None,
        name: "utils".to_string(),
        path: Some("src/utils".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&module).unwrap();

    // Add File children with no functions
    let file1 = Entity {
        id: "file-helpers".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-utils".to_string()),
        name: "helpers.rs".to_string(),
        path: Some("src/utils/helpers.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&file1).unwrap();

    let file2 = Entity {
        id: "file-macros".to_string(),
        project_id: project_id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("mod-utils".to_string()),
        name: "macros.rs".to_string(),
        path: Some("src/utils/macros.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&file2).unwrap();

    let ctx = make_ctx(storage, project_id);
    let tool = GetEntityTool { ctx };

    let result = tool
        .call(json!({"entity_id": "mod-utils", "include_children": true}))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed["entity_id"], "mod-utils");
    assert!(
        parsed.get("children").is_some(),
        "children field should exist"
    );

    let children = parsed["children"].as_array().unwrap();
    assert_eq!(children.len(), 2, "should have two file children");

    // Both files should not have "functions" field (no functions exist)
    for child in children {
        assert!(
            child.get("functions").is_none(),
            "file with no functions should not have functions field"
        );
    }
}
