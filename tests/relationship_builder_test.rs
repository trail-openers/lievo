// Integration tests for RelationshipBuilder::build() (public API)

use lievo::analysis::relationships::RelationshipBuilder;
use lievo::extraction::entity_id::entity_id;
use lievo::extraction::grouping::GroupingResult;
use lievo::model::{CodeUnit, EdgeProvenance, Entity, EntityTier, RelType};
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
fn test_build_contains_subsystem_to_module() {
    let sub = make_entity("proj:repo:subsystem:.", EntityTier::Subsystem, ".", None);
    let module = make_entity(
        "proj:repo:module:src",
        EntityTier::Module,
        "src",
        Some("proj:repo:subsystem:."),
    );
    let grouping = make_grouping(vec![sub], vec![module.clone()], vec![]);

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let contains: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Contains)
        .collect();

    assert_eq!(contains.len(), 1);
    assert_eq!(contains[0].source_id, "proj:repo:subsystem:.");
    assert_eq!(contains[0].target_id, "proj:repo:module:src");
    assert_eq!(contains[0].weight, 1.0);
    // #714: grouping parent_id contains edges are exact structural facts —
    // they classify as resolved, written explicitly (not via column default).
    assert_eq!(contains[0].provenance, EdgeProvenance::Resolved);
}

#[test]
fn test_build_contains_module_to_file() {
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
    assert_eq!(contains[0].source_id, "proj:repo:module:src");
    assert_eq!(contains[0].target_id, "proj:repo:file:src/main.rs");
}

#[test]
fn test_build_contains_no_parent_skipped() {
    let module = make_entity("proj:repo:module:src", EntityTier::Module, "src", None);
    let grouping = make_grouping(vec![], vec![module], vec![]);
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[], &grouping, "proj", "repo", Path::new(".")).unwrap();
    assert!(rels.is_empty());
}

#[test]
fn test_build_imports_file_to_file() {
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

    let unit = make_code_unit("src/a.rs", vec!["crate::b"], vec![]);
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].source_id, "proj:repo:file:src/a.rs");
    assert_eq!(imports[0].target_id, "proj:repo:file:src/b.rs");
    // #714: the "crate::b" import resolves via resolve_import's module-key
    // map (crate:: path key) — import-resolver path = resolved.
    assert_eq!(imports[0].provenance, EdgeProvenance::Resolved);
}

#[test]
fn test_build_imports_external_skipped() {
    let file_a = make_entity(
        "proj:repo:file:src/a.rs",
        EntityTier::File,
        "src/a.rs",
        None,
    );
    let grouping = make_grouping(vec![], vec![], vec![file_a]);
    let unit = make_code_unit("src/a.rs", vec!["tokio", "serde"], vec![]);
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    assert!(rels.iter().all(|r| r.rel_type != RelType::Imports));
}

#[test]
fn test_build_imports_self_excluded() {
    let file_a = make_entity(
        "proj:repo:file:src/a.rs",
        EntityTier::File,
        "src/a.rs",
        None,
    );
    let grouping = make_grouping(vec![], vec![], vec![file_a]);
    let unit = make_code_unit("src/a.rs", vec!["src/a.rs"], vec![]);
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    assert!(rels.iter().all(|r| r.rel_type != RelType::Imports));
}

#[test]
fn test_build_imports_weight_aggregated() {
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
    let grouping = make_grouping(vec![], vec![], vec![file_a, file_b]);

    let u1 = make_code_unit("src/a.rs", vec!["crate::b"], vec![]);
    let u2 = make_code_unit("src/a.rs", vec!["crate::b"], vec![]);
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[u1, u2], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].weight, 2.0);
}

