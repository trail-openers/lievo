// Relationship formatting.

use crate::model::{Entity, Relationship};

/// Resolution signal for the queried entity's repo scope (#690).
///
/// `None` means no unresolved data is recorded (pre-#681 index) — no claim.
/// `Some((0, 0))` means resolution ran and found zero unresolved imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionContext {
    pub unresolved_internal: Option<u64>,
    pub unresolved_external: Option<u64>,
}

impl ResolutionContext {
    /// No data recorded — pre-#681 degradation.
    pub const fn unknown() -> Self {
        Self {
            unresolved_internal: None,
            unresolved_external: None,
        }
    }

    /// Whether the caveat line should be shown in human output.
    ///
    /// Fires only when internal unresolved > 0 (external imports are
    /// expected-unresolved by design and never trigger the warning).
    pub fn has_caveat(&self) -> bool {
        self.unresolved_internal.unwrap_or(0) > 0
    }

    /// JSON value for the `unresolved_imports` field.
    ///
    /// Returns `null` when no data is recorded, the internal count otherwise.
    /// External count is excluded — only internal unresolved imports signal
    /// a potentially broken graph.
    pub fn unresolved_imports_json(&self) -> serde_json::Value {
        match self.unresolved_internal {
            Some(n) => serde_json::json!(n),
            None => serde_json::Value::Null,
        }
    }
}

