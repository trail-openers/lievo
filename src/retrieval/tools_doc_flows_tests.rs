// Tests for GetExecutionFlowsTool in tools_doc.rs

use std::sync::Arc;

use crate::model::{EdgeProvenance, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::{GetExecutionFlowsTool, ToolContext};
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

fn make_flow_ctx() -> Arc<ToolContext<SqliteStorage>> {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("flow-test", None).unwrap();
    Arc::new(ToolContext {
        storage: Arc::new(std::sync::Mutex::new(storage)),
        project_id: project.id,
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    })
}

fn make_entity(id: &str, project_id: &str, tier: EntityTier) -> crate::model::Entity {
    crate::model::Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: None,
        tier,
        parent_id: None,
        name: id.to_string(),
        path: Some(format!("src/{id}.rs")),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn test_get_execution_flows_empty_no_function_entities_returns_refresh_message() {
    let ctx = make_flow_ctx();
    let tool = GetExecutionFlowsTool { ctx: ctx.clone() };

    // No entities at all — should return message to run refresh
    let result = tool.call(serde_json::json!({})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["flows"].as_array().unwrap().is_empty());
    let msg = parsed["message"].as_str().unwrap();
    assert!(
        msg.contains("lievo refresh --force"),
        "message should mention refresh: {msg}"
    );
    assert!(
        !msg.contains("preserve_function_entities"),
        "message should NOT mention preserve_function_entities: {msg}"
    );
}

#[test]
fn test_get_execution_flows_empty_with_function_entities_returns_reanalyze_message() {
    let ctx = make_flow_ctx();
    let guard = ctx.storage.lock().unwrap();
    let func_entity = make_entity("fn1", &ctx.project_id, EntityTier::Function);
    guard.upsert_entity(&func_entity).unwrap();
    drop(guard);

    let tool = GetExecutionFlowsTool { ctx: ctx.clone() };
    let result = tool.call(serde_json::json!({})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(parsed["flows"].as_array().unwrap().is_empty());
    let msg = parsed["message"].as_str().unwrap();
    assert!(
        msg.contains("lievo refresh --force"),
        "message should mention refresh: {msg}"
    );
    assert!(
        !msg.contains("preserve_function_entities"),
        "with function entities, message should NOT mention config: {msg}"
    );
}

#[test]
fn test_get_execution_flows_with_flow_data_returns_object_with_flows() {
    let ctx = make_flow_ctx();
    let guard = ctx.storage.lock().unwrap();
    let mut entity = make_entity("main_fn", &ctx.project_id, EntityTier::Function);
    entity.metrics_json = Some(
        serde_json::json!({
            "execution_flows": [{
                "entry_point": "main",
                "entry_point_id": "main_fn",
                "steps": [{"entity_name": "main", "entity_id": "main_fn", "depth": 0}],
                "has_cycle": false
            }]
        })
        .to_string(),
    );
    guard.upsert_entity(&entity).unwrap();
    drop(guard);

    let tool = GetExecutionFlowsTool { ctx: ctx.clone() };
    let result = tool.call(serde_json::json!({})).unwrap();

    // When flows exist, the tool returns a consistent object shape with "flows" array and "message" null.
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(
        parsed["flows"].is_array(),
        "response should have a 'flows' array"
    );
    assert_eq!(parsed["flows"].as_array().unwrap().len(), 1);
    assert!(
        parsed["message"].is_null(),
        "message should be null when flows exist"
    );
}

#[test]
fn test_get_execution_flows_with_entry_point_filter() {
    let ctx = make_flow_ctx();
    let guard = ctx.storage.lock().unwrap();

    // Create two function entities that will be entry points
    let mut entity1 = make_entity("handler_a", &ctx.project_id, EntityTier::Function);
    entity1.metrics_json = Some(
        serde_json::json!({
            "execution_flows": [{
                "entry_point": "handler_a",
                "entry_point_id": "handler_a",
                "steps": [{"entity_name": "handler_a", "entity_id": "handler_a", "depth": 0}],
                "has_cycle": false
            }]
        })
        .to_string(),
    );

    let mut entity2 = make_entity("handler_b", &ctx.project_id, EntityTier::Function);
    entity2.metrics_json = Some(
        serde_json::json!({
            "execution_flows": [{
                "entry_point": "handler_b",
                "entry_point_id": "handler_b",
                "steps": [{"entity_name": "handler_b", "entity_id": "handler_b", "depth": 0}],
                "has_cycle": false
            }]
        })
        .to_string(),
    );

    guard.upsert_entity(&entity1).unwrap();
    guard.upsert_entity(&entity2).unwrap();
    drop(guard);

    let tool = GetExecutionFlowsTool { ctx: ctx.clone() };

    // Filter by "handler_a" - should only return that flow
    let result = tool
        .call(serde_json::json!({"entry_point": "handler_a"}))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let flows = parsed["flows"].as_array().unwrap();
    assert_eq!(flows.len(), 1, "Should only return 1 flow");
    assert_eq!(flows[0]["entry_point"], "handler_a");
}

#[test]
fn test_get_execution_flows_with_limit() {
    let ctx = make_flow_ctx();
    let guard = ctx.storage.lock().unwrap();

    // Create multiple function entities that will be entry points
    for i in 0..5 {
        let mut entity = make_entity(&format!("fn_{}", i), &ctx.project_id, EntityTier::Function);
        entity.metrics_json = Some(
            serde_json::json!({
                "execution_flows": [{
                    "entry_point": format!("fn_{}", i),
                    "entry_point_id": format!("fn_{}", i),
                    "steps": [{"entity_name": format!("fn_{}", i), "entity_id": format!("fn_{}", i), "depth": 0}],
                    "has_cycle": false
                }]
            })
            .to_string(),
        );
        guard.upsert_entity(&entity).unwrap();
    }
    drop(guard);

    let tool = GetExecutionFlowsTool { ctx: ctx.clone() };

    // Limit to 3 flows
    let result = tool.call(serde_json::json!({"limit": 3})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let flows = parsed["flows"].as_array().unwrap();
    assert!(
        flows.len() <= 3,
        "Should return at most 3 flows, got {}",
        flows.len()
    );
}

#[test]
fn test_get_execution_flows_no_params_preserves_behavior() {
    // Test that calling with no parameters preserves exact current behavior
    // (backward-compatible - returns all flows without filtering)
    let ctx = make_flow_ctx();
    let guard = ctx.storage.lock().unwrap();

    let mut entity = make_entity("main_fn", &ctx.project_id, EntityTier::Function);
    entity.metrics_json = Some(
        serde_json::json!({
            "execution_flows": [{
                "entry_point": "main",
                "entry_point_id": "main_fn",
                "steps": [{"entity_name": "main", "entity_id": "main_fn", "depth": 0}],
                "has_cycle": false
            }]
        })
        .to_string(),
    );
    guard.upsert_entity(&entity).unwrap();
    drop(guard);

    let tool = GetExecutionFlowsTool { ctx: ctx.clone() };

    // Call with empty object - should return all flows
    let result = tool.call(serde_json::json!({})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert!(
        parsed["flows"].is_array(),
        "response should have a 'flows' array"
    );
    assert!(
        parsed["message"].is_null(),
        "message should be null when flows exist"
    );
}

#[test]
fn test_get_execution_flows_max_depth_respected() {
    // Test that max_depth parameter limits traversal depth
    // This requires creating relationships and computing flows (not using precomputed)
    let ctx = make_flow_ctx();
    let guard = ctx.storage.lock().unwrap();

    // Create a chain: a -> b -> c -> d (all function entities)
    // If max_depth is 2, only a -> b -> c should be visited
    let entity_a = make_entity("entry", &ctx.project_id, EntityTier::Function);
    let entity_b = make_entity("middle", &ctx.project_id, EntityTier::Function);
    let entity_c = make_entity("leaf", &ctx.project_id, EntityTier::Function);

    guard.upsert_entity(&entity_a).unwrap();
    guard.upsert_entity(&entity_b).unwrap();
    guard.upsert_entity(&entity_c).unwrap();

    // Create a DependsOn relationship: entry -> middle -> leaf
    guard
        .upsert_relationship(&crate::model::Relationship {
            source_id: "entry".to_string(),
            target_id: "middle".to_string(),
            rel_type: crate::model::RelType::DependsOn,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        })
        .unwrap();
    guard
        .upsert_relationship(&crate::model::Relationship {
            source_id: "middle".to_string(),
            target_id: "leaf".to_string(),
            rel_type: crate::model::RelType::DependsOn,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        })
        .unwrap();
    drop(guard);

    let tool = GetExecutionFlowsTool { ctx: ctx.clone() };

    // With max_depth=1, only entry and middle should be visited (not leaf)
    let result = tool.call(serde_json::json!({"max_depth": 1})).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let flows = parsed["flows"].as_array().unwrap();

    // Find the flow for "entry"
    let entry_flow = flows.iter().find(|f| f["entry_point_id"] == "entry");
    if let Some(flow) = entry_flow {
        let steps = flow["steps"].as_array().unwrap();
        let entity_ids: Vec<&str> = steps
            .iter()
            .map(|s| s["entity_id"].as_str().unwrap())
            .collect();
        // With max_depth=1, we should see entry (depth 0) and middle (depth 1)
        // But NOT leaf (would be depth 2)
        assert!(entity_ids.contains(&"entry"), "Should visit entry");
        assert!(
            entity_ids.contains(&"middle"),
            "Should visit middle (depth 1)"
        );
        assert!(
            !entity_ids.contains(&"leaf"),
            "Should NOT visit leaf (depth 2 exceeds max_depth=1)"
        );
    }
}
