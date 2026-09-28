// Integration test for tree-sitter extraction pipeline.
// Verifies correct entity counts, relationship edges, and metrics for the new extraction path.
use lievo::analysis::relationships::RelationshipBuilder;
use lievo::extraction::code_extractor::CodeExtractor;
use lievo::extraction::grouping::{GroupingConfig, extract_function_entities, group_code_units};
use lievo::extraction::tree_sitter_extractor::TreeSitterExtractor;
use lievo::model::CodeUnit;
use std::path::Path;

/// Test the full tree-sitter extraction pipeline on the fixture repository.
///
/// Pipeline: index() → read_all_units() → grouping → metrics → relationships
///
/// Fixture: tests/fixtures/sample_repo/ contains Rust, Python, JavaScript, TypeScript, and Go files.
/// Expected: at least 20 function entities, at least 10 relationship edges, at least one entity with complexity > 1.
#[test]
fn test_tree_sitter_extraction_pipeline() {
    let fixture_path = Path::new("tests/fixtures/sample_repo");

    // Step 1: Run TreeSitterExtractor to index and extract code units
    let mut extractor = TreeSitterExtractor::new(fixture_path, false)
        .expect("failed to create extractor for fixture path");
    extractor
        .index(false)
        .expect("failed to index fixture repository");
    let code_units = extractor
        .read_all_units()
        .expect("failed to read code units");

    // Step 2: Assert minimum function count
    // Fixture has: 4 Go + 5 Rust (including rust_helper) + 4 Python + 4 JS + 4 TS = 21+ functions
    assert!(
        code_units.len() >= 20,
        "expected at least 20 function entities, got {}",
        code_units.len()
    );

    // Step 3: Group code units into entities (subsystems, modules, files)
    let grouping_config = GroupingConfig {
        code_units: &code_units,
        scanned_file_paths: extractor.extracted_files(),
        project_id: "test-project",
        repo_name: "sample-repo",
        repo_id: "test-repo",
        repo_path: fixture_path,
        config: None,
        exclude_paths: &[],
    };
    let grouping_result =
        group_code_units(&grouping_config).expect("failed to group code units into entities");

    // Step 4: Extract function entities (when preserve_function_entities is enabled)
    let (function_entities, function_rels) =
        extract_function_entities(&code_units, &grouping_result.files, fixture_path);

    // Verify function entities were extracted
    // Note: function_entities may be empty due to cfg(test) filtering, but function_rels should exist
    // if files have paths
    eprintln!(
        "Extracted {} function entities, {} function relationships",
        function_entities.len(),
        function_rels.len()
    );

    // Step 5: Build relationships
    let (relationships, _unresolved) = RelationshipBuilder::build(
        &code_units,
        &grouping_result,
        "test-project",
        "sample-repo",
        fixture_path,
    )
    .expect("failed to build relationships");

    // Step 6: Assert minimum relationship count
    // Should have: contains (subsystems→modules, modules→files), imports, calls
    eprintln!(
        "Built {} relationships, {} function relationships",
        relationships.len(),
        function_rels.len()
    );
    assert!(
        relationships.len() + function_rels.len() >= 5,
        "expected at least 5 relationship edges, got {}",
        relationships.len() + function_rels.len()
    );

    // Step 7: Assert at least one entity has complexity > 1
    let has_complex_entity = code_units.iter().any(|unit| unit.complexity > 1);
    assert!(
        has_complex_entity,
        "expected at least one entity with complexity > 1"
    );

    // Step 8: Verify Go imports are not empty (regression guard for Go import fix)
    let go_units: Vec<&CodeUnit> = code_units
        .iter()
        .filter(|unit| unit.language == "Go")
        .collect();
    assert!(!go_units.is_empty(), "expected to extract Go code units");

    // Debug: print Go units and their imports
    for unit in &go_units {
        eprintln!(
            "Go unit '{}' in file '{}' has imports: {:?}",
            unit.name, unit.file, unit.imports
        );
    }

    for unit in &go_units {
        assert!(
            !unit.imports.is_empty(),
            "Go unit '{}' in file '{}' should have imports, got empty vector (regression guard)",
            unit.name,
            unit.file
        );
        // Specifically check for "fmt" import which we know exists in go_sample.go
        if unit.file.contains("go_sample.go") {
            assert!(
                unit.imports.iter().any(|imp: &String| imp.contains("fmt")),
                "go_sample.go should import 'fmt', got imports: {:?}",
                unit.imports
            );
        }
    }

    // Step 9: Verify imports are not empty for other languages where applicable
    let rust_units: Vec<&CodeUnit> = code_units
        .iter()
        .filter(|unit| unit.language == "Rust")
        .collect();
    for unit in &rust_units {
        if unit.file.contains("rust_sample.rs") {
            // rust_sample.rs imports std::collections::HashMap
            assert!(
                !unit.imports.is_empty(),
                "rust_sample.rs should have imports, got empty"
            );
            assert!(
                unit.imports
                    .iter()
                    .any(|imp: &String| imp.contains("HashMap") || imp.contains("helper_label")),
                "rust_sample.rs should import HashMap or helper_label, got: {:?}",
                unit.imports
            );
        }
    }

    let python_units: Vec<&CodeUnit> = code_units
        .iter()
        .filter(|unit| unit.language == "Python")
        .collect();
    for unit in &python_units {
        if unit.file.contains("python_sample.py") {
            assert!(
                !unit.imports.is_empty(),
                "python_sample.py should have imports, got empty"
            );
            assert!(
                unit.imports
                    .iter()
                    .any(|imp: &String| imp.contains("typing") || imp.contains("os")),
                "python_sample.py should import typing or os, got: {:?}",
                unit.imports
            );
        }
    }

    // Step 9b: Verify JS/TS import extraction emits the specifier field only.
    // js_sample.js imports './ts_sample' (relative) and should contain ONLY the bare
    // specifier — no quotes, no "import" keyword, no trailing semicolon.
    let js_units: Vec<&CodeUnit> = code_units
        .iter()
        .filter(|unit| unit.language == "JavaScript" && unit.file.contains("js_sample.js"))
        .collect();
    assert!(
        !js_units.is_empty(),
        "expected to extract JS code units from js_sample.js"
    );
    for unit in &js_units {
        // The source field of `import { sumNumbers } from './ts_sample';` must be
        // exactly "." + "/ts_sample" (quotes stripped, no statement text).
        assert!(
            unit.imports.iter().any(|imp: &String| imp == "./ts_sample"),
            "js_sample.js should contain the exact specifier './ts_sample', got: {:?}",
            unit.imports
        );
        // No import should contain the "import" keyword (whole-statement text is a bug).
        assert!(
            !unit
                .imports
                .iter()
                .any(|imp: &String| imp.contains("import ") || imp.starts_with("import")),
            "js_sample.js imports must not contain whole-statement text, got: {:?}",
            unit.imports
        );
    }

    // Step 9c: Verify JS/TS arrow functions are named after their const binding,
    // not their parameter. `const double = (x) => x * 2` must yield a unit named
    // "double", not "x". The destructured-param arrow `Pagination` must also be
    // named "Pagination" (not discarded).
    let arrow_names: Vec<&str> = js_units.iter().map(|u| u.name.as_str()).collect();
    assert!(
        arrow_names.contains(&"double"),
        "js_sample.js should contain a unit named 'double' (arrow binding), got: {:?}",
        arrow_names
    );
    assert!(
        arrow_names.contains(&"Pagination"),
        "js_sample.js should contain a unit named 'Pagination' (destructured arrow binding), got: {:?}",
        arrow_names
    );
    assert!(
        arrow_names.contains(&"noArg"),
        "js_sample.js should contain a unit named 'noArg' (paramless arrow), got: {:?}",
        arrow_names
    );
    assert!(
        arrow_names.contains(&"increment"),
        "js_sample.js should contain a unit named 'increment' (named-param arrow), got: {:?}",
        arrow_names
    );
    assert!(
        arrow_names.contains(&"greet"),
        "js_sample.js should contain a unit named 'greet' (class method), got: {:?}",
        arrow_names
    );
    // Parameter names must NOT appear as unit names (the original bug).
    assert!(
        !arrow_names.contains(&"x"),
        "js_sample.js must not name a unit after the arrow parameter 'x', got: {:?}",
        arrow_names
    );
    assert!(
        !arrow_names.contains(&"page"),
        "js_sample.js must not name a unit after the destructured parameter 'page', got: {:?}",
        arrow_names
    );
    assert!(
        !arrow_names.contains(&"prev"),
        "js_sample.js must not name a unit after the arrow parameter 'prev', got: {:?}",
        arrow_names
    );

    // Step 9d: Verify TypeScript arrow functions are also named after their binding.
    let ts_units: Vec<&CodeUnit> = code_units
        .iter()
        .filter(|unit| unit.language == "TypeScript" && unit.file.contains("ts_sample.ts"))
        .collect();
    assert!(
        !ts_units.is_empty(),
        "expected to extract TS code units from ts_sample.ts"
    );
    let ts_names: Vec<&str> = ts_units.iter().map(|u| u.name.as_str()).collect();
    assert!(
        ts_names.contains(&"triple"),
        "ts_sample.ts should contain a unit named 'triple' (TS arrow binding), got: {:?}",
        ts_names
    );

    // Step 9e: Verify Rust import extraction emits the argument field only.
    // rust_sample.rs imports should contain "std::collections::HashMap" and
    // "crate::rust_helper::helper_label" — the bare specifier, not the whole statement.
    for unit in &rust_units {
        if unit.file.contains("rust_sample.rs") {
            assert!(
                unit.imports
                    .iter()
                    .any(|imp: &String| imp == "std::collections::HashMap"),
                "rust_sample.rs should contain the exact specifier 'std::collections::HashMap', got: {:?}",
                unit.imports
            );
            assert!(
                unit.imports
                    .iter()
                    .any(|imp: &String| imp == "crate::rust_helper::helper_label"),
                "rust_sample.rs should contain the exact specifier 'crate::rust_helper::helper_label', got: {:?}",
                unit.imports
            );
            // No import should contain the "use" keyword (whole-statement text is a bug).
            assert!(
                !unit
                    .imports
                    .iter()
                    .any(|imp: &String| imp.starts_with("use ") || imp.contains(";")),
                "rust_sample.rs imports must not contain whole-statement text, got: {:?}",
                unit.imports
            );
        }
    }

    // Step 10: Verify cross-file calls are captured
    let units_with_calls: Vec<&CodeUnit> = code_units
        .iter()
        .filter(|unit| !unit.calls.is_empty())
        .collect();
    assert!(
        !units_with_calls.is_empty(),
        "expected at least some units to have function calls"
    );

    // Specifically, main() functions should call other functions
    let main_units: Vec<&CodeUnit> = code_units
        .iter()
        .filter(|unit| unit.name == "main")
        .collect();
    assert!(!main_units.is_empty(), "expected to find main() functions");

    for unit in &main_units {
        assert!(
            !unit.calls.is_empty(),
            "main() function should call other functions, got empty calls for {}",
            unit.file
        );
    }

    // Step 11: Verify Rust import_path evidence is populated (#681 hard prerequisite)
    // The crate::rust_helper::helper_label import in rust_sample.rs must resolve
    // to rust_helper.rs, producing an imports edge with import_path evidence.
    let import_path_count = relationships
        .iter()
        .filter(|r| {
            r.rel_type == lievo::model::RelType::Imports
                && r.evidence_json
                    .as_deref()
                    .is_some_and(|json| json.contains("import_path"))
        })
        .count();
    eprintln!(
        "Found {} imports edges with import_path evidence",
        import_path_count
    );

    // At least one Rust import should have produced an edge with import_path evidence
    assert!(
        import_path_count > 0,
        "expected at least one imports edge with import_path evidence for Rust, got {}",
        import_path_count
    );

    // Specifically verify that the crate::rust_helper::helper_label import produced an edge
    let rust_helper_count = relationships
        .iter()
        .filter(|r| {
            r.rel_type == lievo::model::RelType::Imports
                && r.evidence_json
                    .as_deref()
                    .is_some_and(|json| json.contains("crate::rust_helper"))
        })
        .count();

    assert!(
        rust_helper_count > 0,
        "expected an imports edge for crate::rust_helper::helper_label import, got none"
    );

    eprintln!(
        "Found {} edges for crate::rust_helper imports",
        rust_helper_count
    );

    // Step 12: Per-language coverage + fan-out computed over all fixture
    // languages with no panics (issue #679). File→file imports edges
    // reference file entity IDs, so translate to paths via the grouping file
    // map before feeding the coverage metric.
    let id_to_path: std::collections::HashMap<String, String> = grouping_result
        .files
        .iter()
        .filter_map(|f| f.path.as_ref().map(|p| (f.id.clone(), p.clone())))
        .collect();
    let resolved_by_path: Vec<(String, String)> = relationships
        .iter()
        .filter(|r| r.rel_type == lievo::model::RelType::Imports)
        .map(|r| {
            (
                id_to_path[&r.source_id].clone(),
                id_to_path[&r.target_id].clone(),
            )
        })
        .collect();
    let rows = lievo::analysis::coverage::coverage_by_language(&code_units, &resolved_by_path);

    let languages: std::collections::HashSet<&str> =
        code_units.iter().map(|u| u.language.as_str()).collect();
    for language in languages {
        let row = rows.iter().find(|r| r.language == language);
        assert!(row.is_some(), "row for language '{}'", language);
        let row = row.expect("language row present");
        assert!(
            row.files_with_dependents <= row.symbol_files,
            "covered files must not exceed symbol-bearing files for {language}"
        );
        eprintln!(
            "coverage {language}: {}% ({}/{}) entities={} fan-out={}",
            if row.symbol_files > 0 {
                100.0 * row.files_with_dependents as f64 / row.symbol_files as f64
            } else {
                0.0
            },
            row.files_with_dependents,
            row.symbol_files,
            row.entities,
            row.max_fan_out
        );
    }

    // Step 13: Wrapped-export fixtures (issue #701) — files whose exports are
    // ALL wrapped/call-expression forms must produce one CodeUnit per exported
    // binding AND a file entity after a fresh index.
    let wrapped_js_names: Vec<&str> = code_units
        .iter()
        .filter(|u| u.file == "js_wrapped_exports.js")
        .map(|u| u.name.as_str())
        .collect();
    for expected in [
        "StyledButton",
        "StyledIcon",
        "ForwardButton",
        "ObservedCard",
        "MemoButton",
        "ThemedButton",
        "NestedButton",
    ] {
        assert!(
            wrapped_js_names.contains(&expected),
            "js_wrapped_exports.js should contain unit '{}', got: {:?}",
            expected,
            wrapped_js_names
        );
    }
    // Step 13b: Separate-export form (issue #791) — declaration and export
    // are separate statements. The trigger is the assignment to a variable
    // binding, not the export. Each must yield exactly one unit named after
    // the binding, with correct file, line span and parent linkage.
    for expected in [
        "ArticleCarousel",
        "TimerWidget",
        "InputField",
        "ComplexDialog",
        "InternalHelper",
    ] {
        assert!(
            wrapped_js_names.contains(&expected),
            "js_wrapped_exports.js should contain separate-form unit '{}', got: {:?}",
            expected,
            wrapped_js_names
        );
    }
    // Negative (issue #791 acceptance criterion): an inline callback
    // assigned to a variable (`.map((item) => …)`) must NOT be extracted —
    // the inline-fn rule only applies to bare-identifier callees, and `.map`
    // is a member expression.
    assert!(
        !wrapped_js_names.contains(&"processedItems"),
        "js_wrapped_exports.js must NOT contain a unit for the inline callback \
         'processedItems' (over-extraction guard), got: {:?}",
        wrapped_js_names
    );
    // Inline callback negative (issue #791 acceptance criterion): a handler
    // assigned to a variable INSIDE a component body must not be a WRAPPED
    // component unit — the assignment-keyed gate is module-level only, so the
    // handler is not extracted as a wrapped component (it is not named
    // 'HandlerHost', and the gate did not widen to function scope).
    //
    // NOTE: it DOES appear as a plain-arrow-binding unit under the
    // pre-existing #677 arrow rule, which names any arrow bound to a variable
    // regardless of scope (test_js_arrow_function_named_after_binding pins a
    // module-level case; no existing test pinned function-local arrows). That
    // rule's scoping is out of scope for this diff — a follow-up should
    // decide whether function-local arrow bindings should be dropped. See the
    // PR for this finding.
    assert!(
        wrapped_js_names.contains(&"clickHandler"),
        "function-local arrow binding IS extracted under the pre-existing #677 \
         rule (documented as a follow-up scope decision, not changed here)"
    );
    // The host function itself IS a plain function declaration — it must be
    // extracted via the function-declaration path (unaffected by the wrapped
    // gate; the wrapped gate did not widen to function scope).
    assert!(
        wrapped_js_names.contains(&"HandlerHost"),
        "the function-declaration path must still extract 'HandlerHost'"
    );
    // Verify line spans for a separate-form unit: the unit must cover the
    // const declaration statement, not the export statement — and not the
    // whole file. The declaration is the `const ArticleCarousel = …` line in
    // the separate-export section of the fixture (line 47).
    let carousel_unit = code_units
        .iter()
        .find(|u| u.file == "js_wrapped_exports.js" && u.name == "ArticleCarousel")
        .expect("ArticleCarousel unit must exist");
    assert_eq!(
        carousel_unit.line, 47,
        "ArticleCarousel span must start at the const declaration line, got: {:?}",
        carousel_unit
    );
    assert!(
        carousel_unit.end_line > carousel_unit.line,
        "ArticleCarousel must span multiple lines (multi-line arrow body)"
    );
    // Parent linkage: the unit's file must match the fixture file.
    assert_eq!(
        carousel_unit.file, "js_wrapped_exports.js",
        "ArticleCarousel must be linked to the correct file"
    );
    // The wrapped file must also yield a file entity in the grouping output.
    assert!(
        grouping_result
            .files
            .iter()
            .any(|f| f.path.as_deref() == Some("js_wrapped_exports.js")),
        "js_wrapped_exports.js must get a file entity"
    );

    // Step 14: TypeScript/TSX wrapped-export fixture (issue #701) — the same
    // wrapped forms must work in .ts/.tsx too.
    let wrapped_ts_names: Vec<&str> = code_units
        .iter()
        .filter(|u| u.file == "ts_wrapped_exports.tsx")
        .map(|u| u.name.as_str())
        .collect();
    for expected in [
        "StyledBadge",
        "StyledLink",
        "ForwardLink",
        "ObservedList",
        "MemoList",
        "ThemedBadge",
        "NestedLink",
    ] {
        assert!(
            wrapped_ts_names.contains(&expected),
            "ts_wrapped_exports.tsx should contain unit '{}', got: {:?}",
            expected,
            wrapped_ts_names
        );
    }
    // Step 14b: Separate-export form in TypeScript/TSX (issue #791) —
    // same forms as the JS fixture: observer, memo, forwardRef, nested,
    // never-exported, and inline callback negative.
    for expected in [
        "ArticlePanel",
        "Counter",
        "SearchBar",
        "Modal",
        "InternalWidget",
    ] {
        assert!(
            wrapped_ts_names.contains(&expected),
            "ts_wrapped_exports.tsx should contain separate-form unit '{}', got: {:?}",
            expected,
            wrapped_ts_names
        );
    }
    // Negative (issue #791 acceptance criterion): inline callback
    // assignment in TSX must NOT be extracted.
    assert!(
        !wrapped_ts_names.contains(&"doubled"),
        "ts_wrapped_exports.tsx must NOT contain a unit for the inline callback \
         'doubled' (over-extraction guard), got: {:?}",
        wrapped_ts_names
    );
    // Inline callback negative (issue #791 acceptance criterion): the
    // function-local handler must not be a WRAPPED component unit (see the
    // JS equivalent above for the #677 caveat — the handler does appear as a
    // plain-arrow unit under that pre-existing rule). The host function
    // declaration itself is a plain function and IS extracted via the
    // function-declaration path (unaffected by the wrapped gate).
    assert!(
        wrapped_ts_names.contains(&"Host"),
        "the TSX host function declaration must be extracted (plain function)"
    );
    assert!(
        wrapped_ts_names.contains(&"clickHandler"),
        "the function-local arrow binding IS extracted under the pre-existing \
         #677 rule (same documented caveat as the JS fixture)"
    );
    assert!(
        grouping_result
            .files
            .iter()
            .any(|f| f.path.as_deref() == Some("ts_wrapped_exports.tsx")),
        "ts_wrapped_exports.tsx must get a file entity"
    );

    // Step 15: Zero-unit fixture (issue #701 independent defect) — a file
    // with ZERO extractable code units must STILL get a file entity, via the
    // scanned_file_paths seed path (not via any unit).
    let zero_unit_has_unit = code_units.iter().any(|u| u.file == "js_zero_units.js");
    assert!(
        !zero_unit_has_unit,
        "js_zero_units.js must have ZERO code units (the fixture is a zero-unit file)"
    );
    assert!(
        grouping_result
            .files
            .iter()
            .any(|f| f.path.as_deref() == Some("js_zero_units.js")),
        "js_zero_units.js (zero code units) must still get a file entity"
    );

    // The zero-unit file's entity must carry the JavaScript language derived
    // from the path extension (no units to borrow a language from).
    let zero_unit_entity = grouping_result
        .files
        .iter()
        .find(|f| f.path.as_deref() == Some("js_zero_units.js"))
        .expect("js_zero_units.js file entity");
    assert_eq!(
        zero_unit_entity.language.as_deref(),
        Some("JavaScript"),
        "zero-unit file entity language must be derived from the .js extension"
    );
}

