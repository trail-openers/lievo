// Output formatting for query commands.
//
// Pure formatting — no I/O for human formatters (return String).
// JSON/NDJSON formatters accept a `&mut dyn Write` and write line-by-line
// for streaming without full-collection buffering.
//
// Submodules:
//   entities      — format_entities_human / format_entities_json
//   relationships — format_relationships_human / format_relationships_json
//   impact        — format_impact_human / format_impact_json
//   conventions   — format_conventions_human / format_conventions_json
//   insights      — format_insights_human / format_insights_json

pub mod conventions;
pub mod entities;
pub mod impact;
pub mod insights;
pub mod relationships;

pub use conventions::{format_conventions_human, format_conventions_json};
pub use entities::{format_entities_human, format_entities_json};
pub use impact::{format_impact_human, format_impact_json};
pub use insights::{format_insights_human, format_insights_json};
pub use relationships::{ResolutionContext, format_relationships_human, format_relationships_json};

use crate::error::LievoError;
use crate::model::Entity;

/// Output format selected by the `--format` CLI flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum OutputFormat {
    /// Aligned columns with header (default)
    #[default]
    Human,
    /// One JSON object per line (NDJSON)
    Json,
}

/// Format an error as a structured stderr envelope (JSON).
///
/// Output format:
/// ```json
/// {
///   "error": {
///     "message": "Human-readable error message",
///     "kind": "ErrorVariantName"
///   },
///   "code": "UPPERCASE_ERROR_CODE",
///   "retryable": true|false
/// }
/// ```
///
/// Field order is deterministic for machine parsing.
pub fn format_error_envelope(error: &LievoError) -> String {
    use serde_json::json;

    let error_code = error.error_code();
    let retryable = error_code.is_retryable();

    // Use json! macro with preserve_order (enabled via Cargo.toml feature)
    // Fields are listed in deterministic order
    json!({
        "error": {
            "message": error.to_string(),
            "kind": error_code.kind(),
        },
        "code": error_code.as_str(),
        "retryable": retryable,
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Metrics helper (shared by submodules)
// ---------------------------------------------------------------------------

/// Extract metrics fields from an entity's `metrics_json`.
///
/// Returns `(module_count, file_count, complexity)` — all default to 0 when absent.
pub(super) fn parse_metrics(entity: &Entity) -> (u64, u64, f64) {
    let json = match entity.metrics_json.as_deref() {
        Some(s) => s,
        None => return (0, 0, 0.0),
    };
    let v: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return (0, 0, 0.0),
    };
    let modules = v.get("module_count").and_then(|x| x.as_u64()).unwrap_or(0);
    let files = v.get("file_count").and_then(|x| x.as_u64()).unwrap_or(0);
    let complexity = v
        .get("complexity_max")
        .and_then(|x| x.as_f64())
        .unwrap_or(0.0);
    (modules, files, complexity)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EntityTier;

    fn make_entity(id: &str, name: &str, tier: EntityTier, metrics: Option<&str>) -> Entity {
        Entity {
            id: id.to_string(),
            project_id: "proj".to_string(),
            repo_id: None,
            tier,
            parent_id: None,
            name: name.to_string(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: metrics.map(|s| s.to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_output_format_default_is_human() {
        let fmt = OutputFormat::default();
        assert_eq!(fmt, OutputFormat::Human);
    }

    #[test]
    fn test_parse_metrics_no_metrics_json_returns_zeros() {
        let e = make_entity("e", "x", EntityTier::Subsystem, None);
        let (m, f, c) = parse_metrics(&e);
        assert_eq!(m, 0);
        assert_eq!(f, 0);
        assert_eq!(c, 0.0);
    }

    #[test]
    fn test_parse_metrics_invalid_json_returns_zeros() {
        let e = make_entity("e", "x", EntityTier::Subsystem, Some("bad"));
        let (m, f, c) = parse_metrics(&e);
        assert_eq!(m, 0);
        assert_eq!(f, 0);
        assert_eq!(c, 0.0);
    }

    #[test]
    fn test_parse_metrics_partial_fields_handled() {
        let e = make_entity(
            "e",
            "x",
            EntityTier::Subsystem,
            Some(r#"{"complexity_max": 7.5}"#),
        );
        let (m, f, c) = parse_metrics(&e);
        assert_eq!(m, 0);
        assert_eq!(f, 0);
        assert_eq!(c, 7.5);
    }

    // ---------------------------------------------------------------------------
    // Format override precedence tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_output_format_human_is_default() {
        let fmt = OutputFormat::default();
        assert_eq!(
            fmt,
            OutputFormat::Human,
            "OutputFormat::default() should be Human"
        );
    }
}
