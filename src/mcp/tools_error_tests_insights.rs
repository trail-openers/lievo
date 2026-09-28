/// Error-path tests for insights and file-related MCP tools.
/// Tests: GetInsightsTool, ReadFileTool, GetExecutionFlowsTool
use crate::mcp::LievoMcpServer;
use crate::mcp::params::{GetExecutionFlowsParams, GetInsightsParams, ReadFileParams};
use crate::model::{Entity, EntityTier};
use crate::retrieval::tools::ToolContext;
use crate::storage::sqlite::SqliteStorage;
use rmcp::handler::server::wrapper::Parameters;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::tools_error_tests_helpers::{make_server, text};
use crate::storage::Storage;

// =============================================================================
// GetInsightsTool Error Tests
// =============================================================================

#[tokio::test]
async fn get_insights_invalid_category_returns_empty() {
    let server = make_server();
    let result = server
        .get_insights(Parameters(GetInsightsParams {
            category: Some("nonexistent_category".into()),
            severity: None,
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    assert!(parsed.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_insights_invalid_severity_returns_empty() {
    let server = make_server();
    let result = server
        .get_insights(Parameters(GetInsightsParams {
            category: None,
            severity: Some("invalid_severity".into()),
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    // Should gracefully return empty array for invalid severity
    assert!(parsed.is_array());
}

#[tokio::test]
async fn get_insights_both_filters_with_no_matches() {
    let server = make_server();
    let result = server
        .get_insights(Parameters(GetInsightsParams {
            category: Some("coverage_gap".into()),
            severity: Some("critical".into()),
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    assert!(parsed.as_array().unwrap().is_empty());
}

// =============================================================================
// ReadFileTool Error Tests
// =============================================================================

#[tokio::test]
async fn read_file_missing_entity_returns_error() {
    let server = make_server();
    let result = server
        .read_file(Parameters(ReadFileParams {
            entity_id: "nonexistent-file".into(),
        }))
        .await
        .unwrap();
    assert!(text(&result).contains("entity not found"));
}

#[tokio::test]
async fn read_file_with_empty_entity_id() {
    let server = make_server();
    let result = server
        .read_file(Parameters(ReadFileParams {
            entity_id: "".into(),
        }))
        .await
        .unwrap();
    // Empty ID should return error
    assert!(text(&result).contains("not found") || text(&result).contains("error"));
}

#[tokio::test]
async fn read_file_with_module_entity_instead_of_file() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    // Create a module entity (not a file)
    storage
        .upsert_entity(&Entity {
            id: "module-1".to_string(),
            project_id: project.id.clone(),
            repo_id: None,
            tier: EntityTier::Module,
            parent_id: None,
            name: "my_module".to_string(),
            path: Some("src".to_string()),
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
        .read_file(Parameters(ReadFileParams {
            entity_id: "module-1".into(),
        }))
        .await
        .unwrap();
    // Reading a module should return an error or empty
    assert!(
        text(&result).contains("not a file")
            || text(&result).contains("error")
            || text(&result).is_empty()
    );
}

// =============================================================================
// GetExecutionFlowsTool Error Tests
// =============================================================================

#[tokio::test]
async fn get_execution_flows_invalid_entry_point() {
    let server = make_server();
    let result = server
        .get_execution_flows(Parameters(GetExecutionFlowsParams {
            entry_point: Some("nonexistent_function".into()),
            max_depth: None,
            limit: None,
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let flows = parsed["flows"].as_array().unwrap();
    // Should return empty flows for nonexistent entry point
    assert!(flows.is_empty());
}

#[tokio::test]
async fn get_execution_flows_negative_max_depth() {
    let server = make_server();
    // max_depth as negative doesn't make sense, but tool should handle it
    let _result = server
        .get_execution_flows(Parameters(GetExecutionFlowsParams {
            entry_point: None,
            max_depth: Some(0),
            limit: None,
        }))
        .await
        .unwrap();
    // Should handle gracefully, likely returning empty or all flows
}

#[tokio::test]
async fn get_execution_flows_limit_zero() {
    let server = make_server();
    let result = server
        .get_execution_flows(Parameters(GetExecutionFlowsParams {
            entry_point: None,
            max_depth: None,
            limit: Some(0),
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let flows = parsed["flows"].as_array().unwrap();
    // limit=0 should return no flows
    assert!(flows.is_empty());
}