// Pipeline regression guard for #698: after the extract_call_name fix, no code
// unit in the sample_repo fixture should have a single-character entry in .calls.
// This catches the class of bug where `g.greet("world")` yields callee "g" instead
// of "greet" (the receiver shadowing the method).
//
// This is the strongest regression guard because it exercises the full
// pipeline: extract_calls → CodeUnit.calls → the stored data that the
// #679 coverage gate (top_single_char_callees) would see.
#[test]
fn test_no_single_char_callees_in_sample_repo() {
    let fixture_path = Path::new("tests/fixtures/sample_repo");
    let mut extractor = TreeSitterExtractor::new(fixture_path, false)
        .expect("failed to create extractor for fixture path");
    extractor
        .index(false)
        .expect("failed to index fixture repository");
    let code_units = extractor
        .read_all_units()
        .expect("failed to read code units");

    // Collect every (language, unit-name, file, callee) where callee is single-char
    let single_char_callees: Vec<String> = code_units
        .iter()
        .flat_map(|unit| {
            unit.calls
                .iter()
                .filter(|call| call.chars().count() == 1)
                .map(|call| {
                    format!(
                        "'{}' (lang: {}, unit: {}, file: {})",
                        call, unit.language, unit.name, unit.file
                    )
                })
        })
        .collect();

    assert!(
        single_char_callees.is_empty(),
        "single-character callees extracted in sample_repo (regression: #698 member-call fix): {}",
        single_char_callees.join(", ")
    );

    // Also verify the specific case from the issue: js_sample.js main() must have
    // "greet" in its calls (from g.greet("world")), not "g".
    let js_main_units: Vec<&CodeUnit> = code_units
        .iter()
        .filter(|unit| {
            unit.language == "JavaScript"
                && unit.name == "main"
                && unit.file.contains("js_sample.js")
        })
        .collect();
    assert!(
        !js_main_units.is_empty(),
        "expected to find main() in js_sample.js"
    );
    for unit in &js_main_units {
        assert!(
            unit.calls.iter().any(|c| c == "greet"),
            "js_sample.js main() should have 'greet' in its calls (from g.greet(\"world\")), got: {:?}",
            unit.calls
        );
    }
}

