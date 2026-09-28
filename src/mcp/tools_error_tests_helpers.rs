/// Shared test helpers for MCP tool error tests.
use crate::mcp::LievoMcpServer;
use crate::retrieval::tools::ToolContext;
use crate::storage::sqlite::SqliteStorage;
use rmcp::model::CallToolResult;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::storage::Storage;

pub fn make_server() -> LievoMcpServer {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    LievoMcpServer::new(ctx)
}

pub fn text(result: &CallToolResult) -> &str {
    result
        .content
        .first()
        .and_then(|content| content.as_text())
        .map(|text| text.text.as_str())
        .expect("text content")
}

/// Assert the result is success-shaped (issue #680/#682 P1.2 contract): the `Result`
/// is `Ok` AND `is_error` is not `Some(true)`. Returns the text content on success.
pub fn assert_success_shaped(result: &CallToolResult) -> &str {
    assert!(
        result.is_error != Some(true),
        "expected success-shaped result (is_error != true), got: {}",
        text(result)
    );
    text(result)
}
