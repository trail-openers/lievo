// Integration tests for the grouping module — tests the public group_code_units() API

use lievo::extraction::grouping::{GroupingConfig, GroupingResult, group_code_units};
use lievo::model::CodeUnit;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn make_code_unit(file: &str, language: &str) -> CodeUnit {
    CodeUnit {
        name: "test".to_string(),
        qualified_name: "test::test".to_string(),
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
        imports: vec![],
    }
}

fn sorted_ids(result: &GroupingResult) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut s: Vec<String> = result.subsystems.iter().map(|e| e.id.clone()).collect();
    let mut m: Vec<String> = result.modules.iter().map(|e| e.id.clone()).collect();
    let mut f: Vec<String> = result.files.iter().map(|e| e.id.clone()).collect();
    s.sort_unstable();
    m.sort_unstable();
    f.sort_unstable();
    (s, m, f)
}

fn call_group_code_units(
    units: &[CodeUnit],
    project_id: &str,
    repo_name: &str,
    repo_id: &str,
    repo_path: &Path,
) -> lievo::error::Result<GroupingResult> {
    let config = GroupingConfig {
        code_units: units,
        scanned_file_paths: &[],
        project_id,
        repo_name,
        repo_id,
        repo_path,
        config: None,
        exclude_paths: &[],
    };
    group_code_units(&config)
}

fn call_group_code_units_with_scanned(
    units: &[CodeUnit],
    scanned_file_paths: &[String],
    project_id: &str,
    repo_name: &str,
    repo_id: &str,
    repo_path: &Path,
) -> lievo::error::Result<GroupingResult> {
    let config = GroupingConfig {
        code_units: units,
        scanned_file_paths,
        project_id,
        repo_name,
        repo_id,
        repo_path,
        config: None,
        exclude_paths: &[],
    };
    group_code_units(&config)
}

#[test]
fn test_group_code_units_zero_unit_file_still_gets_file_entity() {
    // issue #701 (independent defect): a file with ZERO code units must still
    // get a file entity via the scanned_file_paths seed path. The other files
    // have units; the zero-unit one does not.
    let temp = TempDir::new().unwrap();
    let units = vec![
        make_code_unit("src/main.rs", "Rust"),
        make_code_unit("src/utils.rs", "Rust"),
    ];
    let scanned = vec![
        "src/main.rs".to_string(),
        "src/utils.rs".to_string(),
        "js/zero_units.js".to_string(), // zero units — must still get an entity
    ];

    let result = call_group_code_units_with_scanned(
        &units,
        &scanned,
        "test-project",
        "test-repo",
        "test-repo-id",
        temp.path(),
    )
    .unwrap();

    // All three files must appear in the grouping output.
    assert_eq!(
        result.files.len(),
        3,
        "expected 3 file entities, got {}",
        result.files.len()
    );
    let zero_unit = result
        .files
        .iter()
        .find(|f| f.path.as_deref() == Some("js/zero_units.js"))
        .expect("zero-unit scanned path must get a file entity");
    // Its language comes from the extension (no units to borrow from).
    assert_eq!(
        zero_unit.language.as_deref(),
        Some("JavaScript"),
        "zero-unit file entity language must be derived from the .js extension"
    );

    // Sanity: a scanned path that ALSO has units must not duplicate.
    let main_count = result
        .files
        .iter()
        .filter(|f| f.path.as_deref() == Some("src/main.rs"))
        .count();
    assert_eq!(
        main_count, 1,
        "a scanned path with units must not produce duplicate file entities"
    );
}

#[test]
fn test_group_code_units_fallback() {
    let temp = TempDir::new().unwrap();
    let units = vec![
        make_code_unit("src/main.rs", "Rust"),
        make_code_unit("src/utils.rs", "Rust"),
    ];

    let result = call_group_code_units(
        &units,
        "test-project",
        "test-repo",
        "test-repo-id",
        temp.path(),
    )
    .unwrap();

    // Fallback: one subsystem entry at "."
    assert_eq!(result.subsystems.len(), 1);
    assert!(result.subsystems[0].path == Some(".".to_string()));
    assert_eq!(result.files.len(), 2);
}

#[test]
fn test_group_code_units_cargo_workspace() {
    let temp = TempDir::new().unwrap();
    let cargo_toml = temp.path().join("Cargo.toml");
    fs::write(
        &cargo_toml,
        "[workspace]\nmembers = [\"crates/conductor\"]\n",
    )
    .unwrap();

    let units = vec![make_code_unit("crates/conductor/src/main.rs", "Rust")];

    let result = call_group_code_units(
        &units,
        "test-project",
        "test-repo",
        "test-repo-id",
        temp.path(),
    )
    .unwrap();

    assert_eq!(result.subsystems.len(), 2); // conductor + root
    assert!(
        result
            .subsystems
            .iter()
            .any(|s| s.name == "conductor" && s.path == Some("crates/conductor".to_string()))
    );

    assert!(!result.modules.is_empty());
    assert!(!result.files.is_empty());
}

