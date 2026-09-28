use std::sync::Arc;

use rmcp::{
    handler::server::{ServerHandler, tool as tool_mod, wrapper::Parameters},
    model::{
        CallToolResult, ContentBlock, ErrorData as McpError, GetPromptRequestParams,
        GetPromptResult, ListPromptsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo,
    },
    service::RoleServer,
    tool, tool_handler, tool_router,
};
use serde_json::json;

use crate::retrieval::doc_discovery::discover_existing_docs;
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::{
    ExploreTool, GetConventionsTool, GetEntityTool, GetExecutionFlowsTool, GetFunctionTool,
    GetHotspotsTool, GetImpactTool, GetInsightsTool, ListDirectoryTool, ListProjectDocsTool,
    ListRelationshipsTool, ListSubsystemsTool, ReadFileTool, ReadProjectDocTool,
    SearchEntitiesTool, ToolContext,
};
use crate::storage::sqlite::SqliteStorage;

use super::params::*;

#[derive(Clone)]
pub struct LievoMcpServer {
    ctx: Arc<ToolContext<SqliteStorage>>,
    docs: Arc<Vec<(String, u64)>>,
    #[expect(
        dead_code,
        reason = "Required by #[tool_router] macro; populated in new(), read by macro-generated code"
    )]
    tool_router: tool_mod::ToolRouter<LievoMcpServer>,
}

impl LievoMcpServer {
    pub fn new(ctx: Arc<ToolContext<SqliteStorage>>) -> Self {
        let docs = discover_existing_docs(ctx.repo_path.as_path())
            .unwrap_or_default()
            .into_iter()
            .map(|doc| (doc.path.to_string_lossy().to_string(), doc.size_bytes))
            .collect();
        Self {
            ctx,
            docs: Arc::new(docs),
            tool_router: Self::tool_router(),
        }
    }

    fn wrap(result: crate::Result<String>) -> Result<CallToolResult, McpError> {
        match result {
            Ok(text) => Ok(CallToolResult::success(vec![ContentBlock::text(text)])),
            Err(e) => Ok(success_shaped_error(&e)),
        }
    }
}

/// Build a success-shaped `CallToolResult` for a recoverable `LievoError` (issue #680/#682).
///
/// All recoverable conditions — invalid input, entity not found, repo path not configured,
/// zero matches, and any other `LievoError` variant — must return `Ok` (not `Err`/isError)
/// with guidance text, per the P1.2 contract. Only truly unrecoverable internal failures
/// should map to an MCP-level `Err`; the guidance text names the condition and the likely
/// remediation so an agent can continue the session instead of hitting an error wall.
fn success_shaped_error(e: &crate::LievoError) -> CallToolResult {
    let guidance = match e {
        crate::LievoError::InvalidInput(msg) => format!(
            "Invalid request: {msg}. Check the tool's parameter documentation and retry with a corrected request."
        ),
        crate::LievoError::EntityNotFound(id) => format!(
            "Entity not found: {id}. Use `search_entities` to discover valid entity IDs for this project."
        ),
        crate::LievoError::PathNotFound(path) => format!(
            "Path not found: {path}. Use `search_entities` or `list_directory` to confirm the correct path."
        ),
        other => format!(
            "Operation could not be completed: {}. The project may not be fully indexed — run `lievo refresh` to rebuild the index and retry.",
            other
        ),
    };
    CallToolResult::success(vec![ContentBlock::text(guidance)])
}

