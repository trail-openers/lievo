// JS/TS source-root detection (issue #855).
//
// A "source root" is a repo-relative directory that webpack-style tooling
// treats as a module root: bare specifiers whose first segment names a
// directory under the root resolve into files in that root (e.g. a
// Shakapacker `source_path: app/javascript` makes `shop/components/X`
// resolve to `app/javascript/shop/components/X`), applying the resolver's
// existing extension/index rules.
//
// Roots are DETECTED from real configuration — never from executing
// webpack JS config (executable code is out of scope) — in this order of
// preference (binding decision #2, issue #855):
//
// 1. tsconfig.json/jsconfig.json `compilerOptions.baseUrl` (tsconfig wins
//    over jsconfig — the same precedence `load_path_aliases` uses),
// 2. Webpacker/Shakapacker `source_path` (config/webpacker.yml OR
//    config/shakapacker.yml — Shakapacker is the Webpacker 7+ rename; the
//    files use `default: &default` with per-environment `<<: *default`
//    merges, so only the `default` section is read),
// 3. the same file's `additional_paths`, in YAML order.
//
// The result is ONE ordered list; the resolver tries each root through the
// shared `try_candidate` rules and the first root producing an existing
// file wins. A tsconfig.json with no baseUrl must not stop the
// Webpacker-family root from being used.

use crate::extraction::detectors::{detect_npm_workspace, expand_workspace_glob};
use std::collections::HashSet;
use std::path::Path;

/// Detect the ordered JS/TS source roots under `repo_root` (see the
/// module docs for the detection order and precedence).
///
/// Paths are repo-relative POSIX strings. Duplicates are dropped (first
/// occurrence wins); entries that do not name an existing directory are
/// dropped — an absent root can never produce a resolution.
pub fn detect(repo_root: &Path) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    for candidate in tsconfig_base_url(repo_root)
        .into_iter()
        .chain(webpacker_source_roots(repo_root))
    {
        let candidate = normalize(&candidate);
        if candidate.is_empty() || roots.iter().any(|r| r == &candidate) {
            continue;
        }
        // A root that is not an existing directory cannot resolve anything.
        if repo_root.join(&candidate).is_dir() {
            roots.push(candidate);
        }
    }
    roots
}

/// `compilerOptions.baseUrl` from tsconfig.json (winning) or jsconfig.json
/// (only when tsconfig.json is absent — the same rule `load_path_aliases`
/// applies to `paths`). A tsconfig present without baseUrl contributes
/// nothing; it does NOT block the Webpacker-family sources below.
fn tsconfig_base_url(repo_root: &Path) -> Option<String> {
    let tsconfig = repo_root.join("tsconfig.json");
    let config = if tsconfig.exists() {
        tsconfig
    } else {
        let jsconfig = repo_root.join("jsconfig.json");
        if jsconfig.exists() {
            jsconfig
        } else {
            return None;
        }
    };
    let content = std::fs::read_to_string(&config).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("compilerOptions")
        .and_then(|co| co.get("baseUrl"))
        .and_then(|b| b.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Webpacker/Shakapacker `source_path` then `additional_paths` (YAML order)
/// from `config/webpacker.yml` (falling back to `config/shakapacker.yml`
/// when only that exists). Only the `default` section is read; the
/// per-environment `<<: *default` merges are ignored.
fn webpacker_source_roots(repo_root: &Path) -> Vec<String> {
    let shakapacker = repo_root.join("config").join("shakapacker.yml");
    let webpacker = repo_root.join("config").join("webpacker.yml");
    let config = if webpacker.exists() {
        webpacker
    } else if shakapacker.exists() {
        shakapacker
    } else {
        return Vec::new();
    };
    let content = match std::fs::read_to_string(&config) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let value: serde_yaml_ng::Mapping =
        match serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(&content) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
    let default = match value.get("default") {
        Some(d) => d,
        None => return Vec::new(),
    };
    let map = match default.as_mapping() {
        Some(m) => m,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    if let Some(sp) = map.get("source_path").and_then(|v| v.as_str()) {
        out.push(sp.to_string());
    }
    if let Some(extra) = map.get("additional_paths").and_then(|v| v.as_sequence()) {
        for item in extra {
            if let Some(p) = item.as_str() {
                out.push(p.to_string());
            }
        }
    }
    out
}

/// Collapse `.`/`..` and duplicate slashes in a repo-relative POSIX path
/// (shared with the resolver's normalisation rules; a small 3-line helper,
/// deliberately not re-exported to avoid an import cycle in tests).
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            parts.pop();
        } else {
            parts.push(seg);
        }
    }
    parts.join("/")
}

