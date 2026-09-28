//! Tests for the success-shaped tool-call interception (issue #680).
//!
//! These tests exercise the pure intercept logic (`parse_tools_allowlist`,
//! `guidance_for_unlisted`, `intercept_call_tool`, `INSTRUCTIONS`) directly,
//! and the `Allowlist` / `InterceptingMcpServer` wrappers that the server
//! exposes. Env-var-dependent behavior is serialized with a `Mutex` so one
//! test's `LIEVO_MCP_TOOLS` mutation is never observed by another.
//!
//! The interception path itself (the `call_tool` override on
//! `InterceptingMcpServer`) is exercised indirectly here through
//! `intercept_call_tool`, which is the exact function the override calls.
//! A full async integration test would require constructing an rmcp
//! `RequestContext<RoleServer>`, whose `Peer` field is non-public; the
//! override is a thin pass-through to `intercept_call_tool` plus delegation
//! to the inner server, so the pure-logic tests cover the contract.

use std::sync::{Arc, Mutex};

use rmcp::handler::server::ServerHandler;

use crate::mcp::InterceptingMcpServer;
use crate::mcp::intercept::{
    INSTRUCTIONS, guidance_for_unlisted, intercept_call_tool, parse_tools_allowlist,
};
use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

use std::path::PathBuf;

/// The crate-wide env-var lock (issue #863): `LIEVO_MCP_TOOLS` mutations here
/// must serialize against every other env-mutating test in the crate —
/// per-module mutexes do not.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::test_env_support::env_lock()
}

fn make_wrapped() -> InterceptingMcpServer {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test-project", None).unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let inner = crate::mcp::LievoMcpServer::new(ctx);
    InterceptingMcpServer::new(inner)
}

fn call_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str().to_string())
        .expect("text content")
}

// ---------------------------------------------------------------------------
// Pure intercept logic — no env var, no server.
// ---------------------------------------------------------------------------

#[test]
fn intercept_returns_none_for_enabled_tool() {
    let mut enabled = std::collections::HashSet::new();
    enabled.insert("lievo_explore".to_string());
    enabled.insert("get_entity".to_string());

    assert!(intercept_call_tool("lievo_explore", &enabled).is_none());
    assert!(intercept_call_tool("get_entity", &enabled).is_none());
}

#[test]
fn intercept_returns_success_shape_for_disabled_tool() {
    let mut enabled = std::collections::HashSet::new();
    enabled.insert("lievo_explore".to_string());

    let result = intercept_call_tool("get_entity", &enabled).expect("must intercept");
    assert_eq!(
        result.is_error,
        Some(false),
        "intercept must be success-shaped"
    );
    let text = call_text(&result);
    assert!(
        text.contains("get_entity"),
        "guidance must name the tool called: {text}"
    );
    assert!(
        text.contains("lievo_explore"),
        "guidance must name the primary tool: {text}"
    );
    assert!(
        text.contains("LIEVO_MCP_TOOLS"),
        "guidance must name the env var: {text}"
    );
}

#[test]
fn intercept_returns_success_shape_for_unknown_tool() {
    let enabled = std::collections::HashSet::new();
    let result = intercept_call_tool("totally_unknown_tool", &enabled).expect("must intercept");
    assert_eq!(result.is_error, Some(false));
    let text = call_text(&result);
    assert!(text.contains("totally_unknown_tool"));
    assert!(text.contains("lievo_explore"));
    assert!(text.contains("LIEVO_MCP_TOOLS"));
}

#[test]
fn guidance_lists_enabled_tools_in_sorted_order() {
    let enabled: Vec<String> = vec!["get_entity".to_string(), "read_file".to_string()];
    let text = guidance_for_unlisted("search_entities", &enabled);
    let get_entity_pos = text.find("get_entity").expect("names get_entity");
    let read_file_pos = text.find("read_file").expect("names read_file");
    assert!(
        get_entity_pos < read_file_pos,
        "enabled tools must be sorted: {text}"
    );
}

#[test]
fn guidance_mentions_empty_enabled_set_when_no_tools() {
    let text = guidance_for_unlisted("anything", &[]);
    assert!(
        text.contains("none (only the primary tool is available)"),
        "empty enabled set must show the placeholder: {text}"
    );
}

#[test]
fn instructions_direct_agent_to_lievo_explore_first() {
    assert!(INSTRUCTIONS.contains("lievo_explore"));
    assert!(INSTRUCTIONS.contains("FIRST"));
    assert!(INSTRUCTIONS.contains("LIEVO_MCP_TOOLS"));
    assert!(INSTRUCTIONS.contains("Anti-pattern") || INSTRUCTIONS.contains("anti-pattern"));
    // Issue #864: the server indexes the repo automatically in the
    // background when it starts; the instructions must not tell the agent
    // to run `lievo refresh` (the agent cannot run lievo commands while
    // the MCP server is running). The directive instead covers the
    // in-progress case: retry the call shortly.
    assert!(!INSTRUCTIONS.contains("lievo refresh"));
    assert!(INSTRUCTIONS.contains("in progress"), "got: {INSTRUCTIONS}");
    assert!(INSTRUCTIONS.contains("retry"), "got: {INSTRUCTIONS}");
}

