//! Golden test pinning the wire output of every MCP tool (issue #22,
//! acceptance criterion 2).
//!
//! The `#[tool_router]` macro in `tools.rs` builds each tool's wire description
//! from the `#[tool]` attribute literal and each tool's `input_schema` from the
//! `schemars`-derived `Params` struct. The hand-written `Tool::description()` /
//! `Tool::input_schema()` / `Tool::meta()` methods were removed in this PR
//! (see `src/retrieval/tool_trait.rs`); this test proves the wire surface is
//! unchanged by comparing a freshly-serialized dump of every tool against a
//! committed golden file (`tests/fixtures/mcp_wire_tools.json`) generated from
//! `main` BEFORE the removal.
//!
//! If this test fails, the wire output has changed — re-read the diff and
//! update the fixture in the same PR with a note explaining the change.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::handler::server::ServerHandler;

use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

/// The full allowlist string the golden dump needs in `LIEVO_MCP_TOOLS` so
/// `InterceptingMcpServer` serves every tool (the default allowlist would only
/// expose `lievo_explore`, which would make the dump partial). Derived from the
/// golden fixture's `name` fields, not hard-coded: a rename or retire in
/// `tools.rs` surfaces as a mismatch below rather than a silent partial dump.
fn fixture_tool_names(golden: &serde_json::Value) -> Vec<String> {
    golden["tools"]
        .as_array()
        .expect("golden fixture must contain a \"tools\" array")
        .iter()
        .map(|entry| {
            entry["name"]
                .as_str()
                .expect("each tool entry must have a \"name\"")
                .to_string()
        })
        .collect()
}

/// RAII guard that serializes `LIEVO_MCP_TOOLS` mutation against every other
/// env-mutating test in the crate (the lock holds for the guard's lifetime) and
/// guarantees the variable is removed before the guard drops — even on a panic
/// mid-test — so a poisoned lock or leaked variable cannot cascade.
struct ToolEnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl ToolEnvGuard {
    fn set_full_allowlist(tool_names: &[String]) -> Self {
        let _lock = crate::test_env_support::env_lock();
        unsafe { std::env::set_var("LIEVO_MCP_TOOLS", tool_names.join(",")) };
        Self { _lock }
    }
}

impl Drop for ToolEnvGuard {
    fn drop(&mut self) {
        // The env lock (`_lock`) is still held here, so this removal serializes
        // against other env-mutating tests.
        unsafe { std::env::remove_var("LIEVO_MCP_TOOLS") };
    }
}

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
fn build_golden_entries(tool_names: &[String]) -> serde_json::Value {
    let inner = make_server();
    for name in tool_names {
        assert!(
            inner.get_tool(name).is_some(),
            "expected tool {name} to be registered in tools.rs"
        );
    }
    // Note: a tool added to `tools.rs` without updating this fixture would not
    // be detected here — the fixture drives the allowlist, and enumerating the
    // router (`list_tools`) requires a `RequestContext`, so no production
    // accessor is available for a cross-check.
    // Reconstruct the interceptor with the full allowlist so it serves all
    // tools (the default allowlist would only expose lievo_explore, which
    // would make the dump partial).
    let _guard = ToolEnvGuard::set_full_allowlist(tool_names);
    let wrapped = crate::mcp::InterceptingMcpServer::new(inner);
    let mut entries = Vec::new();
    for name in tool_names {
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
    // The guard's Drop removes LIEVO_MCP_TOOLS (holding the env lock), so no
    // full allowlist leaks to other tests even if the rest of this function
    // panics.
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
    let tool_names = fixture_tool_names(&golden);
    let current = build_golden_entries(&tool_names);
    // Compare semantic JSON equality first: the wire surface is the JSON value,
    // not a particular byte layout, so a key-reorder or re-serialization of the
    // fixture must not fail the test.
    if current != golden {
        let golden_str = serde_json::to_string_pretty(&golden).unwrap();
        let current_str = serde_json::to_string_pretty(&current).unwrap();
        panic!(
            "MCP wire output changed from the committed golden file at {golden_path:?}. \
             The change is either intentional (update the golden file in the same PR \
             with a note in the commit message) or the removal of the hand-written Tool \
             trait methods accidentally altered the wire surface (fix the code, not the \
             golden).\n\n--- golden (tests/fixtures/mcp_wire_tools.json) ---\n{golden_str}\n\n--- current ---\n{current_str}\n"
        );
    }
}