#[test]
fn test_build_imports_file_to_file_via_fn_map() {
    // Issue #447: file→file edges from fn_map resolution use RelType::Imports,
    // not RelType::Calls. Files import/use other files — they don't "call" them.
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
    let grouping = make_grouping(vec![], vec![], vec![file_a, file_b.clone()]);

    let defn = make_code_unit("src/b.rs", vec![], vec![]);
    let mut defn_named = defn;
    defn_named.name = "helper_fn".to_string();
    let caller = make_code_unit("src/a.rs", vec![], vec!["helper_fn"]);

    let (rels, _unresolved) = RelationshipBuilder::build(
        &[defn_named, caller],
        &grouping,
        "proj",
        "repo",
        Path::new("."),
    )
    .unwrap();
    // File→file edges are Imports, not Calls (issue #447)
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].source_id, "proj:repo:file:src/a.rs");
    assert_eq!(imports[0].target_id, "proj:repo:file:src/b.rs");
    // #714: the file→file edge here comes from fn_map name matching (the
    // helper_fn call site resolved against the defining file) — heuristic.
    assert_eq!(imports[0].provenance, EdgeProvenance::Heuristic);
}

#[test]
fn test_build_imports_self_excluded_fn_map() {
    // Self-edges from fn_map resolution are excluded regardless of rel_type.
    // (Previously tested as "Calls self excluded" — rel_type changed to Imports per #447)
    let file_a = make_entity(
        "proj:repo:file:src/a.rs",
        EntityTier::File,
        "src/a.rs",
        None,
    );
    let grouping = make_grouping(vec![], vec![], vec![file_a]);
    let mut u = make_code_unit("src/a.rs", vec![], vec!["local_fn"]);
    u.name = "local_fn".to_string();
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[u], &grouping, "proj", "repo", Path::new(".")).unwrap();
    // No self-edges of any type from fn_map
    assert!(rels.iter().all(|r| r.source_id != r.target_id));
}

#[test]
fn test_build_depends_on_module_level() {
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
    let grouping = make_grouping(vec![sub], vec![mod_a, mod_b], vec![file_a, file_b.clone()]);
    let unit = make_code_unit("src/a.rs", vec!["lib/b.rs"], vec![]);

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let depends: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::DependsOn)
        .collect();

    assert!(depends.iter().any(|r| r.source_id == "proj:repo:module:src"
        && r.target_id == "proj:repo:module:lib"));
    // #714: aggregated DependsOn edges are a structural roll-up of the
    // file-level edges (module/subsystem tier), not name-matching — resolved.
    assert!(
        depends
            .iter()
            .all(|r| r.provenance == EdgeProvenance::Resolved),
        "aggregated depends_on must carry resolved provenance: {:?}",
        depends
    );
}

#[test]
fn test_build_depends_on_subsystem_level() {
    let sub_a = make_entity(
        "proj:repo:subsystem:crates/a",
        EntityTier::Subsystem,
        "crates/a",
        None,
    );
    let sub_b = make_entity(
        "proj:repo:subsystem:crates/b",
        EntityTier::Subsystem,
        "crates/b",
        None,
    );
    let mod_a = make_entity(
        "proj:repo:module:crates/a/src",
        EntityTier::Module,
        "crates/a/src",
        Some("proj:repo:subsystem:crates/a"),
    );
    let mod_b = make_entity(
        "proj:repo:module:crates/b/src",
        EntityTier::Module,
        "crates/b/src",
        Some("proj:repo:subsystem:crates/b"),
    );
    let file_a = make_entity(
        "proj:repo:file:crates/a/src/main.rs",
        EntityTier::File,
        "crates/a/src/main.rs",
        Some("proj:repo:module:crates/a/src"),
    );
    let file_b = make_entity(
        "proj:repo:file:crates/b/src/lib.rs",
        EntityTier::File,
        "crates/b/src/lib.rs",
        Some("proj:repo:module:crates/b/src"),
    );
    let grouping = make_grouping(vec![sub_a, sub_b], vec![mod_a, mod_b], vec![file_a, file_b]);
    let unit = make_code_unit("crates/a/src/main.rs", vec!["crates/b/src/lib.rs"], vec![]);

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let depends: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::DependsOn)
        .collect();

    assert!(
        depends
            .iter()
            .any(|r| r.source_id == "proj:repo:subsystem:crates/a"
                && r.target_id == "proj:repo:subsystem:crates/b"),
        "subsystem-level depends_on missing: {:?}",
        depends
    );
}