/// True when a path that looks like a directory prefix is present in the
/// known-path set as a directory. A repo-relative directory "a/b" is
/// satisfied by any known file at "a/b/…" (a file or index file directly
/// inside it), because the resolver only ever resolves into files, so the
/// "is this a directory" question is answered by "does any known path sit
/// inside it". This is what lets the `matches_source_root` predicate answer
/// the honesty classification without a separate filesystem walk.
pub fn known_paths_contains_dir(known_paths: &HashSet<String>, dir_prefix: &str) -> bool {
    let needle = format!("{dir_prefix}/");
    known_paths.iter().any(|p| p.starts_with(&needle))
}

/// The UNION of declared package names from the root package.json AND every
/// workspace member's package.json — dependencies, devDependencies and
/// peerDependencies combined (issue #855 guard #4). A bare specifier whose
/// first segment is in this set is never resolved against a source root.
/// Reuses the member list `detect_npm_workspace` / pnpm-workspace.yaml builds,
/// so the guard covers exactly the members the resolver already knows about.
pub fn load_declared_dependencies(repo_root: &Path) -> HashSet<String> {
    let mut deps: HashSet<String> = HashSet::new();
    // Root package.json.
    deps.extend(read_manifest_dep_names(repo_root, ""));
    // Workspace members (npm `workspaces` + pnpm-workspace.yaml).
    if let Some(npm_map) = detect_npm_workspace(repo_root) {
        for rel_path in npm_map.keys() {
            if rel_path == "." {
                continue;
            }
            deps.extend(read_manifest_dep_names(repo_root, rel_path));
        }
    }
    let pnpm_path = repo_root.join("pnpm-workspace.yaml");
    if pnpm_path.exists()
        && let Ok(content) = std::fs::read_to_string(&pnpm_path)
    {
        #[derive(serde::Deserialize)]
        struct PnpmWorkspace {
            packages: Option<Vec<String>>,
        }
        if let Ok(ws) = serde_yaml_ng::from_str::<PnpmWorkspace>(&content) {
            for pattern in ws.packages.unwrap_or_default() {
                for (rel_path, _) in expand_workspace_glob(repo_root, &pattern) {
                    deps.extend(read_manifest_dep_names(repo_root, &rel_path));
                }
            }
        }
    }
    deps
}

