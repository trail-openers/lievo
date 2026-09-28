//! Success-shaped interception for tool calls that the allowlist hides.
//!
//! rmcp's `ToolRouter::call` answers a call for a disabled or unknown tool
//! with `Err(ErrorData::invalid_params("tool not found"))` — an `isError=true`
//! response. The lievo MCP contract (issue #680) requires ALL recoverable
//! conditions to be success-shaped (`isError=false`) guidance, because an agent
//! guessing a tool name should be steered back to `lievo_explore`, not failed
//! out of the session. `intercept_call_tool` is the hook that converts those
//! calls into guidance.
//!
//! It is wired in front of the router in `LievoMcpServer::call_tool`, which
//! rmcp's `#[tool_handler]` macro only auto-generates when the method is
//! absent.

use rmcp::model::{CallToolResult, ContentBlock, ServerInfo};

/// The one tool listed by default; every other tool is gated behind the
/// `LIEVO_MCP_TOOLS` allowlist.
pub const PRIMARY_TOOL: &str = "lievo_explore";

/// Parse a `LIEVO_MCP_TOOLS` value into the set of named tool names.
/// Trims segments and drops empty ones.
pub(crate) fn parse_tool_names(raw: &str) -> std::collections::HashSet<String> {
    raw.split(',')
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Parse the `LIEVO_MCP_TOOLS` environment variable into the set of tool
/// names to keep enabled. Returns an empty set when unset or empty, which
/// means "only `lievo_explore` is listed" (the default).
///
/// Parsing happens at server construction (not per request): the allowlist
/// snapshot is captured once in `LievoMcpServer::new`.
pub(crate) fn parse_tools_allowlist() -> std::collections::HashSet<String> {
    std::env::var("LIEVO_MCP_TOOLS")
        .map(|raw| parse_tool_names(&raw))
        .unwrap_or_default()
}

/// Build the success-shaped guidance text returned for a call to a tool that
/// is not enabled. Names the enabled tools and the `LIEVO_MCP_TOOLS` env var
/// so the agent knows how to re-enable what it called.
pub(crate) fn guidance_for_unlisted(name: &str, enabled: &[String]) -> String {
    let enabled_list = if enabled.is_empty() {
        "none (only the primary tool is available)".to_string()
    } else {
        enabled.join(", ")
    };
    format!(
        "Tool '{name}' is not enabled on this server.\n\n\
         Call `lievo_explore` for structural questions about this codebase — \
         it returns line-numbered source grouped by file, and relationships \
         with call paths and blast radius available on request (include_depth=true). \
         Do not re-read a file whose source you already received in this session; \
         do not re-request relationship or edge information already returned. \
         When several files or symbols fit one question, put them in one \
         `lievo_explore` call (default max_files is 8, raised modestly) rather \
         than one call per file, and follow the returned/total/next continuation \
         pointer when the response is capped.\n\
         Other lievo tools are hidden by default and can be enabled with the \
         LIEVO_MCP_TOOLS environment variable (comma-separated tool names) \
         before the server starts.\n\
         Enabled tools: {enabled_list}\n\
         How to enable more: export LIEVO_MCP_TOOLS=get_entity,read_file"
    )
}

/// Interception point (issue #680): given the full set of enabled tool names
/// (already including `lievo_explore`), return `Some(success-shaped guidance)`
/// for a call whose tool is disabled or unknown, and `None` to let the request
/// fall through to the rmcp `ToolRouter`.
///
/// A `None` return means the name is enabled, so `ToolRouter::call` dispatches
/// to the real implementation. A `Some` return carries `is_error=false` and
/// names `lievo_explore` plus the `LIEVO_MCP_TOOLS` env var.
pub(crate) fn intercept_call_tool(
    name: &str,
    enabled_tools: &std::collections::HashSet<String>,
) -> Option<CallToolResult> {
    if enabled_tools.contains(name) {
        return None;
    }
    let mut enabled: Vec<String> = enabled_tools.iter().cloned().collect();
    enabled.sort();
    Some(CallToolResult::success(vec![ContentBlock::text(
        guidance_for_unlisted(name, &enabled),
    )]))
}

/// MCP instructions sent in the `initialize` response (issue #680: "MCP
/// server sends instructions in initialize response"). Directs the agent to
/// call `lievo_explore` FIRST for any structural question.
pub(crate) const INSTRUCTIONS: &str = "Use the lievo code-analysis server. For ANY structural \
     question about this codebase — where something is implemented, what calls \
     or is called by something, what a change's blast radius is, or how modules \
     relate — call `lievo_explore` FIRST; it returns line-numbered source \
     grouped by file and relationships in one call, with call paths and blast \
     radius opt-in via include_depth=true, so you usually do not need to grep the working tree.\
     Do not re-read a file whose source you already received in this session; \
     do not re-request relationship or edge information you already have. When \
     several files or symbols fit one question, put them in one `lievo_explore` \
     call (default max_files is 8, raised modestly) rather than one call per file. \
     Stop when the answer is in what you have: a response without a continuation \
     pointer is complete, and with one, stop when returned equals total (follow \
     the returned/total/next pointers to continue only if needed).\
     Anti-pattern: do not guess other tool names. The other 13 lievo tools \
     (get_entity, read_file, search_entities, get_impact, get_hotspots, \
     list_relationships, get_function, list_subsystems, list_directory, \
     get_conventions, get_insights, list_project_docs, read_project_doc, \
     get_execution_flows) are hidden by default; if you must use one, set the \
     LIEVO_MCP_TOOLS environment variable (comma-separated names) before the \
     server starts, otherwise calls to them return guidance instead of results.\n\
     Limitations: lievo_explore is one call with a capped payload; use the \
     `returned`/`total`/`next` continuation pointers it emits when results \
     exceed the cap. The server indexes the repository automatically in the \
     background when it starts (a first index for a never-indexed repo, an \
     incremental refresh when stale); if a call reports indexing is in \
     progress, use your built-in tools for now and retry the call shortly.";

/// Apply the lievo MCP surface to a `ServerInfo` produced by the
/// `#[tool_handler]` macro (issue #680: "MCP server sends instructions in
/// initialize response"). Sets `info.instructions`; the caller still fills
/// `capabilities`.
pub(crate) fn with_instructions(mut info: ServerInfo) -> ServerInfo {
    info.instructions = Some(INSTRUCTIONS.to_string());
    info
}

#[cfg(test)]
#[path = "intercept_tests.rs"]
mod tests;
