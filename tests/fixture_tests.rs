// Fixture-based integration tests for the full analysis pipeline.
//
// Uses in-memory CodeUnit mocks — no external tooling required.
// Tempdir provides filesystem structure for subsystem detectors.

use lievo::analysis::relationships::RelationshipBuilder;
use lievo::extraction::entity_id::entity_id;
use lievo::extraction::grouping::{GroupingConfig, group_code_units};
use lievo::model::{CodeUnit, EntityTier, RelType};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ─── Helpers ────────────────────────────────────────────────────────────────

fn mock_code_unit(
    file: &str,
    name: &str,
    unit_type: &str,
    imports: Vec<String>,
    calls: Vec<String>,
) -> CodeUnit {
    CodeUnit {
        name: name.to_string(),
        qualified_name: format!("{}::{}", file.replace('/', "::"), name),
        unit_type: unit_type.to_string(),
        file: file.to_string(),
        line: 1,
        end_line: 10,
        language: if file.ends_with(".rs") {
            "Rust"
        } else {
            "Python"
        }
        .to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls,
        imports,
    }
}
// ─── Fixture 1: Rust workspace ───────────────────────────────────────────────
//
// Layout:
//   Cargo.toml               [workspace] members = ["crates/core", "crates/cli"]
//   crates/core/src/lib.rs   pub mod util;
//   crates/core/src/util.rs  pub fn helper() {}
//   crates/cli/src/main.rs   use crates/core/src/util.rs (raw path import)
//
// Expected:
//   Subsystems: "root" (.), "core" (crates/core), "cli" (crates/cli) = 3 total
//   cli depends_on core (via aggregated file-level import)
//   Entity IDs follow "<project>:<repo>:<tier>:<path>" format
fn setup_rust_workspace() -> TempDir {
    let temp = TempDir::new().expect("failed to create temp dir for rust workspace fixture");

    // Workspace Cargo.toml — drives detect_cargo_workspace
    fs::write(
        temp.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/core\", \"crates/cli\"]\n",
    )
    .expect("failed to write workspace Cargo.toml");

    // Directories must exist for group_code_units to accept the repo_path
    fs::create_dir_all(temp.path().join("crates/core/src"))
        .expect("failed to create crates/core/src directory");
    fs::create_dir_all(temp.path().join("crates/cli/src"))
        .expect("failed to create crates/cli/src directory");

    temp
}

fn rust_workspace_units() -> Vec<CodeUnit> {
    vec![
        // crates/core/src/lib.rs — top-level module declaration
        mock_code_unit("crates/core/src/lib.rs", "lib", "module", vec![], vec![]),
        // crates/core/src/util.rs — defines helper()
        mock_code_unit(
            "crates/core/src/util.rs",
            "helper",
            "function",
            vec![],
            vec![],
        ),
        // crates/cli/src/main.rs — imports util.rs via raw path (resolved by import_map)
        mock_code_unit(
            "crates/cli/src/main.rs",
            "main",
            "function",
            vec!["crates/core/src/util.rs".to_string()],
            vec![],
        ),
    ]
}

fn call_group_code_units(
    units: &[CodeUnit],
    project_id: &str,
    repo_name: &str,
    repo_id: &str,
    repo_path: &Path,
) -> lievo::error::Result<lievo::extraction::grouping::GroupingResult> {
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

#[test]
fn test_rust_workspace_two_subsystems_detected() {
    let temp = setup_rust_workspace();
    let units = rust_workspace_units();

    let result =
        call_group_code_units(&units, "fixture-proj", "rust-ws", "rust-ws-id", temp.path())
            .expect("group_code_units failed for rust workspace");

    // Cargo workspace detector yields root + core + cli = exactly 3 subsystems
    let names: Vec<&str> = result.subsystems.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        result.subsystems.len(),
        3,
        "Expected exactly 3 subsystems (root, core, cli), got: {:?}",
        names
    );

    assert!(
        names.contains(&"core"),
        "Expected 'core' subsystem, got: {:?}",
        names
    );
    assert!(
        names.contains(&"cli"),
        "Expected 'cli' subsystem, got: {:?}",
        names
    );
    assert!(
        names.contains(&"root"),
        "Expected 'root' subsystem, got: {:?}",
        names
    );
}