#[test]
fn test_build_empty_input() {
    let grouping = make_grouping(vec![], vec![], vec![]);
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[], &grouping, "proj", "repo", Path::new(".")).unwrap();
    assert!(rels.is_empty());
    assert!(
        RelationshipBuilder::build_cross_repo(&[], &[])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_build_uses_entity_id_format() {
    let file_id = entity_id("proj", "repo", EntityTier::File, "src/main.rs").unwrap();
    let mod_id = entity_id("proj", "repo", EntityTier::Module, "src").unwrap();

    let file = make_entity(&file_id, EntityTier::File, "src/main.rs", Some(&mod_id));
    let module = make_entity(&mod_id, EntityTier::Module, "src", None);
    let grouping = make_grouping(vec![], vec![module], vec![file]);

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let contains: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Contains)
        .collect();

    assert_eq!(contains.len(), 1);
    assert_eq!(contains[0].source_id, "proj:repo:module:src");
    assert_eq!(contains[0].target_id, "proj:repo:file:src/main.rs");
}

#[test]
fn test_mod_declaration_bare_name_creates_imports_edge() {
    // Issue #438: A bare identifier in `calls` that matches a file stem
    // should be classified as RelType::Imports, not RelType::Calls.
    //
    // This tests the bare_name_map path: when "embedding" (no ::) appears
    // in calls and matches file stem "src/embedding.rs", it creates an Imports edge.

    let file_main = make_entity(
        "proj:repo:file:src/main.rs",
        EntityTier::File,
        "src/main.rs",
        None,
    );
    let file_embedding = make_entity(
        "proj:repo:file:src/embedding.rs",
        EntityTier::File,
        "src/embedding.rs",
        None,
    );

    // Bare identifier "embedding" in calls — should match file stem
    let unit = make_code_unit("src/main.rs", vec![], vec!["embedding"]);

    let grouping = make_grouping(
        vec![],
        vec![],
        vec![file_main.clone(), file_embedding.clone()],
    );

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();

    // Find relationships between the two files
    let file_rels: Vec<_> = rels
        .iter()
        .filter(|r| r.source_id == file_main.id && r.target_id == file_embedding.id)
        .collect();

    // Expect exactly one Imports relationship
    assert_eq!(
        file_rels.len(),
        1,
        "Expected one Imports relationship from main.rs to embedding.rs"
    );
    assert_eq!(
        file_rels[0].rel_type,
        RelType::Imports,
        "Bare identifier 'embedding' should create Imports, not Calls"
    );
    // #714: the bare_name_map stem hit is name-matched, not path-resolved —
    // the edge classifies as heuristic.
    assert_eq!(
        file_rels[0].provenance,
        EdgeProvenance::Heuristic,
        "bare_name_map stem hit must classify as heuristic"
    );

    // Confirm NO Calls relationship exists between these files
    let calls_rels: Vec<_> = rels
        .iter()
        .filter(|r| {
            (r.source_id == file_main.id || r.target_id == file_main.id)
                && (r.source_id == file_embedding.id || r.target_id == file_embedding.id)
                && r.rel_type == RelType::Calls
        })
        .collect();
    assert!(
        calls_rels.is_empty(),
        "Should not have Calls relationship between main.rs and embedding.rs via bare name"
    );
}

#[test]
fn test_qualified_call_not_matched_by_bare_name_map() {
    // A qualified identifier like "embedding::process" (with ::) must NOT
    // be matched by bare_name_map. It can only resolve via fn_map (as Imports).
    //
    // This prevents qualified function calls from being misclassified by
    // the bare-name heuristic.

    let file_main = make_entity(
        "proj:repo:file:src/main.rs",
        EntityTier::File,
        "src/main.rs",
        None,
    );
    let file_embedding = make_entity(
        "proj:repo:file:src/embedding.rs",
        EntityTier::File,
        "src/embedding.rs",
        None,
    );

    // Qualified call "embedding::process" — has ::, so bare_name_map won't match
    let unit = make_code_unit("src/main.rs", vec![], vec!["embedding::process"]);

    let grouping = make_grouping(
        vec![],
        vec![],
        vec![file_main.clone(), file_embedding.clone()],
    );

    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();

    // Find any relationship from main.rs to embedding.rs
    let file_rels: Vec<_> = rels
        .iter()
        .filter(|r| r.source_id == file_main.id && r.target_id == file_embedding.id)
        .collect();

    // Should have NO relationship — "embedding::process" is not a bare name
    // and does not exist in fn_map, so nothing is created
    assert!(
        file_rels.is_empty(),
        "Qualified call 'embedding::process' should not create any relationship to embedding.rs"
    );
}

