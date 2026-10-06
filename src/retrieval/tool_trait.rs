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
    /// Execute the tool synchronously. Returns a JSON-encoded result string.
    fn call(&self, input: Value) -> Result<String>;
}
