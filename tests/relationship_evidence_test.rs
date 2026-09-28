// Integration tests for evidence_json fields in RelationshipBuilder output.
// evidence_json for imports/calls is a JSON array of evidence entries.
// evidence_json for contains is a single JSON object.

use lievo::analysis::relationships::RelationshipBuilder;
use lievo::extraction::grouping::GroupingResult;
use lievo::model::{CodeUnit, Entity, EntityTier, RelType};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn make_entity(id: &str, tier: EntityTier, path: &str, parent_id: Option<&str>) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier,
        parent_id: parent_id.map(str::to_string),
        name: path.split('/').next_back().unwrap_or(path).to_string(),
        path: Some(path.to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn make_code_unit(file: &str, imports: Vec<&str>, calls: Vec<&str>) -> CodeUnit {
    CodeUnit {
        name: "fn_name".to_string(),
        qualified_name: format!("{}::fn_name", file.replace('/', "::")),
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
        calls: calls.into_iter().map(str::to_string).collect(),
        imports: imports.into_iter().map(str::to_string).collect(),
    }
}

fn make_grouping(
    subsystems: Vec<Entity>,
    modules: Vec<Entity>,
    files: Vec<Entity>,
) -> GroupingResult {
    GroupingResult {
        subsystems,
        modules,
        files,
        preserve_function_entities: false,
    }
}

#[test]
fn test_imports_evidence_json_populated() {
    let file_a = make_entity(
        "proj:repo:file:src/a.rs",
        EntityTier::File,
        "src/a.rs",
        None,
    );
    let file_b = make_entity(
        "proj:repo:file:src/b.rs",
        EntityTier::File,
        "src/b.rs",
        None,
    );
    let grouping = make_grouping(vec![], vec![], vec![file_a.clone(), file_b.clone()]);

    let mut unit = make_code_unit("src/a.rs", vec!["crate::b"], vec![]);
    unit.line = 5;
    unit.end_line = 20;
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    assert_eq!(imports.len(), 1);
    let ev = imports[0]
        .evidence_json
        .as_deref()
        .expect("imports evidence_json must be Some");
    let parsed: serde_json::Value = serde_json::from_str(ev).unwrap();
    // evidence_json is a JSON array; check the first entry
    assert_eq!(parsed[0]["source_file"], "src/a.rs");
    assert_eq!(parsed[0]["source_line"], 5);
    assert_eq!(parsed[0]["source_end_line"], 20);
    assert_eq!(parsed[0]["import_path"], "crate::b");
}

#[test]
fn test_import_path_evidence_rust_argument_field() {
    // Pin the extractor→resolver contract end-to-end: the Rust `argument` field
    // output ("crate::b::helper_fn") must resolve to file b and emit an
    // import_path evidence entry. Without the field-based extraction fix this
    // would be 0 — the whole-statement text "use crate::b::helper_fn;" never
    // matches the import map.
    let file_a = make_entity(
        "proj:repo:file:src/a.rs",
        EntityTier::File,
        "src/a.rs",
        None,
    );
    let file_b = make_entity(
        "proj:repo:file:src/b.rs",
        EntityTier::File,
        "src/b.rs",
        None,
    );
    let grouping = make_grouping(vec![], vec![], vec![file_a.clone(), file_b.clone()]);

    // Simulates the fixed extractor output: the argument field of
    // `use crate::b::helper_fn;` is "crate::b::helper_fn".
    let mut unit = make_code_unit("src/a.rs", vec!["crate::b::helper_fn"], vec![]);
    unit.line = 3;
    unit.end_line = 3;

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    assert_eq!(
        imports.len(),
        1,
        "Rust argument-field import must produce an imports edge"
    );
    let ev = imports[0]
        .evidence_json
        .as_deref()
        .expect("imports evidence_json must be Some");
    let parsed: serde_json::Value = serde_json::from_str(ev).unwrap();
    let import_entries: Vec<serde_json::Value> =
        serde_json::from_value(parsed).expect("evidence must be a JSON array");
    assert!(
        import_entries
            .iter()
            .any(|e| e.get("import_path").is_some()),
        "must have at least one import_path evidence entry, got: {:?}",
        import_entries
    );
    assert!(
        import_entries
            .iter()
            .any(|e| e["import_path"] == "crate::b::helper_fn"),
        "import_path evidence must carry the argument-field specifier, got: {:?}",
        import_entries
    );
}

#[test]
fn test_import_path_evidence_js_source_field() {
    // Pin the extractor→resolver contract for JS: the `source` field output
    // ("./other") is emitted verbatim. Under the tier-1 JS/TS resolver (#681)
    // this specifier resolves relative to src/a.js → src/other.* — absent in
    // this fixture — so no edge is produced and the specifier is counted as
    // unresolved-internal. The pre-#681 assertion (edge via raw import-map
    // key "./other") is superseded by the language-gated JS/TS branch; the
    // post-#681 contract is covered by test_import_path_evidence_js_relative_resolved.
    let file_a = make_entity(
        "proj:repo:file:src/a.js",
        EntityTier::File,
        "src/a.js",
        None,
    );
    let file_b = make_entity("proj:repo:file:./other", EntityTier::File, "./other", None);
    let grouping = make_grouping(vec![], vec![], vec![file_a.clone(), file_b.clone()]);

    // Simulates the fixed extractor output: the source field of
    // `import x from "./other"` is "./other" (quotes stripped).
    let unit = CodeUnit {
        name: "fn_name".to_string(),
        qualified_name: "src::a::fn_name".to_string(),
        unit_type: "function".to_string(),
        file: "src/a.js".to_string(),
        line: 1,
        end_line: 10,
        language: "JavaScript".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec!["./other".to_string()],
    };

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    assert_eq!(
        imports.len(),
        0,
        "With the tier-1 JS/TS resolver (#681), './other' from src/a.js resolves to src/other.* — absent in this fixture — so no edge; counted unresolved-internal"
    );
}

#[test]
fn test_calls_evidence_json_populated() {
    // Issue #447: file→file edges from fn_map use RelType::Imports at file tier.
    // Evidence still records the callee_name for traceability.
    let file_a = make_entity(
        "proj:repo:file:src/a.rs",
        EntityTier::File,
        "src/a.rs",
        None,
    );
    let file_b = make_entity(
        "proj:repo:file:src/b.rs",
        EntityTier::File,
        "src/b.rs",
        None,
    );
    let grouping = make_grouping(vec![], vec![], vec![file_a.clone(), file_b.clone()]);

    let defn = CodeUnit {
        name: "helper_fn".to_string(),
        qualified_name: "src::b::helper_fn".to_string(),
        unit_type: "function".to_string(),
        file: "src/b.rs".to_string(),
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
        imports: vec![],
    };
    let mut caller = make_code_unit("src/a.rs", vec![], vec!["helper_fn"]);
    caller.line = 15;
    caller.end_line = 30;

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[defn, caller], &grouping, "proj", "repo", Path::new("."))
            .unwrap();
    // File→file edges from fn_map are now Imports (issue #447)
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    assert_eq!(imports.len(), 1);
    let ev = imports[0]
        .evidence_json
        .as_deref()
        .expect("imports evidence_json must be Some");
    let parsed: serde_json::Value = serde_json::from_str(ev).unwrap();
    // evidence_json is a JSON array; check the first entry
    assert_eq!(parsed[0]["source_file"], "src/a.rs");
    assert_eq!(parsed[0]["caller_line"], 15);
    assert_eq!(parsed[0]["callee_name"], "helper_fn");
}

