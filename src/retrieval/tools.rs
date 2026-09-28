// Tool definitions for the LLM chat agentic loop (issue #119).
//
// `ToolContext` and struct declarations live here; `Tool` trait implementations
// are in `tools_impl` to keep each file under the 500-line limit.

#[path = "tools_impl.rs"]
pub mod tools_impl;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

/// Shared context injected into every tool.
///
/// Remains generic over `S: Storage` (not boxed to `&dyn Storage`) because MCP
/// instantiates a single `ToolContext<SqliteStorage>` at runtime; boxing buys no benefit.
/// Helper fns in `src/mcp/server.rs` use `&dyn Storage` as a convenience to avoid
/// generic propagation through setup code — this asymmetry is intentional. See #607.
pub struct ToolContext<S: Storage> {
    pub storage: Arc<Mutex<S>>,
    pub project_id: String,
    /// Absolute path to the repository root on disk. Used by `ReadFileTool` to
    /// read raw file content. Set to `PathBuf::new()` when no repo path is
    /// available (e.g. chat mode without a single repo).
    pub repo_path: PathBuf,
    /// Output directory name for documentation (e.g. "lievo_docs"). Entities matching
    /// this directory are excluded from tool responses. Used to prevent the LLM from
    /// documenting generated output directories. None disables filtering.
    pub output_dir: Option<String>,
    /// Guidance text served by `lievo_explore` when the server was started in a
    /// location that cannot serve a repository: no PROJECT argument with the
    /// launch directory outside a git repository (issue #863 decision 4), or a
    /// resolved project with zero registered repositories (decision 5).
    /// `None` when a repository is actually being served. Success-shaped — the
    /// tool returns this text instead of the generic refresh message, and no
    /// tool returns an error.
    pub zero_repo_guidance: Option<String>,
}

/// Searches entities by substring match on name or path.
pub struct SearchEntitiesTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Returns all fields of a single entity including AI summary.
pub struct GetEntityTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Lists what an entity depends on and what depends on it.
pub struct ListRelationshipsTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Lists all top-level subsystem entities.
pub struct ListSubsystemsTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Lists detected coding conventions, optionally filtered by category.
pub struct GetConventionsTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Lists architectural insights, optionally filtered by severity.
pub struct GetInsightsTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Reads raw source content of a file entity from disk.
pub struct ReadFileTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Lists files and subdirectories in a directory relative to the repo root.
pub struct ListDirectoryTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Returns precomputed execution flows from entry point entities.
pub struct GetExecutionFlowsTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Lists all discovered project documentation files with their sizes.
pub struct ListProjectDocsTool {
    pub(crate) docs: Arc<Vec<(String, u64)>>,
}

/// Reads and summarizes a single project documentation file.
/// Uses an LLM provider if available for better summarization; falls back to regex extraction.
pub struct ReadProjectDocTool {
    pub(crate) docs: Arc<Vec<(String, u64)>>,
    pub project_root: std::path::PathBuf,
}

/// Returns detailed CodeUnit data for a function/class entity including source code, signature, calls, and called_by.
pub struct GetFunctionTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Analyses the blast radius of changing one or more files.
pub struct GetImpactTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Returns the highest-complexity entities in the codebase sorted by complexity score.
pub struct GetHotspotsTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Primary exploration tool (issue #680/#682): two-tier progressive disclosure
/// over a query. Tier 1 (default) returns a per-symbol map — name, kind,
/// qualified path, signature, one-line summary, call paths, blast radius —
/// with no source bodies; small-body symbols carry their body inline instead of
/// a summary (the Complexity Trap). Tier 2 (`include_source=true`) returns
/// verbatim line-numbered source for the matched symbols. Output is capped at
/// 24K chars with a continuation pointer.
pub struct ExploreTool<S: Storage> {
    pub(crate) ctx: Arc<ToolContext<S>>,
}

/// Build the full set of lievo tools ready for injection into the tool loop.
pub fn create_tools<S: Storage + Send + 'static>(ctx: Arc<ToolContext<S>>) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(SearchEntitiesTool { ctx: ctx.clone() }),
        Box::new(GetEntityTool { ctx: ctx.clone() }),
        Box::new(ListRelationshipsTool { ctx: ctx.clone() }),
        Box::new(ListSubsystemsTool { ctx: ctx.clone() }),
        Box::new(GetConventionsTool { ctx: ctx.clone() }),
        Box::new(GetInsightsTool { ctx: ctx.clone() }),
        Box::new(ReadFileTool { ctx: ctx.clone() }),
        Box::new(ListDirectoryTool { ctx: ctx.clone() }),
        Box::new(GetFunctionTool { ctx: ctx.clone() }),
        Box::new(GetExecutionFlowsTool { ctx: ctx.clone() }),
    ];

    // Always register doc tools — empty list is fine, tools handle it gracefully.
    // Discover existing documentation files at repo_path for on-demand access (issue #253).
    let docs =
        crate::retrieval::doc_discovery::discover_existing_docs(&ctx.repo_path).unwrap_or_default();
    let docs_vec: Vec<(String, u64)> = docs
        .into_iter()
        .map(|d| (d.path.to_string_lossy().to_string(), d.size_bytes))
        .collect();
    let docs_arc = std::sync::Arc::new(docs_vec);

    tools.push(Box::new(ListProjectDocsTool {
        docs: docs_arc.clone(),
    }));

    // ReadProjectDocTool created without LLM provider here.
    // Doc-gen code can later create a version WITH the provider.
    tools.push(Box::new(ReadProjectDocTool {
        docs: docs_arc,
        project_root: ctx.repo_path.clone(),
    }));
    tools.push(Box::new(GetImpactTool { ctx: ctx.clone() }));
    tools.push(Box::new(GetHotspotsTool { ctx: ctx.clone() }));
    tools.push(Box::new(ExploreTool { ctx }));

    tools
}
