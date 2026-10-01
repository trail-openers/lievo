use crate::mcp::LievoMcpServer;
use crate::mcp::params::{
    GetEntityParams, GetExecutionFlowsParams, GetFunctionParams, GetHotspotsParams,
    GetImpactParams, GetInsightsParams, ListDirectoryParams, ListRelationshipsParams,
    ReadFileParams, ReadProjectDocParams,
};
use crate::model::{Entity, EntityTier, Insight};
use crate::retrieval::tools::ToolContext;
use crate::storage::sqlite::SqliteStorage;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::storage::Storage;

fn make_server() -> LievoMcpServer {
    make_server_with_repo_path(PathBuf::new())
}

fn make_server_with_repo_path(repo_path: PathBuf) -> LievoMcpServer {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path,
        output_dir: None,
        zero_repo_guidance: None,
    });
    LievoMcpServer::new(ctx)
}

fn text(result: &CallToolResult) -> &str {
    result
        .content
        .first()
        .and_then(|content| content.as_text())
        .map(|text| text.text.as_str())
        .expect("text content")
}

/// Assert the result is success-shaped (issue #680/#682 P1.2 contract): the `Result`
/// is `Ok` AND `is_error` is not `Some(true)`. Returns the text content on success.
fn assert_success_shaped(result: &CallToolResult) -> &str {
    assert!(
        result.is_error != Some(true),
        "expected success-shaped result (is_error != true), got: {}",
        text(result)
    );
    text(result)
}
#[tokio::test]
async fn get_execution_flows_wrapper_respects_limit_parameter() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();

    // Create two function entities with execution flows
    let entity1 = Entity {
        id: "fn1".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: None,
        name: "handler_a".to_string(),
        path: Some("src/lib.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: Some(
            serde_json::json!({
                "execution_flows": [{
                    "entry_point": "handler_a",
                    "entry_point_id": "fn1",
                    "steps": [{"entity_name": "handler_a", "entity_id": "fn1", "depth": 0}],
                    "has_cycle": false
                }]
            })
            .to_string(),
        ),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let entity2 = Entity {
        id: "fn2".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: None,
        name: "handler_b".to_string(),
        path: Some("src/lib.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: Some(
            serde_json::json!({
                "execution_flows": [{
                    "entry_point": "handler_b",
                    "entry_point_id": "fn2",
                    "steps": [{"entity_name": "handler_b", "entity_id": "fn2", "depth": 0}],
                    "has_cycle": false
                }]
            })
            .to_string(),
        ),
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    storage.upsert_entity(&entity1).unwrap();
    storage.upsert_entity(&entity2).unwrap();

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);

    // Request only 1 flow
    let result = server
        .get_execution_flows(Parameters(GetExecutionFlowsParams {
            entry_point: None,
            max_depth: None,
            limit: Some(1),
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let flows = parsed["flows"].as_array().unwrap();
    assert!(
        flows.len() <= 1,
        "with limit=1, expected at most 1 flow, got {}",
        flows.len()
    );
}
#[tokio::test]
async fn get_entity_wrapper_returns_not_found_error() {
    let server = make_server();
    let result = server
        .get_entity(Parameters(GetEntityParams {
            entity_id: "missing-id".into(),
            include_children: false,
        }))
        .await
        .unwrap();
    assert!(text(&result).contains("entity not found"));
}
#[tokio::test]
async fn list_relationships_returns_success_shaped_guidance_for_missing_entity() {
    // Issue #680/#682 P1.2: a missing entity is a recoverable condition and must
    // return success-shaped guidance naming the failure, not an MCP error.
    let server = make_server();
    let result = server
        .list_relationships(Parameters(ListRelationshipsParams {
            entity_id: "missing-id".into(),
        }))
        .await
        .expect("missing entity must be success-shaped, not an MCP error");
    assert_success_shaped(&result);
}
#[tokio::test]
async fn list_subsystems_wrapper_returns_empty_array() {
    let server = make_server();
    let result = server.list_subsystems().await.unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(text(&result))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn list_subsystems_wrapper_returns_populated_subsystem_tier() {
    // AC-4 (issue #689): list_subsystems returns >1 entry for a populated
    // www-shaped subsystem tier. The tool reads list_entities at the
    // Subsystem tier, so no tool change is needed — this test pins that a
    // populated tier flows through the wrapper unchanged.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    let now = "2024-01-01T00:00:00Z".to_string();
    for (id, name, path) in [
        ("sub-root", "root", "."),
        ("sub-ssr", "ssr", "ssr"),
        ("sub-js", "javascript", "app/javascript"),
        ("sub-cypress", "cypress", "cypress"),
    ] {
        storage
            .upsert_entity(&Entity {
                id: id.to_string(),
                project_id: project.id.clone(),
                repo_id: None,
                tier: EntityTier::Subsystem,
                parent_id: None,
                name: name.to_string(),
                path: Some(path.to_string()),
                language: Some("JavaScript".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .unwrap();
    }

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);

    let result = server.list_subsystems().await.unwrap();
    let parsed = serde_json::from_str::<serde_json::Value>(text(&result)).unwrap();
    let entries = parsed
        .as_array()
        .expect("subsystem list must be a JSON array");
    assert!(
        entries.len() > 1,
        "www-shaped subsystem tier must return >1 entry, got {}",
        entries.len()
    );
    let paths: Vec<&str> = entries.iter().filter_map(|e| e["path"].as_str()).collect();
    assert!(
        paths.contains(&"app/javascript") && paths.contains(&"cypress"),
        "unmatched top-level JS/TS dirs must be present: {paths:?}"
    );
}
#[tokio::test]
async fn get_insights_wrapper_returns_empty_array() {
    let server = make_server();
    let result = server
        .get_insights(Parameters(GetInsightsParams {
            category: None,
            severity: None,
        }))
        .await
        .unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(text(&result))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn read_file_wrapper_returns_not_found_error() {
    let server = make_server();
    let result = server
        .read_file(Parameters(ReadFileParams {
            entity_id: "missing-id".into(),
        }))
        .await
        .unwrap();
    assert!(text(&result).contains("entity not found"));
}

#[tokio::test]
async fn list_directory_wrapper_returns_not_configured_error() {
    let server = make_server();
    let result = server
        .list_directory(Parameters(ListDirectoryParams { path: "src".into() }))
        .await
        .unwrap();
    assert!(text(&result).contains("repo path not configured"));
}

#[tokio::test]
async fn get_execution_flows_wrapper_returns_empty_with_message() {
    let server = make_server();
    let result = server
        .get_execution_flows(Parameters(GetExecutionFlowsParams {
            entry_point: None,
            max_depth: None,
            limit: None,
        }))
        .await
        .unwrap();
    let parsed = serde_json::from_str::<serde_json::Value>(text(&result)).unwrap();
    assert!(
        parsed["flows"].as_array().unwrap().is_empty(),
        "flows should be empty when no entities exist"
    );
    assert!(
        parsed["message"].as_str().is_some(),
        "empty-state should include an explanatory message"
    );
    let msg = parsed["message"].as_str().unwrap();
    assert!(
        msg.contains("lievo refresh --force"),
        "message should mention 'lievo refresh --force': {msg}"
    );
    assert!(
        !msg.contains("preserve_function_entities"),
        "message should NOT mention preserve_function_entities: {msg}"
    );
}

#[tokio::test]
async fn get_execution_flows_wrapper_returns_force_message_when_function_entities_exist() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    storage
        .upsert_entity(&Entity {
            id: "fn1".to_string(),
            project_id: project.id.clone(),
            repo_id: None,
            tier: EntityTier::Function,
            parent_id: None,
            name: "my_func".to_string(),
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
        .get_execution_flows(Parameters(GetExecutionFlowsParams {
            entry_point: None,
            max_depth: None,
            limit: None,
        }))
        .await
        .unwrap();
    let parsed = serde_json::from_str::<serde_json::Value>(text(&result)).unwrap();
    assert!(
        parsed["flows"].as_array().unwrap().is_empty(),
        "flows should be empty when no flow data exists"
    );
    let msg = parsed["message"]
        .as_str()
        .expect("message should be present");
    assert!(
        msg.contains("lievo refresh --force"),
        "message should mention 'lievo refresh --force': {msg}"
    );
    assert!(
        !msg.contains("preserve_function_entities"),
        "with function entities present, message should NOT mention config: {msg}"
    );
}

#[tokio::test]
async fn list_project_docs_wrapper_returns_empty_array() {
    let server = make_server();
    let result = server.list_project_docs().await.unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(text(&result))
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn read_project_doc_wrapper_returns_success_shaped_guidance_for_invalid_path() {
    // Issue #680/#682 P1.2: recoverable conditions return success-shaped guidance,
    // not MCP errors. An invalid doc path is recoverable — the agent can retry with
    // a path from list_project_docs.
    let server = make_server();
    let result = server
        .read_project_doc(Parameters(ReadProjectDocParams {
            path: "missing.md".into(),
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
async fn read_file_wrapper_returns_success_shaped_guidance_when_repo_path_not_configured() {
    // Issue #680/#682 P1.2: a not-indexed / not-configured repo path is a recoverable
    // condition and must return success-shaped guidance naming 'lievo refresh', not an
    // MCP error.
    let server = make_server(); // repo_path = PathBuf::new() (not configured)
    let result = server
        .read_file(Parameters(ReadFileParams {
            entity_id: "missing-file".into(),
        }))
        .await
        .expect("not-configured repo path must be success-shaped, not an MCP error");
    assert_success_shaped(&result);
}

#[test]
fn search_entities_tool_has_schema() {
    use rmcp::handler::server::ServerHandler;
    let server = make_server();
    // get_tool is part of ServerHandler trait and returns registered tools
    let tool = server
        .get_tool("search_entities")
        .expect("search_entities must be registered");
    assert_eq!(tool.name, "search_entities");
    // input_schema must exist and have properties
    assert!(
        tool.input_schema.get("properties").is_some(),
        "search_entities schema must have properties with 'query' field"
    );
}

#[test]
fn search_entities_wire_description_describes_embedding_not_treesitter() {
    // Issue #15: the WIRE description agents see for semantic=true mode must
    // describe the embedding-based (model2vec + usearch) vector search, not the
    // stale "tree-sitter" phrasing (tree-sitter is for extraction, not
    // semantic search).
    use rmcp::handler::server::ServerHandler;
    let server = make_server();
    let tool = server
        .get_tool("search_entities")
        .expect("search_entities must be registered");
    let desc = tool.description.as_deref().unwrap_or("");
    assert!(
        !desc.contains("tree-sitter"),
        "wire description must not claim semantic search is tree-sitter based: {desc}"
    );
    assert!(
        desc.contains("embedding"),
        "wire description must describe semantic search as embedding-based: {desc}"
    );
}

#[test]
fn search_entities_tool_description_and_schema_describe_embedding_not_treesitter() {
    // Issue #15: the hand-written SearchEntitiesTool::description() and the
    // input_schema() semantic property description must also describe the
    // embedding-based vector search, not the stale tree-sitter phrasing
    use crate::retrieval::tool_trait::Tool;
    use crate::retrieval::tools::SearchEntitiesTool;
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = SearchEntitiesTool { ctx };
    let desc = tool.description();
    assert!(
        !desc.contains("tree-sitter"),
        "Tool::description must not claim semantic search is tree-sitter based: {desc}"
    );
    assert!(
        desc.contains("embedding"),
        "Tool::description must describe semantic search as embedding-based: {desc}"
    );

    let schema = tool.input_schema();
    let semantic_desc = schema
        .pointer("/properties/semantic/description")
        .and_then(|v| v.as_str())
        .expect("semantic property description must exist in input_schema");
    assert!(
        !semantic_desc.contains("tree-sitter"),
        "semantic property description must not claim tree-sitter based search: {semantic_desc}"
    );
    assert!(
        semantic_desc.contains("embedding"),
        "semantic property description must be embedding-based: {semantic_desc}"
    );
}

#[test]
fn lievo_explore_wire_description_carrying_call_directives() {
    // Issue #745: the WIRE description the agent sees must carry the four
    // directives — do-not-re-read, do-not-re-request, batch, and stop-when-complete
    // — using only signals present in today's response shape (no completeness field,
    // that lands in #743).
    use rmcp::handler::server::ServerHandler;
    let server = make_server();
    let tool = server
        .get_tool("lievo_explore")
        .expect("lievo_explore must be registered");
    let desc = tool.description.as_deref().unwrap_or("");
    // F1: do not re-read a file whose source was already received.
    assert!(
        desc.contains("Do not re-read"),
        "wire description must carry the do-not-re-read directive: {desc}"
    );
    // F2: do not re-request edges/relationships already returned.
    assert!(
        desc.contains("do not re-request"),
        "wire description must carry the do-not-re-request directive: {desc}"
    );
    // F3: batch related files in one call.
    assert!(
        desc.contains("Batch"),
        "wire description must carry the batch directive: {desc}"
    );
    // F4: stop when the result set is complete (returned == total, no continuation).
    assert!(
        desc.contains("Stop"),
        "wire description must carry the stop directive: {desc}"
    );
    // The subsystem-bundle mode (issue #743, now landed) names its structured
    // completeness field on the wire so the agent knows a complete bundle means
    // it can stop. (This guard previously asserted the field was NOT yet
    // referenced — it has since landed.)
    assert!(
        desc.contains("completeness field"),
        "wire description must name the bundle's structured completeness field (issue #743): {desc}"
    );
    assert!(
        desc.contains("bundle="),
        "wire description must document the bundle mode selector (issue #743): {desc}"
    );
    assert!(
        desc.contains("do not re-read any file"),
        "wire description must carry the bundle do-not-re-read directive (issue #743): {desc}"
    );
}

#[tokio::test]
async fn get_function_wrapper_returns_not_found_error() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    // Add a function entity so the zero-function check passes, then
    // querying a missing id returns "Entity not found".
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
            entity_id: "missing-id".into(),
        }))
        .await
        .unwrap();
    assert!(text(&result).contains("Entity not found"));
}

#[tokio::test]
async fn get_impact_wrapper_returns_empty_for_empty_files() {
    let server = make_server();
    let result = server
        .get_impact(Parameters(GetImpactParams { files: vec![] }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    assert_eq!(parsed["files"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["dependents"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn get_hotspots_wrapper_returns_empty_for_empty_db() {
    let server = make_server();
    let result = server
        .get_hotspots(Parameters(GetHotspotsParams {
            limit: 10,
            tier: "file".to_string(),
        }))
        .await
        .unwrap();
    assert_eq!(text(&result), "");
}

#[tokio::test]
async fn get_hotspots_wrapper_returns_success_shaped_guidance_for_limit_zero() {
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
async fn get_insights_with_category_coverage_gap_returns_results() {
    // Regression test for issue #568/#569: category filter was broken because
    // detect_coverage_gaps() skipped modules with language: None files.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();

    // Insert a coverage_gap insight directly, bypassing detect_coverage_gaps()
    let insight = Insight {
        id: "cg-1".to_string(),
        project_id: project.id.clone(),
        category: "coverage_gap".to_string(),
        severity: Some("high".to_string()),
        title: "Undocumented function".to_string(),
        description: Some("Function lacks documentation".to_string()),
        entity_ids_json: None,
        detected_at: "2024-01-01T00:00:00Z".to_string(),
        still_valid: true,
    };
    storage.upsert_insight(&insight).unwrap();

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);

    let result = server
        .get_insights(Parameters(GetInsightsParams {
            category: Some("coverage_gap".to_string()),
            severity: None,
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let insights = parsed.as_array().unwrap();
    assert_eq!(
        insights.len(),
        1,
        "should return exactly 1 coverage_gap insight"
    );
    assert_eq!(
        insights[0]["category"].as_str().unwrap(),
        "coverage_gap",
        "insight category must be coverage_gap"
    );
}

#[cfg(test)]
mod tools_error_tests_helpers;

#[cfg(test)]
#[path = "tools_error_tests_entity.rs"]
mod error_tests_entity;

#[cfg(test)]
#[path = "tools_error_tests_insights.rs"]
mod error_tests_insights;

#[cfg(test)]
#[path = "tools_error_tests_io.rs"]
mod error_tests_io;
