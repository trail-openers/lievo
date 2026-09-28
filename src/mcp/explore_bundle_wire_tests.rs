//! Subsystem-bundle mode — MCP-wire layer (issue #743, task-b).
//!
//! Wired via `#[cfg(test)] #[path]` from `tools.rs` (sibling of the
//! `explore_tests`/`explore_scope_tests` modules there, hence the
//! `super::explore_tests::` imports).
//!
//! This file is in task-b's scope (the MCP wire: `src/mcp/params.rs`
//! `ExploreParams` + `src/mcp/tools.rs` `explore_input_json` / wire
//! description). It exercises ONLY the wire boundary — parameter
//! deserialization and the conditional `bundle` key forwarding — through the
//! full `LievoMcpServer` MCP path.
//!
//! The response-shape ACs (per-file source, intra-scope edges,
//! `not_shown_files`, structured `completeness`, 24K cap) are implemented at
//! the retrieval layer by the sibling workstream (`tools_explore_bundle.rs`)
//! and are asserted there. These wire tests are written to pass against the
//! documented bundle contract and do NOT depend on the sibling module's
//! internal structure.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::handler::server::ServerHandler;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::Value;

use super::explore_tests::file_entity;
use crate::mcp::LievoMcpServer;
use crate::mcp::params::ExploreParams;
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

fn make_server(storage: SqliteStorage, project_id: String, repo_path: PathBuf) -> LievoMcpServer {
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id,
        repo_path,
        output_dir: None,
        zero_repo_guidance: None,
    });
    LievoMcpServer::new(ctx)
}

/// A bundle request through the full MCP wire path: the `bundle` param is
/// present (non-blank), so `explore_input_json` attaches the `bundle` key and
/// the retrieval layer short-circuits into bundle mode.
async fn call_bundle(server: &LievoMcpServer, bundle: &str) -> Value {
    let result = server
        .lievo_explore(Parameters(ExploreParams {
            query: String::new(),
            include_source: true,
            include_depth: false,
            bundle: Some(bundle.to_string()),
            ..Default::default()
        }))
        .await
        .unwrap();
    serde_json::from_str(text(&result)).unwrap()
}

#[tokio::test]
async fn wire_bundle_present_triggers_bundle_mode_not_query_or_scope() {
    // A non-blank `bundle` param must short-circuit query word-match,
    // `files`, and `scope` listing into bundle mode. Seed a file matching the
    // query word "alpha" that is OUT of the bundle scope, plus a file under
    // the scope: a fall-through to word-match/scope listing would NOT produce
    // the structured completeness object.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    let _repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("alpha.rs"), "fn alpha() {}\n").unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:alpha.rs",
            &project.id,
            "alpha",
            "alpha.rs",
            None,
        ))
        .unwrap();
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/mod.rs"), "fn mod() {}\n").unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:src/mod.rs",
            &project.id,
            "mod",
            "src/mod.rs",
            None,
        ))
        .unwrap();
    let server = make_server(storage, project.id, tmp.path().canonicalize().unwrap());

    // bundle="src" with a query word that would match "alpha" elsewhere: the
    // response MUST be the bundle shape (structured completeness object), not
    // the word-match/scope shape.
    let parsed = call_bundle(&server, "src").await;
    let completeness = parsed
        .get("completeness")
        .expect("bundle mode must return a completeness field");
    let completeness_obj = completeness.as_object().expect(
        "bundle mode must return the STRUCTURED completeness object \
         {complete, omitted_files, omitted_edges}, not a string",
    );
    assert!(
        completeness_obj
            .get("complete")
            .and_then(|v| v.as_bool())
            .is_some(),
        "structured completeness must carry `complete: bool`, got: {completeness}"
    );
    assert!(
        completeness_obj
            .get("omitted_files")
            .and_then(|v| v.as_u64())
            .is_some(),
        "structured completeness must carry `omitted_files: int`, got: {completeness}"
    );
    assert!(
        completeness_obj
            .get("omitted_edges")
            .and_then(|v| v.as_u64())
            .is_some(),
        "structured completeness must carry `omitted_edges: int`, got: {completeness}"
    );
    // Not a scope-listing shape (which has a `files` array) — the
    // bundle-specific key is `not_shown_files`, distinct from the word-match
    // integer `not_shown`.
    assert!(
        parsed.get("files").is_none(),
        "bundle mode must not emit the scope-listing `files` key: {parsed}"
    );
}

