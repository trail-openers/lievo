//! Bare-default `lievo_explore` wire test (issue #731): a call that omits
//! `include_depth` must return symbols WITHOUT `call_paths`/`blast_radius`
//! (the new lean default) through the full MCP path.
//!
//! Lives in its own `#[path]`-wired module because
//! `explore_tests.rs` sits at the 800-line test-file budget.

use std::sync::{Arc, Mutex};

use rmcp::handler::server::wrapper::Parameters;

use super::explore_tests::file_entity;
use crate::mcp::LievoMcpServer;
use crate::mcp::params::ExploreParams;
use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use rmcp::model::CallToolResult;

fn text(result: &CallToolResult) -> &str {
    result
        .content
        .first()
        .and_then(|content| content.as_text())
        .map(|text| text.text.as_str())
        .expect("text content")
}

#[tokio::test]
async fn lievo_explore_bare_default_drops_depth_keys_through_mcp_wire() {
    // Issue #731: with `include_depth` absent on the wire, the response is
    // lean (no call_paths/blast_radius); the explicit-true arm still carries
    // them, so the flip is visible end-to-end, not just in serde.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("depth.rs"), "fn caller() { callee() }\n").unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:depth.rs",
            &project.id,
            "depth",
            "depth.rs",
            None,
        ))
        .unwrap();
    let make_fn = |id: &str, name: &str| Entity {
        id: id.to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::Function,
        parent_id: Some("p:repo1:file:depth.rs".to_string()),
        name: name.to_string(),
        path: Some("depth.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage
        .upsert_entity(&make_fn("fn-depth-caller", "caller"))
        .unwrap();
    storage
        .upsert_entity(&make_fn("fn-depth-callee", "callee"))
        .unwrap();
    storage
        .upsert_relationship(&Relationship {
            source_id: "fn-depth-caller".to_string(),
            target_id: "fn-depth-callee".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        })
        .unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: tmp.path().to_path_buf(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);

    // Bare default: include_depth field absent on the wire.
    let bare = server
        .lievo_explore(Parameters(ExploreParams {
            query: "depth".into(),
            include_depth: false, // absent on the wire: serde applies the default
            ..Default::default()
        }))
        .await
        .unwrap();
    let bare_parsed: serde_json::Value = serde_json::from_str(text(&bare)).unwrap();
    assert_eq!(
        bare_parsed["symbols"].as_array().unwrap().len(),
        1,
        "bare-default call must still return the matched symbol: {bare_parsed}"
    );
    for sym in bare_parsed["symbols"].as_array().unwrap() {
        assert!(
            sym.get("call_paths").is_none(),
            "bare-default (absent include_depth) must omit call_paths: {sym}"
        );
        assert!(
            sym.get("blast_radius").is_none(),
            "bare-default (absent include_depth) must omit blast_radius: {sym}"
        );
    }

    // Explicit true still carries the full neighborhood (opt-in unchanged).
    let full = server
        .lievo_explore(Parameters(ExploreParams {
            query: "depth".into(),
            include_depth: true,
            ..Default::default()
        }))
        .await
        .unwrap();
    let full_parsed: serde_json::Value = serde_json::from_str(text(&full)).unwrap();
    let full_sym = &full_parsed["symbols"].as_array().unwrap()[0];
    assert!(
        !full_sym["call_paths"].as_array().unwrap().is_empty(),
        "explicit include_depth=true must carry a non-empty call_paths: {full_sym}"
    );
}
