/// Error-path tests for I/O and impact-related MCP tools.
/// Tests: ListDirectoryTool, GetConventionsTool, ReadProjectDocTool,
///        GetImpactTool, GetHotspotsTool
use crate::mcp::LievoMcpServer;
use crate::mcp::params::{
    GetConventionsParams, GetHotspotsParams, GetImpactParams, ListDirectoryParams,
    ReadProjectDocParams,
};
use crate::retrieval::tools::ToolContext;
use crate::storage::sqlite::SqliteStorage;
use rmcp::handler::server::wrapper::Parameters;

use std::sync::{Arc, Mutex};

use super::tools_error_tests_helpers::{assert_success_shaped, make_server, text};
use crate::storage::Storage;

// =============================================================================
// ListDirectoryTool Error Tests
// =============================================================================

#[tokio::test]
async fn list_directory_without_repo_path() {
    let server = make_server();
    let result = server
        .list_directory(Parameters(ListDirectoryParams { path: "src".into() }))
        .await
        .unwrap();
    assert!(text(&result).contains("repo path not configured"));
}

#[tokio::test]
async fn list_directory_with_path_traversal_attempt() {
    // Create server with repo_path set to a specific directory
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    // Use temp_dir as repo path, which is a safe root
    let temp_dir = std::env::temp_dir();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: temp_dir.clone(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);
    // Try a path traversal attack
    let result = server
        .list_directory(Parameters(ListDirectoryParams {
            path: "../../../../etc/passwd".into(),
        }))
        .await
        .unwrap();
    // Verify sandboxing: the operation should be rejected as a path traversal attempt.
    // The tool rejects ParentDir (..) components at line 54 of tools_directory.rs
    let response_text = text(&result);
    assert!(
        response_text.contains("path traversal not allowed"),
        "expected path-traversal rejection, got: {}",
        response_text
    );
}

#[tokio::test]
async fn list_directory_with_empty_path() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    let temp_dir = std::env::temp_dir();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: temp_dir,
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);
    let result = server
        .list_directory(Parameters(ListDirectoryParams { path: "".into() }))
        .await
        .unwrap();
    let response_text = text(&result);
    // Empty path should list the root directory (treated as ".")
    // Response should contain entries array and a count
    let parsed: serde_json::Value = serde_json::from_str(response_text).unwrap();
    assert!(
        parsed["entries"].is_array(),
        "empty path should return entries array"
    );
    assert!(
        parsed["count"].is_number(),
        "empty path should return count"
    );
}

#[tokio::test]
async fn list_directory_with_absolute_path_returns_error() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    let temp_dir = std::env::temp_dir();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: temp_dir,
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);
    let result = server
        .list_directory(Parameters(ListDirectoryParams {
            path: "/etc/passwd".to_string(),
        }))
        .await
        .unwrap();
    let response_text = text(&result);
    assert!(
        response_text.contains("path traversal not allowed"),
        "expected absolute path rejection, got: {}",
        response_text
    );
}

// =============================================================================
// GetConventionsTool Error Tests
// =============================================================================

