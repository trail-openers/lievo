// Grouping heuristic tests

use super::*;
use crate::extraction::grouping_helpers::determine_module_path;
use crate::model::CodeUnit;

fn make_code_unit(file: &str, language: &str) -> CodeUnit {
    CodeUnit {
        name: "test".to_string(),
        unit_type: "function".to_string(),
        file: file.to_string(),
        line: 1,
        end_line: 10,
        language: language.to_string(),
        signature: None,
        code: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
        qualified_name: "test::test".to_string(),
        docstring: None,
        parent_class: None,
    }
}

#[test]
fn test_determine_module_path_root_subsystem() {
    assert_eq!(determine_module_path("src/main.rs", ".", 1), "src");
}

#[test]
fn test_determine_module_path_file_at_root() {
    assert_eq!(determine_module_path("README.md", ".", 1), ".");
}

#[test]
fn test_determine_module_path_nested_subsystem() {
    assert_eq!(
        determine_module_path("crates/my-crate/src/main.rs", "crates/my-crate", 1),
        "crates/my-crate/src"
    );
}

#[test]
fn test_determine_module_path_file_at_subsystem_root() {
    assert_eq!(
        determine_module_path("crates/my-crate/lib.rs", "crates/my-crate", 1),
        "crates/my-crate"
    );
}

#[test]
fn test_determine_module_path_depth_2_captures_two_levels() {
    // With depth=2, "app/models/user.rb" under subsystem "app" → "app/models"
    assert_eq!(
        determine_module_path("app/models/user.rb", "app", 2),
        "app/models"
    );
}

#[test]
fn test_determine_module_path_depth_2_root_subsystem() {
    // With depth=2, two dirs are captured below "."
    assert_eq!(
        determine_module_path("app/models/user.rb", ".", 2),
        "app/models"
    );
}

#[test]
fn test_determine_module_path_depth_2_shallow_file() {
    // Only 1 dir available even though depth=2 — take what's there
    assert_eq!(determine_module_path("app/user.rb", "app", 2), "app");
}

#[test]
fn test_determine_module_path_depth_2_deep_file_under_named_subsystem() {
    // Under a named subsystem (e.g. "app"), depth=2 means 2 levels below
    // the subsystem root. app/models/concerns/validatable.rb under subsystem "app"
    // → module "app/models/concerns" (both levels consumed, no cap needed since
    //   the path has exactly 2 dir components below subsystem root).
    assert_eq!(
        determine_module_path("app/models/concerns/validatable.rb", "app", 2),
        "app/models/concerns"
    );
}

#[test]
fn test_group_into_files() {
    let units = vec![
        make_code_unit("src/main.rs", "Rust"),
        make_code_unit("src/main.rs", "Rust"),
        make_code_unit("src/utils.rs", "Rust"),
        make_code_unit("test.py", "Python"),
    ];
    let files = group_into_files(
        &units,
        &[],
        "test-project",
        "test-repo",
        "test-repo-id",
        &[],
    )
    .unwrap();
    assert_eq!(files.len(), 3);
    assert!(
        files
            .iter()
            .any(|f| f.path.as_deref() == Some("src/main.rs"))
    );
    assert!(
        files
            .iter()
            .any(|f| f.path.as_deref() == Some("src/utils.rs"))
    );
    assert!(files.iter().any(|f| f.path.as_deref() == Some("test.py")));
}

#[test]
fn test_group_into_files_excludes_paths() {
    let units = vec![
        make_code_unit("src/main.rs", "Rust"),
        make_code_unit("docs/README.md", "Markdown"),
        make_code_unit("docs/index.md", "Markdown"),
    ];
    let exclude = vec!["docs".to_string()];
    let files = group_into_files(
        &units,
        &[],
        "test-project",
        "test-repo",
        "test-repo-id",
        &exclude,
    )
    .unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        files
            .iter()
            .any(|f| f.path.as_deref() == Some("src/main.rs"))
    );
    assert!(
        !files
            .iter()
            .any(|f| f.path.as_deref() == Some("docs/README.md"))
    );
    assert!(
        !files
            .iter()
            .any(|f| f.path.as_deref() == Some("docs/index.md"))
    );
}

#[test]
fn test_group_into_files_sets_repo_id() {
    let units = vec![make_code_unit("src/main.rs", "Rust")];
    let files = group_into_files(
        &units,
        &[],
        "test-project",
        "test-repo",
        "test-repo-id",
        &[],
    )
    .unwrap();
    assert_eq!(files[0].repo_id, Some("test-repo-id".to_string()));
}