#[tool_router]
impl LievoMcpServer {
    #[tool(
        name = "search_entities",
        description = "Search for entities by name or keyword. Start here to discover entity IDs for use with get_entity, list_relationships, and read_file. Set semantic=true to use tree-sitter-based semantic code search — falls back to name/path matching if no vector index exists (run 'lievo refresh' to build the index)."
    )]
    async fn search_entities(
        &self,
        Parameters(params): Parameters<SearchEntitiesParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            SearchEntitiesTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "query": params.query, "limit": params.limit, "semantic": params.semantic })),
        )
    }

    #[tool(
        name = "get_entity",
        description = "Get full details for a single entity including metrics and tier. Use entity IDs returned by search_entities or list_subsystems. Set include_children=true to fetch structural children (for modules/subsystems). This replaces get_module_details."
    )]
    async fn get_entity(
        &self,
        Parameters(params): Parameters<GetEntityParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            GetEntityTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "entity_id": params.entity_id, "include_children": params.include_children })),
        )
    }

    #[tool(
        name = "list_relationships",
        description = "List what an entity depends on and what depends on it. Useful for coupling and impact analysis. Works on any entity type. The response also carries `unresolved_imports` (count of internal imports in the entity's repo that failed resolution; null when not recorded) and `resolution_coverage` (null when unknown). Each edge also carries `provenance` (\"resolved\" or \"heuristic\") — resolved edges come from exact import-resolution or structural grouping; heuristic edges from name/fn-map matching. An empty `depended_by` with `unresolved_imports > 0` does NOT mean dead code — absence of dependents may reflect unresolved imports."
    )]
    async fn list_relationships(
        &self,
        Parameters(params): Parameters<ListRelationshipsParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            ListRelationshipsTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "entity_id": params.entity_id })),
        )
    }

    #[tool(
        name = "list_subsystems",
        description = "List all top-level subsystems. Use FIRST when asked about architecture or project overview."
    )]
    async fn list_subsystems(&self) -> Result<CallToolResult, McpError> {
        Self::wrap(
            ListSubsystemsTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({})),
        )
    }

    #[tool(
        name = "get_function",
        description = "Get detailed information about a function or class entity including source code, signature, what it calls, and what calls it. More token-efficient than read_file for understanding a single function. Use after search_entities to drill into specific functions. If get_function returns an error about missing entities, run 'lievo refresh --force <project>' to re-index."
    )]
    async fn get_function(
        &self,
        Parameters(params): Parameters<GetFunctionParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            GetFunctionTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "entity_id": params.entity_id })),
        )
    }

    #[tool(
        name = "get_conventions",
        description = "List detected coding conventions"
    )]
    async fn get_conventions(
        &self,
        Parameters(params): Parameters<GetConventionsParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            GetConventionsTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "category": params.category })),
        )
    }

    #[tool(
        name = "get_insights",
        description = "Get architectural insights like complexity hotspots, coupling warnings, and coverage gaps. Filter by severity (critical/high/medium/low) or category (complexity_hotspot, high_coupling, coverage_gap, god_module, circular_dependency, naming, testing, structure). Use when asked about code quality or technical debt."
    )]
    async fn get_insights(
        &self,
        Parameters(params): Parameters<GetInsightsParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            GetInsightsTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "category": params.category, "severity": params.severity })),
        )
    }

    #[tool(
        name = "read_file",
        description = "Read the raw source code of a file entity. Takes a file entity ID (not a file path) — use search_entities with a filename to get the entity ID. Content is truncated to 32K chars for large files."
    )]
    async fn read_file(
        &self,
        Parameters(params): Parameters<ReadFileParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            ReadFileTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "entity_id": params.entity_id })),
        )
    }

    #[tool(
        name = "list_directory",
        description = "List files in a directory relative to the repo root. Returns direct children only (non-recursive). Use to browse project structure when you know the directory path."
    )]
    async fn list_directory(
        &self,
        Parameters(params): Parameters<ListDirectoryParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            ListDirectoryTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "path": params.path })),
        )
    }

    #[tool(
        name = "get_execution_flows",
        description = "Get precomputed execution flows showing call sequences from entry points. Use when asked about how functions are called or what the execution path is. When empty, the response includes a message explaining the limitations of the current data."
    )]
    async fn get_execution_flows(
        &self,
        Parameters(params): Parameters<GetExecutionFlowsParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            GetExecutionFlowsTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({
                "entry_point": params.entry_point,
                "max_depth": params.max_depth,
                "limit": params.limit,
            })),
        )
    }

    #[tool(
        name = "list_project_docs",
        description = "List all discovered project documentation files with their sizes. Use to discover what documentation is available, then call read_project_doc to read specific files."
    )]
    async fn list_project_docs(&self) -> Result<CallToolResult, McpError> {
        Self::wrap(
            ListProjectDocsTool {
                docs: self.docs.clone(),
            }
            .call(json!({})),
        )
    }

    #[tool(
        name = "read_project_doc",
        description = "Read a project documentation file by path. Pass the 'path' field exactly as returned by list_project_docs. Returns raw file content by default; pass format='structured' for regex-extracted headings, decisions, terminology, and config_values."
    )]
    async fn read_project_doc(
        &self,
        Parameters(params): Parameters<ReadProjectDocParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            ReadProjectDocTool {
                docs: self.docs.clone(),
                project_root: self.ctx.repo_path.clone(),
            }
            .call(json!({ "path": params.path, "format": params.format })),
        )
    }

    #[tool(
        name = "get_impact",
        description = "Analyse the impact of changing one or more files. Returns {files, dependents:[{path, hop}], unresolved_imports, resolution_coverage}. Each dependent is a repo-relative path with hop 0 (direct) or 1 (second-hop), counting import, call, depends-on, and implements edges. unresolved_imports/resolution_coverage are null when not recorded; an empty dependents list with unresolved_imports > 0 does NOT mean dead code."
    )]
    async fn get_impact(
        &self,
        Parameters(params): Parameters<GetImpactParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            GetImpactTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "files": params.files })),
        )
    }

    #[tool(
        name = "get_hotspots",
        description = "Return the highest-complexity entities sorted by complexity score. Defaults to file-tier only (pass tier='module' or tier='subsystem' to change). Use to identify risky code, focus refactoring effort, or orient in an unfamiliar codebase. Limit defaults to 10 (max 50)."
    )]
    async fn get_hotspots(
        &self,
        Parameters(params): Parameters<GetHotspotsParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            GetHotspotsTool {
                ctx: self.ctx.clone(),
            }
            .call(json!({ "limit": params.limit, "tier": params.tier })),
        )
    }

    #[tool(
        name = "lievo_explore",
        description = "PRIMARY TOOL — call FIRST for any structural question about this codebase. Tier 1 (default): per-symbol map (name, kind, qualified_path, signature, one-line summary), no source bodies, no call paths or blast radius. Tier 2 (include_source=true): verbatim line-numbered source for matched symbols. include_depth=true adds the per-file call_paths and blast_radius to each symbol (blast-radius/impact opt-in; ignored in scope mode). Pass scope='<dir>' (e.g. 'src/retrieval') to switch to scope-membership listing: the sorted, repo-relative indexed files under that directory prefix; scope is the sole trigger for this mode. Pass files=[...] (repo-relative paths, one batched call) to fetch files directly; files short-circuits query word-match and scope. Pass bundle='<dir>' (a repo-relative directory prefix) to select subsystem-bundle mode: one call under the 24K output cap returns the verbatim line-numbered source for the packed files, the intra-scope Calls/Imports edges between those files, a not_shown_files list, and a structured completeness field {complete, omitted_files, omitted_edges}. Output capped at 24K chars: when the cap is hit, a structured completeness field names what was omitted, with returned/total/next continuation pointers. Batch: request related files in one call instead of one call per file, and page the rest with returned/total/next or offset. Do not re-read a file whose source you already received this session, and do not re-request edges or relationships already returned. A files-mode or bundle response with completeness.complete == true has returned everything requested for that scope — stop, and do not re-read any file whose source was already returned. Stop: when the response has no continuation pointer and returned equals total, the result set is complete — answer from it."
    )]
    async fn lievo_explore(
        &self,
        Parameters(params): Parameters<ExploreParams>,
    ) -> Result<CallToolResult, McpError> {
        Self::wrap(
            ExploreTool {
                ctx: self.ctx.clone(),
            }
            .call(explore_input_json(params)),
        )
    }
}