/// Read the dependency-name union (dependencies/devDependencies/peerDependencies
/// keys) from a package.json at `repo_root.join(rel_path)`. `rel_path` is
/// empty for the root manifest. Returns an empty set when the file is absent
/// or malformed.
fn read_manifest_dep_names(repo_root: &Path, rel_path: &str) -> HashSet<String> {
    let path = if rel_path.is_empty() {
        repo_root.join("package.json")
    } else {
        repo_root.join(rel_path).join("package.json")
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return HashSet::new(),
    };
    let mut out = HashSet::new();
    for key in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(obj) = value.get(key).and_then(|v| v.as_object()) {
            for name in obj.keys() {
                out.insert(name.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(repo: &Path, rel: &str, content: &str) {
        let full = repo.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    const ANCHORED_YML: &str = r#"
default: &default
  source_path: app/javascript
  additional_paths: ['app/assets', 'lib/frontend']
development:
  <<: *default
  compile: true
production:
  <<: *default
"#;

    #[test]
    fn detect_shakapacker_only_source_path_and_additional_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        write(tmp.path(), "app/javascript/x.js", "export {};\n");
        write(tmp.path(), "app/assets/a.css", "/* */\n");
        write(tmp.path(), "lib/frontend/l.js", "export {};\n");
        write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
        assert_eq!(
            detect(tmp.path()),
            vec![
                "app/javascript".to_string(),
                "app/assets".to_string(),
                "lib/frontend".to_string()
            ]
        );
    }

    #[test]
    fn detect_webpacker_yml_when_both_exist_webpacker_wins() {
        let tmp = tempfile::TempDir::new().unwrap();
        write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
        write(
            tmp.path(),
            "config/webpacker.yml",
            "default:\n  source_path: src\n",
        );
        write(tmp.path(), "src/main.js", "export {};\n");
        assert_eq!(detect(tmp.path()), vec!["src".to_string()]);
    }

    #[test]
    fn detect_base_url_beats_webpacker_source_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        write(
            tmp.path(),
            "jsconfig.json",
            r#"{"compilerOptions": {"baseUrl": "src"}}"#,
        );
        write(
            tmp.path(),
            "config/webpacker.yml",
            "default:\n  source_path: app/javascript\n",
        );
        write(tmp.path(), "src/a.js", "export {};\n");
        write(tmp.path(), "app/javascript/b.js", "export {};\n");
        assert_eq!(
            detect(tmp.path()),
            vec!["src".to_string(), "app/javascript".to_string()]
        );
    }

    #[test]
    fn detect_tsconfig_wins_over_jsconfig_base_url() {
        let tmp = tempfile::TempDir::new().unwrap();
        write(
            tmp.path(),
            "tsconfig.json",
            r#"{"compilerOptions": {"baseUrl": "src", "paths": {}}}"#,
        );
        write(
            tmp.path(),
            "jsconfig.json",
            r#"{"compilerOptions": {"baseUrl": "other"}}"#,
        );
        write(tmp.path(), "src/a.js", "export {};\n");
        assert_eq!(detect(tmp.path()), vec!["src".to_string()]);
    }

    #[test]
    fn detect_tsconfig_without_base_url_does_not_block_webpacker_root() {
        // The www-repo shape: tsconfig.json exists (no baseUrl, no paths)
        // and only shakapacker.yml carries the source root.
        let tmp = tempfile::TempDir::new().unwrap();
        write(
            tmp.path(),
            "tsconfig.json",
            r#"{"compilerOptions": {"experimentalDecorators": true, "allowJs": true}}"#,
        );
        write(tmp.path(), "app/javascript/x.js", "export {};\n");
        write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
        write(tmp.path(), "app/assets/a.css", "/* */\n");
        write(tmp.path(), "lib/frontend/l.js", "export {};\n");
        assert_eq!(
            detect(tmp.path()),
            vec![
                "app/javascript".to_string(),
                "app/assets".to_string(),
                "lib/frontend".to_string()
            ]
        );
    }

    #[test]
    fn detect_missing_config_yields_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(detect(tmp.path()).is_empty());
    }

    #[test]
    fn detect_nonexistent_root_entry_dropped() {
        let tmp = tempfile::TempDir::new().unwrap();
        write(tmp.path(), "src/a.js", "export {};\n");
        write(
            tmp.path(),
            "config/webpacker.yml",
            "default:\n  source_path: src\n  additional_paths: ['missing/dir']\n",
        );
        assert_eq!(detect(tmp.path()), vec!["src".to_string()]);
    }

    #[test]
    fn detect_per_environment_override_of_source_path_ignored() {
        // The `default` section is the sole source; a per-environment
        // override of source_path must not appear in the result.
        let tmp = tempfile::TempDir::new().unwrap();
        write(tmp.path(), "src/a.js", "export {};\n");
        write(
            tmp.path(),
            "config/shakapacker.yml",
            "default: &default\n  source_path: src\ndevelopment:\n  <<: *default\n  source_path: other\n",
        );
        assert_eq!(detect(tmp.path()), vec!["src".to_string()]);
    }

    #[test]
    fn detect_duplicate_root_deduplicated() {
        let tmp = tempfile::TempDir::new().unwrap();
        write(tmp.path(), "src/a.js", "export {};\n");
        write(
            tmp.path(),
            "jsconfig.json",
            r#"{"compilerOptions": {"baseUrl": "src"}}"#,
        );
        write(
            tmp.path(),
            "config/webpacker.yml",
            "default:\n  source_path: src\n",
        );
        assert_eq!(detect(tmp.path()), vec!["src".to_string()]);
    }
}