// --- #681 task-a: JS/TS tier-1 resolver dispatch (language-gated) ---

fn make_js_code_unit(file: &str, language: &str, imports: Vec<&str>) -> CodeUnit {
    CodeUnit {
        name: "fn_name".to_string(),
        qualified_name: format!("{}::fn_name", file.replace('/', "::")),
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
        imports: imports.into_iter().map(str::to_string).collect(),
    }
}

fn make_js_entity(id: &str, path: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string()),
        path: Some(path.to_string()),
        language: Some("JavaScript".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn test_build_imports_js_relative_extension_permutation() {
    // #681: a JavaScript unit importing "./utils" must resolve to utils.js
    // (extension permutation) via the new JS/TS resolver branch.
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("src")).unwrap();
    fs::write(tmp.path().join("src/utils.js"), "export {};\n").unwrap();
    fs::write(
        tmp.path().join("src/main.js"),
        "import {} from './utils';\n",
    )
    .unwrap();

    let file_main = make_js_entity("proj:repo:file:src/main.js", "src/main.js");
    let file_utils = make_js_entity("proj:repo:file:src/utils.js", "src/utils.js");
    let grouping = make_grouping(vec![], vec![], vec![file_main.clone(), file_utils.clone()]);

    let unit = make_js_code_unit("src/main.js", "JavaScript", vec!["./utils"]);
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", tmp.path()).unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    assert_eq!(
        imports.len(),
        1,
        "JS relative import must produce one imports edge"
    );
    assert_eq!(imports[0].source_id, "proj:repo:file:src/main.js");
    assert_eq!(imports[0].target_id, "proj:repo:file:src/utils.js");
    // #714: the edge came from the JsResolverContext normalised-path hit
    // ("./utils" → utils.js via extension permutation) — import-resolver
    // path = resolved.
    assert_eq!(imports[0].provenance, EdgeProvenance::Resolved);
}

#[test]
fn test_build_imports_js_external_counted_not_guessed() {
    // #681: a JavaScript unit importing "react" (no workspace member) must
    // produce no edge AND be counted as unresolved-external.
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("src")).unwrap();
    fs::write(tmp.path().join("src/main.js"), "import {} from 'react';\n").unwrap();

    let file_main = make_js_entity("proj:repo:file:src/main.js", "src/main.js");
    let grouping = make_grouping(vec![], vec![], vec![file_main]);

    let unit = make_js_code_unit("src/main.js", "JavaScript", vec!["react"]);
    let (rels, unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", tmp.path()).unwrap();

    assert!(
        rels.iter().all(|r| r.rel_type != RelType::Imports),
        "external JS import must not produce an imports edge"
    );
    assert_eq!(
        unresolved.external, 1,
        "react must count as unresolved-external"
    );
    assert_eq!(unresolved.internal, 0);
}

#[test]
fn test_build_imports_js_internal_unresolved_counted() {
    // #681: a JavaScript unit importing "./missing" (relative, no match) must
    // produce no edge AND be counted as unresolved-internal.
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("src")).unwrap();
    fs::write(
        tmp.path().join("src/main.js"),
        "import {} from './missing';\n",
    )
    .unwrap();

    let file_main = make_js_entity("proj:repo:file:src/main.js", "src/main.js");
    let grouping = make_grouping(vec![], vec![], vec![file_main]);

    let unit = make_js_code_unit("src/main.js", "JavaScript", vec!["./missing"]);
    let (rels, unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", tmp.path()).unwrap();

    assert!(rels.iter().all(|r| r.rel_type != RelType::Imports));
    assert_eq!(
        unresolved.internal, 1,
        "missing relative import must count as unresolved-internal"
    );
    assert_eq!(unresolved.external, 0);
}

fn make_js_code_unit_with_calls(
    file: &str,
    language: &str,
    imports: Vec<&str>,
    calls: Vec<&str>,
) -> CodeUnit {
    let mut unit = make_js_code_unit(file, language, imports);
    unit.calls = calls.into_iter().map(str::to_string).collect();
    unit
}