#[test]
fn test_rust_workspace_module_grouping() {
    let temp = setup_rust_workspace();
    let units = rust_workspace_units();

    let result =
        call_group_code_units(&units, "fixture-proj", "rust-ws", "rust-ws-id", temp.path())
            .expect("group_code_units failed for rust workspace");

    // crates/core/ should contain lib.rs and util.rs
    let core_files: Vec<_> = result
        .files
        .iter()
        .filter(|f| {
            f.path
                .as_deref()
                .map(|p| p.starts_with("crates/core/"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(
        core_files.len(),
        2,
        "Expected 2 files under crates/core/, got: {:?}",
        core_files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );

    // crates/cli/ should contain main.rs
    let cli_files: Vec<_> = result
        .files
        .iter()
        .filter(|f| {
            f.path
                .as_deref()
                .map(|p| p.starts_with("crates/cli/"))
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(cli_files.len(), 1, "Expected 1 file under crates/cli/");

    // Verify the single cli file is main.rs
    assert_eq!(
        cli_files[0].path.as_deref(),
        Some("crates/cli/src/main.rs"),
        "Expected cli file to be main.rs"
    );

    // Every file must have a parent module
    for file in result.files.iter() {
        assert!(
            file.parent_id.is_some(),
            "File {:?} should have a parent module",
            file.path
        );
    }
}

#[test]
fn test_rust_workspace_cli_depends_on_core() {
    let temp = setup_rust_workspace();
    let units = rust_workspace_units();

    let grouping =
        call_group_code_units(&units, "fixture-proj", "rust-ws", "rust-ws-id", temp.path())
            .expect("group_code_units failed for rust workspace");
    let (rels, _unresolved) =
        RelationshipBuilder::build(&units, &grouping, "fixture-proj", "rust-ws", temp.path())
            .expect("RelationshipBuilder::build failed for rust workspace");

    let core_sub = grouping
        .subsystems
        .iter()
        .find(|s| s.name == "core")
        .expect("core subsystem not found");
    let cli_sub = grouping
        .subsystems
        .iter()
        .find(|s| s.name == "cli")
        .expect("cli subsystem not found");

    let depends_on: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::DependsOn)
        .collect();

    assert!(
        !depends_on.is_empty(),
        "Expected at least one DependsOn relationship, but got none"
    );
    assert!(
        depends_on
            .iter()
            .any(|r| r.source_id == cli_sub.id && r.target_id == core_sub.id),
        "Expected cli depends_on core. Got: {:?}",
        depends_on
            .iter()
            .map(|r| (&r.source_id, &r.target_id))
            .collect::<Vec<_>>()
    );
}
// ─── Fixture 2: Python package ───────────────────────────────────────────────
//
// Layout:
//   src/auth/__init__.py
//   src/auth/jwt.py        import hashlib; def verify(): pass
//   src/auth/oauth.py      from auth.jwt import verify
//   src/api/__init__.py
//   src/api/handlers.py    from auth import jwt
//
// detect_top_level_src finds src/auth and src/api as subsystems.
//
// Expected:
//   Subsystems: "root" (.), "auth" (src/auth), "api" (src/api) = 3 total
//   api depends_on auth (via raw file path import)
fn setup_python_package() -> TempDir {
    let temp = TempDir::new().expect("failed to create temp dir for python package fixture");

    // detect_top_level_src scans subdirs of src/
    fs::create_dir_all(temp.path().join("src/auth")).expect("failed to create src/auth directory");
    fs::create_dir_all(temp.path().join("src/api")).expect("failed to create src/api directory");

    // __init__.py present (not required by detect_top_level_src but realistic)
    fs::write(temp.path().join("src/auth/__init__.py"), "")
        .expect("failed to write src/auth/__init__.py");
    fs::write(temp.path().join("src/api/__init__.py"), "")
        .expect("failed to write src/api/__init__.py");

    temp
}

fn python_package_units() -> Vec<CodeUnit> {
    vec![
        // src/auth/jwt.py — defines verify()
        mock_code_unit(
            "src/auth/jwt.py",
            "verify",
            "function",
            vec!["hashlib".to_string()],
            vec![],
        ),
        // src/auth/oauth.py — intra-package import (stays within auth subsystem)
        mock_code_unit(
            "src/auth/oauth.py",
            "oauth_verify",
            "function",
            vec!["src/auth/jwt.py".to_string()],
            vec![],
        ),
        // src/api/handlers.py — cross-subsystem import into auth
        mock_code_unit(
            "src/api/handlers.py",
            "handle",
            "function",
            vec!["src/auth/jwt.py".to_string()],
            vec![],
        ),
    ]
}

#[test]
fn test_python_package_two_subsystems_detected() {
    let temp = setup_python_package();
    let units = python_package_units();

    let result = call_group_code_units(&units, "fixture-proj", "py-pkg", "py-pkg-id", temp.path())
        .expect("group_code_units failed for python package");

    let names: Vec<&str> = result.subsystems.iter().map(|s| s.name.as_str()).collect();

    // detect_python_packages detects src/auth and src/api as Python packages (no "." root subsystem)
    // due to issue #395 fix to prevent module explosion
    assert_eq!(
        result.subsystems.len(),
        2,
        "Expected exactly 2 subsystems (auth, api), got: {:?}",
        names
    );

    assert!(
        names.contains(&"auth"),
        "Expected 'auth' subsystem, got: {:?}",
        names
    );
    assert!(
        names.contains(&"api"),
        "Expected 'api' subsystem, got: {:?}",
        names
    );
}

#[test]
fn test_python_package_api_depends_on_auth() {
    let temp = setup_python_package();
    let units = python_package_units();

    let grouping =
        call_group_code_units(&units, "fixture-proj", "py-pkg", "py-pkg-id", temp.path())
            .expect("group_code_units failed for python package");
    let (rels, _unresolved) =
        RelationshipBuilder::build(&units, &grouping, "fixture-proj", "py-pkg", temp.path())
            .expect("RelationshipBuilder::build failed for python package");

    let auth_sub = grouping
        .subsystems
        .iter()
        .find(|s| s.name == "auth")
        .expect("auth subsystem not found");
    let api_sub = grouping
        .subsystems
        .iter()
        .find(|s| s.name == "api")
        .expect("api subsystem not found");

    let depends_on: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::DependsOn)
        .collect();

    assert!(
        !depends_on.is_empty(),
        "Expected at least one DependsOn relationship, but got none"
    );
    assert!(
        depends_on
            .iter()
            .any(|r| r.source_id == api_sub.id && r.target_id == auth_sub.id),
        "Expected api depends_on auth. Got: {:?}",
        depends_on
            .iter()
            .map(|r| (&r.source_id, &r.target_id))
            .collect::<Vec<_>>()
    );
}
// ─── Entity ID format tests ──────────────────────────────────────────────────
#[test]
fn test_entity_ids_deterministic_format() {
    // Verify the canonical format: "<project>:<repo>:<tier>:<path>"
    let file_id = entity_id(
        "fixture-proj",
        "rust-ws",
        EntityTier::File,
        "crates/core/src/lib.rs",
    )
    .expect("entity_id failed for file tier");
    assert_eq!(file_id, "fixture-proj:rust-ws:file:crates/core/src/lib.rs");

    let sub_id = entity_id(
        "fixture-proj",
        "rust-ws",
        EntityTier::Subsystem,
        "crates/core",
    )
    .expect("entity_id failed for subsystem tier");
    assert_eq!(sub_id, "fixture-proj:rust-ws:subsystem:crates/core");

    let mod_id = entity_id(
        "fixture-proj",
        "rust-ws",
        EntityTier::Module,
        "crates/core/src",
    )
    .expect("entity_id failed for module tier");
    assert_eq!(mod_id, "fixture-proj:rust-ws:module:crates/core/src");
}

#[test]
fn test_rust_workspace_entity_ids_follow_format() {
    let temp = setup_rust_workspace();
    let units = rust_workspace_units();

    let result =
        call_group_code_units(&units, "fixture-proj", "rust-ws", "rust-ws-id", temp.path())
            .expect("group_code_units failed for rust workspace");

    for sub in &result.subsystems {
        assert!(
            sub.id.starts_with("fixture-proj:rust-ws:subsystem:"),
            "Subsystem ID malformed: {}",
            sub.id
        );
    }
    for module in &result.modules {
        assert!(
            module.id.starts_with("fixture-proj:rust-ws:module:"),
            "Module ID malformed: {}",
            module.id
        );
    }
    for file in &result.files {
        assert!(
            file.id.starts_with("fixture-proj:rust-ws:file:"),
            "File ID malformed: {}",
            file.id
        );
    }
}
// ─── Edge case tests ─────────────────────────────────────────────────────────
#[test]
fn test_group_code_units_empty_input_returns_empty_without_panic() {
    // Empty input must not panic and must produce empty grouping results.
    let temp = TempDir::new().expect("failed to create temp dir for empty input test");

    let units: Vec<CodeUnit> = vec![];
    let result = call_group_code_units(
        &units,
        "fixture-proj",
        "empty-repo",
        "empty-repo-id",
        temp.path(),
    )
    .expect("group_code_units should not fail on empty input");

    assert!(
        result.subsystems.len() <= 1,
        "Expected at most a root subsystem for empty input, got: {:?}",
        result
            .subsystems
            .iter()
            .map(|s| &s.name)
            .collect::<Vec<_>>()
    );
    assert!(
        result.files.is_empty(),
        "Expected no files for empty input, got {} files",
        result.files.len()
    );
    assert!(
        result.modules.is_empty(),
        "Expected no modules for empty input, got {} modules",
        result.modules.len()
    );
}

#[test]
fn test_group_code_units_single_file_input_creates_one_file() {
    // A single file with no workspace markers falls back to a flat grouping
    // with exactly one file entity and a parent module/subsystem.
    let temp = TempDir::new().expect("failed to create temp dir for single-file test");
    fs::create_dir_all(temp.path().join("src"))
        .expect("failed to create src directory for single-file test");

    let units = vec![mock_code_unit(
        "src/main.rs",
        "main",
        "function",
        vec![],
        vec![],
    )];

    let result = call_group_code_units(
        &units,
        "fixture-proj",
        "single-file",
        "single-file-id",
        temp.path(),
    )
    .expect("group_code_units should not fail on single-file input");

    assert_eq!(
        result.files.len(),
        1,
        "Expected exactly 1 file entity, got: {:?}",
        result.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    assert_eq!(
        result.files[0].path.as_deref(),
        Some("src/main.rs"),
        "Expected file path to be src/main.rs"
    );
    assert!(
        result.files[0].parent_id.is_some(),
        "Single file should have a parent module/subsystem"
    );
}
