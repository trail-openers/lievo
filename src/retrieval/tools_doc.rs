// Documentation tool implementations.

use serde_json::{Value, json};
use std::fs;
use std::io::Read;

use crate::model::EntityTier;
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::{
    GetConventionsTool, GetExecutionFlowsTool, GetInsightsTool, ListProjectDocsTool,
    ReadProjectDocTool,
};
use crate::storage::Storage;

use super::tools_search::lock_storage;
use crate::retrieval::doc_parsers::extract_doc_summary;

/// Maximum allowed size for documentation files (1MB).
/// Prevents resource exhaustion from reading unexpectedly large documentation.
const MAX_DOC_SIZE: u64 = 1_048_576;

// ---------------------------------------------------------------------------
// GetConventionsTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for GetConventionsTool<S> {
    fn name(&self) -> &str {
        "get_conventions"
    }

    fn description(&self) -> &str {
        "Get detected coding conventions and patterns. \
         Use when asked about code style or best practices."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "description": "Optional category filter"
                }
            }
        })
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let category = input.get("category").and_then(|v| v.as_str());

        let guard = lock_storage!(self.ctx.storage);
        let conventions = guard.list_conventions(&self.ctx.project_id, category)?;

        let result: Vec<Value> = conventions
            .into_iter()
            .map(|c| {
                json!({
                    "id": c.id,
                    "category": c.category,
                    "title": c.title,
                    "description": c.description,
                    "confidence": c.confidence,
                    "example_code": c.example_code
                })
            })
            .collect();

        if !result.is_empty() {
            return Ok(json!({"conventions": result, "message": null}).to_string());
        }

        let entities = guard.list_entities(&self.ctx.project_id, None)?;
        let has_function_entities = entities.iter().any(|e| e.tier == EntityTier::Function);

        let message = if has_function_entities {
            "No conventions detected. Run 'lievo refresh --force <project>' to recompute conventions."
        } else {
            "No conventions detected. Convention detection requires function-level entity indexing — \
             re-run 'lievo refresh --force <project>' to enable."
        };

        Ok(json!({
            "conventions": result,
            "message": message
        })
        .to_string())
    }
}

// ---------------------------------------------------------------------------
// GetInsightsTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for GetInsightsTool<S> {
    fn name(&self) -> &str {
        "get_insights"
    }

    fn description(&self) -> &str {
        "Get architectural insights like coupling hotspots and complexity warnings. \
         Use when asked about code quality."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "description": "Optional category filter"
                },
                "severity": {
                    "type": "string",
                    "description": "Filter: critical, high, medium, low"
                }
            }
        })
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let category = input.get("category").and_then(|v| v.as_str());
        let severity = input.get("severity").and_then(|v| v.as_str());

        let guard = lock_storage!(self.ctx.storage);
        let insights = guard.list_insights(&self.ctx.project_id, category, severity, 50)?;

        let result: Vec<Value> = insights
            .into_iter()
            .map(|i| {
                json!({
                    "id": i.id,
                    "category": i.category,
                    "severity": i.severity,
                    "title": i.title,
                    "description": i.description
                })
            })
            .collect();

        Ok(json!(result).to_string())
    }
}

// ---------------------------------------------------------------------------
// GetExecutionFlowsTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for GetExecutionFlowsTool<S> {
    fn name(&self) -> &str {
        "get_execution_flows"
    }

    fn description(&self) -> &str {
        "Get precomputed execution flows showing call sequences from entry points. \
          Use when asked about how functions are called or what the execution path is."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "entry_point": {
                    "type": "string",
                    "description": "Optional substring filter on flow entry point name or path (case-insensitive)"
                },
                "max_depth": {
                    "type": "integer",
                    "description": "Maximum traversal depth for flow tracing (default 20)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of flows to return (default: unlimited)"
                }
            }
        })
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        use crate::analysis::flow_tracer::ExecutionFlow;
        use crate::analysis::flow_tracer::FlowTracer;

        let entry_point_filter = input
            .get("entry_point")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_lowercase());

        let max_depth: Option<usize> = input
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let limit: Option<usize> = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        let guard = lock_storage!(self.ctx.storage);
        let entities = guard.list_entities(&self.ctx.project_id, None)?;
        let relationships = guard.list_all_relationships(&self.ctx.project_id)?;

        let mut flows: Vec<ExecutionFlow> = Vec::new();
        let mut has_function_entities = false;

        // Pre-filter relationships to only those we care about (for performance)
        for entity in &entities {
            if entity.tier == EntityTier::Function {
                has_function_entities = true;
            }
        }

        // Only check for stored flows if the project has function entities
        if has_function_entities {
            for entity in &entities {
                if let Some(metrics_json) = &entity.metrics_json
                    && let Ok(metrics) = serde_json::from_str::<serde_json::Value>(metrics_json)
                    && let Some(execution_flows) = metrics.get("execution_flows")
                    && let Ok(flows_array) =
                        serde_json::from_value::<Vec<ExecutionFlow>>(execution_flows.clone())
                {
                    flows.extend(flows_array);
                }
            }
        }

        // If no precomputed flows, attempt to compute flows using the bulk relationships
        if flows.is_empty() && has_function_entities {
            flows = FlowTracer::trace_flows(&entities, &relationships, max_depth);
        }

        // Apply entry_point substring filter (case-insensitive)
        if let Some(ref filter) = entry_point_filter {
            flows.retain(|flow| {
                flow.entry_point.to_lowercase().contains(filter)
                    || flow.entry_point_id.to_lowercase().contains(filter)
            });
        }

        // Apply limit (0 means no limit)
        if let Some(n) = limit.filter(|&n| n > 0) {
            flows.truncate(n);
        }

        if !flows.is_empty() {
            return Ok(json!({"flows": flows, "message": null}).to_string());
        }

        // Empty-state: explain why flows are missing so the LLM can inform the user.
        let message = if has_function_entities {
            "No execution flows indexed. Run 'lievo refresh --force <project>' to recompute flows from function-level call chains."
        } else {
            "No execution flows indexed. Re-run 'lievo refresh --force <project>' to index function entities."
        };

        Ok(json!({
            "flows": flows,
            "message": message
        })
        .to_string())
    }
}