/// Format relationships as an aligned human-readable table.
///
/// `direction` is a label for the table header (e.g. `"Dependencies of <id>"`)
pub fn format_relationships_human(
    rels: &[(Relationship, Entity)],
    direction: &str,
    resolution: &ResolutionContext,
) -> String {
    let mut lines = Vec::new();
    lines.push(direction.to_string());

    if rels.is_empty() {
        lines.push("  (none)".to_string());
        if resolution.has_caveat() {
            let n = resolution.unresolved_internal.unwrap_or(0);
            lines.push(format!(
                "  ⚠ {n} unresolved internal import(s) in repo scope — absence of dependents may not indicate dead code"
            ));
        }
        return lines.join("\n");
    }

    let name_w = rels
        .iter()
        .map(|(_, e)| e.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let type_w = rels
        .iter()
        .map(|(r, _)| r.rel_type.to_string().len())
        .max()
        .unwrap_or(4)
        .max(4);

    let header = format!(
        "  {:<name_w$}  {:<type_w$}  {:>6}",
        "Name", "Type", "Weight"
    );
    let sep = "━".repeat(header.chars().count());
    lines.push(sep);
    lines.push(header);
    for (rel, entity) in rels {
        lines.push(format!(
            "  {:<name_w$}  {:<type_w$}  {:>6.1}",
            entity.name,
            rel.rel_type.to_string(),
            rel.weight
        ));
    }

    lines.join("\n")
}

/// Format relationships as NDJSON (one JSON object per line) written to `writer`.
/// Always emits at least an empty JSON array "[]" for deterministic output.
/// Writes each line immediately — no full-collection buffering.
///
/// When `resolution` has recorded unresolved counts, a `#`-prefixed metadata
/// line is emitted before the array line (the PM decision on the empty-JSON
/// shape — keeps the bare `[]` for the payload, adds the signal non-invasively).
pub fn format_relationships_json(
    rels: &[(Relationship, Entity)],
    resolution: &ResolutionContext,
    writer: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    // Emit metadata line only when data is recorded (not pre-#681 null).
    if resolution.unresolved_internal.is_some() || resolution.unresolved_external.is_some() {
        writeln!(
            writer,
            "# {{\"unresolved_imports\": {}, \"unresolved_external\": {}}}",
            resolution
                .unresolved_internal
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".into()),
            resolution
                .unresolved_external
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".into()),
        )?;
    }

    if rels.is_empty() {
        writeln!(writer, "[]")?;
        return Ok(());
    }
    for (rel, entity) in rels {
        writeln!(
            writer,
            "{}",
            serde_json::json!({
                "entity_id": entity.id,
                "entity_name": entity.name,
                "entity_tier": entity.tier.to_string(),
                "rel_type": rel.rel_type.to_string(),
                "weight": rel.weight,
                // Additive (issue #714, #690 precedent): resolved | heuristic.
                "provenance": rel.provenance.to_string(),
            })
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeProvenance, EntityTier, RelType};

    fn make_entity(id: &str, name: &str, tier: EntityTier) -> Entity {
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
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_format_relationships_human_empty() {
        let result =
            format_relationships_human(&[], "Dependencies of X", &ResolutionContext::unknown());
        assert!(result.contains("Dependencies of X"));
        assert!(result.contains("(none)"));
        // No caveat when resolution is unknown (pre-#681)
        assert!(
            !result.contains("unresolved"),
            "caveat must not fire on unknown resolution"
        );
    }

    #[test]
    fn test_format_relationships_human_empty_with_caveat() {
        let ctx = ResolutionContext {
            unresolved_internal: Some(3),
            unresolved_external: Some(5),
        };
        let result = format_relationships_human(&[], "Dependents of X", &ctx);
        assert!(result.contains("(none)"));
        assert!(
            result.contains("3 unresolved internal import(s)"),
            "caveat must show the count, got: {result}"
        );
        assert!(
            result.contains("may not indicate dead code"),
            "caveat must warn against dead-code misread, got: {result}"
        );
    }

    #[test]
    fn test_format_relationships_human_empty_zero_unresolved_no_caveat() {
        let ctx = ResolutionContext {
            unresolved_internal: Some(0),
            unresolved_external: Some(2),
        };
        let result = format_relationships_human(&[], "Dependents of X", &ctx);
        assert!(result.contains("(none)"));
        // Zero internal unresolved — no caveat (full resolution, true 0 dependents)
        assert!(
            !result.contains("unresolved"),
            "no caveat for zero internal unresolved"
        );
    }

    #[test]
    fn test_format_relationships_human_shows_name_type_weight() {
        let entity = make_entity("e1", "target-module", EntityTier::Module);
        let rel = Relationship {
            source_id: "src".to_string(),
            target_id: "e1".to_string(),
            rel_type: RelType::DependsOn,
            weight: 2.5,
            evidence_json: None,
            provenance: EdgeProvenance::default(),
        };
        let result = format_relationships_human(
            &[(rel, entity)],
            "Dependencies of src",
            &ResolutionContext::unknown(),
        );
        assert!(result.contains("target-module"), "name missing");
        assert!(result.contains("depends_on"), "rel type missing");
        assert!(result.contains("2.5"), "weight missing");
    }

    #[test]
    fn test_format_relationships_human_nonempty_suppresses_caveat() {
        // Caveat must NOT fire when there ARE dependents — the dead-code
        // misread only matters on the empty path.
        let entity = make_entity("e1", "target", EntityTier::Module);
        let rel = Relationship {
            source_id: "src".to_string(),
            target_id: "e1".to_string(),
            rel_type: RelType::DependsOn,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::default(),
        };
        let ctx = ResolutionContext {
            unresolved_internal: Some(99),
            unresolved_external: Some(0),
        };
        let result = format_relationships_human(&[(rel, entity)], "Dependents of src", &ctx);
        assert!(
            !result.contains("unresolved"),
            "caveat must be suppressed when dependents exist"
        );
    }

    fn write_json(rels: &[(Relationship, Entity)], ctx: &ResolutionContext) -> String {
        let mut buf = Vec::new();
        format_relationships_json(rels, ctx, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn test_format_relationships_json_empty() {
        let result = write_json(&[], &ResolutionContext::unknown());
        assert_eq!(result.trim(), "[]");
        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_format_relationships_json_empty_with_metadata_line() {
        let ctx = ResolutionContext {
            unresolved_internal: Some(4),
            unresolved_external: Some(2),
        };
        let result = write_json(&[], &ctx);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "expected metadata + empty array, got: {result}"
        );
        // First line is the # metadata line
        assert!(
            lines[0].starts_with('#'),
            "first line must be # metadata, got: {}",
            lines[0]
        );
        let meta_body = lines[0].trim_start_matches('#').trim();
        let meta: serde_json::Value = serde_json::from_str(meta_body).unwrap();
        assert_eq!(meta["unresolved_imports"], 4);
        assert_eq!(meta["unresolved_external"], 2);
        // Second line is the bare empty array
        assert_eq!(lines[1], "[]");
    }

    #[test]
    fn test_format_relationships_json_empty_pre681_no_metadata() {
        // Pre-#681: no metadata line, just the bare []
        let result = write_json(&[], &ResolutionContext::unknown());
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "pre-#681 must have no metadata line, got: {result}"
        );
        assert_eq!(lines[0], "[]");
    }

    #[test]
    fn test_format_relationships_json_single_valid_json() {
        let entity = make_entity("e1", "target-mod", EntityTier::Module);
        let rel = Relationship {
            source_id: "src".to_string(),
            target_id: "e1".to_string(),
            rel_type: RelType::Imports,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Resolved,
        };
        let result = write_json(&[(rel, entity)], &ResolutionContext::unknown());
        let parsed: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert_eq!(parsed["entity_id"], "e1");
        assert_eq!(parsed["entity_name"], "target-mod");
        assert_eq!(parsed["rel_type"], "imports");
        assert_eq!(parsed["weight"], 1.0);
    }

    #[test]
    fn test_format_relationships_json_provenance_field_additive() {
        // Issue #714: provenance must appear additively in the NDJSON row
        // without disturbing any existing field.
        let entity = make_entity("e1", "target-mod", EntityTier::Module);
        let resolved_rel = Relationship {
            source_id: "src".to_string(),
            target_id: "e1".to_string(),
            rel_type: RelType::Imports,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Resolved,
        };
        let result = write_json(&[(resolved_rel, entity)], &ResolutionContext::unknown());
        let parsed: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert_eq!(parsed["provenance"], "resolved");

        let entity2 = make_entity("e2", "heur-mod", EntityTier::Module);
        let heuristic_rel = Relationship {
            source_id: "src".to_string(),
            target_id: "e2".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        };
        let result2 = write_json(&[(heuristic_rel, entity2)], &ResolutionContext::unknown());
        let parsed2: serde_json::Value = serde_json::from_str(result2.trim()).unwrap();
        assert_eq!(parsed2["provenance"], "heuristic");
    }

    #[test]
    fn test_format_relationships_json_multiple_lines() {
        let e1 = make_entity("e1", "a", EntityTier::Module);
        let e2 = make_entity("e2", "b", EntityTier::Module);
        let r1 = Relationship {
            source_id: "src".to_string(),
            target_id: "e1".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::default(),
        };
        let r2 = Relationship {
            source_id: "src".to_string(),
            target_id: "e2".to_string(),
            rel_type: RelType::Contains,
            weight: 0.5,
            evidence_json: None,
            provenance: EdgeProvenance::default(),
        };
        let result = write_json(&[(r1, e1), (r2, e2)], &ResolutionContext::unknown());
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }

    #[test]
    fn test_format_relationships_json_nonempty_with_metadata() {
        // Metadata line is emitted even when rels are non-empty (consistent shape)
        let e1 = make_entity("e1", "a", EntityTier::Module);
        let r1 = Relationship {
            source_id: "src".to_string(),
            target_id: "e1".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::default(),
        };
        let ctx = ResolutionContext {
            unresolved_internal: Some(1),
            unresolved_external: Some(0),
        };
        let result = write_json(&[(r1, e1)], &ctx);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(lines.len(), 2, "metadata + 1 entity, got: {result}");
        assert!(lines[0].starts_with('#'));
    }
}
