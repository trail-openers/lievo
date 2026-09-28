// Documentation parsing utilities — extracted from tools_doc.rs.

use serde_json::Value;
use serde_json::json;

/// Extract structured summary from documentation file without LLM.
///
/// Returns headings, decisions, terminology, and configuration values found in the doc.
pub(super) fn extract_doc_summary(path: &str, content: &str) -> Value {
    let headings = extract_headings(content);
    let decisions = extract_decisions(content);
    let terminology = extract_terminology(content);
    let config_values = extract_config_values(content);

    json!({
        "file": path,
        "headings": headings,
        "decisions": decisions,
        "terminology": terminology,
        "config_values": config_values
    })
}

/// Extract markdown headings from content.
fn extract_headings(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| line.starts_with('#'))
        .take(20)
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .collect()
}

/// Extract key decision statements (lines containing "decision", "designed", "architecture").
fn extract_decisions(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            (lower.contains("decision")
                || lower.contains("designed")
                || lower.contains("architecture"))
                && line.len() > 20
        })
        .take(10)
        .map(|line| line.trim().to_string())
        .collect()
}

/// Extract terminology definitions (lines containing colons or dashes in early lines).
fn extract_terminology(content: &str) -> Vec<String> {
    content
        .lines()
        .take(100)
        .filter(|line| {
            (line.contains(": ") || line.contains(" - "))
                && line.len() > 10
                && !line.starts_with('#')
        })
        .take(15)
        .map(|line| line.trim().to_string())
        .collect()
}

/// Extract environment variables and config keys (patterns like VAR_NAME=value).
fn extract_config_values(content: &str) -> Vec<String> {
    let mut values = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.contains('=') && !trimmed.is_empty() {
            // Extract the key part (before =) and check if it looks like a config key
            // (all uppercase or uppercase with underscores).
            if let Some(key) = trimmed.split('=').next()
                && !key.is_empty()
                && key.chars().all(|c| c.is_uppercase() || c == '_')
            {
                values.push(trimmed.to_string());
            }
        }
    }
    values.truncate(10);
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_headings_from_markdown() {
        let content = "# Main Title\n## Section\n### Subsection\nSome text";
        let headings = extract_headings(content);
        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0], "Main Title");
        assert_eq!(headings[1], "Section");
        assert_eq!(headings[2], "Subsection");
    }

    #[test]
    fn test_extract_decisions_from_content() {
        let content = "This is a design decision made early on.\n\
                        The architecture decision was to use X.\n\
                        Regular text line.\n\
                        Designed to be extensible.";
        let decisions = extract_decisions(content);
        assert!(decisions.iter().any(|d| d.contains("design decision")));
        assert!(decisions.iter().any(|d| d.contains("architecture")));
    }

    #[test]
    fn test_extract_terminology_from_content() {
        let content = "# Terminology\n\
                        Entity: A code artifact\n\
                        Subsystem - A major component\n\
                        Module: A logical grouping";
        let terms = extract_terminology(content);
        assert!(terms.iter().any(|t| t.contains("Entity")));
        assert!(terms.iter().any(|t| t.contains("Subsystem")));
    }

    #[test]
    fn test_extract_config_values_from_content() {
        let content = "DATABASE_URL=postgres://localhost\nPORT=8080\nregular text";
        let configs = extract_config_values(content);
        assert_eq!(configs.len(), 2);
        assert!(configs.iter().any(|c| c.contains("DATABASE_URL")));
        assert!(configs.iter().any(|c| c.contains("PORT")));
    }

    #[test]
    fn test_extract_doc_summary_produces_valid_json() {
        let content = "# Main Title\n## Section\n\nDesign decision made here.\n\nEntity: thing";
        let summary = extract_doc_summary("test.md", content);

        assert_eq!(summary["file"], "test.md");
        assert!(summary["headings"].is_array());
        assert!(summary["decisions"].is_array());
        assert!(summary["terminology"].is_array());
        assert!(summary["config_values"].is_array());
    }
}
