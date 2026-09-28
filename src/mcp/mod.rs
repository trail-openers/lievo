pub mod intercept;
pub mod params;
pub mod prompts;
pub mod repo_resolution;
pub mod server;
pub mod tools;

use rmcp::handler::server::ServerHandler;
use rmcp::model::Meta;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ErrorData as McpError, GetPromptRequestParams,
    GetPromptResult, ListPromptsResult, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RoleServer;
use serde_json::Map;

pub use server::serve;
pub use tools::LievoMcpServer;

/// The intercept's snapshot of enabled tool names, captured once at
/// construction (issue #680: allowlist parsed at server construction, not
/// per request). Always contains `lievo_explore` plus any tools named in
/// `LIEVO_MCP_TOOLS`.
#[derive(Clone, Debug)]
pub struct Allowlist {
    /// Full set of enabled tool names, including the primary tool.
    enabled: std::collections::HashSet<String>,
}

impl Allowlist {
    /// Parse the `LIEVO_MCP_TOOLS` allowlist (issue #680: env var drives the
    /// enablement rule — primary tool always in, every named tool additionally
    /// in). Called per construction rather than cached: env-mutating tests
    /// (serialized by `crate::test_env_support::env_lock`) expect to observe
    /// the current process environment, and parsing a comma-separated string
    /// at construction time is negligible.
    fn capture() -> Self {
        // Default (LIEVO_MCP_TOOLS unset/empty): only the primary tool is
        // listed. A non-empty allowlist restricts the set to the primary
        // tool plus the named tools (issue #680: "unlist the other 13").
        let from_env = intercept::parse_tools_allowlist();
        let enabled: std::collections::HashSet<String> = if from_env.is_empty() {
            std::iter::once(intercept::PRIMARY_TOOL.to_string()).collect()
        } else {
            std::iter::once(intercept::PRIMARY_TOOL.to_string())
                .chain(from_env)
                .collect()
        };
        Allowlist { enabled }
    }

    pub(crate) fn is_enabled(&self, name: &str) -> bool {
        self.enabled.contains(name)
    }

    /// The full enabled set (sorted). Exposed for tests.
    pub fn enabled_tools(&self) -> Vec<String> {
        let mut v: Vec<String> = self.enabled.iter().cloned().collect();
        v.sort();
        v
    }
}

/// A `ServerHandler` that fronts `LievoMcpServer` with the lievo MCP surface
/// (issue #680). It:
///
/// 1. Intercepts `call_tool` for any tool that is not enabled, returning a
///    success-shaped (`isError=false`) guidance result naming `lievo_explore`
///    and the `LIEVO_MCP_TOOLS` env var — instead of letting rmcp's router
///    answer `Err(invalid_params("tool not found"))`.
/// 2. Filters `list_tools` and `get_tool` to the enabled set, so unlisted tools
///    are not advertised.
/// 3. Sends `instructions` in the `initialize` response directing the agent to
///    call `lievo_explore` FIRST.
///
/// `LievoMcpServer` remains the implementation behind it; this wrapper only
/// adds the surface. `tools.rs` (the macro-generated router + 14 tool
/// methods) is left untouched.
#[derive(Clone)]
pub struct InterceptingMcpServer {
    inner: LievoMcpServer,
    allowlist: Allowlist,
}

impl InterceptingMcpServer {
    /// Wrap an already-constructed `LievoMcpServer`. Captures the
    /// `LIEVO_MCP_TOOLS` allowlist.
    pub fn new(inner: LievoMcpServer) -> Self {
        Self {
            allowlist: Allowlist::capture(),
            inner,
        }
    }

    /// Inner server (for tests / direct access).
    pub fn inner(&self) -> &LievoMcpServer {
        &self.inner
    }

    /// The captured allowlist (for tests / direct access).
    pub fn allowlist(&self) -> &Allowlist {
        &self.allowlist
    }
}

#[allow(unused_variables)]
impl ServerHandler for InterceptingMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = self.inner.get_info();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_prompts()
            .build();
        intercept::with_instructions(info)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        // Issue #680 interception point: disabled or unknown tool names
        // return success-shaped guidance instead of isError.
        if let Some(guidance) =
            intercept::intercept_call_tool(&request.name, &self.allowlist.enabled)
        {
            return Ok(guidance);
        }
        self.inner.call_tool(request, context).await
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let all = self.inner.list_tools(request, context).await?;
        Ok(ListToolsResult {
            tools: all
                .tools
                .into_iter()
                .filter(|tool| self.allowlist.is_enabled(&tool.name))
                .map(|mut tool| {
                    if tool.name == intercept::PRIMARY_TOOL {
                        let mut map = Map::new();
                        map.insert(
                            "anthropic/alwaysLoad".to_string(),
                            serde_json::Value::Bool(true),
                        );
                        tool.meta = Some(Meta(map));
                    }
                    tool
                })
                .collect(),
            next_cursor: all.next_cursor,
            meta: all.meta,
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        if !self.allowlist.is_enabled(name) {
            return None;
        }
        let mut tool = self.inner.get_tool(name)?;
        if tool.name == intercept::PRIMARY_TOOL {
            let mut map = Map::new();
            map.insert(
                "anthropic/alwaysLoad".to_string(),
                serde_json::Value::Bool(true),
            );
            tool.meta = Some(Meta(map));
        }
        Some(tool)
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        self.inner.get_prompt(request, context).await
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        self.inner.list_prompts(request, context).await
    }
}