// ---------------------------------------------------------------------------
// ListProjectDocsTool
// ---------------------------------------------------------------------------

impl Tool for ListProjectDocsTool {
    fn name(&self) -> &str {
        "list_project_docs"
    }

    fn description(&self) -> &str {
        "List all discovered project documentation files with their sizes. \
         Use to discover what documentation is available, then use read_project_doc to read specific files."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn call(&self, _input: Value) -> crate::Result<String> {
        let result: Vec<Value> = self
            .docs
            .iter()
            .map(|(path, size_bytes)| {
                json!({
                    "path": path,
                    "size_bytes": size_bytes
                })
            })
            .collect();

        Ok(json!(result).to_string())
    }
}

// ---------------------------------------------------------------------------
// ReadProjectDocTool
// ---------------------------------------------------------------------------

impl Tool for ReadProjectDocTool {
    fn name(&self) -> &str {
        "read_project_doc"
    }

    fn description(&self) -> &str {
        "Read a project documentation file by path. \
         Pass the 'path' field exactly as returned by list_project_docs. \
         Returns raw file content by default (format='raw'); \
         pass format='structured' for extracted headings, decisions, terminology, and config_values."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path from list_project_docs results — must match exactly as returned.",
                },
                "format": {
                    "type": "string",
                    "description": "Output format. 'raw' returns verbatim file content (default). 'structured' returns extracted fields: headings, decisions, terminology, config_values.",
                    "enum": ["raw", "structured"],
                    "default": "raw"
                }
            },
            "required": ["path"]
        })
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let requested_path = input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::LievoError::InvalidInput("missing 'path' parameter".into()))?;

        let format = input
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("raw");

        // Validate format value
        if !matches!(format, "raw" | "structured") {
            return Err(crate::LievoError::InvalidInput(format!(
                "invalid format '{}': must be 'raw' or 'structured'",
                format
            )));
        }

        // Validate that the path is in the discovered docs list (security check).
        let (validated_path, size_bytes) = self
            .docs
            .iter()
            .find(|(path, _)| path == requested_path)
            .map(|(path, size)| (path, *size))
            .ok_or_else(|| {
                crate::LievoError::InvalidInput(format!(
                    "path '{}' is not in discovered docs list",
                    requested_path
                ))
            })?;

        // Verify file size is within limits to prevent resource exhaustion.
        if size_bytes > MAX_DOC_SIZE {
            return Err(crate::LievoError::InvalidInput(format!(
                "Documentation file '{}' is too large ({} bytes, max {})",
                validated_path, size_bytes, MAX_DOC_SIZE
            )));
        }

        // Security check: canonicalize and verify path is within project root
        let canonical_path = fs::canonicalize(validated_path).map_err(|e| {
            crate::LievoError::InvalidInput(format!("path validation error: {}", e))
        })?;

        let canonical_root = fs::canonicalize(&self.project_root).map_err(|e| {
            crate::LievoError::InvalidInput(format!("project root validation error: {}", e))
        })?;

        if !canonical_path.starts_with(&canonical_root) {
            return Err(crate::LievoError::InvalidInput(
                "path is outside the project root".to_string(),
            ));
        }

        // Open once, verify size from the same handle, then read through that handle.
        let file = fs::File::open(&canonical_path)
            .map_err(|e| crate::LievoError::InvalidInput(format!("could not open file: {}", e)))?;

        let actual_size = file
            .metadata()
            .map_err(|e| {
                crate::LievoError::InvalidInput(format!("could not read file metadata: {}", e))
            })?
            .len();
        if actual_size > MAX_DOC_SIZE {
            return Err(crate::LievoError::InvalidInput(format!(
                "Documentation file '{}' is too large ({} bytes, max {})",
                validated_path, actual_size, MAX_DOC_SIZE
            )));
        }

        // Read through the same handle, bounded to MAX_DOC_SIZE + 1 bytes.
        // The +1 lets us detect if the file grew past the limit during the read.
        let mut content = String::new();
        file.take(MAX_DOC_SIZE + 1)
            .read_to_string(&mut content)
            .map_err(crate::LievoError::Io)?;

        if content.len() as u64 > MAX_DOC_SIZE {
            return Err(crate::LievoError::InvalidInput(format!(
                "Documentation file '{}' exceeded size limit during read",
                validated_path
            )));
        }

        // Branch on format AFTER reading the file content:
        if format == "structured" {
            Ok(json!(extract_doc_summary(validated_path, &content)).to_string())
        } else {
            // raw mode — return verbatim content
            Ok(json!({
                "file": validated_path,
                "content": content
            })
            .to_string())
        }
    }
}

#[cfg(test)]
#[path = "tools_doc_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tools_doc_flows_tests.rs"]
mod tests_flows;
