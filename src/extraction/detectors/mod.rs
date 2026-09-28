// Subsystem boundary detectors - identifies logical subsystem groupings in repos

mod cargo;
mod js_ts;
mod npm;
mod python;
mod top_level;

use std::collections::HashMap;
use std::path::Path;

use crate::error::Result;
use crate::extraction::framework::{FrameworkProfile, detect_framework_static};

use js_ts::detect_js_ts_subsystems; // called here directly and by npm.rs's supplement

pub use cargo::detect_cargo_workspace;
pub use npm::detect_npm_workspace;
pub(crate) use npm::expand_workspace_glob;
pub use python::detect_python_packages;
pub use top_level::detect_top_level_src;

/// Detect subsystem boundaries for a repository.
///
/// Tries detectors in priority order: Cargo workspace, npm workspace,
/// framework lookup table, JS/TS top-level source dirs, Python packages,
/// top-level src/ subdirs, fallback (entire repo).
///
/// The JS/TS detector runs AFTER the framework table: framework-defined
/// subsystems (Rails, Laravel, NestJS, ...) take precedence over the generic
/// JS/TS top-level scan, so a Laravel repo keeps its framework profile while
/// a plain JS/TS or Next.js repo (no framework match) still gets a
/// multi-module hierarchy from its src/ layout.
///
/// Returns `(subsystem_map, module_depth, framework_profile)` where
/// `module_depth` is the inferred default depth for the detected project type
/// and `framework_profile` is populated by the manifest lookup table.
pub fn detect_subsystems(
    repo_path: &Path,
) -> Result<(HashMap<String, String>, usize, Option<FrameworkProfile>)> {
    if let Some(subsystems) = detect_cargo_workspace(repo_path) {
        return Ok((subsystems, 1, None));
    }

    if let Some(subsystems) = detect_npm_workspace(repo_path) {
        return Ok((subsystems, 1, None));
    }

    // A matched framework profile that produced no usable subsystems keeps
    // its name and doc_template through the JS/TS scan, the Python package
    // scan, and the top-level src/ scan below — only the bare repo-name
    // fallback at the end drops it.
    let mut surviving_profile = None;

    // Framework lookup table: covers Rails, Django, Next.js, Laravel, Spring Boot, etc.
    if let Some(profile) = detect_framework_static(repo_path)? {
        // Filter subsystems to only include directories that actually exist,
        // so callers get a map reflecting the real repo layout.
        let depth = profile.module_depth;
        let subsystems: HashMap<String, String> = profile
            .subsystems
            .iter()
            .filter(|(path, _)| repo_path.join(path).exists())
            .map(|(path, name)| (path.clone(), name.clone()))
            .collect();

        // Django and Go profiles do dynamic filesystem scanning that adds subsystems.
        // For other frameworks, if filtering removes all subsystems, fall through to
        // detect_python_packages(). The profile itself is preserved either way: a
        // profile-degenerate repo (profile matched, zero subsystems) keeps its name
        // and doc_template, so downstream doc generation still sees the framework.
        if !subsystems.is_empty() || profile.has_dynamic_subsystem_detection {
            return Ok((subsystems, depth, Some(profile)));
        }
        // Profile matched but produced no subsystems - fall through to next
        // detector, keeping the profile so the name/doc_template survive.
        surviving_profile = Some(profile);
    }

    // JS/TS repos (package.json without workspaces, no framework profile):
    // one subsystem per top-level directory containing JS/TS source.
    // Runs after the framework table so framework-defined layouts (Laravel,
    // Rails, NestJS) are not pre-empted by the generic scan.
    if let Some(dirs) = detect_js_ts_subsystems(repo_path) {
        let mut subsystems = HashMap::new();
        subsystems.insert(".".to_string(), "root".to_string());
        for (rel_path, name) in dirs {
            subsystems.insert(rel_path, name);
        }
        return Ok((subsystems, 2, surviving_profile));
    }

    if let Some((subsystems, depth)) = detect_python_packages(repo_path) {
        return Ok((subsystems, depth, surviving_profile));
    }

    if let Some(subsystems) = detect_top_level_src(repo_path) {
        return Ok((subsystems, 1, surviving_profile));
    }

    // Fallback: entire repo as one subsystem
    let repo_name = repo_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("root");

    let mut subsystems = HashMap::new();
    subsystems.insert(".".to_string(), repo_name.to_string());
    Ok((subsystems, 1, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_rails_repo(dir: &std::path::Path, use_spec: bool) {
        fs::write(dir.join("Gemfile"), "gem 'rails', '~> 7.0'\n").unwrap();
        fs::create_dir_all(dir.join("config")).unwrap();
        fs::write(dir.join("config/application.rb"), "# app\n").unwrap();
        fs::create_dir_all(dir.join("app")).unwrap();
        fs::create_dir_all(dir.join(if use_spec { "spec" } else { "test" })).unwrap();
    }

    #[test]
    fn test_detect_rails_with_spec_dir() {
        let tmp = TempDir::new().unwrap();
        make_rails_repo(tmp.path(), true);

        let (subsystems, depth, _profile) = detect_subsystems(tmp.path()).unwrap();
        assert_eq!(depth, 2);
        assert_eq!(
            subsystems.get(".").map(String::as_str),
            Some("infrastructure")
        );
        assert_eq!(subsystems.get("app").map(String::as_str), Some("app"));
        assert_eq!(subsystems.get("spec").map(String::as_str), Some("testing"));
        assert!(!subsystems.contains_key("test"));
    }

    #[test]
    fn test_detect_rails_with_test_dir() {
        let tmp = TempDir::new().unwrap();
        make_rails_repo(tmp.path(), false);

        let (subsystems, depth, _profile) = detect_subsystems(tmp.path()).unwrap();
        assert_eq!(depth, 2);
        assert_eq!(subsystems.get("test").map(String::as_str), Some("testing"));
        assert!(!subsystems.contains_key("spec"));
    }

    #[test]
    fn test_detect_non_rails_ruby_returns_depth_1() {
        let tmp = TempDir::new().unwrap();
        // Sinatra Gemfile — no rails dep → not detected as Rails
        fs::write(tmp.path().join("Gemfile"), "gem 'sinatra'\n").unwrap();

        let (_subsystems, depth, _profile) = detect_subsystems(tmp.path()).unwrap();
        assert_ne!(depth, 2, "Sinatra should not be detected as Rails");
    }

    #[test]
    fn test_detect_rails_profile_returned() {
        let tmp = TempDir::new().unwrap();
        make_rails_repo(tmp.path(), true);

        let (_subsystems, _depth, profile) = detect_subsystems(tmp.path()).unwrap();
        let profile = profile.expect("Rails profile should be Some");
        assert_eq!(profile.name, "Rails");
        assert_eq!(profile.module_depth, 2);
    }

    #[test]
    fn test_detect_subsystems_fallback_returns_depth_1() {
        let tmp = TempDir::new().unwrap();
        let (_subsystems, depth, _profile) = detect_subsystems(tmp.path()).unwrap();
        assert_eq!(depth, 1);
    }

    #[test]
    fn test_detect_subsystems_profile_degenerate_repo_preserves_profile() {
        // A profile-degenerate repo: the framework profile matches but none of
        // its subsystem dirs exist, so the framework branch falls through to
        // the JS/TS scan. The profile (name + doc_template) must still be
        // returned so downstream doc generation sees the framework.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Next.js profile: manifest match requires the "next" dep; its
        // subsystems list is empty, so the framework branch falls through.
        fs::write(
            root.join("package.json"),
            r#"{"name": "app", "dependencies": {"next": "14.0.0"}}"#,
        )
        .unwrap();
        // JS/TS source so the JS/TS branch (the fall-through target) fires.
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.tsx"), "// tsx\n").unwrap();

        let (subsystems, depth, profile) = detect_subsystems(root).unwrap();
        let profile = profile.expect("profile must survive the degenerate fall-through");
        assert_eq!(profile.name, "Next.js");
        assert_eq!(profile.doc_template, "frontend_app");
        assert_eq!(depth, 2);
        assert!(subsystems.contains_key("src"));
    }

    fn make_python_package(dir: &std::path::Path, name: &str) {
        let pkg = dir.join(name);
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("__init__.py"), "").unwrap();
    }

    fn make_python_sub_package(dir: &std::path::Path, pkg: &str, sub: &str) {
        let sub_dir = dir.join(pkg).join(sub);
        fs::create_dir_all(&sub_dir).unwrap();
        fs::write(sub_dir.join("__init__.py"), "").unwrap();
    }

    #[test]
    fn test_python_flat_package_returns_depth_1() {
        let tmp = TempDir::new().unwrap();
        make_python_package(tmp.path(), "myapp");

        let result = detect_python_packages(tmp.path());
        assert!(result.is_some());
        let (subsystems, depth) = result.unwrap();
        assert_eq!(depth, 1);
        assert!(subsystems.contains_key("myapp"));
    }

    #[test]
    fn test_python_nested_package_returns_depth_2() {
        let tmp = TempDir::new().unwrap();
        make_python_package(tmp.path(), "myapp");
        make_python_sub_package(tmp.path(), "myapp", "core");

        let result = detect_python_packages(tmp.path());
        assert!(result.is_some());
        let (subsystems, depth) = result.unwrap();
        assert_eq!(depth, 2);
        assert!(subsystems.contains_key("myapp"));
    }

    #[test]
    fn test_python_tests_dir_detected() {
        let tmp = TempDir::new().unwrap();
        make_python_package(tmp.path(), "myapp");
        fs::create_dir_all(tmp.path().join("tests")).unwrap();

        let result = detect_python_packages(tmp.path());
        assert!(result.is_some());
        let (subsystems, _depth) = result.unwrap();
        assert_eq!(subsystems.get("tests").map(String::as_str), Some("testing"));
    }

    #[test]
    fn test_python_test_dir_detected() {
        let tmp = TempDir::new().unwrap();
        make_python_package(tmp.path(), "myapp");
        fs::create_dir_all(tmp.path().join("test")).unwrap();

        let result = detect_python_packages(tmp.path());
        assert!(result.is_some());
        let (subsystems, _depth) = result.unwrap();
        assert_eq!(subsystems.get("test").map(String::as_str), Some("testing"));
    }

    #[test]
    fn test_python_venv_with_init_py_not_detected_as_package() {
        let tmp = TempDir::new().unwrap();
        make_python_package(tmp.path(), "myapp");
        make_python_package(tmp.path(), "venv");

        let result = detect_python_packages(tmp.path());
        assert!(result.is_some());
        let (subsystems, _depth) = result.unwrap();
        assert!(!subsystems.contains_key("venv"));
        assert!(subsystems.contains_key("myapp"));
    }
}
