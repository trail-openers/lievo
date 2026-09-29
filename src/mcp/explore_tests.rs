//! Integration tests for the `lievo_explore` MCP tool (issues #680/#682).
//!
//! Wired via `#[cfg(test)] #[path]` from `tools.rs`. Exercises the full MCP
//! path (wrapper + success-shaped wrap + parameter deserialization) plus the
//! intercept layer's surface guarantees: instructions on initialize, the
//! default tool listing, and the `anthropic/alwaysLoad` metadata.
//!
//! Tier-1/tier-2 behavior (map vs verbatim source, Complexity Trap, 24K cap,
//! continuation pointers, UTF-8 boundary safety, payload-size ratio) is
//! covered at the retrieval layer in `src/retrieval/tools_explore_tests.rs`
//! and `src/retrieval/tools_explore_tier_tests.rs`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rmcp::handler::server::ServerHandler;

use crate::mcp::InterceptingMcpServer;
use crate::mcp::LievoMcpServer;
use crate::mcp::params::ExploreParams;
use crate::model::{Entity, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;

pub(super) fn make_server_with_repo_path(repo_path: PathBuf) -> LievoMcpServer {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("explore-int", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
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

/// Assert success-shaped (issue #680 P1.2): Ok result AND is_error != Some(true).
pub(super) fn assert_success_shaped(result: &CallToolResult) -> &str {
    assert!(
        result.is_error != Some(true),
        "expected success-shaped result (is_error != true), got: {}",
        text(result)
    );
    text(result)
}

pub(super) fn file_entity(
    id: &str,
    project_id: &str,
    name: &str,
    path: &str,
    summary: Option<&str>,
) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: name.to_string(),
        path: Some(path.to_string()),
        language: Some("Rust".to_string()),
        summary: summary.map(|s| s.to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

#[test]
fn lievo_explore_schema_has_query_max_files_include_source_only() {
    let server = make_server_with_repo_path(PathBuf::new());
    let tool = server
        .get_tool("lievo_explore")
        .expect("lievo_explore must be registered");
    let props = tool
        .input_schema
        .get("properties")
        .and_then(|p| p.as_object());
    let props = props.expect("schema must expose properties");
    assert!(props.contains_key("query"));
    assert!(props.contains_key("max_files"));
    assert!(props.contains_key("include_source"));
    // issue #723: include_depth is the optional depth-volume lever — a plain
    // boolean, optional, defaulting true (see the required-set guards below).
    assert!(props.contains_key("include_depth"));
    assert_eq!(props["include_depth"]["type"], serde_json::json!("boolean"));
    // No project_path parameter — the server binds to one project at
    // construction (ToolContext.project_id), issue #680.
    assert!(
        !props.contains_key("project_path"),
        "lievo_explore must NOT expose a project_path parameter"
    );
    // include_source and include_depth are the only disclosure/depth knobs —
    // no third one (tier/depth/detail) may exist.
    assert!(
        !props.contains_key("tier")
            && !props.contains_key("depth")
            && !props.contains_key("detail"),
        "no third disclosure knob may exist"
    );
}

#[test]
fn lievo_explore_schema_omits_project_path_and_defaults() {
    let server = make_server_with_repo_path(PathBuf::new());
    let tool = server.get_tool("lievo_explore").unwrap();
    // max_files has a default of 8 in ExploreParams; include_source defaults false.
    // Both must be optional (not required) so the minimal call is just {query}.
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
        !required.iter().any(|r| r == "max_files"),
        "max_files must be optional (default 8)"
    );
    assert!(
        !required.iter().any(|r| r == "include_source"),
        "include_source must be optional (default false = tier 1)"
    );
    assert!(
        !required.iter().any(|r| r == "include_depth"),
        "include_depth must be optional (default true = full depth, issue #723)"
    );
}

#[test]
fn lievo_explore_schema_exposes_scope_and_offset_as_optional() {
    // Issue #712 PM decision: scope + offset are the wire-boundary additions
    // for scope-membership listing, both optional, only `query` required.
    let server = make_server_with_repo_path(PathBuf::new());
    let tool = server.get_tool("lievo_explore").unwrap();
    let props = tool
        .input_schema
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("schema must expose properties");
    assert!(props.contains_key("scope"), "schema must expose `scope`");
    assert!(props.contains_key("offset"), "schema must expose `offset`");
    // rmcp/schemars renders `Option<String>` as a nullable type array
    // (`["string", "null"]`), unlike the hand-authored schema in
    // `ExploreTool::input_schema` (`tools_explore.rs`) which uses a plain
    // `"string"` — both are the same optional-string contract at the wire
    // boundary, just different schema renderings from two independent
    // schema authors (rmcp macro vs. hand-written json!).
    assert_eq!(
        props["scope"]["type"],
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
        !required.iter().any(|r| r == "scope"),
        "scope must be optional"
    );
    assert!(
        !required.iter().any(|r| r == "offset"),
        "offset must be optional"
    );
}

#[test]
fn lievo_explore_required_fields_do_not_diverge_between_the_two_schemas() {
    // Round-2 review (cheap guard, in lieu of the larger schema-unification
    // refactor): `ExploreParams` (rmcp/schemars-derived, `mcp/params.rs`)
    // and `ExploreTool::input_schema` (hand-written `json!`,
    // `retrieval/tools_explore.rs`) are two independently authored schemas
    // for the same wire contract. This test fails the moment their
    // `required` field sets diverge, without requiring the two to be
    // generated from one source.
    let server = make_server_with_repo_path(PathBuf::new());
    let mcp_tool = server.get_tool("lievo_explore").unwrap();
    let mcp_required: std::collections::BTreeSet<String> = mcp_tool
        .input_schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|r| r.iter().map(|v| v.as_str().unwrap().to_string()).collect())
        .unwrap_or_default();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("schema-guard", None).unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let explore_tool = crate::retrieval::tools::ExploreTool { ctx };
    let hand_written_required: std::collections::BTreeSet<String> = explore_tool
        .input_schema()
        .get("required")
        .and_then(|r| r.as_array())
        .map(|r| r.iter().map(|v| v.as_str().unwrap().to_string()).collect())
        .unwrap_or_default();

    assert_eq!(
        mcp_required, hand_written_required,
        "the rmcp/schemars-derived schema and the hand-written \
         ExploreTool::input_schema schema must agree on which fields are \
         required — they diverged; update whichever one is stale"
    );
}

// ---------------------------------------------------------------------------
// lievo_explore wrapper behavior through the MCP path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn lievo_explore_zero_match_returns_success_shaped_guidance() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:real.rs",
            &project.id,
            "real",
            "real.rs",
            None,
        ))
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
        .lievo_explore(Parameters(ExploreParams {
            query: "nomatchxyz".into(),
            include_depth: true,
            ..Default::default()
        }))
        .await
        .expect("zero matches must be success-shaped, not an MCP error");
    let guidance = assert_success_shaped(&result);
    let parsed: serde_json::Value = serde_json::from_str(guidance).unwrap();
    assert_eq!(parsed["symbols"].as_array().unwrap().len(), 0);
    assert!(
        parsed["warning"]
            .as_str()
            .unwrap()
            .contains("No matching files or symbols")
    );
}

