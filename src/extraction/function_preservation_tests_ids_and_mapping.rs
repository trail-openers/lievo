use crate::extraction::function_preservation::{build_function_map, function_id, is_function_unit};
use crate::model::{Entity, EntityTier};

#[test]
fn test_function_id_is_deterministic() {
    let id1 = function_id("file-1", "my_func");
    let id2 = function_id("file-1", "my_func");

    assert_eq!(id1, id2);
}

#[test]
fn test_function_id_differs_for_different_names() {
    let id1 = function_id("file-1", "func_a");
    let id2 = function_id("file-1", "func_b");

    assert_ne!(id1, id2);
}

#[test]
fn test_function_id_differs_for_different_files() {
    let id1 = function_id("file-1", "my_func");
    let id2 = function_id("file-2", "my_func");

    assert_ne!(id1, id2);
}

#[test]
fn test_is_function_unit() {
    // Function-like unit types should return true
    assert!(is_function_unit("function"));
    assert!(is_function_unit("regular_function"));
    assert!(is_function_unit("method"));
    assert!(is_function_unit("closure"));
    assert!(is_function_unit("async_function"));

    // Case-insensitive matching
    assert!(is_function_unit("Function"));
    assert!(is_function_unit("METHOD"));
    assert!(is_function_unit("Closure"));

    // Non-function unit types should return false
    assert!(!is_function_unit("file"));
    assert!(!is_function_unit("module"));
    assert!(!is_function_unit("class"));
    assert!(!is_function_unit("interface"));
    assert!(!is_function_unit("variable"));
    assert!(!is_function_unit("parameter"));
}

#[test]
fn test_build_function_map_includes_all() {
    // Create two functions with the same name in different files
    let fn_a = Entity {
        id: "fn-a".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File, // dummy tier, parent_id is what matters
        parent_id: Some("file-a".to_string()),
        name: "validate".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let fn_b = Entity {
        id: "fn-b".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("file-b".to_string()),
        name: "validate".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let map = build_function_map(&[fn_a, fn_b]);
    // "validate" is defined in 2 different files → both IDs should be present
    assert!(
        map.contains_key("validate"),
        "ambiguous function names should still be in the map"
    );
    let ids = map.get("validate").unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"fn-a".to_string()));
    assert!(ids.contains(&"fn-b".to_string()));
}

#[test]
fn test_build_function_map_unambiguous() {
    // Create a function defined in only one file
    let fn_a = Entity {
        id: "fn-a".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("file-a".to_string()),
        name: "validate".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let map = build_function_map(&[fn_a]);
    // "validate" defined in exactly one file → should have single ID
    assert_eq!(map.get("validate"), Some(&vec!["fn-a".to_string()]));
}

#[test]
fn test_issue_521_ambiguous_names_included() {
    // Test that common function names appearing in multiple files are NOT dropped
    // This fixes #521: called_by empty for ambiguous function names
    let fn_a_search = Entity {
        id: "fn-a-search".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-auth".to_string()),
        name: "search".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let fn_b_search = Entity {
        id: "fn-b-search".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-user".to_string()),
        name: "search".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let map = build_function_map(&[fn_a_search, fn_b_search]);
    // Both "search" functions should be in the map
    assert!(
        map.contains_key("search"),
        "ambiguous function name 'search' should be in the map"
    );
    let ids = map.get("search").unwrap();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"fn-a-search".to_string()));
    assert!(ids.contains(&"fn-b-search".to_string()));
}

#[test]
fn test_issue_521_multiple_common_names() {
    // Test multiple common function names (search, new, get, etc.)
    let fn_search_1 = Entity {
        id: "fn-search-1".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-a".to_string()),
        name: "search".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let fn_search_2 = Entity {
        id: "fn-search-2".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-b".to_string()),
        name: "search".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let fn_new_1 = Entity {
        id: "fn-new-1".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-a".to_string()),
        name: "new".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let fn_new_2 = Entity {
        id: "fn-new-2".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-b".to_string()),
        name: "new".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let map = build_function_map(&[fn_search_1, fn_search_2, fn_new_1, fn_new_2]);
    // Both names should be present with two IDs each
    assert_eq!(map.get("search").map(|v| v.len()), Some(2));
    assert_eq!(map.get("new").map(|v| v.len()), Some(2));
}

#[test]
fn test_issue_534_is_function_unit_excludes_type_units() {
    // Ensure is_function_unit returns false for type units
    // (this function is used for cfg(test) detection, which doesn't apply to types)
    assert!(!is_function_unit("struct"));
    assert!(!is_function_unit("enum"));
    assert!(!is_function_unit("class"));
    assert!(!is_function_unit("interface"));
    assert!(!is_function_unit("trait"));
    assert!(!is_function_unit("type"));

    // But it should still match functions
    assert!(is_function_unit("function"));
    assert!(is_function_unit("method"));
}