#[test]
fn test_entity_id_format() {
    let temp = TempDir::new().unwrap();
    let units = vec![make_code_unit("src/main.rs", "Rust")];

    let result = call_group_code_units(
        &units,
        "test-project",
        "test-repo",
        "test-repo-id",
        temp.path(),
    )
    .unwrap();

    let file_entity = &result.files[0];
    assert!(file_entity.id.starts_with("test-project:test-repo:file:"));
    assert!(file_entity.id.ends_with("src/main.rs"));

    let subsystem_entity = &result.subsystems[0];
    assert!(
        subsystem_entity
            .id
            .starts_with("test-project:test-repo:subsystem:")
    );

    if !result.modules.is_empty() {
        let module_entity = &result.modules[0];
        assert!(
            module_entity
                .id
                .starts_with("test-project:test-repo:module:")
        );
    }
}

#[test]
fn test_parent_id_linking() {
    let temp = TempDir::new().unwrap();
    let units = vec![make_code_unit("src/main.rs", "Rust")];

    let result = call_group_code_units(
        &units,
        "test-proj",
        "test-repo",
        "test-repo-id",
        temp.path(),
    )
    .unwrap();

    for file in &result.files {
        assert!(file.parent_id.is_some(), "File should have parent_id");
    }
    for module in &result.modules {
        assert!(module.parent_id.is_some(), "Module should have parent_id");
    }
    for subsystem in &result.subsystems {
        assert!(
            subsystem.parent_id.is_none(),
            "Subsystem should not have parent_id"
        );
    }
}

#[test]
fn test_repo_id_set_on_all_entities() {
    let temp = TempDir::new().unwrap();
    let units = vec![
        make_code_unit("src/main.rs", "Rust"),
        make_code_unit("src/utils.rs", "Rust"),
    ];

    let result = call_group_code_units(
        &units,
        "test-proj",
        "test-repo",
        "test-repo-id",
        temp.path(),
    )
    .unwrap();

    for file in &result.files {
        assert_eq!(
            file.repo_id,
            Some("test-repo-id".to_string()),
            "File entity should have repo_id set"
        );
    }
    for module in &result.modules {
        assert_eq!(
            module.repo_id,
            Some("test-repo-id".to_string()),
            "Module entity should have repo_id set"
        );
    }
    for subsystem in &result.subsystems {
        assert_eq!(
            subsystem.repo_id,
            Some("test-repo-id".to_string()),
            "Subsystem entity should have repo_id set"
        );
    }
}

#[test]
fn test_deterministic_output() {
    let temp = TempDir::new().unwrap();
    let units = vec![
        make_code_unit("src/main.rs", "Rust"),
        make_code_unit("src/utils.rs", "Rust"),
    ];

    let result1 = call_group_code_units(
        &units,
        "test-proj",
        "test-repo",
        "test-repo-id",
        temp.path(),
    )
    .unwrap();
    let result2 = call_group_code_units(
        &units,
        "test-proj",
        "test-repo",
        "test-repo-id",
        temp.path(),
    )
    .unwrap();

    let (s1, m1, f1) = sorted_ids(&result1);
    let (s2, m2, f2) = sorted_ids(&result2);
    assert_eq!(s1, s2);
    assert_eq!(m1, m2);
    assert_eq!(f1, f2);
}

#[test]
fn test_files_not_in_workspace_go_to_root() {
    let temp = TempDir::new().unwrap();
    let cargo_toml = temp.path().join("Cargo.toml");
    fs::write(
        &cargo_toml,
        "[workspace]\nmembers = [\"crates/conductor\"]\n",
    )
    .unwrap();

    let units = vec![make_code_unit("README.md", "Markdown")];

    let result = call_group_code_units(
        &units,
        "test-proj",
        "test-repo",
        "test-repo-id",
        temp.path(),
    )
    .unwrap();

    assert!(
        result
            .subsystems
            .iter()
            .any(|s| s.name == "root" && s.path == Some(".".to_string()))
    );
}

// ── JS/TS hierarchy (#689) ─────────────────────────────────────────────────

fn write_js_ts(root: &Path, rel: &str, name: &str) {
    let dir = root.join(rel);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(name), "// stub\n").unwrap();
}