#[tokio::test]
async fn lievo_explore_not_indexed_returns_refresh_guidance_success_shaped() {
    let server = make_server_with_repo_path(PathBuf::new()); // no entities at all
    let result = server
        .lievo_explore(Parameters(ExploreParams {
            query: "anything".into(),
            include_depth: true,
            ..Default::default()
        }))
        .await
        .expect("not-indexed must be success-shaped, not an MCP error");
    let guidance = assert_success_shaped(&result);
    let parsed: serde_json::Value = serde_json::from_str(guidance).unwrap();
    assert_eq!(parsed["symbols"].as_array().unwrap().len(), 0);
    let warning = parsed["warning"].as_str().unwrap();
    // Issue #864: the not-indexed guidance no longer tells the agent to run
    // `lievo refresh` (the agent cannot run lievo commands while the MCP
    // server is running; indexing is automatic in the background). The
    // success-shaped guidance still names the condition so the agent knows
    // what to do (use built-in tools / retry later).
    assert!(warning.contains("No entities indexed"), "got: {warning}");
    assert!(!warning.contains("lievo refresh"), "got: {warning}");
}

#[tokio::test]
async fn lievo_explore_empty_query_returns_success_shaped_guidance() {
    let server = make_server_with_repo_path(PathBuf::new());
    let result = server
        .lievo_explore(Parameters(ExploreParams {
            // Empty query — the point of this test; the Default impl also
            // yields `""` for query, but we keep it explicit so the test
            // reads as "empty query" rather than "default query".
            query: "".into(),
            include_depth: true,
            ..Default::default()
        }))
        .await
        .expect("empty query must be success-shaped, not an MCP error");
    assert_success_shaped(&result);
}