#[tool_handler]
impl ServerHandler for LievoMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_prompts()
            .build();
        info
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let _ = context;
        super::prompts::get_prompt(request)
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(super::prompts::list_prompts())
    }
}

/// Compose the hand-built input for `ExploreTool::call` (issue #741): the
/// files-list mode is only active when `params.files` is Some, so the key is
/// attached conditionally — an absent key (not an empty array) keeps the
/// retrieval layer's word-match/scope paths byte-identical for calls that
/// never pass `files`.
fn explore_input_json(params: ExploreParams) -> serde_json::Value {
    let mut input = json!({
        "query": params.query,
        "max_files": params.max_files,
        "include_source": params.include_source,
        "include_depth": params.include_depth,
        "scope": params.scope,
        "offset": params.offset,
    });
    if let Some(files) = &params.files {
        input["files"] = json!(files);
    }
    // Bundle mode (issue #743): attach `bundle` only when the param is present
    // and non-blank, so calls that never pass it keep the retrieval layer's
    // word-match/scope/files paths byte-identical (an absent `bundle` key must
    // NOT flip a call into bundle mode). A blank value is treated as absent,
    // mirroring how a blank `scope` does not trigger scope mode.
    if let Some(scope) = params
        .bundle
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        input["bundle"] = json!(scope);
    }
    input
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "explore_tests.rs"]
mod explore_tests;

#[cfg(test)]
#[path = "explore_wire_default_tests.rs"]
mod explore_wire_default_tests;

#[cfg(test)]
#[path = "explore_scope_tests.rs"]
mod explore_scope_tests;

#[cfg(test)]
#[path = "explore_depth_wire_tests.rs"]
mod explore_depth_wire_tests;

#[cfg(test)]
#[path = "explore_bundle_wire_tests.rs"]
mod explore_bundle_wire_tests;

#[cfg(test)]
#[path = "explore_wire_size_tests.rs"]
mod explore_wire_size_tests;
