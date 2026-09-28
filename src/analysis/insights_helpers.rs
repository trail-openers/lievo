// Standalone helper functions for insight detection.
// These are stateless, synchronous utilities with no Storage dependency.

use crate::model::Entity;

/// Extract complexity from entity's `metrics_json`.
pub fn complexity_of(entity: &Entity) -> f64 {
    let json = match entity.metrics_json.as_deref() {
        Some(s) => s,
        None => return 0.0,
    };
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("complexity_max").and_then(|c| c.as_f64()))
        .unwrap_or(0.0)
}

/// Return true if the entity's filename or path indicates a test file.
///
/// Checks both the filename (last path component) for naming patterns and the
/// full path for common test directory names (`/tests/`, `/test/`, `/spec/`,
/// `/__tests__/`). Uses word-boundary patterns on the filename to avoid
/// false positives like "latest", "contest", or "attestation".
///
/// Examples: `user_test.rs`, `test_foo.py`, `bar_spec.rb`, `tests/integration.rs`.
pub fn is_test_file(entity: &Entity) -> bool {
    let raw = entity.path.as_deref().unwrap_or(&entity.name);
    let filename = raw.rsplit('/').next().unwrap_or(raw).to_lowercase();
    // Match patterns: test_*.ext, *_test.ext, *_test_*, *_tests.ext (plural), *_spec.ext, *.test.ext, *.spec.ext
    let name_match = filename.starts_with("test_")
        || filename.starts_with("tests_")
        || filename.contains("_test.")
        || filename.contains("_tests.")
        || filename.contains("_test_")
        || filename.contains("_tests_")
        || filename.ends_with("_test")
        || filename.ends_with("_tests")
        || filename.contains("_spec.")
        || filename.contains("_spec_")
        || filename.ends_with("_spec")
        || filename.contains(".test.")
        || filename.contains(".spec.")
        || filename.starts_with("spec_");

    if name_match {
        return true;
    }

    // Also match files inside common test directories.
    // Prepend '/' so that root-relative paths ("tests/foo.rs") and absolute paths
    // ("/project/tests/foo.rs") both match the same directory patterns.
    let path_lower = raw.replace('\\', "/").to_lowercase();
    let anchored = format!("/{path_lower}");
    anchored.contains("/tests/")
        || anchored.contains("/test/")
        || anchored.contains("/spec/")
        || anchored.contains("/__tests__/")
}

/// Return the stem (filename without extension) of an entity's path, lowercased.
pub fn file_stem(entity: &Entity) -> Option<String> {
    let raw = entity.path.as_deref().unwrap_or(&entity.name);
    let filename = raw.rsplit('/').next().unwrap_or(raw);
    // Strip the first extension (e.g. "foo_test.rs" → "foo_test", "foo.test.ts" → "foo.test")
    let stem = filename
        .rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(filename);
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_lowercase())
    }
}

