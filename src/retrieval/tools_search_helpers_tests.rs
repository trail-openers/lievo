// Helper function tests for tools_search — split from tools_search_tests.rs.

use super::*;

#[test]
fn test_try_load_semantic_searcher_model_failure_msg_contains_guidance() {
    // Use a temp dir that doesn't exist to force model load failure
    let nonexistent_path = "/nonexistent/index/path/that/does/not/exist";
    let (searcher_opt, warning_opt) = try_load_semantic_searcher(nonexistent_path);

    // Should return None for searcher since model will fail to load
    assert!(searcher_opt.is_none());
    // Should return a warning message
    assert!(warning_opt.is_some());

    let warning = warning_opt.unwrap();
    // Verify the warning contains guidance about literal entity names
    assert!(
        warning.contains("literal entity name"),
        "Warning should mention using literal entity names like 'MemoryStore': {}",
        warning
    );
    assert!(
        warning.contains("exact name/path matching"),
        "Warning should mention exact name/path matching: {}",
        warning
    );
    assert!(
        warning.contains("lievo refresh"),
        "Warning should mention running 'lievo refresh': {}",
        warning
    );
}

#[test]
fn test_should_exclude_entity_exact_match() {
    assert!(should_exclude_entity(
        Some("lievo_docs"),
        &Some("lievo_docs".to_string())
    ));
}

#[test]
fn test_should_exclude_entity_child_path() {
    assert!(should_exclude_entity(
        Some("lievo_docs/index.md"),
        &Some("lievo_docs".to_string())
    ));
}

#[test]
fn test_should_exclude_entity_nested_child() {
    assert!(should_exclude_entity(
        Some("lievo_docs/subsystem/module.md"),
        &Some("lievo_docs".to_string())
    ));
}

#[test]
fn test_should_exclude_entity_custom_dir() {
    assert!(should_exclude_entity(
        Some("docs_output/README.md"),
        &Some("docs_output".to_string())
    ));
}

#[test]
fn test_should_exclude_entity_no_match_different_name() {
    assert!(!should_exclude_entity(
        Some("src/main.rs"),
        &Some("lievo_docs".to_string())
    ));
}

#[test]
fn test_should_exclude_entity_no_match_partial_name() {
    assert!(!should_exclude_entity(
        Some("my_lievo_docs/file.md"),
        &Some("lievo_docs".to_string())
    ));
}

#[test]
fn test_should_exclude_entity_exact_child_path() {
    // Entity path "lievo_docs_v2/file.md" is a child of output_dir "lievo_docs_v2"
    assert!(should_exclude_entity(
        Some("lievo_docs_v2/file.md"),
        &Some("lievo_docs_v2".to_string())
    ));
}

#[test]
fn test_should_exclude_entity_no_output_dir() {
    assert!(!should_exclude_entity(Some("any/path"), &None));
}

#[test]
fn test_should_exclude_entity_no_entity_path() {
    assert!(!should_exclude_entity(
        None,
        &Some("lievo_docs".to_string())
    ));
}

#[test]
fn test_truncate_short_string() {
    assert_eq!(truncate("hello", 10), "hello");
}

#[test]
fn test_truncate_exact_length() {
    assert_eq!(truncate("hello", 5), "hello");
}

#[test]
fn test_truncate_long_string() {
    assert_eq!(truncate("hello world", 8), "hello...");
}

#[test]
fn test_truncate_utf8_boundary() {
    // Test that truncate works correctly with UTF-8 characters
    let s = "你好世界"; // 4 Chinese characters

    // When max >= string length, return original string
    assert_eq!(truncate(s, 4), "你好世界");
    assert_eq!(truncate(s, 5), "你好世界");
    assert_eq!(truncate(s, 10), "你好世界");

    // When max < 3, return only truncator since no room for content + "..."
    assert_eq!(truncate(s, 0), "...");
    assert_eq!(truncate(s, 1), "...");
    assert_eq!(truncate(s, 2), "...");
    assert_eq!(truncate(s, 3), "...");

    // When max >= 3 but < string length, truncate to max-3 chars + "..."
    // String '你好好好你好好好' has 8 characters
    let long = "你好好好你好好好";
    // With max=5: take 5-3=2 chars → "你好" + "..." = "你好..."
    assert_eq!(truncate(long, 5), "你好...");

    // With max=6: take 6-3=3 chars → "你好好" + "..." = "你好好..."
    assert_eq!(truncate(long, 6), "你好好...");

    // With max=8 (exact length), return original
    assert_eq!(truncate(long, 8), "你好好好你好好好");
}
