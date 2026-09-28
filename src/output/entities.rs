// Entity (subsystems / modules / files) formatting.

use super::parse_metrics;
use crate::model::{Entity, EntityTier};

/// Format a list of entities as an aligned human-readable table.
///
/// The header row and column widths adapt to the tier:
/// - Subsystem: Name | Modules | Files | Complexity
/// - Module:    Name | Files   | Complexity
/// - File:      Name | Language | Complexity
pub fn format_entities_human(entities: &[Entity], tier: EntityTier) -> String {
    if entities.is_empty() {
        return match tier {
            EntityTier::Subsystem => "No subsystems found.".to_string(),
            EntityTier::Module => "No modules found.".to_string(),
            EntityTier::File => "No files found.".to_string(),
            EntityTier::Function => "No functions found.".to_string(),
        };
    }

    let mut lines = Vec::new();

    match tier {
        EntityTier::Subsystem => {
            let name_w = entities
                .iter()
                .map(|e| e.name.len())
                .max()
                .unwrap_or(4)
                .max(4);
            let header = format!(
                "  {:<name_w$}  {:>7}  {:>5}  {:>10}",
                "Name", "Modules", "Files", "Complexity"
            );
            let sep = "━".repeat(header.chars().count());
            lines.push("SUBSYSTEMS".to_string());
            lines.push(sep);
            lines.push(header);
            for e in entities {
                let (modules, files, complexity) = parse_metrics(e);
                lines.push(format!(
                    "  {:<name_w$}  {:>7}  {:>5}  {:>10.1}",
                    e.name, modules, files, complexity
                ));
            }
        }
        EntityTier::Module => {
            let name_w = entities
                .iter()
                .map(|e| e.name.len())
                .max()
                .unwrap_or(4)
                .max(4);
            let header = format!(
                "  {:<name_w$}  {:>5}  {:>10}",
                "Name", "Files", "Complexity"
            );
            let sep = "━".repeat(header.chars().count());
            lines.push("MODULES".to_string());
            lines.push(sep);
            lines.push(header);
            for e in entities {
                let (_, files, complexity) = parse_metrics(e);
                lines.push(format!(
                    "  {:<name_w$}  {:>5}  {:>10.1}",
                    e.name, files, complexity
                ));
            }
        }
        EntityTier::File => {
            let name_w = entities
                .iter()
                .map(|e| e.name.len())
                .max()
                .unwrap_or(4)
                .max(4);
            let lang_w = entities
                .iter()
                .map(|e| e.language.as_deref().unwrap_or("-").len())
                .max()
                .unwrap_or(8)
                .max(8);
            let header = format!(
                "  {:<name_w$}  {:<lang_w$}  {:>10}",
                "Name", "Language", "Complexity"
            );
            let sep = "━".repeat(header.chars().count());
            lines.push("FILES".to_string());
            lines.push(sep);
            lines.push(header);
            for e in entities {
                let lang = e.language.as_deref().unwrap_or("-");
                let (_, _, complexity) = parse_metrics(e);
                lines.push(format!(
                    "  {:<name_w$}  {:<lang_w$}  {:>10.1}",
                    e.name, lang, complexity
                ));
            }
        }
        EntityTier::Function => {
            let name_w = entities
                .iter()
                .map(|e| e.name.len())
                .max()
                .unwrap_or(8)
                .max(8);
            let lang_w = entities
                .iter()
                .map(|e| e.language.as_deref().unwrap_or("-").len())
                .max()
                .unwrap_or(8)
                .max(8);
            let header = format!(
                "  {:<name_w$}  {:<lang_w$}  {:>10}",
                "Name", "Language", "Complexity"
            );
            let sep = "━".repeat(header.chars().count());
            lines.push("FUNCTIONS".to_string());
            lines.push(sep);
            lines.push(header);
            for e in entities {
                let lang = e.language.as_deref().unwrap_or("-");
                let (_, _, complexity) = parse_metrics(e);
                lines.push(format!(
                    "  {:<name_w$}  {:<lang_w$}  {:>10.1}",
                    e.name, lang, complexity
                ));
            }
        }
    }

    lines.join("\n")
}

