//! Golden test pinning the byte-identical wire output of every MCP tool
//! (issue #22, acceptance criterion 2).
//!
//! The `#[tool_router]` macro in `tools.rs` builds each tool's wire description
//! from the `#[tool]` attribute literal and each tool's `input_schema` from the
//! `schemars`-derived `Params` struct. The hand-written `Tool::description()` /
//! `Tool::input_schema()` / `Tool::meta()` methods were removed in this PR
//! (see `src/retrieval/tool_trait.rs`); this test proves the wire surface is
//! unchanged by comparing a freshly-serialized dump of every tool against a
//! committed golden file (`tests/fixtures/mcp_wire_tools.json`) generated
//! from `main` BEFORE the removal.
//!
//! If this test fails, the wire output has changed — re-read the diff and
//! update the fixture in the same PR with a note explaining the change.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::handler::server::ServerHandler;

use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

/// The wire tools registered by the `#[tool_router]` macro in `tools.rs`
/// (name literals from the `#[tool(name = ...)]` attributes). Kept in sync
/// with the fixture; a rename or retire in `tools.rs` fails this test loudly
/// so the golden file is updated in the same PR.
const WIRE_TOOL_NAMES: [&str; 15] = [
    "search_entities",
    "get_entity",
    "list_relationships",
    "list_subsystems",
    "get_function",
    "get_conventions",
    "get_insights",
    "read_file",
    "list_directory",
    "get_execution_flows",
    "list_project_docs",
    "read_project_doc",
    "get_impact",
    "get_hotspots",
    "lievo_explore",
];

fn make_server() -> crate::mcp::LievoMcpServer {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("wire-golden", None).unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    crate::mcp::LievoMcpServer::new(ctx)
}

/// Serialize every wire tool (name, description, input_schema, meta) the way
/// the golden file was generated, so the comparison is apples-to-apples.
fn build_golden_entries() -> serde_json::Value {
    let _guard = crate::test_env_support::env_lock();
    let inner = make_server();
    // Verify every expected tool is registered on the inner server before
    // wrapping, so a rename/retire in tools.rs fails loudly rather than
    // silently producing a partial dump.
    for name in WIRE_TOOL_NAMES {
        assert!(
            inner.get_tool(name).is_some(),
            "expected tool {name} to be registered in tools.rs"
        );
    }
    // Reconstruct the interceptor with the full allowlist so it serves all
    // tools (the default allowlist would only expose lievo_explore, which
    // would make the dump partial).
    unsafe { std::env::set_var("LIEVO_MCP_TOOLS", WIRE_TOOL_NAMES.join(",")) };
    let wrapped = crate::mcp::InterceptingMcpServer::new(inner);
    let names: Vec<&str> = WIRE_TOOL_NAMES.to_vec();
    let mut entries = Vec::new();
    for name in names {
        let tool = wrapped
            .get_tool(name)
            .unwrap_or_else(|| panic!("tool {name} not served after full allowlist"));
        entries.push(serde_json::json!({
            "name": tool.name,
            "description": tool.description.as_ref().map(|d| d.to_string()),
            "input_schema": (*tool.input_schema).clone(),
            "meta": tool.meta.as_ref().map(|m| m.0.clone()),
        }));
    }
    let doc = serde_json::json!({ "tools": entries });
    // Reset the env var so this test doesn't leak the full allowlist to
    // other tests (env_lock serializes us; removing the var here is the
    // polite thing to do for any follow-on test in the same process).
    unsafe { std::env::remove_var("LIEVO_MCP_TOOLS") };
    doc
}

#[test]
fn wire_tools_match_golden() {
    let golden_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp_wire_tools.json");
    let golden_bytes = std::fs::read(&golden_path)
        .unwrap_or_else(|e| panic!("failed to read golden file {golden_path:?}: {e}"));
    let golden: serde_json::Value = serde_json::from_slice(&golden_bytes)
        .unwrap_or_else(|e| panic!("golden file {golden_path:?} is not valid JSON: {e}"));
    let current = build_golden_entries();
    let golden_str = serde_json::to_string_pretty(&golden).unwrap();
    let current_str = serde_json::to_string_pretty(&current).unwrap();
    assert_eq!(
        current_str, golden_str,
        "MCP wire output changed from the committed golden file at {golden_path:?}. \
         Either the change is intentional (update the golden file in the same PR with a \
         note in the commit message) or the removal of the hand-written Tool trait \
         methods accidentally altered the wire surface (fix the code, not the golden)."
    );
}
