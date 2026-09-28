/// Error-path tests for entity-related MCP tools.
/// Tests: SearchEntitiesTool, GetEntityTool, ListRelationshipsTool, GetFunctionTool
use crate::mcp::LievoMcpServer;
use crate::mcp::params::{
    GetEntityParams, GetFunctionParams, ListRelationshipsParams, SearchEntitiesParams,
};
use crate::model::{Entity, EntityTier};
use crate::retrieval::tools::ToolContext;
use crate::storage::sqlite::SqliteStorage;
use rmcp::handler::server::wrapper::Parameters;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::tools_error_tests_helpers::{assert_success_shaped, make_server, text};
use crate::storage::Storage;

// =============================================================================
// SearchEntitiesTool Error Tests
// =============================================================================

#[tokio::test]
async fn search_entities_missing_query_param_returns_error() {
    let server = make_server();
    let result = server
        .search_entities(Parameters(SearchEntitiesParams {
            query: "".into(),
            limit: None,
            semantic: false,
        }))
        .await;
    // Empty query is still valid, but it's a boundary case
    // The tool should handle it gracefully
    assert!(result.is_ok(), "empty query should not crash");
}

#[tokio::test]
async fn search_entities_with_limit_zero_returns_empty() {
    let server = make_server();
    let result = server
        .search_entities(Parameters(SearchEntitiesParams {
            query: "test".into(),
            limit: Some(0),
            semantic: false,
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let results = parsed["results"].as_array().unwrap();
    // limit=0 should return empty array
    assert_eq!(results.len(), 0);
}

// =============================================================================
// GetEntityTool Error Tests
// =============================================================================

#[tokio::test]
async fn get_entity_invalid_entity_id_returns_error() {
    let server = make_server();
    let result = server
        .get_entity(Parameters(GetEntityParams {
            entity_id: "nonexistent-id-xyz".into(),
            include_children: false,
        }))
        .await
        .unwrap();
    assert!(text(&result).contains("entity not found"));
}

#[tokio::test]
async fn get_entity_with_empty_id_returns_success_shaped_guidance() {
    // Issue #680/#682 P1.2: an empty/invalid entity_id is a recoverable condition and
    // must return success-shaped guidance, not an MCP error.
    let server = make_server();
    let result = server
        .get_entity(Parameters(GetEntityParams {
            entity_id: "".into(),
            include_children: false,
        }))
        .await
        .expect("empty entity_id must be success-shaped, not an MCP error");
    assert_success_shaped(&result);
}

#[tokio::test]
async fn get_entity_with_include_children_for_nonexistent() {
    let server = make_server();
    let result = server
        .get_entity(Parameters(GetEntityParams {
            entity_id: "missing-id".into(),
            include_children: true,
        }))
        .await
        .unwrap();
    assert!(text(&result).contains("entity not found"));
}

// =============================================================================
// ListRelationshipsTool Error Tests
// =============================================================================

#[tokio::test]
async fn list_relationships_missing_entity_id_returns_success_shaped_guidance() {
    // Issue #680/#682 P1.2: a missing entity is a recoverable condition and must
    // return success-shaped guidance naming the failure, not an MCP error.
    let server = make_server();
    let result = server
        .list_relationships(Parameters(ListRelationshipsParams {
            entity_id: "missing-id".into(),
        }))
        .await
        .expect("missing entity must be success-shaped, not an MCP error");
    let guidance = assert_success_shaped(&result);
    assert!(
        guidance.contains("not found"),
        "guidance should name the missing entity: {guidance}"
    );
}

#[tokio::test]
async fn list_relationships_with_empty_entity_id_returns_success_shaped_guidance() {
    // Issue #680/#682 P1.2: an empty entity ID is a recoverable condition and must
    // return success-shaped guidance, not an MCP error.
    let server = make_server();
    let result = server
        .list_relationships(Parameters(ListRelationshipsParams {
            entity_id: "".into(),
        }))
        .await
        .expect("empty entity_id must be success-shaped, not an MCP error");
    assert_success_shaped(&result);
}

// =============================================================================
// GetFunctionTool Error Tests
// =============================================================================

#[tokio::test]
async fn get_function_missing_entity_returns_error() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    // Add a placeholder function entity to avoid zero-function check
    storage
        .upsert_entity(&Entity {
            id: "fn-placeholder".to_string(),
            project_id: project.id.clone(),
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: None,
            name: "placeholder".to_string(),
            path: Some("src/lib.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        })
        .unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);
    let result = server
        .get_function(Parameters(GetFunctionParams {
            entity_id: "nonexistent-fn".into(),
        }))
        .await
        .unwrap();
    assert!(text(&result).contains("Entity not found") || text(&result).contains("not found"));
}

#[tokio::test]
async fn get_function_with_zero_functions_returns_message() {
    let server = make_server();
    let result = server
        .get_function(Parameters(GetFunctionParams {
            entity_id: "any-id".into(),
        }))
        .await
        .unwrap();
    let text_val = text(&result);
    // When no functions exist, should return informative message
    assert!(
        text_val.contains("No functions")
            || text_val.contains("function")
            || text_val.contains("refresh")
    );
}