/// Return true if `needle` appears as a whole word in `haystack`, where word
/// boundaries are `_`, `-`, start-of-string, or end-of-string.
pub(crate) fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    for (i, _) in haystack.match_indices(needle) {
        let before_ok = i == 0 || matches!(haystack.as_bytes()[i - 1], b'_' | b'-');
        let after = i + needle.len();
        let after_ok = after == haystack.len() || matches!(haystack.as_bytes()[after], b'_' | b'-');
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// Return true if `test_stem` likely covers the given module.
///
/// Checks whether the test file's stem contains the module name OR any child
/// file stem as a whole word (delimited by `_`, `-`, start, or end of string).
/// Only child stems of 4+ characters are matched to avoid spurious hits on
/// very short names like "mod" or "lib". The caller is responsible for
/// filtering short stems before passing `child_stems`.
pub(crate) fn test_file_matches_module(
    test_stem: &str,
    module_name: &str,
    child_stems: &[String],
) -> bool {
    // Check if test name contains module name as a whole word (skip very short module names)
    if module_name.len() >= 4 && contains_word(test_stem, module_name) {
        return true;
    }
    // Check if test name contains any child file stem as a whole word
    child_stems
        .iter()
        .any(|child| contains_word(test_stem, child.as_str()))
}

/// Build a deterministic insight ID: `{project_id}:insight:{category}:{entity_id}`.
pub fn insight_id(project_id: &str, category: &str, entity_id: &str) -> String {
    format!("{project_id}:insight:{category}:{entity_id}")
}

/// Numeric sort key for severity (lower = higher priority).
pub fn severity_order(severity: Option<&str>) -> u8 {
    match severity {
        Some("critical") => 0,
        Some("high") => 1,
        Some("medium") => 2,
        Some("low") => 3,
        _ => 4,
    }
}

pub fn now() -> String {
    use chrono::Utc;
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EntityTier;

    fn test_entity(id: &str, path: &str, language: &str) -> Entity {
        Entity {
            id: id.to_string(),
            project_id: "proj".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "test".to_string(),
            path: Some(path.to_string()),
            language: Some(language.to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_is_test_file_patterns() {
        assert!(is_test_file(&test_entity("e1", "src/user_test.rs", "Rust")));
        assert!(is_test_file(&test_entity(
            "e2",
            "src/foo_test.py",
            "Python"
        )));
        assert!(is_test_file(&test_entity(
            "e3",
            "tests/integration.rs",
            "Rust"
        )));
        assert!(is_test_file(&test_entity("e4", "spec/foo_spec.rb", "Ruby")));
        assert!(!is_test_file(&test_entity("e5", "src/main.rs", "Rust")));
        assert!(!is_test_file(&test_entity("e6", "src/latest.rs", "Rust")));
        // tests_ prefix (plural)
        assert!(is_test_file(&test_entity(
            "e7",
            "src/config/tests_utils.rs",
            "Rust"
        )));
        assert!(is_test_file(&test_entity(
            "e8",
            "tests_helpers.py",
            "Python"
        )));
        assert!(is_test_file(&test_entity(
            "e9",
            "lib/unit_tests.rs",
            "Rust"
        )));
    }

    #[test]
    fn test_complexity_of() {
        let entity = test_entity("e1", "foo.rs", "Rust");
        assert_eq!(complexity_of(&entity), 0.0);

        let mut e = test_entity("e2", "bar.rs", "Rust");
        e.metrics_json = Some(r#"{"complexity_max": 42.5}"#.to_string());
        assert_eq!(complexity_of(&e), 42.5);
    }

    #[test]
    fn test_insight_id_format() {
        let id = insight_id("proj1", "coverage_gap", "entity5");
        assert_eq!(id, "proj1:insight:coverage_gap:entity5");
    }

    #[test]
    fn test_severity_order() {
        assert_eq!(severity_order(Some("critical")), 0);
        assert_eq!(severity_order(Some("high")), 1);
        assert_eq!(severity_order(Some("medium")), 2);
        assert_eq!(severity_order(Some("low")), 3);
        assert_eq!(severity_order(None), 4);
        assert_eq!(severity_order(Some("unknown")), 4);
    }

    #[test]
    fn test_file_stem() {
        assert_eq!(
            file_stem(&test_entity("e1", "src/lib.rs", "Rust")),
            Some("lib".to_string())
        );
        assert_eq!(
            file_stem(&test_entity("e2", "src/foo_test.rs", "Rust")),
            Some("foo_test".to_string())
        );
        assert_eq!(
            file_stem(&test_entity("e3", "bar", "Rust")),
            Some("bar".to_string())
        );
    }

    #[test]
    fn test_contains_word() {
        assert!(contains_word("foo_bar", "foo"));
        assert!(contains_word("foo_bar_baz", "bar"));
        assert!(!contains_word("foobar", "foo"));
        assert!(!contains_word("foo_bar", "o"));
    }

    #[test]
    fn test_test_file_matches_module() {
        assert!(test_file_matches_module(
            "user_test",
            "user",
            &["auth".to_string()]
        ));
        assert!(test_file_matches_module("auth_service_test", "auth", &[]));
        assert!(!test_file_matches_module("foo_test", "bar", &[]));
    }
}