#[tokio::test]
async fn lievo_explore_tier2_returns_verbatim_line_numbered_source_grouped_by_file() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let body = "fn hello() { println!(\"hi\"); }\nfn world() { 1 }\n";
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(tmp.path().join("src/hello.rs"), body).unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:src/hello.rs",
            &project.id,
            "hello",
            "src/hello.rs",
            Some("Hello-world demo."),
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
            query: "hello".into(),
            include_source: true,
            include_depth: true,
            ..Default::default()
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert_eq!(sym["qualified_path"], "src/hello.rs");
    let source = sym["source"].as_str().unwrap();
    assert!(
        source.starts_with("1\tfn hello() { println!(\"hi\"); }\n2\tfn world() { 1 }\n"),
        "expected <n>\\t<line> Read-tool shape, got: {source}"
    );
}

#[tokio::test]
async fn lievo_explore_tier1_default_has_no_source_for_large_bodies() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let big = format!("fn auth() {{\n  {}\n}}\n", "let x = 1; ".repeat(200));
    std::fs::write(tmp.path().join("auth.rs"), &big).unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:auth.rs",
            &project.id,
            "auth",
            "auth.rs",
            Some("Implements token-based authentication."),
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
            query: "auth".into(),
            include_depth: true,
            ..Default::default()
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert!(
        sym.get("source").is_none(),
        "tier-1 large-body symbol must not carry a source body, got: {sym}"
    );
    assert!(sym.get("summary").is_some());
}

// ---------------------------------------------------------------------------
// MCP surface (intercept layer) — issue #680 AC verification
// ---------------------------------------------------------------------------

#[test]
fn default_listing_exposes_only_lievo_explore() {
    let inner = make_server_with_repo_path(PathBuf::new());
    let wrapped = InterceptingMcpServer::new(inner);
    // With LIEVO_MCP_TOOLS unset, the allowlist is exactly the primary tool.
    assert_eq!(
        wrapped.allowlist().enabled_tools(),
        vec!["lievo_explore".to_string()],
        "with LIEVO_MCP_TOOLS unset, exactly lievo_explore must be enabled"
    );
}

#[test]
fn wrapped_server_get_tool_returns_lievo_explore_with_always_load_meta() {
    let inner = make_server_with_repo_path(PathBuf::new());
    let wrapped = InterceptingMcpServer::new(inner);
    let tool = wrapped
        .get_tool("lievo_explore")
        .expect("lievo_explore must be enabled by default");
    let meta = tool
        .meta
        .as_ref()
        .expect("lievo_explore must carry _meta (alwaysLoad)");
    assert_eq!(
        meta.get("anthropic/alwaysLoad"),
        Some(&serde_json::Value::Bool(true)),
        "anthropic/alwaysLoad must be true"
    );
}

#[test]
fn wrapped_server_get_tool_returns_none_for_unlisted_tool() {
    let inner = make_server_with_repo_path(PathBuf::new());
    let wrapped = InterceptingMcpServer::new(inner);
    assert!(
        wrapped.get_tool("get_entity").is_none(),
        "unlisted tools must not be advertised"
    );
}

#[test]
fn instructions_reference_lievo_explore_first_and_env_var() {
    let inner = make_server_with_repo_path(PathBuf::new());
    let wrapped = InterceptingMcpServer::new(inner);
    let info = wrapped.get_info();
    let instructions = info
        .instructions
        .expect("initialize response must carry instructions");
    assert!(instructions.contains("lievo_explore"));
    assert!(instructions.to_uppercase().contains("FIRST"));
    assert!(instructions.contains("LIEVO_MCP_TOOLS"));
}

