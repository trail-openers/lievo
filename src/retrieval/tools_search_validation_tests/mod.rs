// SearchEntitiesTool input validation tests split into focused modules.

pub use crate::retrieval::tool_trait::Tool;
pub use crate::retrieval::tools::SearchEntitiesTool;
pub use serde_json::{Value, json};

mod advanced;
mod basic;