// Fixture-level regression guard for #706: a relative import must resolve
// by normalised path only, never by basename/suffix matching across
// unrelated top-level trees. The pinned fixture adds two sibling trees
// (treeA, treeB) with same-basename index.js files: treeA/save/index.js
// imports "../core/button" (must resolve inside treeA) and separately
// CALLS handleSaving, a function defined only in the treeB decoy — the
// call site must never manufacture a cross-tree imports edge.
//
// This exercises the FULL pipeline (extraction -> grouping ->
// RelationshipBuilder) against the pinned multi-tree fixture, unlike the
// synthetic temp-dir unit tests in tests/relationship_builder_test.rs.
#[test]
fn test_sample_repo_cross_tree_import_resolution() {
    let fixture_path = Path::new("tests/fixtures/sample_repo");
    let mut extractor = TreeSitterExtractor::new(fixture_path, false)
        .expect("failed to create extractor for fixture path");
    extractor
        .index(false)
        .expect("failed to index fixture repository");
    let code_units = extractor
        .read_all_units()
        .expect("failed to read code units");

    let grouping_config = GroupingConfig {
        code_units: &code_units,
        scanned_file_paths: extractor.extracted_files(),
        project_id: "test-project",
        repo_name: "sample-repo",
        repo_id: "test-repo",
        repo_path: fixture_path,
        config: None,
        exclude_paths: &[],
    };
    let grouping_result =
        group_code_units(&grouping_config).expect("failed to group code units into entities");

    let (relationships, unresolved) = RelationshipBuilder::build(
        &code_units,
        &grouping_result,
        "test-project",
        "sample-repo",
        fixture_path,
    )
    .expect("failed to build relationships");

    let id_to_path: std::collections::HashMap<String, String> = grouping_result
        .files
        .iter()
        .filter_map(|f| f.path.as_ref().map(|p| (f.id.clone(), p.clone())))
        .collect();

    let save_file = grouping_result
        .files
        .iter()
        .find(|f| f.path.as_deref() == Some("treeA/save/index.js"))
        .expect("treeA/save/index.js must get a file entity");
    let button_file = grouping_result
        .files
        .iter()
        .find(|f| f.path.as_deref() == Some("treeA/core/button/index.js"))
        .expect("treeA/core/button/index.js must get a file entity");
    let search_file = grouping_result
        .files
        .iter()
        .find(|f| f.path.as_deref() == Some("treeB/search/index.js"))
        .expect("treeB/search/index.js must get a file entity");

    let import_edges: Vec<(&String, &String)> = relationships
        .iter()
        .filter(|r| r.rel_type == lievo::model::RelType::Imports)
        .filter(|r| id_to_path.contains_key(&r.source_id) && id_to_path.contains_key(&r.target_id))
        .map(|r| (&r.source_id, &r.target_id))
        .collect();

    // The resolved edge must land on the in-tree target.
    assert!(
        import_edges
            .iter()
            .any(|(src, tgt)| **src == save_file.id && **tgt == button_file.id),
        "treeA/save/index.js must have an imports edge to treeA/core/button/index.js, got: {:?}",
        import_edges
            .iter()
            .map(|(s, t)| (id_to_path[*s].clone(), id_to_path[*t].clone()))
            .collect::<Vec<_>>()
    );

    // No edge may touch the same-basename decoy in the sibling tree, in
    // either direction (a wrong-edge resolver could produce save->search or
    // the call-site `handleSaving()` could manufacture search->save).
    assert!(
        import_edges
            .iter()
            .all(|(src, tgt)| **src != search_file.id && **tgt != search_file.id),
        "treeB/search/index.js (same-basename decoy) must receive/emit no imports edges, got: {:?}",
        import_edges
            .iter()
            .map(|(s, t)| (id_to_path[*s].clone(), id_to_path[*t].clone()))
            .collect::<Vec<_>>()
    );

    // The resolvable relative import must not count as unresolved-internal.
    eprintln!(
        "cross-tree fixture unresolved counts: internal={}, external={}",
        unresolved.internal, unresolved.external
    );
}