#[test]
fn test_contains_evidence_json_populated() {
    let module = make_entity("proj:repo:module:src", EntityTier::Module, "src", None);
    let file = make_entity(
        "proj:repo:file:src/main.rs",
        EntityTier::File,
        "src/main.rs",
        Some("proj:repo:module:src"),
    );
    let grouping = make_grouping(vec![], vec![module], vec![file]);

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let contains: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Contains)
        .collect();

    assert_eq!(contains.len(), 1);
    let ev = contains[0]
        .evidence_json
        .as_deref()
        .expect("contains evidence_json must be Some");
    let parsed: serde_json::Value = serde_json::from_str(ev).unwrap();
    // contains evidence_json is a single JSON object (not an array)
    assert_eq!(parsed["child_path"], "src/main.rs");
}

#[test]
fn test_depends_on_evidence_json_is_none() {
    let sub = make_entity("proj:repo:subsystem:.", EntityTier::Subsystem, ".", None);
    let mod_a = make_entity(
        "proj:repo:module:src",
        EntityTier::Module,
        "src",
        Some("proj:repo:subsystem:."),
    );
    let mod_b = make_entity(
        "proj:repo:module:lib",
        EntityTier::Module,
        "lib",
        Some("proj:repo:subsystem:."),
    );
    let file_a = make_entity(
        "proj:repo:file:src/a.rs",
        EntityTier::File,
        "src/a.rs",
        Some("proj:repo:module:src"),
    );
    let file_b = make_entity(
        "proj:repo:file:lib/b.rs",
        EntityTier::File,
        "lib/b.rs",
        Some("proj:repo:module:lib"),
    );
    let grouping = make_grouping(vec![sub], vec![mod_a, mod_b], vec![file_a, file_b]);
    let unit = make_code_unit("src/a.rs", vec!["lib/b.rs"], vec![]);

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let depends: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::DependsOn)
        .collect();

    assert!(!depends.is_empty(), "must have at least one depends_on");
    for dep in &depends {
        assert!(
            dep.evidence_json.is_none(),
            "depends_on evidence_json must be None, got {:?}",
            dep.evidence_json
        );
    }
}

