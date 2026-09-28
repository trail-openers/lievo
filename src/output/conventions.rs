// Convention formatting for `query conventions`.

use crate::model::Convention;

/// Format conventions as a human-readable table.
///
/// Columns: Category | Confidence | Title | Description
pub fn format_conventions_human(conventions: &[Convention]) -> String {
    if conventions.is_empty() {
        return "No conventions found.".to_string();
    }

    let cat_w = conventions
        .iter()
        .map(|c| c.category.len())
        .max()
        .unwrap_or(8)
        .max(8);

    let title_w = conventions
        .iter()
        .map(|c| c.title.len())
        .max()
        .unwrap_or(5)
        .max(5);

    let header = format!(
        "  {:<cat_w$}  {:>10}  {:<title_w$}  Description",
        "Category", "Confidence", "Title"
    );
    let sep = "━".repeat(header.chars().count());

    let mut lines = vec!["CONVENTIONS".to_string(), sep, header];

    for c in conventions {
        let desc = c.description.as_deref().unwrap_or("-");
        lines.push(format!(
            "  {:<cat_w$}  {:>10.2}  {:<title_w$}  {desc}",
            c.category, c.confidence, c.title
        ));
    }

    lines.join("\n")
}

/// Format conventions as NDJSON (one JSON object per line) written to `writer`.
/// Writes each line immediately — no full-collection buffering.
pub fn format_conventions_json(
    conventions: &[Convention],
    writer: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    for c in conventions {
        writeln!(
            writer,
            "{}",
            serde_json::json!({
                "id": c.id,
                "category": c.category,
                "title": c.title,
                "description": c.description,
                "example_code": c.example_code,
                "confidence": c.confidence,
                "detected_at": c.detected_at,
                "still_valid": c.still_valid,
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

    fn make_convention(category: &str, title: &str, confidence: f64) -> Convention {
        Convention {
            id: "conv-1".to_string(),
            project_id: "proj-1".to_string(),
            category: category.to_string(),
            title: title.to_string(),
            description: Some("A test convention.".to_string()),
            example_code: None,
            confidence,
            entity_ids_json: None,
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            still_valid: true,
        }
    }

    #[test]
    fn test_format_conventions_human_empty() {
        let result = format_conventions_human(&[]);
        assert_eq!(result, "No conventions found.");
    }

    #[test]
    fn test_format_conventions_human_has_header() {
        let c = make_convention("naming", "SnakeCase", 0.9);
        let result = format_conventions_human(&[c]);
        assert!(result.contains("CONVENTIONS"));
        assert!(result.contains("Category"));
        assert!(result.contains("Confidence"));
        assert!(result.contains("Title"));
    }

    #[test]
    fn test_format_conventions_human_contains_data() {
        let c = make_convention("naming", "SnakeCase", 0.9);
        let result = format_conventions_human(&[c]);
        assert!(result.contains("naming"));
        assert!(result.contains("SnakeCase"));
        assert!(result.contains("0.90"));
    }

    #[test]
    fn test_format_conventions_human_no_description_shows_dash() {
        let mut c = make_convention("testing", "UnitTests", 0.8);
        c.description = None;
        let result = format_conventions_human(&[c]);
        assert!(result.contains('-'));
    }

    fn write_json(conventions: &[Convention]) -> String {
        let mut buf = Vec::new();
        format_conventions_json(conventions, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn test_format_conventions_json_empty() {
        let result = write_json(&[]);
        assert_eq!(result.trim(), "");
    }

    #[test]
    fn test_format_conventions_json_single_valid_json() {
        let c = make_convention("architecture", "LayerSeparation", 0.95);
        let result = write_json(&[c]);
        let parsed: serde_json::Value =
            serde_json::from_str(result.trim()).expect("must be valid JSON");
        assert_eq!(parsed["category"], "architecture");
        assert_eq!(parsed["title"], "LayerSeparation");
        assert_eq!(parsed["confidence"], 0.95);
    }

    #[test]
    fn test_format_conventions_json_multiple_lines() {
        let convs = vec![
            make_convention("naming", "SnakeCase", 0.9),
            make_convention("testing", "UnitTests", 0.8),
        ];
        let result = write_json(&convs);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            serde_json::from_str::<serde_json::Value>(line).expect("each line must be valid JSON");
        }
    }

    #[test]
    fn test_format_conventions_json_includes_all_fields() {
        let mut c = make_convention("error_handling", "ResultType", 0.85);
        c.example_code = Some("fn foo() -> Result<()>".to_string());
        let result = write_json(&[c]);
        let parsed: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert_eq!(parsed["id"], "conv-1");
        assert_eq!(parsed["still_valid"], true);
        assert_eq!(parsed["example_code"], "fn foo() -> Result<()>");
    }
}