/// Format a list of entities as NDJSON (one JSON object per line) written to `writer`.
///
/// Each line includes: id, name, tier, and any metrics fields present.
/// Writes each line immediately — no full-collection buffering.
pub fn format_entities_json(
    entities: &[Entity],
    writer: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    for e in entities {
        let (modules, files, complexity) = parse_metrics(e);
        writeln!(
            writer,
            "{}",
            serde_json::json!({
                "id": e.id,
                "name": e.name,
                "tier": e.tier.to_string(),
                "module_count": modules,
                "file_count": files,
                "complexity_max": complexity,
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
    fn test_format_entities_human_subsystem_empty() {
        let result = format_entities_human(&[], EntityTier::Subsystem);
        assert_eq!(result, "No subsystems found.");
    }

    #[test]
    fn test_format_entities_human_module_empty() {
        let result = format_entities_human(&[], EntityTier::Module);
        assert_eq!(result, "No modules found.");
    }

    #[test]
    fn test_format_entities_human_file_empty() {
        let result = format_entities_human(&[], EntityTier::File);
        assert_eq!(result, "No files found.");
    }

    #[test]
    fn test_format_entities_human_subsystem_has_header() {
        let e = make_entity(
            "s1",
            "my-subsystem",
            EntityTier::Subsystem,
            Some(r#"{"module_count":3,"file_count":12,"complexity_max":42}"#),
        );
        let result = format_entities_human(&[e], EntityTier::Subsystem);
        assert!(result.contains("SUBSYSTEMS"), "missing SUBSYSTEMS header");
        assert!(result.contains("Name"), "missing Name column");
        assert!(result.contains("Modules"), "missing Modules column");
        assert!(result.contains("Files"), "missing Files column");
        assert!(result.contains("Complexity"), "missing Complexity column");
        assert!(result.contains("my-subsystem"), "missing entity name");
    }

    #[test]
    fn test_format_entities_human_subsystem_metrics_parsed() {
        let e = make_entity(
            "s1",
            "my-sub",
            EntityTier::Subsystem,
            Some(r#"{"module_count":3,"file_count":12,"complexity_max":42.5}"#),
        );
        let result = format_entities_human(&[e], EntityTier::Subsystem);
        assert!(result.contains("3"), "module count missing");
        assert!(result.contains("12"), "file count missing");
        assert!(result.contains("42.5"), "complexity missing");
    }

    #[test]
    fn test_format_entities_human_module_has_header() {
        let e = make_entity(
            "m1",
            "my-module",
            EntityTier::Module,
            Some(r#"{"file_count":5,"complexity_max":10}"#),
        );
        let result = format_entities_human(&[e], EntityTier::Module);
        assert!(result.contains("MODULES"), "missing MODULES header");
        assert!(result.contains("my-module"), "missing name");
        assert!(result.contains("5"), "file count missing");
    }

    #[test]
    fn test_format_entities_human_file_has_header() {
        let mut e = make_entity("f1", "main.rs", EntityTier::File, None);
        e.language = Some("Rust".to_string());
        let result = format_entities_human(&[e], EntityTier::File);
        assert!(result.contains("FILES"), "missing FILES header");
        assert!(result.contains("main.rs"), "missing name");
        assert!(result.contains("Rust"), "missing language");
    }

    #[test]
    fn test_format_entities_human_no_metrics_shows_zeros() {
        let e = make_entity("s1", "bare", EntityTier::Subsystem, None);
        let result = format_entities_human(&[e], EntityTier::Subsystem);
        assert!(result.contains("bare"));
        assert!(result.contains("0.0") || result.contains("0"));
    }

    #[test]
    fn test_format_entities_human_invalid_metrics_shows_zeros() {
        let e = make_entity("s1", "bad", EntityTier::Subsystem, Some("not-json"));
        let result = format_entities_human(&[e], EntityTier::Subsystem);
        assert!(result.contains("bad"));
    }

    fn write_json(entities: &[Entity]) -> String {
        let mut buf = Vec::new();
        format_entities_json(entities, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn test_format_entities_json_empty() {
        let result = write_json(&[]);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_entities_json_single_entity_valid_json() {
        let e = make_entity(
            "s1",
            "my-subsystem",
            EntityTier::Subsystem,
            Some(r#"{"module_count":3,"file_count":12,"complexity_max":42}"#),
        );
        let result = write_json(&[e]);
        let parsed: serde_json::Value =
            serde_json::from_str(result.trim()).expect("must be valid JSON");
        assert_eq!(parsed["name"], "my-subsystem");
        assert_eq!(parsed["tier"], "subsystem");
        assert_eq!(parsed["module_count"], 3);
        assert_eq!(parsed["file_count"], 12);
        assert_eq!(parsed["complexity_max"], 42.0);
    }

    #[test]
    fn test_format_entities_json_multiple_entities_each_line_valid_json() {
        let entities = vec![
            make_entity("s1", "sub-a", EntityTier::Subsystem, None),
            make_entity("s2", "sub-b", EntityTier::Subsystem, None),
        ];
        let result = write_json(&entities);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(lines.len(), 2, "expected 2 NDJSON lines");
        for line in lines {
            serde_json::from_str::<serde_json::Value>(line).expect("each line must be valid JSON");
        }
    }

    #[test]
    fn test_format_entities_json_includes_id_and_name() {
        let e = make_entity("entity-42", "cool-name", EntityTier::Module, None);
        let result = write_json(&[e]);
        let parsed: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert_eq!(parsed["id"], "entity-42");
        assert_eq!(parsed["name"], "cool-name");
    }
}
