// Framework detection — manifest readers for 11 ecosystems.
// Pure extraction helpers live in `framework_readers`.
// Static lookup table and dep_matches live in `framework_profiles`.
// Django/Go subsystem detection lives in `detectors`.

mod detectors;

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::extraction::framework_profiles::{dep_matches, framework_lookup_table};
use crate::extraction::framework_readers::{
    extract_cargo_deps, extract_csproj_deps, extract_gemfile_deps, extract_go_mod_deps,
    extract_gradle_deps, extract_mix_deps, extract_pom_artifact_ids, extract_pyproject_deps,
    extract_requirements_deps, extract_swift_deps, read_file_limited,
};

use detectors::{detect_django_apps, detect_go_layout};

fn default_doc_template() -> String {
    "generic".to_string()
}

/// Typed framework profile produced by manifest analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameworkProfile {
    pub name: String,
    pub module_depth: usize,
    pub subsystems: Vec<(String, String)>, // (relative_path, display_name)
    pub key_files: Vec<String>,
    /// Document template variant for the documentation pipeline.
    /// One of: "mvc_backend", "frontend_app", or "generic".
    #[serde(default = "default_doc_template")]
    pub doc_template: String,
    /// Indicates whether this framework performs dynamic filesystem scanning
    /// to detect subsystems (e.g., Django scans for apps/ directories with
    /// models.py + views.py, Go frameworks scan for cmd/internal/pkg layout).
    /// When true, the profile is returned even if no static subsystems are found,
    /// since dynamic scanning may add them later.
    #[serde(default = "default_has_dynamic_subsystem_detection")]
    pub has_dynamic_subsystem_detection: bool,
}

fn default_has_dynamic_subsystem_detection() -> bool {
    false
}

/// Try each ecosystem's manifest in turn.
///
/// Returns `(ecosystem_tag, dependency_names)` for the first manifest found,
/// or `Ok(None)` when no recognized manifest exists.
pub fn read_manifest(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    // Priority order: most specific first
    if let Some(r) = read_gemfile(repo_path)? {
        return Ok(Some(r));
    }
    // pyproject.toml takes priority over requirements.txt, but only when it
    // actually contains dependencies. An empty dep list (e.g. a build-only
    // pyproject.toml) must fall through so requirements.txt is consulted.
    if let Some((tag, deps)) = read_pyproject_toml(repo_path)?
        && !deps.is_empty()
    {
        return Ok(Some((tag, deps)));
    }
    if let Some(r) = read_requirements_txt(repo_path)? {
        return Ok(Some(r));
    }
    if let Some(r) = read_package_json(repo_path)? {
        return Ok(Some(r));
    }
    if let Some(r) = read_composer_json(repo_path)? {
        return Ok(Some(r));
    }
    if let Some(r) = read_go_mod(repo_path)? {
        return Ok(Some(r));
    }
    if let Some(r) = read_pom_xml(repo_path)? {
        return Ok(Some(r));
    }
    if let Some(r) = read_build_gradle(repo_path)? {
        return Ok(Some(r));
    }
    if let Some(r) = read_mix_exs(repo_path)? {
        return Ok(Some(r));
    }
    if let Some(r) = read_csproj(repo_path)? {
        return Ok(Some(r));
    }
    if let Some(r) = read_package_swift(repo_path)? {
        return Ok(Some(r));
    }
    if let Some(r) = read_cargo_toml(repo_path)? {
        return Ok(Some(r));
    }
    Ok(None)
}

/// Detect framework from manifest using the static lookup table.
///
/// Reads the manifest to get `(ecosystem, deps)`, then finds the first entry
/// in the lookup table where the ecosystem matches and any dep matches the key.
/// Match semantics depend on ecosystem — see `dep_matches` for details.
///
/// After a static match, post-processes Django and Go projects to detect
/// subsystems from the filesystem (apps for Django, standard layout for Go).
pub fn detect_framework_static(repo_path: &Path) -> Result<Option<FrameworkProfile>> {
    let manifest = read_manifest(repo_path)?;

    if let Some((ref ecosystem, ref deps)) = manifest {
        let table = framework_lookup_table();
        for (eco, dep_key, mut profile) in table {
            if eco != ecosystem.as_str() {
                continue;
            }
            let needle = dep_key.to_lowercase();
            if deps.iter().any(|d| dep_matches(eco, d, &needle)) {
                match profile.name.as_str() {
                    "Django" => {
                        let apps = detect_django_apps(repo_path);
                        if !apps.is_empty() {
                            profile.subsystems = apps;
                        }
                    }
                    "Gin" | "Echo" => {
                        let layout = detect_go_layout(repo_path);
                        if !layout.is_empty() {
                            profile.subsystems = layout;
                        }
                    }
                    _ => {}
                }
                return Ok(Some(profile));
            }
        }

        // No framework match — but Go standard layout applies to all Go projects.
        if ecosystem == "go" {
            let layout = detect_go_layout(repo_path);
            if !layout.is_empty() {
                return Ok(Some(FrameworkProfile {
                    name: "Go".to_string(),
                    module_depth: 1,
                    subsystems: layout,
                    key_files: vec!["go.mod".to_string(), "main.go".to_string()],
                    doc_template: "generic".to_string(),
                    has_dynamic_subsystem_detection: false,
                }));
            }
        }
    }

    Ok(None)
}

// ── Individual manifest readers ────────────────────────────────────────────

fn read_gemfile(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("Gemfile"))? else {
        return Ok(None);
    };
    Ok(Some(("ruby".to_string(), extract_gemfile_deps(&content))))
}

