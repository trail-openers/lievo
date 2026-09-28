use crate::analysis::relationship_helpers::resolve_import_for;
use crate::analysis::relationships::RelationshipBuilder;
use crate::extraction::grouping::GroupingResult;
use crate::model::{CodeUnit, Entity, EntityTier, RelType};
use std::path::Path;

/// Test helper: a CodeUnit with imports (the language field is overridable
/// per-test where the test's point is the language gate itself).
fn make_code_unit_with_imports(
    name: &str,
    file: &str,
    language: &str,
    imports: Vec<String>,
) -> CodeUnit {
    CodeUnit {
        name: name.to_string(),
        qualified_name: name.to_string(),
        unit_type: "function".to_string(),
        file: file.to_string(),
        line: 1,
        end_line: 10,
        language: language.to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports,
    }
}

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

/// Test helper: create a CodeUnit with a specific unit_type.
fn make_code_unit_with_type(
    name: &str,
    file: &str,
    unit_type: &str,
    calls: Vec<String>,
) -> CodeUnit {
    let mut unit = make_code_unit(name, file, calls);
    unit.unit_type = unit_type.to_string();
    unit
}

/// Test helper: create a minimal File entity.
fn make_file(id: &str, name: &str, path: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: name.to_string(),
        path: Some(path.to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

/// Test helper: create a minimal Function entity.
fn make_function(id: &str, file_id: &str, name: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: Some(file_id.to_string()),
        name: name.to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

/// Test helper: create a minimal GroupingResult.
fn make_grouping(files: Vec<Entity>) -> GroupingResult {
    GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files,
        preserve_function_entities: false,
    }
}

/// Regression test for issue #522: test function calls attributed to production functions
///
/// Before the fix, every CodeUnit's calls were attributed to ALL functions in the file.
/// This meant if a test function called `validate`, the validation call was attributed
/// to the production `validate` function as well.
///
/// After the fix, each CodeUnit's calls are only attributed to the function with
/// the matching name in the same file, based on the CodeUnit.name field.
#[test]
fn test_issue_522_production_calls_not_from_test() {
    let code_units = vec![
        make_code_unit(
            "validate_products",
            "src/validation.rs",
            vec!["helper_fn".to_string()],
        ),
        make_code_unit(
            "test_validate_products",
            "src/validation.rs",
            vec!["helper_fn".to_string(), "mock_data".to_string()],
        ),
    ];

    let file = make_file("file-validation", "validation.rs", "src/validation.rs");
    let file_utils = make_file("file-utils", "utils.rs", "src/utils.rs");

    let fn_validate_products = make_function(
        "fn-validate-products",
        "file-validation",
        "validate_products",
    );
    let fn_test_validate_products = make_function(
        "fn-test-validate-products",
        "file-validation",
        "test_validate_products",
    );
    let fn_helper = make_function("fn-helper", "file-utils", "helper_fn");
    let fn_mock_data = make_function("fn-mock-data", "file-utils", "mock_data");

    let grouping = make_grouping(vec![file, file_utils]);

    let (rels, _unresolved) = RelationshipBuilder::build_with_functions(
        &code_units,
        &grouping,
        "p",
        "r",
        Path::new(".").canonicalize().unwrap().as_path(),
        Some(&[
            fn_validate_products,
            fn_test_validate_products,
            fn_helper,
            fn_mock_data,
        ]),
    )
    .unwrap();

    // Verify that production function only calls helper_fn (not mock_data)
    let validate_products_calls: Vec<_> = rels
        .iter()
        .filter(|r| r.source_id == "fn-validate-products" && r.rel_type == RelType::Calls)
        .map(|r| r.target_id.as_str())
        .collect();

    assert!(
        validate_products_calls.contains(&"fn-helper"),
        "Production function should call helper_fn"
    );
    assert!(
        !validate_products_calls.contains(&"fn-mock-data"),
        "Issue #522: test function calls were attributed to production functions - \
             Production function should NOT call mock_data (test-only call)"
    );

    // Verify that test function calls both helper_fn and mock_data
    let test_validate_products_calls: Vec<_> = rels
        .iter()
        .filter(|r| r.source_id == "fn-test-validate-products" && r.rel_type == RelType::Calls)
        .map(|r| r.target_id.as_str())
        .collect();

    assert!(
        test_validate_products_calls.contains(&"fn-helper"),
        "Test function should call helper_fn"
    );
    assert!(
        test_validate_products_calls.contains(&"fn-mock-data"),
        "Test function should call mock_data"
    );
}

/// End-to-end test for ambiguous CALLS edge creation.
///
/// Verifies that when a function name is ambiguous (defined in multiple files),
/// CALLS edges are created to ALL matching candidates rather than silently
/// dropping the call.
#[test]
fn test_ambiguous_calls_creates_edges_to_all_candidates() {
    let file_a = make_file("file-a", "auth.rs", "src/auth.rs");
    let file_b = make_file("file-b", "user.rs", "src/user.rs");
    let file_c = make_file("file-c", "main.rs", "src/main.rs");

    let fn_search_a = make_function("fn-search-a", "file-a", "search");
    let fn_search_b = make_function("fn-search-b", "file-b", "search");
    let fn_caller = make_function("fn-caller", "file-c", "main");

    let code_unit = make_code_unit("main", "src/main.rs", vec!["search".to_string()]);

    let grouping = make_grouping(vec![file_a, file_b, file_c]);

    let (rels, _unresolved) = RelationshipBuilder::build_with_functions(
        &[code_unit],
        &grouping,
        "p",
        "r",
        Path::new(".").canonicalize().unwrap().as_path(),
        Some(&[fn_search_a, fn_search_b, fn_caller]),
    )
    .unwrap();

    let caller_calls: Vec<_> = rels
        .iter()
        .filter(|r| r.source_id == "fn-caller" && r.rel_type == RelType::Calls)
        .collect();

    assert!(
        caller_calls.iter().any(|r| r.target_id == "fn-search-a"),
        "CALLS edge should exist to search function in file-a"
    );
    assert!(
        caller_calls.iter().any(|r| r.target_id == "fn-search-b"),
        "CALLS edge should exist to search function in file-b"
    );
    assert_eq!(
        caller_calls.len(),
        2,
        "Exactly 2 CALLS edges should exist for ambiguous function name"
    );
}

/// Test for warning message when CodeUnit name doesn't match any function entity.
///
/// Verifies that when a CodeUnit has a name that doesn't correspond to any
/// function entity, the relationship building completes gracefully without errors
/// and no CALLS edges are created for the unmatched unit. The warning eprintln!
/// is emitted but execution continues.
#[test]
fn test_unmatched_code_unit_name_skipped_gracefully() {
    let file = make_file("file-1", "main.rs", "src/main.rs");
    let fn_real = make_function("fn-real", "file-1", "real_function");
    let code_unit = make_code_unit(
        "nonexistent_function",
        "src/main.rs",
        vec!["other_function".to_string()],
    );

    let grouping = make_grouping(vec![file]);

    let (rels, _unresolved) = RelationshipBuilder::build_with_functions(
        &[code_unit],
        &grouping,
        "p",
        "r",
        Path::new(".").canonicalize().unwrap().as_path(),
        Some(&[fn_real]),
    )
    .expect("Relationship building should not fail for unmatched CodeUnit name");

    let has_any_calls = rels.iter().any(|r| r.rel_type == RelType::Calls);
    assert!(
        !has_any_calls,
        "No CALLS edges should exist when CodeUnit name doesn't match any function entity"
    );
}

/// Regression test for issue #528: non-function code units are silently skipped.
///
/// Before the fix, ALL CodeUnits (including raw_code_N, struct, constants, markdown, YAML, TOML)
/// were iterated and queried against fn_name_lookup, flooding stderr with warnings like:
/// - "call graph: no function entity matched unit name 'Cargo' in file 'Cargo.toml'"
/// - "call graph: no function entity matched unit name 'MemoryStore' in file 'src/memory/store.rs'"
///
/// After the fix, non-function code units are filtered out before any processing,
/// so no warnings are emitted and no CALLS edges are created for them.
#[test]
fn test_issue_528_non_function_code_units_silently_skipped() {
    let file = make_file("file-1", "store.rs", "src/memory/store.rs");
    let fn_real = make_function("fn-real", "file-1", "real_function");

    let raw_code_unit = make_code_unit_with_type(
        "raw_code_1",
        "src/memory/store.rs",
        "raw_code",
        vec!["some_call".to_string()],
    );
    let struct_unit = make_code_unit_with_type(
        "MemoryStore",
        "src/memory/store.rs",
        "struct",
        vec!["another_call".to_string()],
    );
    let constant_unit = make_code_unit_with_type(
        "MAX_SIZE",
        "src/memory/store.rs",
        "constant",
        vec!["yet_another_call".to_string()],
    );
    let function_unit = make_code_unit(
        "real_function",
        "src/memory/store.rs",
        vec!["helper".to_string()],
    );

    let grouping = make_grouping(vec![file]);

    let (rels, _unresolved) = RelationshipBuilder::build_with_functions(
        &[raw_code_unit, struct_unit, constant_unit, function_unit],
        &grouping,
        "p",
        "r",
        Path::new(".").canonicalize().unwrap().as_path(),
        Some(&[fn_real]),
    )
    .unwrap();

    let calls_from_function: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Calls && r.source_id == "fn-real")
        .collect();

    assert_eq!(
        calls_from_function.len(),
        0,
        "Function unit should not create CALLS edges when target doesn't exist"
    );
}

/// Test that function units continue to be processed correctly after the #528 fix.
///
/// Verifies that the filter for non-function units doesn't accidentally skip
/// actual function-like units (function, method, closure, etc.).
#[test]
fn test_issue_528_function_units_still_processed() {
    let file = make_file("file-1", "main.rs", "src/main.rs");
    let fn_caller = make_function("fn-caller", "file-1", "caller");
    let fn_helper = make_function("fn-helper", "file-1", "helper");

    let function_unit = make_code_unit_with_type(
        "caller",
        "src/main.rs",
        "function",
        vec!["helper".to_string()],
    );
    let method_unit = make_code_unit_with_type(
        "caller",
        "src/main.rs",
        "method",
        vec!["helper".to_string()],
    );
    let closure_unit = make_code_unit_with_type(
        "caller",
        "src/main.rs",
        "closure",
        vec!["helper".to_string()],
    );

    for unit in [function_unit, method_unit, closure_unit] {
        let unit_type = unit.unit_type.clone();
        let grouping = make_grouping(vec![file.clone()]);

        let (rels, _unresolved) = RelationshipBuilder::build_with_functions(
            &[unit],
            &grouping,
            "p",
            "r",
            Path::new(".").canonicalize().unwrap().as_path(),
            Some(&[fn_caller.clone(), fn_helper.clone()]),
        )
        .unwrap();

        let has_call = rels.iter().any(|r| {
            r.source_id == "fn-caller" && r.target_id == "fn-helper" && r.rel_type == RelType::Calls
        });
        assert!(
            has_call,
            "Function-like unit_type '{}' should create CALLS edges",
            unit_type
        );
    }
}

// ── #742: language gate at the call site ──────────────────────────────

#[test]
fn test_non_rust_super_shaped_specifier_does_not_resolve_via_walk() {
    // A non-Rust unit carrying a `super::`-shaped specifier must NOT get an
    // edge through the relative walk — the call site passes the module
    // context only for Rust units (spec #742: the gate lives at the call
    // site, not inside the resolver).
    let unit = make_code_unit_with_imports(
        "f",
        "src/storage/util.py",
        "Python",
        vec!["super::thing".to_string()],
    );
    let target = make_file("file-parent", "storage.rs", "src/storage.rs");
    let grouping = GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files: vec![target],
        preserve_function_entities: false,
    };
    let (rels, unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "p", "r", Path::new(".")).unwrap();
    assert!(
        !rels
            .iter()
            .any(|r| r.rel_type == RelType::Imports && r.source_id == "file-parent"),
        "a non-Rust unit's super::-shaped specifier must not resolve via the walk"
    );
    // The unresolved-kind side-channel still classifies it Internal (the
    // specifier is a relative internal one that resolved to nothing).
    assert_eq!(unresolved.internal, 1);
    assert_eq!(unresolved.external, 0);
}

#[test]
fn test_rust_super_shaped_specifier_resolves_via_walk() {
    // Contrast: the SAME specifier in a Rust unit DOES resolve to the parent
    // module file (the walk is Rust-only, not disabled).
    //
    // A function-like CodeUnit's import edges are emitted in the function
    // tier (section 6, which needs Function entities), so this test pins the
    // file-level seam directly: the same `resolve_import_for` call the
    // call-site gate feeds — Rust units get the module context (Some), so
    // the super:: walk must resolve the specifier to the parent module file.
    let source = make_file("file-util", "util.rs", "src/storage/util.rs");
    let target = make_file("file-parent", "storage.rs", "src/storage.rs");
    let import_map =
        crate::analysis::relationship_helpers::build_import_map(&[source, target], "p", "r");
    assert_eq!(
        resolve_import_for("super::thing", &import_map, Some("src/storage/util.rs")),
        Some("file-parent"),
        "a Rust unit's super:: specifier must resolve to the parent module file"
    );
}
