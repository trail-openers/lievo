use crate::error::Result;
use serde_json::Value;

/// A tool that the LLM can call during chat.
///
/// Implementations must be `Send + Sync` so they can be used across async
/// task boundaries. `call()` is intentionally synchronous because analysis
/// tools query SQLite — there is no I/O latency to hide with async.
pub trait Tool: Send + Sync {
    /// Stable identifier used by the LLM to invoke this tool.
    fn name(&self) -> &str;
    /// Human-readable description sent to the LLM so it knows when to call this tool.
    fn description(&self) -> &str;
    /// JSON Schema (as a `serde_json::Value`) describing the accepted input.
    fn input_schema(&self) -> Value;
    /// Optional client metadata for the MCP tool advertisement (issue #680),
    /// e.g. `{"anthropic/alwaysLoad": true}` so Claude Code does not defer the
    /// tool behind ToolSearch. `None` when the tool carries no metadata.
    fn meta(&self) -> Option<serde_json::Value> {
        None
    }
    /// Execute the tool synchronously. Returns a JSON-encoded result string.
    fn call(&self, input: Value) -> Result<String>;
}
