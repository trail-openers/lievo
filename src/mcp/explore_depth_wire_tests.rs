//! `include_depth` wire-path tests for the `lievo_explore` MCP tool (issue #723).
//!
//! Wired via `#[cfg(test)] #[path]` from `tools.rs` (sibling of the
//! `explore_tests` module there, hence the `super::explore_tests::` imports).
//! Split out of `explore_tests.rs` under the 800-line test-file budget
//! (issue #702 policy, scripts/check_file_size.rs).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::explore_tests::{assert_success_shaped, file_entity};
use crate::mcp::LievoMcpServer;
use crate::mcp::params::ExploreParams;
use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use rmcp::handler::server::wrapper::Parameters;

use rmcp::model::CallToolResult;

fn text(result: &CallToolResult) -> &str {
    result
        .content
        .first()
        .and_then(|content| content.as_text())
        .map(|text| text.text.as_str())
        .expect("text content")
}

#[test]
fn explore_params_include_depth_defaults_false_when_absent() {
    // Wire-boundary guarantee (issue #723, flipped #731): a client that omits
    // the param gets the lean default-false, and an explicit true is honored.
    let params: ExploreParams =
        serde_json::from_str(r#"{"query": "x"}"#).expect("include_depth must be optional");
    assert!(
        !params.include_depth,
        "absent include_depth must default to false (lean response)"
    );
    let explicit: ExploreParams =
        serde_json::from_str(r#"{"query": "x", "include_depth": true}"#).unwrap();
    assert!(explicit.include_depth, "explicit true must be honored");
}

#[tokio::test]
async fn lievo_explore_include_depth_false_drops_depth_keys_through_mcp_wire() {
    // End-to-end guard for the hand-built json! forwarding in tools.rs:
    // if include_depth is not forwarded, the MCP path silently defaults to
    // true while the retrieval path honors the flag.
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

    let full = server
        .lievo_explore(Parameters(ExploreParams {
            query: "depth".into(),
            include_depth: true,
            ..Default::default()
        }))
        .await
        .unwrap();
    let minimal = server
        .lievo_explore(Parameters(ExploreParams {
            query: "depth".into(),
            // Explicit false, not the new bare default: the lean shape is
            // pinned end-to-end for the default in
            // explore_wire_default_tests.rs.
            include_depth: false,
            ..Default::default()
        }))
        .await
        .unwrap();

    let full_parsed: serde_json::Value = serde_json::from_str(text(&full)).unwrap();
    let full_sym = &full_parsed["symbols"].as_array().unwrap()[0];
    assert!(
        !full_sym["call_paths"].as_array().unwrap().is_empty(),
        "full-depth call must carry a non-empty call_paths: {full_sym}"
    );

    let parsed: serde_json::Value = serde_json::from_str(text(&minimal)).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert!(
        sym.get("call_paths").is_none(),
        "include_depth=false through the MCP wire must omit call_paths: {sym}"
    );
    assert!(sym.get("blast_radius").is_none());
    // entity_id intentionally absent from the per-symbol payload (issue #834).
    assert!(sym.get("entity_id").is_none());
    for key in [
        "name",
        "kind",
        "qualified_path",
        "signature",
        "score",
        "reason",
    ] {
        assert!(sym.get(key).is_some(), "missing '{key}': {sym}");
    }
    // Measurably smaller serialized response through the wire (issue #723 AC #4).
    assert!(
        text(&minimal).len() < text(&full).len(),
        "minimal ({}) must be shorter than full ({})",
        text(&minimal).len(),
        text(&full).len()
    );
}

#[tokio::test]
async fn lievo_explore_include_depth_false_orthogonal_to_include_source_through_mcp_wire() {
    // include_depth=false + include_source=true: tier-2 source bodies still
    // come back while the depth keys are omitted (issue #723 edge case).
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("ortho.rs"), "fn tiny() { 1 }\n").unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:ortho.rs",
            &project.id,
            "ortho",
            "ortho.rs",
            None,
        ))
        .unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: tmp.path().to_path_buf(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);
    let result = server
        .lievo_explore(Parameters(ExploreParams {
            query: "ortho".into(),
            include_source: true,
            include_depth: false,
            ..Default::default()
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert!(sym.get("source").is_some(), "source must be present: {sym}");
    assert!(sym.get("call_paths").is_none());
    assert!(sym.get("blast_radius").is_none());
}

#[tokio::test]
async fn lievo_explore_include_depth_false_inert_in_scope_mode() {
    // The flag is inert in scope mode: the scope-listing shape is unchanged
    // (files[] entries carry no depth keys regardless of the flag).
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    for (id, name, path) in [
        ("p:repo1:file:src/b.rs", "b", "src/b.rs"),
        ("p:repo1:file:src/a.rs", "a", "src/a.rs"),
    ] {
        storage
            .upsert_entity(&file_entity(id, &project.id, name, path, None))
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
    let result = server
        .lievo_explore(Parameters(ExploreParams {
            query: "unused".into(),
            include_depth: false,
            scope: Some("src".into()),
            ..Default::default()
        }))
        .await
        .expect("scope listing with include_depth=false must be success-shaped");
    let parsed: serde_json::Value = serde_json::from_str(assert_success_shaped(&result)).unwrap();
    let files = parsed["files"].as_array().unwrap();
    assert_eq!(
        files.len(),
        2,
        "scope listing shape must be unchanged: {parsed}"
    );
    for f in files {
        assert!(f.get("call_paths").is_none());
        assert!(f.get("blast_radius").is_none());
        assert!(f.get("source").is_none());
    }
    assert_eq!(parsed["total"], 2);
}