#[tokio::test]
async fn get_conventions_invalid_category() {
    let server = make_server();
    let result = server
        .get_conventions(Parameters(GetConventionsParams {
            category: Some("nonexistent_category".into()),
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let conventions = parsed["conventions"].as_array().unwrap();
    // Should return empty for invalid category
    assert!(conventions.is_empty());
}

// =============================================================================
// ReadProjectDocTool Error Tests
// =============================================================================

#[tokio::test]
async fn read_project_doc_invalid_path_returns_success_shaped_guidance() {
    // Issue #680/#682 P1.2: recoverable conditions (including invalid/unknown paths)
    // return success-shaped guidance (Ok, is_error=false) instead of an MCP error.
    let server = make_server();
    let result = server
        .read_project_doc(Parameters(ReadProjectDocParams {
            path: "nonexistent_doc.md".into(),
            format: None,
        }))
        .await
        .expect("invalid doc path must be success-shaped, not an MCP error");
    let guidance = assert_success_shaped(&result);
    assert!(
        guidance.contains("not in discovered docs list"),
        "guidance should name the reason the path was rejected: {guidance}"
    );
}

#[tokio::test]
async fn read_project_doc_with_traversal_attempt_returns_success_shaped_guidance() {
    // Issue #680/#682 P1.2: path-traversal attempts are a recoverable condition and
    // must return success-shaped guidance naming the rejection reason, not an MCP error.
    let server = make_server();
    let result = server
        .read_project_doc(Parameters(ReadProjectDocParams {
            path: "../../etc/passwd".into(),
            format: None,
        }))
        .await
        .expect("path traversal attempt must be success-shaped, not an MCP error");
    let guidance = assert_success_shaped(&result);
    assert!(
        guidance.contains("not in discovered docs list")
            || guidance.contains("outside the project root"),
        "expected path validation rejection guidance, got: {guidance}"
    );
}

#[tokio::test]
async fn read_project_doc_structured_format_invalid_path_returns_success_shaped_guidance() {
    let server = make_server();
    let result = server
        .read_project_doc(Parameters(ReadProjectDocParams {
            path: "missing.md".into(),
            format: Some("structured".into()),
        }))
        .await
        .expect("invalid doc path with structured format must be success-shaped");
    assert_success_shaped(&result);
}

// =============================================================================
// GetImpactTool Error Tests
// =============================================================================

#[tokio::test]
async fn get_impact_missing_files_array() {
    let server = make_server();
    let result = server
        .get_impact(Parameters(GetImpactParams { files: vec![] }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    // Empty files should return empty impact analysis (lean contract, issue #840)
    assert_eq!(parsed["files"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["dependents"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn get_impact_nonexistent_files_returns_success_shaped_guidance() {
    // Issue #680/#682 P1.2: referencing nonexistent files is a recoverable condition
    // and must return success-shaped guidance, not an MCP error.
    let server = make_server();
    let result = server
        .get_impact(Parameters(GetImpactParams {
            files: vec!["nonexistent_file.rs".into()],
        }))
        .await
        .expect("nonexistent files must be success-shaped, not an MCP error");
    assert_success_shaped(&result);
}

// =============================================================================
// GetHotspotsTool Error Tests
// =============================================================================

#[tokio::test]
async fn get_hotspots_limit_zero_returns_success_shaped_guidance() {
    // Issue #680/#682 P1.2: limit=0 is a recoverable input-validation condition and
    // must return success-shaped guidance (Ok, is_error=false) naming the problem,
    // not an MCP error.
    let server = make_server();
    let result = server
        .get_hotspots(Parameters(GetHotspotsParams {
            limit: 0,
            tier: "file".to_string(),
        }))
        .await
        .expect("limit=0 must be success-shaped, not an MCP error");
    let guidance = assert_success_shaped(&result);
    assert!(
        guidance.contains("limit must be greater than 0"),
        "guidance should mention limit: {guidance}"
    );
}

#[tokio::test]
async fn get_hotspots_invalid_tier_returns_success_shaped_guidance() {
    // Issue #680/#682 P1.2: invalid tier is a recoverable input-validation condition
    // and must return success-shaped guidance naming the problem, not an MCP error.
    let server = make_server();
    let result = server
        .get_hotspots(Parameters(GetHotspotsParams {
            limit: 10,
            tier: "invalid_tier".to_string(),
        }))
        .await
        .expect("invalid tier must be success-shaped, not an MCP error");
    assert_success_shaped(&result);
}

#[tokio::test]
async fn get_hotspots_limit_exceeds_max() {
    let server = make_server();
    let result = server
        .get_hotspots(Parameters(GetHotspotsParams {
            limit: 1000,
            tier: "file".to_string(),
        }))
        .await
        .unwrap();
    // Should succeed but limit result to max (50)
    let lines: Vec<&str> = text(&result).lines().collect();
    assert!(lines.len() <= 50, "result should be limited to max 50");
}

// Note: symlink-escape rejection is provided by fs::canonicalize() + starts_with check in
// production code (tools_directory.rs). Cross-platform symlink test creation is out of scope
// for unit tests.