#[test]
fn test_group_into_files_language_selection() {
    let u1 = make_code_unit("test.rs", "Rust");
    let u2 = make_code_unit("test.rs", "Rust");
    let u3 = make_code_unit("test.rs", "Python");
    let units = vec![u1, u2, u3];
    let files = group_into_files(
        &units,
        &[],
        "test-project",
        "test-repo",
        "test-repo-id",
        &[],
    )
    .unwrap();
    assert_eq!(files[0].language, Some("Rust".to_string()));
}

#[test]
fn test_group_into_files_seeds_zero_unit_paths() {
    // A file that produced no code units at all must still get a file entity
    // (issue #701), and its language comes from the extension.
    let units = vec![make_code_unit("src/main.rs", "Rust")];
    let scanned = vec!["src/main.rs".to_string(), "src/zero_units.js".to_string()];
    let files = group_into_files(
        &units,
        &scanned,
        "test-project",
        "test-repo",
        "test-repo-id",
        &[],
    )
    .unwrap();
    let zero_unit_file = files
        .iter()
        .find(|f| f.path.as_deref() == Some("src/zero_units.js"))
        .expect("zero-unit scanned path should get a file entity");
    assert_eq!(zero_unit_file.language, Some("JavaScript".to_string()));
}

#[test]
fn test_group_into_files_zero_unit_paths_respect_excludes() {
    // A scanned path under an excluded directory must not get an entity.
    let units = vec![make_code_unit("src/main.rs", "Rust")];
    let scanned = vec!["src/main.rs".to_string(), "docs/output/zero.py".to_string()];
    let exclude = vec!["docs".to_string()];
    let files = group_into_files(
        &units,
        &scanned,
        "test-project",
        "test-repo",
        "test-repo-id",
        &exclude,
    )
    .unwrap();
    assert_eq!(files.len(), 1);
    assert!(
        !files
            .iter()
            .any(|f| f.path.as_deref() == Some("docs/output/zero.py"))
    );
}

#[test]
fn test_group_into_files_unit_language_wins_over_extension() {
    // When a file has units, the entity language comes from the units (via
    // most_common_language), not from the path extension — even when the
    // scanned path has a different-looking extension.
    let units = vec![make_code_unit("weird.rust", "Rust")];
    let scanned = vec!["weird.rust".to_string()];
    let files = group_into_files(
        &units,
        &scanned,
        "test-project",
        "test-repo",
        "test-repo-id",
        &[],
    )
    .unwrap();
    let file = files
        .iter()
        .find(|f| f.path.as_deref() == Some("weird.rust"))
        .unwrap();
    assert_eq!(file.language, Some("Rust".to_string()));
}

#[test]
fn test_group_into_files_no_units_only_zero_unit_paths() {
    // Empty unit set plus scanned paths: one file entity per scanned path.
    let scanned = vec!["app/js.js".to_string(), "app/ts.tsx".to_string()];
    let files = group_into_files(
        &[],
        &scanned,
        "test-project",
        "test-repo",
        "test-repo-id",
        &[],
    )
    .unwrap();
    assert_eq!(files.len(), 2);
    let js_file = files
        .iter()
        .find(|f| f.path.as_deref() == Some("app/js.js"))
        .unwrap();
    let ts_file = files
        .iter()
        .find(|f| f.path.as_deref() == Some("app/ts.tsx"))
        .unwrap();
    assert_eq!(js_file.language, Some("JavaScript".to_string()));
    assert_eq!(ts_file.language, Some("TypeScript".to_string()));
}

#[test]
fn test_group_code_units_rails_project_uses_depth_2_automatically() {
    use crate::config::RepoConfig;
    use tempfile::tempdir;

    let tmp = tempdir().unwrap();
    // Create Rails signals
    std::fs::write(
        tmp.path().join("Gemfile"),
        "source 'https://rubygems.org'\ngem 'rails', '~> 7.0'\n",
    )
    .unwrap();
    std::fs::create_dir_all(tmp.path().join("config")).unwrap();
    std::fs::write(
        tmp.path().join("config").join("application.rb"),
        "module App; end",
    )
    .unwrap();
    std::fs::create_dir_all(tmp.path().join("app").join("models")).unwrap();

    // Verify detect_subsystems returns depth=2 for Rails
    let (subsystems, depth, _profile) =
        crate::extraction::detectors::detect_subsystems(tmp.path()).unwrap();
    assert_eq!(depth, 2, "Rails project should auto-select module_depth=2");
    assert!(
        subsystems.contains_key("app"),
        "Rails should have app subsystem"
    );

    // Verify that default RepoConfig (None module_depth) does NOT override detected depth=2
    let config = RepoConfig::default();
    let resolved_depth = config.module_depth.unwrap_or(depth);
    assert_eq!(
        resolved_depth, 2,
        "Default config should not override Rails depth=2"
    );

    // Verify explicit override DOES work
    let override_config = RepoConfig {
        module_depth: Some(1),
        ..Default::default()
    };
    let override_depth = override_config.module_depth.unwrap_or(depth);
    assert_eq!(
        override_depth, 1,
        "Explicit module_depth=1 should override detected depth=2"
    );
}