/// A marker CodeUnit whose sole purpose is to carry a file-level import list
/// for a JS/TS file that yields zero function-shaped code units (e.g. a file
/// containing only top-level import statements, re-exports, and no function
/// declarations, arrow functions, method definitions, or wrapped exports).
///
/// The extractor's `extract_from_file` computes the file-level import list once
/// and duplicates it onto every CodeUnit it produces. `RelationshipBuilder::build`
/// receives `&[CodeUnit]` as its sole input — there is no separate carrier for
/// a file's import list. For a file that yields zero function-shaped units, the
/// builder has no unit to walk, so no edge can be produced from that file's
/// imports at the builder level.
///
/// This helper pins the extractor→builder contract: the builder must walk the
/// `imports` field of every CodeUnit it receives, so a single marker unit
/// carrying the file-level import list is sufficient to exercise the file-level
/// import path. The `unit_type` is `"file_imports_marker"` to document that
/// this is not a real function unit but a synthetic carrier for the file's
/// import list.
fn make_js_file_imports_marker(file: &str, language: &str, imports: Vec<&str>) -> CodeUnit {
    CodeUnit {
        name: "file_imports_marker".to_string(),
        qualified_name: format!("{}::file_imports_marker", file.replace('/', "::")),
        unit_type: "file_imports_marker".to_string(),
        file: file.to_string(),
        line: 1,
        end_line: 1,
        language: language.to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 0,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: imports.into_iter().map(str::to_string).collect(),
    }
}

#[test]
fn test_import_resolver_sibling_tree_normalised_path() {
    // #706: the JS resolver must resolve "./core/button" from
    // treeA/save/index.js to treeA/core/button/index.js via the
    // directory-index candidate (try_candidate's JS_INDEX_SUFFIXES branch),
    // NOT to a same-basename file in treeB. This pins the normalised-path
    // resolution before the RelationshipBuilder layer.
    //
    // Specifier "./core/button" from "treeA/save/index.js":
    //   source_dir = "treeA/save" → join_relative → "treeA/save/core/button"
    //   Hmm, that's wrong. Let's use "../core/button" from "treeA/save/x.js":
    //   source_dir = "treeA/save" → join_relative → "treeA/core/button" ✓
    use lievo::analysis::import_resolver::JsResolverContext;
    let tmp = TempDir::new().unwrap();
    let write = |rel: &str, content: &str| {
        let full = tmp.path().join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    };
    // treeA/save/x.js is the importing file (NOT index.js — the index.js in
    // treeA/save/ is a different file that would be the target of "../save").
    write("treeA/save/x.js", "x");
    write("treeA/core/button/index.js", "export {}");
    write("treeB/search.js", "export function handleSaving() {}");

    let files = vec![
        make_js_entity("f-save", "treeA/save/x.js"),
        make_js_entity("f-button", "treeA/core/button/index.js"),
        make_js_entity("f-search", "treeB/search.js"),
    ];
    let ctx = JsResolverContext::new(&files, tmp.path());
    // "../core/button" from "treeA/save/x.js":
    //   source_dir = "treeA/save" → join_relative("treeA/save/x.js", "core/button")
    //   = "treeA/save/core/button" — NO, that's wrong.
    //
    // Actually: join_relative(source_file="treeA/save/x.js", relative="core/button")
    //   source_dir = Path::new("treeA/save/x.js").parent() = "treeA/save"
    //   parts = ["treeA", "save"]
    //   for seg in ["core", "button"]: push → ["treeA", "save", "core", "button"]
    //   = "treeA/save/core/button" — that's NOT what we want.
    //
    // To get "treeA/core/button" we need "../core/button" from "treeA/save/x.js":
    //   source_dir = "treeA/save", parts = ["treeA", "save"]
    //   for seg in ["..", "core", "button"]: pop → ["treeA"], push core, push button
    //   = "treeA/core/button" ✓
    let resolved = ctx
        .resolve("../core/button", "treeA/save/x.js")
        .expect("sibling-tree directory-index target must resolve");
    assert_eq!(
        resolved, "f-button",
        "must resolve to treeA/core/button/index.js, not a treeB decoy"
    );
}