// --- #681 task-a: import_path evidence through the new JS/TS resolver ---

#[test]
fn test_import_path_evidence_js_relative_resolved() {
    // #681: after the tier-1 JS/TS resolver is wired in, a resolved JS
    // relative import must produce an imports edge whose evidence_json
    // carries the "import_path" key with the specifier — the
    // "import_path evidence count > 0" acceptance criterion for JS.
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("src")).unwrap();
    fs::write(
        tmp.path().join("src/main.js"),
        "import {} from './utils';\n",
    )
    .unwrap();
    fs::write(tmp.path().join("src/utils.js"), "export {};\n").unwrap();

    let file_a = Entity {
        id: "proj:repo:file:src/main.js".to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "main.js".to_string(),
        path: Some("src/main.js".to_string()),
        language: Some("JavaScript".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let file_b = Entity {
        id: "proj:repo:file:src/utils.js".to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "utils.js".to_string(),
        path: Some("src/utils.js".to_string()),
        language: Some("JavaScript".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let grouping = make_grouping(vec![], vec![], vec![file_a, file_b]);

    let unit = CodeUnit {
        name: "fn_name".to_string(),
        qualified_name: "src::main::fn_name".to_string(),
        unit_type: "function".to_string(),
        file: "src/main.js".to_string(),
        line: 2,
        end_line: 2,
        language: "JavaScript".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec!["./utils".to_string()],
    };

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", tmp.path()).unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    assert_eq!(
        imports.len(),
        1,
        "resolved JS import must produce one imports edge"
    );
    let ev = imports[0]
        .evidence_json
        .as_deref()
        .expect("imports evidence_json must be Some");
    let parsed: serde_json::Value = serde_json::from_str(ev).unwrap();
    let import_entries: Vec<serde_json::Value> =
        serde_json::from_value(parsed).expect("evidence must be a JSON array");
    assert!(
        import_entries
            .iter()
            .any(|e| e.get("import_path").is_some() && e["import_path"] == "./utils"),
        "import_path evidence must carry the resolved JS specifier, got: {:?}",
        import_entries
    );
}

#[test]
fn test_import_path_evidence_js_unresolved_no_edge() {
    // #681: an unresolvable JS import ("fs" — Node built-in, no workspace
    // match) must NOT produce an edge and must increment the unresolved
    // external counter (the "counted, not guessed" contract).
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("src")).unwrap();
    fs::write(tmp.path().join("src/main.js"), "import {} from 'fs';\n").unwrap();

    let file_a = Entity {
        id: "proj:repo:file:src/main.js".to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "main.js".to_string(),
        path: Some("src/main.js".to_string()),
        language: Some("JavaScript".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let grouping = make_grouping(vec![], vec![], vec![file_a]);

    let unit = CodeUnit {
        name: "fn_name".to_string(),
        qualified_name: "src::main::fn_name".to_string(),
        unit_type: "function".to_string(),
        file: "src/main.js".to_string(),
        line: 1,
        end_line: 1,
        language: "JavaScript".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec!["fs".to_string()],
    };

    let (rels, unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", tmp.path()).unwrap();

    assert!(
        rels.iter().all(|r| r.rel_type != RelType::Imports),
        "unresolvable JS import must not produce an edge"
    );
    assert_eq!(
        unresolved.external, 1,
        "'fs' must count as unresolved-external"
    );
}