#[test]
fn test_group_code_units_plain_js_ts_repo_multi_module_hierarchy() {
    // Plain JS/TS repo: package.json (no workspaces), src/ with ≥2 subdirs
    // each containing ≥1 .ts/.tsx file. Must produce ≥2 Module entities and
    // ≥1 Subsystem that is NOT the "." fallback.
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    write_js_ts(root, "src/components", "Button.tsx");
    write_js_ts(root, "src/hooks", "useThing.ts");

    fs::write(
        root.join("package.json"),
        r#"{"name": "plain", "dependencies": {"react": "18.0.0"}}"#,
    )
    .unwrap();

    let units = vec![
        make_code_unit("src/components/Button.tsx", "TypeScript"),
        make_code_unit("src/hooks/useThing.ts", "TypeScript"),
    ];

    let result =
        call_group_code_units(&units, "test-project", "test-repo", "test-repo-id", root).unwrap();

    // ≥1 subsystem that is not the "." fallback
    assert!(
        result
            .subsystems
            .iter()
            .any(|s| s.path.as_deref() == Some("src")),
        "expected a src subsystem, got: {:?}",
        result
            .subsystems
            .iter()
            .map(|s| s.path.clone())
            .collect::<Vec<_>>()
    );

    // ≥2 modules: src/components and src/hooks are distinct modules
    let module_paths: Vec<String> = result
        .modules
        .iter()
        .map(|m| m.path.clone().unwrap_or_default())
        .collect();
    assert!(
        module_paths.contains(&"src/components".to_string()),
        "modules: {module_paths:?}"
    );
    assert!(
        module_paths.contains(&"src/hooks".to_string()),
        "modules: {module_paths:?}"
    );
    assert!(result.modules.len() >= 2);
}

#[test]
fn test_group_code_units_www_shaped_no_degenerate_collapse() {
    // www-shaped repo: package.json workspaces:["ssr"] + bulk JS/TS in
    // unmatched top-level dirs (app/javascript/, cypress/, lib/). Must yield
    // ≥2 subsystems and ≥2 modules — the 1/1 degenerate collapse is fixed.
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    write_js_ts(root, "ssr/src", "index.js");
    write_js_ts(root, "app/javascript", "main.js");
    write_js_ts(root, "cypress", "spec.ts");
    write_js_ts(root, "lib", "util.tsx");

    fs::write(
        root.join("package.json"),
        r#"{"name": "www", "workspaces": ["ssr"]}"#,
    )
    .unwrap();

    let units = vec![
        make_code_unit("ssr/src/index.js", "JavaScript"),
        make_code_unit("app/javascript/main.js", "JavaScript"),
        make_code_unit("cypress/spec.ts", "TypeScript"),
        make_code_unit("lib/util.tsx", "TypeScript"),
    ];

    let result =
        call_group_code_units(&units, "test-project", "test-repo", "test-repo-id", root).unwrap();

    assert!(
        result.subsystems.len() >= 2,
        "subsystems: {:?}",
        result
            .subsystems
            .iter()
            .map(|s| s.path.clone())
            .collect::<Vec<_>>()
    );
    assert!(result.modules.len() >= 2);

    // Each unmatched top-level dir is its own subsystem
    for expected in ["ssr", "app", "cypress", "lib"] {
        assert!(
            result
                .subsystems
                .iter()
                .any(|s| s.path.as_deref() == Some(expected)),
            "missing subsystem {expected}; got: {:?}",
            result
                .subsystems
                .iter()
                .map(|s| s.path.clone())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn test_group_code_units_pure_monorepo_not_supplemented() {
    // A pure monorepo (workspaces covering the layout, no unmatched bulk
    // JS/TS dirs) must NOT be supplemented: no recursive JS/TS scan runs when
    // the npm detector's map already has multiple members. The www-shaped
    // fixture above (1 member) proves the supplement path; this one proves
    // the skip path keeps the pure-monorepo map exact.
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    write_js_ts(root, "packages/a/src", "index.js");
    write_js_ts(root, "packages/b/src", "index.ts");

    fs::write(
        root.join("package.json"),
        r#"{"name": "mono", "workspaces": ["packages/a", "packages/b"]}"#,
    )
    .unwrap();

    let units = vec![
        make_code_unit("packages/a/src/index.js", "JavaScript"),
        make_code_unit("packages/b/src/index.ts", "TypeScript"),
    ];
    let result =
        call_group_code_units(&units, "test-project", "test-repo", "test-repo-id", root).unwrap();

    let subsystem_paths: Vec<String> = result
        .subsystems
        .iter()
        .filter_map(|s| s.path.clone())
        .collect();
    // Pure monorepo: the members are present, nothing supplemented.
    assert!(
        subsystem_paths.iter().any(|p| p == "packages/a"),
        "subsystems: {subsystem_paths:?}"
    );
    assert!(
        subsystem_paths.iter().any(|p| p == "packages/b"),
        "subsystems: {subsystem_paths:?}"
    );
    assert!(
        !subsystem_paths.iter().any(|p| p == "src"),
        "no phantom src subsystem for pure monorepo: {subsystem_paths:?}"
    );
}