#[test]
fn instructions_direct_agent_not_to_reread_rerequest_or_batch() {
    // F1: do not re-read a file whose source was already received
    assert!(
        INSTRUCTIONS.contains("Do not re-read a file whose source you already received"),
        "F1 missing do-not-re-read directive"
    );
    // F2: do not re-request relationship/edge information already received
    assert!(
        INSTRUCTIONS.contains("do not re-request relationship or edge information"),
        "F2 missing do-not-re-request directive"
    );
    // F3: batch multiple files/symbols in one call rather than one call per file
    assert!(
        INSTRUCTIONS.contains("one call per file"),
        "F3 missing batching directive (one call per file)"
    );
    assert!(
        INSTRUCTIONS.contains("default max_files is 8"),
        "F3 missing max_files default reference"
    );
    // F4: stop when the answer is in what you have (uses only today's signals)
    assert!(
        INSTRUCTIONS.contains("Stop when the answer is in what you have"),
        "F4 missing stop-when-enough directive"
    );
    assert!(
        INSTRUCTIONS.contains("returned equals total"),
        "F4 missing returned==total stopping signal"
    );
    assert!(
        INSTRUCTIONS.contains("response without a continuation pointer is complete"),
        "F4 missing no-continuation-completeness signal"
    );
    // No phantom completeness field (that belongs to #743, not yet landed)
    assert!(
        !INSTRUCTIONS.contains("completeness"),
        "must not reference a completeness field that does not exist yet"
    );
}

#[test]
fn guidance_for_unlisted_includes_anti_pattern_directives() {
    let text = guidance_for_unlisted("some_tool", &[]);
    // F1
    assert!(
        text.contains("Do not re-read a file whose source you already received"),
        "guidance missing do-not-re-read directive"
    );
    // F2
    assert!(
        text.contains("do not re-request relationship or edge information"),
        "guidance missing do-not-re-request directive"
    );
    // F3
    assert!(
        text.contains("one call per file"),
        "guidance missing batching directive"
    );
    // F4
    assert!(
        text.contains("returned/total/next continuation"),
        "guidance missing continuation pointer reference"
    );
    // No phantom completeness field
    assert!(
        !text.contains("completeness"),
        "guidance must not reference a completeness field that does not exist yet"
    );
}

// ---------------------------------------------------------------------------
// parse_tools_allowlist — env-var dependent; serialized.
// ---------------------------------------------------------------------------

#[test]
fn parse_tools_allowlist_empty_when_unset() {
    let _guard = env_lock();
    unsafe { std::env::remove_var("LIEVO_MCP_TOOLS") };
    assert!(parse_tools_allowlist().is_empty());
}

#[test]
fn parse_tools_allowlist_empty_string_yields_empty_set() {
    let _guard = env_lock();
    unsafe { std::env::set_var("LIEVO_MCP_TOOLS", "") };
    assert!(parse_tools_allowlist().is_empty());
}

#[test]
fn parse_tools_allowlist_parses_comma_separated_list() {
    let _guard = env_lock();
    unsafe { std::env::set_var("LIEVO_MCP_TOOLS", "get_entity,read_file , get_impact") };
    let got = parse_tools_allowlist();
    assert_eq!(
        got,
        [
            "get_entity".to_string(),
            "read_file".to_string(),
            "get_impact".to_string()
        ]
        .into_iter()
        .collect::<std::collections::HashSet<_>>()
    );
    unsafe { std::env::remove_var("LIEVO_MCP_TOOLS") };
}

#[test]
fn parse_tools_allowlist_drops_empty_segments() {
    let _guard = env_lock();
    unsafe { std::env::set_var("LIEVO_MCP_TOOLS", ",get_entity,,read_file,") };
    let got = parse_tools_allowlist();
    assert_eq!(
        got,
        ["get_entity".to_string(), "read_file".to_string()]
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
    );
    unsafe { std::env::remove_var("LIEVO_MCP_TOOLS") };
}

// ---------------------------------------------------------------------------
// InterceptingMcpServer — construction + get_info (no RequestContext needed).
// ---------------------------------------------------------------------------

#[test]
fn wrapped_server_get_info_has_instructions() {
    let wrapped = make_wrapped();
    let info = wrapped.get_info();
    let instructions = info
        .instructions
        .expect("instructions must be set on get_info");
    assert!(instructions.contains("lievo_explore"));
    assert!(instructions.contains("LIEVO_MCP_TOOLS"));
}

#[test]
fn wrapped_server_exposes_inner_server() {
    let wrapped = make_wrapped();
    let inner: &crate::mcp::LievoMcpServer = wrapped.inner();
    // Inner's get_info() must still work through the reference.
    let _ = inner.get_info();
}

#[test]
fn wrapped_server_allowlist_contains_primary_tool() {
    let wrapped = make_wrapped();
    let allowlist = wrapped.allowlist();
    assert!(
        allowlist.is_enabled("lievo_explore"),
        "lievo_explore must always be in the allowlist"
    );
}