#[test]
fn test_group_into_files_sorted_deterministically() {
    let units = vec![
        make_code_unit("z_file.rs", "Rust"),
        make_code_unit("a_file.rs", "Rust"),
        make_code_unit("m_file.rs", "Rust"),
    ];
    let files1 = group_into_files(
        &units,
        &[],
        "test-project",
        "test-repo",
        "test-repo-id",
        &[],
    )
    .unwrap();
    let files2 = group_into_files(
        &units,
        &[],
        "test-project",
        "test-repo",
        "test-repo-id",
        &[],
    )
    .unwrap();
    let ids1: Vec<&str> = files1.iter().map(|f| f.id.as_str()).collect();
    let ids2: Vec<&str> = files2.iter().map(|f| f.id.as_str()).collect();
    assert_eq!(ids1, ids2);
}

#[test]
fn test_config_subsystems_override_auto_detected() {
    use crate::config::RepoConfig;
    use std::fs;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Create a FastAPI project
    fs::write(
        root.join("pyproject.toml"),
        "[project]\ndependencies = [\"fastapi\"]\n",
    )
    .unwrap();

    // Create an auto-detected Python package
    let myapp_dir = root.join("myapp");
    fs::create_dir_all(&myapp_dir).unwrap();
    fs::write(myapp_dir.join("__init__.py"), "").unwrap();

    // Create config that overrides with custom subsystems
    let config = RepoConfig {
        subsystems: vec![crate::config::SubsystemOverride {
            name: "custom-core".to_string(),
            paths: vec!["myapp".to_string()],
        }],
        ..Default::default()
    };

    // Call detect_subsystems directly to verify the merge happens
    let (mut subsystem_map, _depth, _profile) =
        crate::extraction::detectors::detect_subsystems(root).unwrap();

    // Verify auto-detected entry exists
    assert_eq!(
        subsystem_map.get("myapp").map(String::as_str),
        Some("myapp"),
        "auto-detected should be present before merge"
    );

    // Apply the merge logic (same as in group_code_units)
    for subsystem in &config.subsystems {
        for path in &subsystem.paths {
            subsystem_map.insert(path.clone(), subsystem.name.clone());
        }
    }

    // Verify config override took precedence
    assert_eq!(
        subsystem_map.get("myapp").map(String::as_str),
        Some("custom-core"),
        "config subsystem name should override auto-detected"
    );
}

#[test]
fn test_config_subsystems_adds_new_entries() {
    use crate::config::RepoConfig;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // No package detection, so fallback to "."
    let (mut subsystem_map, _depth, _profile) =
        crate::extraction::detectors::detect_subsystems(root).unwrap();

    // Verify only root exists initially
    assert_eq!(subsystem_map.len(), 1);
    assert!(subsystem_map.contains_key("."));

    // Create config that adds new subsystems
    let config = RepoConfig {
        subsystems: vec![
            crate::config::SubsystemOverride {
                name: "core".to_string(),
                paths: vec!["src/core".to_string(), "src/utils".to_string()],
            },
            crate::config::SubsystemOverride {
                name: "cli".to_string(),
                paths: vec!["src/bin".to_string()],
            },
        ],
        ..Default::default()
    };

    // Apply the merge logic
    for subsystem in &config.subsystems {
        for path in &subsystem.paths {
            subsystem_map.insert(path.clone(), subsystem.name.clone());
        }
    }

    // Verify new entries were added
    assert_eq!(
        subsystem_map.len(),
        4,
        "should have root + 3 config subsystems"
    );
    assert_eq!(
        subsystem_map.get("src/core").map(String::as_str),
        Some("core")
    );
    assert_eq!(
        subsystem_map.get("src/utils").map(String::as_str),
        Some("core")
    );
    assert_eq!(
        subsystem_map.get("src/bin").map(String::as_str),
        Some("cli")
    );
}

#[test]
fn test_preserve_function_entities_defaults_to_true_when_no_config() {
    // When config.config is None, preserve_function_entities should default to true
    // (matching RepoConfig::default().preserve_function_entities)
    let config = GroupingConfig {
        code_units: &[],
        scanned_file_paths: &[],
        project_id: "test-project",
        repo_name: "test-repo",
        repo_id: "test-repo-id",
        repo_path: std::path::Path::new("."),
        config: None,
        exclude_paths: &[],
    };
    let result = group_code_units(&config).unwrap();
    assert!(
        result.preserve_function_entities,
        "preserve_function_entities should be true when config is None"
    );
}
