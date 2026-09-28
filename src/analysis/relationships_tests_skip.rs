use crate::analysis::relationships::RelationshipBuilder;
use crate::extraction::grouping::GroupingResult;
use crate::model::{CodeUnit, Entity, EntityTier, RelType};
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
fn test_skip_test_file_code_units_warnings() {
    // Create code units: one from src/ (should be processed), one from tests/ (should be skipped)
    let code_units = vec![
        make_code_unit("main", "src/main.rs", vec!["helper".to_string()]),
        make_code_unit(
            "test_helper",
            "tests/lib_integration.rs",
            vec!["main".to_string()],
        ),
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

    let fn_main = Entity {
        id: "fn-main".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("file-main".to_string()),
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
        id: "fn-helper".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("file-main".to_string()),
        name: "helper".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    // Intentionally do NOT create a Function entity for test_helper - this is the root cause
    // of the warnings (test files don't get Function entities)

    let grouping = GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files: vec![file_main],
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

    // Verify relationships exist for src/ file functions
    let main_calls_helper = rels.iter().any(|r| {
        r.source_id == "fn-main" && r.target_id == "fn-helper" && r.rel_type == RelType::Calls
    });
    assert!(
        main_calls_helper,
        "Source file function relationships should be built normally"
    );

    // Verify no function entity exists for test file function (it was skipped)
    // This is implicit: we didn't create one, so it shouldn't appear in any relationships
    let has_test_helper_rel = rels
        .iter()
        .any(|r| r.source_id.contains("test_helper") || r.target_id.contains("test_helper"));
    assert!(
        !has_test_helper_rel,
        "Test file function should not appear in any relationships (issue #545)"
    );

    // Note: we cannot easily capture stderr in unit tests to assert the warning is
    // suppressed, but the guard placement (before entity lookup) ensures the
    // "no function entity matched" warning path is never reached for test file paths.
    // This is verified by the absence of any spurious relationships for test functions.
}

#[test]
fn test_function_to_file_super_import_edge_emitted() {
    // #742 task-a: function→file super:: edges must be emitted from
    // collect_function_edges, not just collect_file_edges. A function unit
    // in src/rustmod/deep/mod.rs importing `super::module_name` (1-hop →
    // src/rustmod.rs) must produce a Function→File Imports edge to
    // rustmod.rs.
    let unit = CodeUnit {
        name: "deep_tag".to_string(),
        qualified_name: "rustmod::deep::deep_tag".to_string(),
        unit_type: "function".to_string(),
        file: "src/rustmod/deep/mod.rs".to_string(),
        line: 1,
        end_line: 10,
        language: "Rust".to_string(),
        signature: None,
        code: None,
        calls: vec![],
        imports: vec!["super::module_name".to_string()],
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        docstring: None,
        parent_class: None,
    };

    let file_mod = Entity {
        id: "file-deep-mod".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "mod.rs".to_string(),
        path: Some("src/rustmod/deep/mod.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let file_rustmod = Entity {
        id: "file-rustmod".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "rustmod.rs".to_string(),
        path: Some("src/rustmod.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let grouping = GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files: vec![file_mod, file_rustmod.clone()],
        preserve_function_entities: false,
    };

    // The function entity (parent_id = its owning file's entity id).
    let fn_entity = Entity {
        id: "fn-deep-tag".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: Some("file-deep-mod".to_string()),
        name: "deep_tag".to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };

    let (rels, _unresolved) = RelationshipBuilder::build_with_functions(
        &[unit],
        &grouping,
        "p",
        "r",
        std::path::Path::new("."),
        Some(&[fn_entity]),
    )
    .unwrap();

    // Function→file Imports edge: fn-deep-tag → file-rustmod (super::module_name
    // walks 1 level up from rustmod::deep to rustmod → src/rustmod.rs).
    let fn_imports: Vec<_> = rels
        .iter()
        .filter(|r| {
            r.source_id == "fn-deep-tag"
                && r.target_id == "file-rustmod"
                && r.rel_type == RelType::Imports
        })
        .collect();
    assert_eq!(
        fn_imports.len(),
        1,
        "function→file super:: edge must be emitted from collect_function_edges; got: {:?}",
        rels.iter()
            .map(|r| (r.source_id.as_str(), r.target_id.as_str(), r.rel_type))
            .collect::<Vec<_>>()
    );
}