#[tokio::test]
async fn wire_bundle_absent_does_not_trigger_bundle_mode() {
    // A query-only call (no bundle) must NOT activate bundle mode — it goes
    // through the word-match path and returns a map (string completeness or
    // guidance), never the structured bundle completeness object.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    let _repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("solo.rs"), "fn solo() {}\n").unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:solo.rs",
            &project.id,
            "solo",
            "solo.rs",
            None,
        ))
        .unwrap();
    let server = make_server(storage, project.id, tmp.path().canonicalize().unwrap());

    let result = server
        .lievo_explore(Parameters(ExploreParams {
            query: "solo".to_string(),
            include_source: true,
            include_depth: false,
            // absent: must NOT trigger bundle mode
            bundle: None,
            ..Default::default()
        }))
        .await
        .unwrap();
    let parsed: Value = serde_json::from_str(text(&result)).unwrap();
    if let Some(completeness) = parsed.get("completeness") {
        assert!(
            completeness.is_string(),
            "without `bundle`, completeness must be the string form, not the \
             structured bundle object, got: {completeness}"
        );
    }
    assert!(
        parsed.get("not_shown_files").is_none(),
        "without `bundle`, the bundle-only `not_shown_files` key must be absent: {parsed}"
    );
}

#[tokio::test]
async fn wire_blank_bundle_is_treated_as_absent() {
    // A blank/whitespace-only `bundle` value is treated as absent (mirroring
    // how a blank `scope` does not trigger scope mode): with an empty query
    // and no scope/files, the retrieval layer falls through to empty-query
    // guidance, never bundle mode (no structured completeness object).
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    let _repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    let server = make_server(storage, project.id, PathBuf::new());

    let result = server
        .lievo_explore(Parameters(ExploreParams {
            query: String::new(),
            include_source: true,
            include_depth: false,
            bundle: Some("   ".to_string()), // blank: treated as absent
            ..Default::default()
        }))
        .await
        .unwrap();
    let parsed: Value = serde_json::from_str(text(&result)).unwrap();
    let completeness = parsed.get("completeness");
    assert!(
        completeness.is_none() || !completeness.unwrap().is_object(),
        "blank bundle must not trigger bundle mode (no structured completeness): {parsed}"
    );
}

#[tokio::test]
async fn wire_bundle_present_in_mcp_schema_as_optional_string() {
    // The MCP wire schema (rmcp/schemars-rendered ExploreParams) exposes
    // `bundle` as an optional string and does NOT require it.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    let server = make_server(storage, project.id, PathBuf::new());
    let tool = server
        .get_tool("lievo_explore")
        .expect("lievo_explore must be registered");
    let props = tool
        .input_schema
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("schema must expose properties");
    assert!(
        props.contains_key("bundle"),
        "MCP schema must expose `bundle`"
    );
    // rmcp/schemars renders Option<String> as ["string", "null"].
    assert_eq!(
        props["bundle"]["type"],
        serde_json::json!(["string", "null"])
    );
    let required = tool
        .input_schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|r| {
            r.iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        !required.iter().any(|r| r == "bundle"),
        "bundle must be optional in the MCP wire schema"
    );
    // query must remain optional on the wire: bundle mode is a scope
    // selection with no meaningful query, so no field is required.
    assert!(
        required.iter().all(|r| r != "query"),
        "query must stay optional on the wire (bundle mode is a scope selection)"
    );
}