fn read_requirements_txt(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("requirements.txt"))? else {
        return Ok(None);
    };
    Ok(Some((
        "python".to_string(),
        extract_requirements_deps(&content),
    )))
}

fn read_pyproject_toml(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("pyproject.toml"))? else {
        return Ok(None);
    };
    let value: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(Some(("python".to_string(), vec![]))),
    };
    Ok(Some(("python".to_string(), extract_pyproject_deps(&value))))
}

fn read_package_json(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("package.json"))? else {
        return Ok(None);
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(Some(("javascript".to_string(), vec![]))),
    };
    let mut deps = Vec::new();
    for section in &["dependencies", "devDependencies"] {
        if let Some(obj) = json.get(section).and_then(|v| v.as_object()) {
            for key in obj.keys() {
                deps.push(key.clone());
            }
        }
    }
    deps.sort();
    deps.dedup();
    Ok(Some(("javascript".to_string(), deps)))
}

fn read_composer_json(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("composer.json"))? else {
        return Ok(None);
    };
    let json: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(Some(("php".to_string(), vec![]))),
    };
    let mut deps = Vec::new();
    if let Some(obj) = json.get("require").and_then(|v| v.as_object()) {
        for key in obj.keys() {
            deps.push(key.clone());
        }
    }
    deps.sort();
    Ok(Some(("php".to_string(), deps)))
}

fn read_go_mod(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("go.mod"))? else {
        return Ok(None);
    };
    Ok(Some(("go".to_string(), extract_go_mod_deps(&content))))
}

fn read_pom_xml(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("pom.xml"))? else {
        return Ok(None);
    };
    Ok(Some((
        "java".to_string(),
        extract_pom_artifact_ids(&content),
    )))
}

fn read_build_gradle(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let kts = repo_path.join("build.gradle.kts");
    let plain = repo_path.join("build.gradle");
    let path = if kts.exists() {
        kts
    } else if plain.exists() {
        plain
    } else {
        return Ok(None);
    };
    let Some(content) = read_file_limited(&path)? else {
        return Ok(None);
    };
    Ok(Some(("java".to_string(), extract_gradle_deps(&content))))
}

fn read_mix_exs(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("mix.exs"))? else {
        return Ok(None);
    };
    Ok(Some(("elixir".to_string(), extract_mix_deps(&content))))
}

fn read_csproj(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    // Collect all .csproj entries, sort for determinism, pick first.
    let entries = match fs::read_dir(repo_path) {
        Ok(e) => e,
        Err(_) => return Ok(None),
    };
    let mut csproj_files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("csproj"))
        .collect();
    csproj_files.sort_by_key(|p| p.file_name().map(|n| n.to_os_string()));
    let path = match csproj_files.into_iter().next() {
        Some(p) => p,
        None => return Ok(None),
    };
    let Some(content) = read_file_limited(&path)? else {
        return Ok(None);
    };
    Ok(Some(("csharp".to_string(), extract_csproj_deps(&content))))
}

fn read_package_swift(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("Package.swift"))? else {
        return Ok(None);
    };
    Ok(Some(("swift".to_string(), extract_swift_deps(&content))))
}

fn read_cargo_toml(repo_path: &Path) -> Result<Option<(String, Vec<String>)>> {
    let Some(content) = read_file_limited(&repo_path.join("Cargo.toml"))? else {
        return Ok(None);
    };
    Ok(Some(("rust".to_string(), extract_cargo_deps(&content))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_django_app(base: &Path, name: &str) {
        let dir = base.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("models.py"), "").unwrap();
        fs::write(dir.join("views.py"), "").unwrap();
    }

    #[test]
    fn test_detect_django_apps_skips_venv() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // venv with models.py + views.py should be skipped
        make_django_app(root, "venv");
        // real app
        make_django_app(root, "blog");

        let apps = detect_django_apps(root);
        let names: Vec<&str> = apps.iter().map(|(_, n)| n.as_str()).collect();
        assert!(!names.contains(&"venv"), "venv should be skipped");
        assert!(names.contains(&"blog"), "blog should be detected");
    }

    #[test]
    fn test_detect_django_apps_skips_hidden_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // hidden dir with models.py + views.py should be skipped
        make_django_app(root, ".hidden");
        // real app
        make_django_app(root, "users");

        let apps = detect_django_apps(root);
        let names: Vec<&str> = apps.iter().map(|(_, n)| n.as_str()).collect();
        assert!(!names.contains(&".hidden"), "hidden dir should be skipped");
        assert!(names.contains(&"users"), "users should be detected");
    }

    #[test]
    fn test_framework_profile_falls_through_when_subsystems_empty_or_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create a FastAPI project with pyproject.toml but no conventional dirs
        let pyproject = root.join("pyproject.toml");
        fs::write(
            &pyproject,
            r#"[project]
 dependencies = ["fastapi"]
"#,
        )
        .unwrap();
        // Create a Python package so fallback would find something
        let myapp_dir = root.join("myapp");
        fs::create_dir_all(&myapp_dir).unwrap();
        fs::write(myapp_dir.join("__init__.py"), "").unwrap();

        // FastAPI profile matches but all subsystems paths don't exist
        // This should return a profile (detect_framework_static itself doesn't filter),
        // but detect_subsystems will filter and fall through
        let result = detect_framework_static(root).unwrap();
        assert!(
            result.is_some(),
            "detect_framework_static should return profile, filtering happens in detect_subsystems"
        );
    }
}