#[tokio::test]
async fn unlisted_tool_call_via_call_tool_returns_success_shaped_guidance() {
    // The interception point (issue #680 gate-resolution AC): a disabled or
    // unknown tool name returns Ok with is_error=false naming lievo_explore
    // and LIEVO_MCP_TOOLS, instead of rmcp's Err(invalid_params("tool not found")).
    let wrapped = InterceptingMcpServer::new(make_server_with_repo_path(PathBuf::new()));

    // Directly exercise the same hook the ServerHandler override calls:
    // intercept_call_tool is the exact function in the override's guard.
    let enabled_set: std::collections::HashSet<String> =
        wrapped.allowlist().enabled_tools().into_iter().collect();
    for name in [
        "get_entity",
        "search_entities",
        "read_file",
        "get_impact",
        "get_hotspots",
        "totally_unknown_tool",
    ] {
        let result = crate::mcp::intercept::intercept_call_tool(name, &enabled_set)
            .expect("disabled/unknown tool must be intercepted");
        assert_eq!(
            result.is_error,
            Some(false),
            "{name} must be success-shaped"
        );
        let guidance = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str().to_string())
            .unwrap();
        assert!(guidance.contains("lievo_explore"), "{name}");
        assert!(guidance.contains("LIEVO_MCP_TOOLS"), "{name}");
    }

    // The enabled-set itself confirms the default listing: primary tool in,
    // unlisted tools out (they are intercepted, not advertised).
    assert!(wrapped.allowlist().is_enabled("lievo_explore"));
    assert!(!wrapped.allowlist().is_enabled("get_entity"));
}

// The crate-wide env-var lock (issue #863): `LIEVO_MCP_TOOLS` mutations here
// must serialize against every other env-mutating test in the crate.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::test_env_support::env_lock()
}

// ---------------------------------------------------------------------------
// get_impact allowlisting (issue #840): named via LIEVO_MCP_TOOLS, not by
// default. The process-wide env is read once by `Allowlist::capture` (and
// other in-process tests rely on it being unset), so the named case is
// exercised against the pure parser plus the enablement rule — primary
// tool always in, every named tool additionally in, unnamed tools out.
// ---------------------------------------------------------------------------

#[test]
fn get_impact_allowlisting_is_empty_when_env_unset_or_empty() {
    let _guard = env_lock();
    unsafe { std::env::remove_var("LIEVO_MCP_TOOLS") };
    assert!(
        crate::mcp::intercept::parse_tools_allowlist().is_empty(),
        "unset LIEVO_MCP_TOOLS must list no extra tools — get_impact stays hidden"
    );
    unsafe { std::env::set_var("LIEVO_MCP_TOOLS", "") };
    assert!(
        crate::mcp::intercept::parse_tools_allowlist().is_empty(),
        "empty LIEVO_MCP_TOOLS must list no extra tools — get_impact stays hidden"
    );
    unsafe { std::env::remove_var("LIEVO_MCP_TOOLS") };
}

#[test]
fn get_impact_allowlisting_enables_named_tool_only() {
    let named = crate::mcp::intercept::parse_tool_names("lievo_explore, get_impact ,get_entity");
    let enabled: std::collections::HashSet<&str> = named
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("lievo_explore"))
        .collect();
    // Named: get_impact is listed when named in the allowlist…
    assert!(enabled.contains("get_impact"));
    assert!(enabled.contains("get_entity"));
    // …and the primary tool stays listed regardless (issue #680).
    assert!(enabled.contains("lievo_explore"));
    // Unnamed tools remain hidden — the allowlist names, it does not default.
    assert!(!enabled.contains("get_hotspots"));
    assert!(!enabled.contains("read_file"));
}

#[tokio::test]
async fn lievo_explore_utf8_multibyte_output_is_valid_json() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let content = format!(
        "fn x() {{ let s = \"{}\"; }}\n",
        "héllo — wörld — 日本語".repeat(10)
    );
    std::fs::write(tmp.path().join("utf8.rs"), &content).unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:utf8.rs",
            &project.id,
            "utf8",
            "utf8.rs",
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
            query: "utf8".into(),
            include_source: true,
            include_depth: true,
            ..Default::default()
        }))
        .await
        .unwrap();
    // Multi-byte content at the cap must not split a char: output stays valid JSON.
    let parsed: serde_json::Value = serde_json::from_str(text(&result)).unwrap();
    let _ = parsed["symbols"].as_array().unwrap();
}
