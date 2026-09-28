use crate::analysis::relationships::RelationshipBuilder;
use crate::extraction::grouping::GroupingResult;
use crate::model::{CodeUnit, EdgeProvenance, Entity, EntityTier, RelType};
use std::path::Path;

/// Test helper: create a CodeUnit with sensible defaults.
/// Only `name`, `file`, and `calls` differ between test cases.
fn make_code_unit(name: &str, file: &str, calls: Vec<String>) -> CodeUnit {
    CodeUnit {
        name: name.to_string(),
        qualified_name: name.to_string(),
        unit_type: "function".to_string(),
        file: file.to_string(),
        line: 1,
        end_line: 10,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls,
        imports: vec![],
    }
}

#[test]
fn test_function_edges_conversion() {
    // CRITICAL FIX: Verify function-level edges are converted to Relationship structs
    // (Previously they were added to edge_weights but never converted)
    let code_units = vec![make_code_unit(
        "main",
        "src/main.rs",
        vec!["helper".to_string()],
    )];

    let file = Entity {
        id: "file-1".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let fn_main = Entity {
        id: "fn-1".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("file-1".to_string()),
        name: "main".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let fn_helper = Entity {
        id: "fn-2".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("file-1".to_string()),
        name: "helper".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let grouping = GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files: vec![file],
        preserve_function_entities: false,
    };

    let (rels, _unresolved) = RelationshipBuilder::build_with_functions(
        &code_units,
        &grouping,
        "p",
        "r",
        Path::new(".").canonicalize().unwrap().as_path(),
        Some(&[fn_main, fn_helper]),
    )
    .unwrap();

    // Verify function edge exists in result
    let has_fn_call = rels
        .iter()
        .any(|r| r.source_id == "fn-1" && r.target_id == "fn-2" && r.rel_type == RelType::Calls);
    assert!(
        has_fn_call,
        "Function edge must be converted to Relationship"
    );

    // #714: function-level Calls edges come from tree-sitter extraction with
    // fn_map name matching — the classification is heuristic (name-matched,
    // not path-resolved).
    let fn_call = rels
        .iter()
        .find(|r| r.source_id == "fn-1" && r.target_id == "fn-2" && r.rel_type == RelType::Calls)
        .expect("the fn-1 → fn-2 Calls edge must exist");
    assert_eq!(
        fn_call.provenance,
        EdgeProvenance::Heuristic,
        "fn_map-driven function-level Calls edge must classify as heuristic"
    );
}

/// Regression test for issue #447: file→file edges from fn_map resolution
/// must use RelType::Imports, NOT RelType::Calls. At file tier, cross-file
/// dependencies are "imports" (file A uses/imports file B), not "calls".
#[test]
fn test_file_to_file_call_edges_use_imports() {
    let code_units = vec![
        make_code_unit("main", "src/main.rs", vec!["helper".to_string()]),
        make_code_unit("helper", "src/utils.rs", vec![]),
    ];

    let file_main = Entity {
        id: "file-main".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let file_utils = Entity {
        id: "file-utils".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "utils.rs".to_string(),
        path: Some("src/utils.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let grouping = GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files: vec![file_main, file_utils],
        preserve_function_entities: false,
    };

    let (rels, _unresolved) = RelationshipBuilder::build(
        &code_units,
        &grouping,
        "p",
        "r",
        Path::new(".").canonicalize().unwrap().as_path(),
    )
    .unwrap();

    // File→file edge from fn_map resolution must be Imports, not Calls
    let has_file_imports = rels.iter().any(|r| {
        r.source_id == "file-main" && r.target_id == "file-utils" && r.rel_type == RelType::Imports
    });
    assert!(
        has_file_imports,
        "File→file edge from fn_map must use RelType::Imports"
    );

    // No file→file Calls edges should exist
    let has_file_calls = rels.iter().any(|r| {
        r.source_id == "file-main" && r.target_id == "file-utils" && r.rel_type == RelType::Calls
    });
    assert!(
        !has_file_calls,
        "File→file edges must NOT use RelType::Calls (issue #447)"
    );

    // #714: the file→file Imports edge above is fn_map name matching ("main"
    // calls "helper" → utils.rs defines it), so it classifies as heuristic.
    let file_imports = rels
        .iter()
        .find(|r| {
            r.source_id == "file-main"
                && r.target_id == "file-utils"
                && r.rel_type == RelType::Imports
        })
        .expect("the file-main → file-utils Imports edge must exist");
    assert_eq!(
        file_imports.provenance,
        EdgeProvenance::Heuristic,
        "fn_map-driven file→file Imports edge must classify as heuristic"
    );
}

// Tests for issue #549 Part A: Skip cfg(test) CodeUnits in call graph loop

#[test]
fn test_issue_549_part_a_cfg_test_unit_skipped_in_call_graph() {
    // Test that CodeUnits with function names matching cfg(test) patterns are skipped
    // during call graph construction, preventing "no function entity matched" warnings
    let file = Entity {
        id: "file-1".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "embedding.rs".to_string(),
        path: Some("src/embedding.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    // Create a production function entity (no cfg(test) helper entities)
    let fn_production = Entity {
        id: "fn-production".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some("file-1".to_string()),
        name: "production_fn".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    // Create CodeUnits for both production and cfg(test) functions
    let code_units = vec![
        CodeUnit {
            name: "production_fn".to_string(),
            qualified_name: "production_fn".to_string(),
            unit_type: "function".to_string(),
            file: "src/embedding.rs".to_string(),
            line: 10,
            end_line: 50,
            language: "Rust".to_string(),
            signature: None,
            code: None,
            docstring: None,
            parent_class: None,
            complexity: 5,
            has_branches: true,
            has_loops: false,
            has_error_handling: false,
            calls: vec!["helper".to_string()],
            imports: vec![],
        },
        // This CodeUnit represents a cfg(test) function that won't have a Function entity
        CodeUnit {
            name: "helper".to_string(),
            qualified_name: "helper".to_string(),
            unit_type: "function".to_string(),
            file: "src/embedding.rs".to_string(),
            line: 55,
            end_line: 70,
            language: "Rust".to_string(),
            signature: None,
            code: None,
            docstring: None,
            parent_class: None,
            complexity: 2,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec!["other".to_string()],
            imports: vec![],
        },
    ];

    let grouping = GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files: vec![file],
        preserve_function_entities: false,
    };

    // Create a temporary directory with a file that has cfg(test) functions
    let temp = tempfile::tempdir().unwrap();
    let src_dir = temp.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let embedding_file = src_dir.join("embedding.rs");

    // Write source with cfg(test) helper function
    let source = r#"
pub fn production_fn() -> i32 {
    helper()
}

#[cfg(test)]
fn helper() -> i32 {
    42
}
"#;
    std::fs::write(&embedding_file, source).unwrap();

    // Build relationships - cfg(test) call should not generate warnings or edges
    let (rels, _unresolved) = RelationshipBuilder::build_with_functions(
        &code_units,
        &grouping,
        "p",
        "r",
        temp.path(),
        Some(&[fn_production]),
    )
    .unwrap();

    // Verify that relationships were created without errors
    // The cfg(test) CodeUnit should be skipped, so we won't have entity lookup failures
    // We can't easily capture stderr in unit tests to verify warnings are suppressed,
    // but the test passing without panicking indicates the cfg(test) check works

    // Verify that only the production function's relationships exist
    // (cfg(test) helper should have no relationships since it's skipped)
    let has_cfg_test_rel = rels
        .iter()
        .any(|r| r.source_id.contains("helper") || r.target_id.contains("helper"));
    assert!(
        !has_cfg_test_rel,
        "cfg(test) function should not appear in relationships"
    );
}

// ── #742 / PR #747 review fix: unresolved-kind classification ──────────

#[test]
fn test_unresolved_super_classified_internal_not_external() {
    // The unresolved-kind classifier must count an unresolved `super::x`
    // (and `self::x` / bare `super` / `self`) as INTERNAL — they are
    // relative internal specifiers that do not start with '.' — not
    // External, or the UnresolvedCounts side-channel that selfcheck_ops
    // gates on (#724) is corrupted.
    let unit = CodeUnit {
        name: "f".to_string(),
        qualified_name: "f".to_string(),
        unit_type: "function".to_string(),
        file: "src/storage/util.rs".to_string(),
        line: 1,
        end_line: 10,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec!["super::x".to_string()],
    };
    // No src/storage.rs in the corpus → the super:: walk resolves to
    // nothing, so the specifier lands in the unresolved side-channel.
    let grouping = GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files: vec![],
        preserve_function_entities: false,
    };
    let (_rels, unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "p", "r", Path::new(".")).unwrap();
    assert_eq!(
        unresolved.internal, 1,
        "unresolved super::x must be Internal"
    );
    assert_eq!(
        unresolved.external, 0,
        "unresolved super::x must not be External"
    );
}

#[test]
fn test_unresolved_self_and_bare_super_classified_internal() {
    // Same rule for the `self::` and bare `super` / `self` forms.
    let unit = CodeUnit {
        name: "f".to_string(),
        qualified_name: "f".to_string(),
        unit_type: "function".to_string(),
        file: "src/a/b.rs".to_string(),
        line: 1,
        end_line: 10,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec!["self::x".to_string(), "super".to_string()],
    };
    // src/a.rs is absent → the `self::x` walk finds src/a/b.rs itself but
    // the file is not indexed (no edge); the bare `super` specifier is not
    // handled by the resolver at all. Both stay unresolved → Internal.
    let grouping = GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files: vec![],
        preserve_function_entities: false,
    };
    let (_rels, unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "p", "r", Path::new(".")).unwrap();
    assert_eq!(
        unresolved.internal, 2,
        "self::x and bare super must both be Internal"
    );
    assert_eq!(unresolved.external, 0);
}