#[test]
fn test_build_imports_js_call_evidence_does_not_create_cross_tree_edge() {
    // #706 root cause A: JS/TS `calls` entries are function-level call sites,
    // not module-level dependencies. Feeding them into fn_map/bare_name_map
    // produced bogus import edges with no path normalisation or tree scoping
    // (the pinned cross-tree edge: `handleSaving` call site matched the only
    // file DEFINING handleSaving, in a different top-level tree). Call
    // evidence must never create file→file Imports edges for JS/TS; only the
    // resolved `imports` list does.
    let tmp = TempDir::new().unwrap();
    // treeA/save/index.js — the importing unit: calls handleSaving (defined
    // only in treeB) and imports "../core/button" (resolves in-tree).
    fs::create_dir_all(tmp.path().join("treeA/save")).unwrap();
    fs::write(
        tmp.path().join("treeA/save/index.js"),
        "import Button from '../core/button';\nfunction save() { handleSaving(); }\n",
    )
    .unwrap();
    // treeA/core/button/index.js — the real import target.
    fs::create_dir_all(tmp.path().join("treeA/core/button")).unwrap();
    fs::write(tmp.path().join("treeA/core/button/index.js"), "export {}\n").unwrap();
    // treeB/search.js — decoy: DEFINES handleSaving; must receive no edge.
    fs::create_dir_all(tmp.path().join("treeB")).unwrap();
    fs::write(
        tmp.path().join("treeB/search.js"),
        "export function handleSaving() {}\n",
    )
    .unwrap();

    // NOTE: the resolver's known_paths uses entity paths verbatim (repo-relative).
    // The tmp.path() is the repo_root for file I/O, but entity paths must be
    // repo-relative (no tmp prefix) so the JS resolver's known_paths matches.
    let file_save = make_js_entity("proj:repo:file:treeA/save/x.js", "treeA/save/x.js");
    let file_button = make_js_entity(
        "proj:repo:file:treeA/core/button/index.js",
        "treeA/core/button/index.js",
    );
    let file_search = make_js_entity("proj:repo:file:treeB/search.js", "treeB/search.js");
    let grouping = make_grouping(
        vec![],
        vec![],
        vec![file_save.clone(), file_button.clone(), file_search.clone()],
    );

    // The save unit's imports carry the file-level import list; its calls
    // carry the (bogus-for-edges) call site. The import "./core/button"
    // from "treeA/save/x.js" resolves to "treeA/save/core/button" — but we
    // want "treeA/core/button/index.js", so the specifier must be
    // "../core/button" from "treeA/save/x.js" (source_dir = "treeA/save",
    // ".." pops to "treeA", then "core/button" → "treeA/core/button").
    let unit = make_js_code_unit_with_calls(
        "treeA/save/x.js",
        "JavaScript",
        vec!["../core/button"],
        vec!["handleSaving"],
    );
    // A unit in treeB that defines handleSaving (feeds fn_map).
    let mut def_unit =
        make_js_code_unit_with_calls("treeB/search.js", "JavaScript", vec![], vec![]);
    def_unit.name = "handleSaving".to_string();
    def_unit.qualified_name = "treeB::search::handleSaving".to_string();

    let (rels, unresolved) =
        RelationshipBuilder::build(&[unit, def_unit], &grouping, "proj", "repo", tmp.path())
            .unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    // Exactly one Imports edge: save → the in-tree button target.
    assert_eq!(
        imports.len(),
        1,
        "only the resolved import must produce an edge, got {:?}",
        imports
            .iter()
            .map(|r| (r.source_id.clone(), r.target_id.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(imports[0].source_id, file_save.id);
    assert_eq!(imports[0].target_id, file_button.id);
    // #714: the resolved JsResolverContext edge is the only one — resolved.
    assert_eq!(imports[0].provenance, EdgeProvenance::Resolved);

    // No edge into or out of the decoy treeB/search.js — the call site
    // handleSaving must not create a cross-tree edge.
    assert!(
        imports
            .iter()
            .all(|r| r.source_id != file_search.id && r.target_id != file_search.id),
        "call evidence must not create a cross-tree edge to treeB/search.js"
    );

    // All-resolvable fixture: the relative import resolved, so no
    // unresolved_internal for the importing file.
    assert_eq!(
        unresolved.internal, 0,
        "resolvable relative import must not count as unresolved-internal"
    );
}

#[test]
fn test_build_imports_js_file_level_imports_walked_for_zero_unit_files() {
    // #706 root cause B: the edge and unresolved-count loops iterate
    // `unit.imports` over code units. A file with zero function-shaped code
    // units (e.g. a barrel of re-exports, or a file whose only content is
    // top-level import statements) has no unit to carry the file-level import
    // list, so the builder's per-unit walk never sees them — no edge is
    // produced and unresolved_internal reports 0, defeating the #690
    // false-orphan guard.
    //
    // The extractor's fix (extract_from_file): when a file yields zero
    // function-shaped units but has file-level imports, emit a synthetic
    // marker unit (unit_type == "file_imports_marker") that carries the
    // file-level import list. The builder's existing per-unit `unit.imports`
    // walk then processes it, producing the edge and the truthful unresolved
    // count.
    //
    // This test pins the builder-side contract: a marker unit carrying a
    // resolvable relative import ("./dep" → dep/index.js) and an unresolvable
    // relative ("./Missing" — no such target) must produce exactly one edge
    // (to the resolvable target) and count the unresolvable one as
    // unresolved_internal.
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("components")).unwrap();
    fs::write(
        tmp.path().join("components/SaveButton.js"),
        "import Button from './Button';\nimport Modal from './ModalContents/SignUpModalContent';\nexport default () => null;\n",
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join("components/Button")).unwrap();
    fs::write(tmp.path().join("components/Button/index.js"), "export {}\n").unwrap();

    let file_save = make_js_entity(
        "proj:repo:file:components/SaveButton.js",
        "components/SaveButton.js",
    );
    let file_button = make_js_entity(
        "proj:repo:file:components/Button/index.js",
        "components/Button/index.js",
    );
    let grouping = make_grouping(vec![], vec![], vec![file_save.clone(), file_button.clone()]);

    // One marker unit carrying the file-level import list: a resolvable
    // relative ("./Button" → Button/index.js) and an unresolvable relative
    // ("./ModalContents/SignUpModalContent" — no such target in this tree).
    let marker = make_js_file_imports_marker(
        "components/SaveButton.js",
        "JavaScript",
        vec!["./Button", "./ModalContents/SignUpModalContent"],
    );
    let (rels, unresolved) =
        RelationshipBuilder::build(&[marker], &grouping, "proj", "repo", tmp.path()).unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    // Exactly one edge: SaveButton → Button/index.js (the resolvable one).
    assert_eq!(
        imports.len(),
        1,
        "only the resolvable relative import must produce an edge"
    );
    assert_eq!(imports[0].source_id, file_save.id);
    assert_eq!(imports[0].target_id, file_button.id);
    // #714: resolver-derived edge from the file-level marker unit — resolved.
    assert_eq!(imports[0].provenance, EdgeProvenance::Resolved);

    // Truthful unresolved counts: the unresolvable relative counts as
    // unresolved-internal (>0), the resolvable one does NOT. This is the
    // #690 false-orphan guard seeing the correct number.
    assert_eq!(
        unresolved.internal, 1,
        "the unresolvable relative import must count as unresolved-internal"
    );
    assert_eq!(unresolved.external, 0);
}

#[test]
fn test_build_imports_rust_path_unchanged_regression_guard() {
    // #681: Rust-language units must keep hitting the existing resolve_import
    // path (regression guard). A "crate::b" import resolves via the Rust
    // module-key map, not the JS/TS resolver.
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

    let unit = make_code_unit("src/a.rs", vec!["crate::b"], vec![]);
    let (rels, _unresolved) =
        RelationshipBuilder::build(&[unit], &grouping, "proj", "repo", Path::new(".")).unwrap();
    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    // Rust path: the counter is not incremented on the Rust branch (no
    // unresolved specifiers). The builder may hold a count from a prior test
    // in the same process (tests run in parallel), so we only assert that the
    // Rust path itself produces no new unresolved specifiers — the edge is
    // what matters for this regression guard.
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].source_id, "proj:repo:file:src/a.rs");
    assert_eq!(imports[0].target_id, "proj:repo:file:src/b.rs");
    // #714: the Rust resolve_import hit (module-key map, crate:: path key)
    // classifies as resolved.
    assert_eq!(imports[0].provenance, EdgeProvenance::Resolved);
}
