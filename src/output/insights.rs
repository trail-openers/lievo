// Insight formatting for `insights` command.

use crate::model::Insight;

/// Format insights as a human-readable table.
///
/// Columns: Severity | Category | Title | Description
pub fn format_insights_human(insights: &[Insight]) -> String {
    if insights.is_empty() {
        return "No insights found.".to_string();
    }

    let sev_w = insights
        .iter()
        .map(|i| i.severity.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(8)
        .max(8);

    let cat_w = insights
        .iter()
        .map(|i| i.category.len())
        .max()
        .unwrap_or(8)
        .max(8);

    let title_w = insights
        .iter()
        .map(|i| i.title.len())
        .max()
        .unwrap_or(5)
        .max(5);

    let header = format!(
        "  {:<sev_w$}  {:<cat_w$}  {:<title_w$}  Description",
        "Severity", "Category", "Title"
    );
    let sep = "━".repeat(header.chars().count());

    let mut lines = vec!["INSIGHTS".to_string(), sep, header];

    for insight in insights {
        let sev = insight.severity.as_deref().unwrap_or("-");
        let desc = insight.description.as_deref().unwrap_or("-");
        lines.push(format!(
            "  {:<sev_w$}  {:<cat_w$}  {:<title_w$}  {desc}",
            sev, insight.category, insight.title
        ));
    }

    lines.join("\n")
}

/// Format insights as NDJSON (one JSON object per line) written to `writer`.
/// Writes each line immediately — no full-collection buffering.
pub fn format_insights_json(
    insights: &[Insight],
    writer: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    for i in insights {
        writeln!(
            writer,
            "{}",
            serde_json::json!({
                "id": i.id,
                "category": i.category,
                "severity": i.severity,
                "title": i.title,
                "description": i.description,
                "entity_ids_json": i.entity_ids_json,
                "detected_at": i.detected_at,
                "still_valid": i.still_valid,
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

    fn make_insight(category: &str, severity: Option<&str>, title: &str) -> Insight {
        Insight {
            id: "insight-1".to_string(),
            project_id: "proj-1".to_string(),
            category: category.to_string(),
            severity: severity.map(str::to_string),
            title: title.to_string(),
            description: Some("A test insight.".to_string()),
            entity_ids_json: None,
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            still_valid: true,
        }
    }

    #[test]
    fn test_format_insights_human_empty() {
        assert_eq!(format_insights_human(&[]), "No insights found.");
    }

    #[test]
    fn test_format_insights_human_has_header() {
        let i = make_insight(
            "complexity_hotspot",
            Some("critical"),
            "High complexity: foo",
        );
        let result = format_insights_human(&[i]);
        assert!(result.contains("INSIGHTS"));
        assert!(result.contains("Severity"));
        assert!(result.contains("Category"));
        assert!(result.contains("Title"));
    }

    #[test]
    fn test_format_insights_human_contains_data() {
        let i = make_insight("high_coupling", Some("high"), "High coupling: bar");
        let result = format_insights_human(&[i]);
        assert!(result.contains("high_coupling"));
        assert!(result.contains("High coupling: bar"));
        assert!(result.contains("high"));
    }

    #[test]
    fn test_format_insights_human_no_severity_shows_dash() {
        let i = make_insight("coverage_gap", None, "Low coverage");
        let result = format_insights_human(&[i]);
        assert!(result.contains('-'));
    }

    #[test]
    fn test_format_insights_human_no_description_shows_dash() {
        let mut i = make_insight("coverage_gap", Some("medium"), "Low coverage");
        i.description = None;
        let result = format_insights_human(&[i]);
        // The last column should show '-' for missing description
        assert!(result.contains('-'));
    }

    fn write_json(insights: &[Insight]) -> String {
        let mut buf = Vec::new();
        format_insights_json(insights, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn test_format_insights_json_empty() {
        let result = write_json(&[]);
        assert_eq!(result.trim(), "");
    }

    #[test]
    fn test_format_insights_json_single_valid_json() {
        let i = make_insight(
            "complexity_hotspot",
            Some("critical"),
            "High complexity: foo",
        );
        let result = write_json(&[i]);
        let parsed: serde_json::Value =
            serde_json::from_str(result.trim()).expect("must be valid JSON");
        assert_eq!(parsed["category"], "complexity_hotspot");
        assert_eq!(parsed["severity"], "critical");
        assert_eq!(parsed["title"], "High complexity: foo");
        assert_eq!(parsed["still_valid"], true);
    }

    #[test]
    fn test_format_insights_json_multiple_lines() {
        let insights = vec![
            make_insight(
                "complexity_hotspot",
                Some("critical"),
                "High complexity: foo",
            ),
            make_insight("coverage_gap", Some("medium"), "Low test coverage: bar"),
        ];
        let result = write_json(&insights);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(lines.len(), 2);
        for line in lines {
            serde_json::from_str::<serde_json::Value>(line).expect("each line must be valid JSON");
        }
    }

    #[test]
    fn test_format_insights_json_includes_all_fields() {
        let mut i = make_insight("high_coupling", Some("high"), "High coupling: baz");
        i.entity_ids_json = Some(r#"["entity-1"]"#.to_string());
        let result = write_json(&[i]);
        let parsed: serde_json::Value = serde_json::from_str(result.trim()).unwrap();
        assert_eq!(parsed["id"], "insight-1");
        assert_eq!(parsed["entity_ids_json"], r#"["entity-1"]"#);
        assert_eq!(parsed["detected_at"], "2024-01-01T00:00:00Z");
    }
}
