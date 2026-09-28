//! Resident prompt-surface size pins (issue #850).
//!
//! Wired via `#[cfg(test)] #[path]` from `tools.rs` (sibling of the
//! `explore_tests`/`explore_bundle_wire_tests` modules there, hence the
//! `super::explore_tests::` helper imports).
//!
//! The lievo_explore resident prompt surface — the input schema, the tool
//! description and the server instructions — is sent to the agent on EVERY
//! turn, so its size is a per-run token cost. Issue #850 trimmed it; these
//! upper-bound pins stop it silently regrowing. Bounds are measured the same
//! way as the issue's numbers (`chars().count()` of the serialized strings
//! the client receives) with ~25% headroom over the post-trim measurement,
//! so legitimate edits pass while a re-growth fails the build.

use std::path::PathBuf;

use rmcp::handler::server::ServerHandler;

use super::explore_tests::make_server_with_repo_path;

/// Serialized char count of the wire input schema the client receives
/// (the rmcp/schemars-derived schema from `ExploreParams` field docs).
fn wire_schema_chars() -> usize {
    let server = make_server_with_repo_path(PathBuf::new());
    let tool = server
        .get_tool("lievo_explore")
        .expect("lievo_explore must be registered");
    serde_json::Value::Object((*tool.input_schema).clone())
        .to_string()
        .chars()
        .count()
}

/// Char count of the lievo_explore wire tool description.
fn wire_description_chars() -> usize {
    let server = make_server_with_repo_path(PathBuf::new());
    let tool = server
        .get_tool("lievo_explore")
        .expect("lievo_explore must be registered");
    tool.description.clone().unwrap_or_default().chars().count()
}

/// Char count of the server instructions sent in the initialize response.
fn server_instructions_chars() -> usize {
    use crate::mcp::InterceptingMcpServer;
    let inner = make_server_with_repo_path(PathBuf::new());
    let wrapped = InterceptingMcpServer::new(inner);
    let info = wrapped.get_info();
    let instructions = info
        .instructions
        .expect("initialize response must carry instructions");
    instructions.chars().count()
}

// ---------------------------------------------------------------------------
// Size pins (issue #850): upper bounds, ~25% headroom over post-trim.
//
// Post-trim measurement (issue #850, measured via the MCP wire, same method
// as the issue's 948d930 numbers):
//   input schema       2,248 chars   bound 2,800
//   tool description   1,818 chars   bound 2,200
//   server instructions 1,662 chars  bound 2,000
//   total              5,728 chars   (pre-trim: 8,218)
// ---------------------------------------------------------------------------

#[test]
fn wire_input_schema_size_is_bounded() {
    let got = wire_schema_chars();
    eprintln!("lievo_explore wire input schema: {got} chars (bound 2800)");
    assert!(
        got <= 2800,
        "lievo_explore wire input schema grew to {got} chars (bound 2800); \
         trim the ExploreParams field doc comments in src/mcp/params.rs — \
         they are serialized into the schema the model reads every turn"
    );
}

#[test]
fn wire_description_size_is_bounded() {
    let got = wire_description_chars();
    eprintln!("lievo_explore wire description: {got} chars (bound 2200)");
    assert!(
        got <= 2200,
        "lievo_explore wire description grew to {got} chars (bound 2200); \
         trim the #[tool] description in src/mcp/tools.rs — it is sent on \
         every turn"
    );
}

#[test]
fn server_instructions_size_is_bounded() {
    let got = server_instructions_chars();
    eprintln!("lievo_explore server instructions: {got} chars (bound 2000)");
    assert!(
        got <= 2000,
        "lievo_explore server instructions grew to {got} chars (bound 2000); \
         trim INSTRUCTIONS in src/mcp/intercept.rs — it is sent on every turn"
    );
}

// ---------------------------------------------------------------------------
// Parameter documentation guards (issue #850): the schema is generated from
// ExploreParams field doc comments, so a field whose doc comment is deleted
// loses its description in the wire schema — the agent can no longer choose
// that argument. These guards keep the docs non-empty and mode triggers
// named.
// ---------------------------------------------------------------------------

/// Every lievo_explore parameter carries a non-empty description in the
/// wire schema (issue #850: the model reads these to choose arguments).
#[test]
fn every_wire_parameter_has_non_empty_description() {
    let server = make_server_with_repo_path(PathBuf::new());
    let tool = server
        .get_tool("lievo_explore")
        .expect("lievo_explore must be registered");
    let object = (*tool.input_schema).clone();
    let properties = object
        .get("properties")
        .and_then(|p| p.as_object())
        .expect("schema must expose properties");
    let expected = [
        "query",
        "files",
        "max_files",
        "include_source",
        "scope",
        "offset",
        "include_depth",
        "bundle",
    ];
    for name in expected {
        let prop = properties
            .get(name)
            .unwrap_or_else(|| panic!("parameter `{name}` missing from wire schema"));
        let desc = prop
            .get("description")
            .and_then(|d| d.as_str())
            .unwrap_or("");
        assert!(
            !desc.trim().is_empty(),
            "parameter `{name}` must carry a non-empty description in the \
             wire schema (the model reads it to choose arguments); got: {desc:?}"
        );
    }
    assert_eq!(
        properties.len(),
        expected.len(),
        "parameter set changed; update the expected list if a parameter was \
         deliberately added (issue #850 keeps the current set)"
    );
}

/// Each mode's triggering parameter is still named in the model-facing
/// text (issue #850: trimming must not make it ambiguous which parameter
/// selects which mode).
#[test]
fn model_facing_text_names_every_mode_trigger() {
    let server = make_server_with_repo_path(PathBuf::new());
    let tool = server
        .get_tool("lievo_explore")
        .expect("lievo_explore must be registered");
    let desc = tool.description.clone().unwrap_or_default();
    let schema = serde_json::Value::Object((*tool.input_schema).clone()).to_string();
    let surface = format!("{desc}\n{schema}");
    for trigger in [
        "scope",
        "bundle",
        "files",
        "include_depth",
        "include_source",
    ] {
        assert!(
            surface.contains(trigger),
            "model-facing text (wire description + schema) must name the \
             `{trigger}` mode trigger"
        );
    }
}
