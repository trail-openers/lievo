// Tier-1 cross-file import resolution for JavaScript/TypeScript.
//
// Companion to `relationship_helpers::resolve_import` (Rust path — unchanged).
// `JsResolverContext::resolve` resolves a bare JS/TS specifier (the extractor
// emits the `source`/`argument` field verbatim after #677) to a file entity id
// using, in order:
//
// 1. tsconfig.json/jsconfig.json `paths` aliases (tsconfig wins; jsconfig is
//    consulted only when tsconfig.json is absent — both root-level only),
// 2. extension permutations relative to the importing file (fixed priority:
//    .js, .jsx, .ts, .tsx, .mjs, .cjs, then index.js, index.ts, index.jsx,
//    index.tsx, index.mjs),
// 3. workspace-member resolution for bare package specifiers (package.json
//    workspaces via `detect_npm_workspace`; pnpm-workspace.yaml members via
//    `serde_yaml_ng` + `expand_workspace_glob`).
//
// Resolution is against the in-memory entity path set: resolved repo-relative
// paths are mapped back through the entity id map, so a file on disk that
// grouping did not index can never produce an edge to a nonexistent entity.
//
// Unresolvable specifiers are NOT guessed: they increment a side-channel
// counter (`UnresolvedCounts`, split internal/external per the #690 amendment)
// that `RelationshipBuilder` exposes via `last_unresolved`.

use crate::analysis::js_source_root;
use crate::extraction::detectors::{detect_npm_workspace, expand_workspace_glob};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::relationship_helpers::build_import_map;

/// Extension permutation priority. First match wins — the list is fixed so
/// results are deterministic (no HashMap iteration order involved).
const JS_EXTENSIONS: &[&str] = &[".js", ".jsx", ".ts", ".tsx", ".mjs", ".cjs"];
const JS_INDEX_SUFFIXES: &[&str] = &[
    "index.js",
    "index.ts",
    "index.jsx",
    "index.tsx",
    "index.mjs",
];

/// Side-channel unresolved-import counters (per the #690 amendment).
/// `internal` = relative/alias specifiers that resolved to nothing;
/// `external` = bare package specifiers matching no workspace member
/// (third-party deps, Node built-ins, typos). Neither category produces an edge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnresolvedCounts {
    pub internal: u32,
    pub external: u32,
}

/// Which unresolved bucket a failed specifier belongs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedKind {
    Internal,
    External,
}

impl UnresolvedKind {
    pub fn apply(self, counts: &mut UnresolvedCounts) {
        match self {
            UnresolvedKind::Internal => counts.internal = counts.internal.saturating_add(1),
            UnresolvedKind::External => counts.external = counts.external.saturating_add(1),
        }
    }
}

/// Precomputed per-repo resolver context: entity path set, aliases,
/// workspace members, declared dependencies, and source roots (issue #855).
/// Built once — all filesystem reads happen here, so per-import resolution
/// is pure in-memory lookup.
pub struct JsResolverContext {
    /// raw repo-relative JS/TS file path → file entity id
    path_map: HashMap<String, String>,
    /// repo-relative paths known to exist (entity paths ∪ on-disk index.* files)
    known_paths: HashSet<String>,
    /// alias key → ordered target templates (file order, sorted keys)
    aliases: Vec<(String, Vec<String>)>,
    /// workspace member package name → member directory (repo-relative)
    workspace_members: HashMap<String, String>,
    /// declared package names (root ∪ workspace members, union of
    /// dependencies/devDependencies/peerDependencies) — guard #4 (#855)
    declared_dependencies: HashSet<String>,
    /// detected JS source roots in deterministic preference order (#855)
    source_roots: Vec<String>,
}

impl JsResolverContext {
    pub fn new(files: &[crate::model::Entity], repo_root: &Path) -> Self {
        // build_import_map gives raw-path keys (all languages) + Rust module
        // keys; keep only keys that look like JS/TS file paths.
        let import_map = build_import_map(files, "", "");
        let path_map: HashMap<String, String> = import_map
            .iter()
            .filter(|(k, _)| {
                let k = k.as_str();
                JS_EXTENSIONS
                    .iter()
                    .chain(JS_INDEX_SUFFIXES)
                    .any(|ext| k.ends_with(ext))
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        // Known paths: all entity paths + any on-disk index.* file grouping
        // did not index (directory-index resolution target; mapped back
        // through path_map, so unindexed files resolve to no entity id).
        let mut known_paths: HashSet<String> =
            files.iter().filter_map(|f| f.path.clone()).collect();
        let repo_root_abs = repo_root
            .canonicalize()
            .unwrap_or_else(|_| repo_root.to_path_buf());
        let mut index_paths: Vec<String> = Vec::new();
        collect_index_files(&repo_root_abs, &repo_root_abs, &mut index_paths, 0);
        known_paths.extend(index_paths);

        Self {
            path_map,
            known_paths,
            aliases: load_path_aliases(repo_root),
            workspace_members: load_workspace_members(repo_root),
            declared_dependencies: js_source_root::load_declared_dependencies(repo_root),
            source_roots: js_source_root::detect(repo_root),
        }
    }

    /// Resolve a JS/TS specifier to a file entity id.
    /// `source_file` is the repo-relative path of the importing file.
    /// Returns None for unresolvable specifiers (the caller counts them).
    pub fn resolve(&self, import: &str, source_file: &str) -> Option<&str> {
        // 1. tsconfig/jsconfig paths alias (exact key, then "*" wildcards).
        for candidate in self.alias_candidates(import) {
            if let Some(id) = self.try_candidate(&candidate) {
                return Some(id);
            }
        }

        // 2. Relative specifiers — permutations relative to the importing file.
        //    Handles "./x" (sibling/child) and "../x" (parent) forms.
        //    "./" is stripped before join_relative (redundant when joining onto
        //    the source dir); "../" is passed through as-is (the ".." segment
        //    is the parent-traversal component that join_relative collapses).
        let relative_base: Option<&str> = import
            .strip_prefix("./")
            .or_else(|| import.strip_prefix("../").map(|_| import));
        if let Some(base) = relative_base.map(|rb| join_relative(source_file, rb)) {
            return self.try_candidate(&base);
        }

        // 2b. Absolute specifiers resolve repo-root-relative.
        if import.starts_with('/') && !import.starts_with("//") {
            return self.try_candidate(&normalize_posix_path(import));
        }

        // 3. Bare specifiers — workspace member resolution
        // ("pkg-name" or "pkg-name/sub/path"). Reached when the import is
        // neither "./..." nor an absolute "/..." path (and no alias matched).
        if !import.starts_with('.') {
            let (pkg, rest) = match import.find('/') {
                Some(pos) => import.split_at(pos),
                None => (import, ""),
            };
            if !pkg.is_empty() && !pkg.contains('/') {
                if let Some(member_dir) = self.workspace_members.get(pkg) {
                    let base = if rest.is_empty() {
                        // Bare package name → member's index file.
                        member_dir.to_string()
                    } else {
                        format!("{member_dir}/{rest}")
                    };
                    if let Some(id) = self.try_candidate(&base) {
                        return Some(id);
                    }
                }

                // 4. Source-root resolution (issue #855): bare specifier
                // resolves against a detected JS source root. Declared
                // package dependencies are excluded (guard #4).
                if !self.declared_dependencies.contains(pkg) && !self.source_roots.is_empty() {
                    for root in &self.source_roots {
                        let base = format!("{root}/{import}");
                        if let Some(id) = self.try_candidate(&base) {
                            return Some(id);
                        }
                    }
                }
            }
        }

        None
    }

    /// True when the unresolved bare `import` should count as INTERNAL
    /// (honesty classification, issue #855): first segment is an existing
    /// directory under a detected source root and not a declared dependency.
    pub fn matches_source_root(&self, import: &str) -> bool {
        if import.starts_with('.') || import.starts_with('/') {
            return false;
        }
        let pkg = import.split('/').next().unwrap_or(import);
        if pkg.is_empty() || self.declared_dependencies.contains(pkg) {
            return false;
        }
        self.source_roots.iter().any(|root| {
            js_source_root::known_paths_contains_dir(&self.known_paths, &format!("{root}/{pkg}"))
        })
    }

    /// Apply tsconfig/jsconfig `paths` matching for `import`.
    /// Exact keys are tried first (in sorted-key order); then the most
    /// specific "*" wildcard key, with each of its target templates in order.
    /// Note: tsconfig keys are matched verbatim (no "./*" normalization), so
    /// "@/*" keys match "@/..." specifiers only — "src/*" keys match "src/..."
    /// specifiers, which are not valid bare imports and are harmless here.
    fn alias_candidates(&self, import: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (key, targets) in &self.aliases {
            if key == import {
                out.extend(targets.iter().cloned());
                break;
            }
        }
        if out.is_empty() {
            // Wildcard keys: longest key matching wins (most specific).
            if let Some((key, targets)) = self
                .aliases
                .iter()
                .filter(|(key, _)| key.contains('*'))
                .filter(|(key, _)| {
                    let pos = key.find('*').unwrap();
                    let prefix = &key[..pos];
                    let suffix = &key[pos + 1..];
                    import.starts_with(prefix)
                        && import.ends_with(suffix)
                        && import.len() >= key.len() - 1
                })
                .max_by_key(|(key, _)| key.len())
            {
                let pos = key.find('*').unwrap();
                let mid = import[pos..import.len() - key.len() + pos + 1].to_string();
                for target in targets {
                    match target.find('*') {
                        Some(tpos) => {
                            out.push(format!("{}{}{}", &target[..tpos], mid, &target[tpos + 1..]))
                        }
                        None => out.push(target.clone()),
                    }
                }
            }
        }
        out
    }

    /// Try a base path (possibly without extension) against the known path
    /// set: exact match, then fixed-priority extension permutations, then
    /// directory index files. First hit wins — later permutations never
    /// overwrite an earlier-resolved id.
    fn try_candidate(&self, base: &str) -> Option<&str> {
        let base = normalize_posix_path(base);
        if base.is_empty() {
            return None;
        }
        if self.known_paths.contains(&base) {
            return self.path_map.get(base.as_str()).map(|v| v.as_str());
        }
        for ext in JS_EXTENSIONS {
            let candidate = format!("{base}{ext}");
            if self.known_paths.contains(&candidate) {
                return self.path_map.get(candidate.as_str()).map(|v| v.as_str());
            }
        }
        for suffix in JS_INDEX_SUFFIXES {
            let candidate = format!("{base}/{suffix}");
            if self.known_paths.contains(&candidate) {
                return self.path_map.get(candidate.as_str()).map(|v| v.as_str());
            }
        }
        None
    }
}

/// tsconfig.json (winning) or jsconfig.json (only when tsconfig.json is
/// absent) `paths` entries, root-level only. Returns (alias_key, targets)
/// pairs; keys are sorted so alias precedence is deterministic across builds.
fn load_path_aliases(repo_root: &Path) -> Vec<(String, Vec<String>)> {
    let tsconfig = repo_root.join("tsconfig.json");
    let config = if tsconfig.exists() {
        tsconfig
    } else {
        let jsconfig = repo_root.join("jsconfig.json");
        if jsconfig.exists() {
            jsconfig
        } else {
            return Vec::new();
        }
    };
    let content = match std::fs::read_to_string(&config) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let paths = match value
        .get("compilerOptions")
        .and_then(|co| co.get("paths"))
        .and_then(|p| p.as_object())
    {
        Some(p) => p,
        None => return Vec::new(),
    };
    let mut keys: Vec<&String> = paths.keys().collect();
    keys.sort();
    let mut out = Vec::new();
    for key in keys {
        let targets: Vec<String> = match paths.get(key) {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            _ => Vec::new(),
        };
        if !targets.is_empty() {
            out.push((key.clone(), targets));
        }
    }
    out
}

/// Workspace member map: package "name" (from each member's package.json,
/// falling back to directory basename) → member directory (repo-relative).
/// Sources: package.json `workspaces` (detect_npm_workspace) and
/// pnpm-workspace.yaml `packages` (serde_yaml_ng + expand_workspace_glob).
fn load_workspace_members(repo_root: &Path) -> HashMap<String, String> {
    let mut members: HashMap<String, String> = HashMap::new();

    if let Some(npm_map) = detect_npm_workspace(repo_root) {
        for rel_path in npm_map.keys() {
            if rel_path == "." {
                continue;
            }
            let pkg_name = read_member_package_name(repo_root, rel_path)
                .unwrap_or_else(|| member_dir_basename(rel_path));
            members.entry(pkg_name).or_insert_with(|| rel_path.clone());
        }
    }

    // pnpm-workspace.yaml: `packages: ["packages/*", ...]`
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
                    let pkg_name = read_member_package_name(repo_root, &rel_path)
                        .unwrap_or_else(|| member_dir_basename(&rel_path));
                    members.entry(pkg_name).or_insert_with(|| rel_path.clone());
                }
            }
        }
    }

    members
}

fn member_dir_basename(rel_path: &str) -> String {
    rel_path.rsplit('/').next().unwrap_or(rel_path).to_string()
}

fn read_member_package_name(repo_root: &Path, rel_path: &str) -> Option<String> {
    let pkg = repo_root.join(rel_path).join("package.json");
    let content = std::fs::read_to_string(pkg).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    value
        .get("name")
        .and_then(|n| n.as_str())
        .filter(|n| !n.is_empty())
        .map(String::from)
}

/// Recursively collect `index.{js,ts,jsx,tsx,mjs}` files under `dir` into
/// `known_paths` (repo-relative, "/" separators). Depth-capped; skips hidden
/// dirs and node_modules so the once-per-repo index build stays bounded.
fn collect_index_files(root: &Path, dir: &Path, out: &mut Vec<String>, depth: u32) {
    if depth > 6 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_index_files(root, &path, out, depth + 1);
        } else if JS_INDEX_SUFFIXES.iter().any(|s| name == *s)
            && let Ok(rel) = path.strip_prefix(root)
        {
            let rel = rel
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            out.push(rel);
        }
    }
}

/// Join a relative specifier onto the importing file's directory, collapsing
/// `.`/`..` segments (POSIX-style, repo-relative paths only).
fn join_relative(source_file: &str, relative: &str) -> String {
    let source_dir = Path::new(source_file)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .filter(|d| !d.is_empty())
        .unwrap_or_default();
    let mut parts: Vec<&str> = if source_dir.is_empty() {
        Vec::new()
    } else {
        source_dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in relative.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            // Popping past the repo root is a mis-resolution: "../x" from a
            // root-level file has no valid in-repo target. Return empty so the
            // caller (try_candidate) treats it as unresolvable.
            if parts.is_empty() {
                return String::new();
            }
            parts.pop();
        } else {
            parts.push(seg);
        }
    }
    parts.join("/")
}

/// Collapse `.`/`..` and duplicate slashes in a repo-relative POSIX path.
fn normalize_posix_path(path: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Entity, EntityTier};
    use std::fs;
    use tempfile::TempDir;

    fn make_file(id: &str, path: &str) -> Entity {
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

    fn write(repo: &Path, rel: &str, content: &str) {
        let full = repo.join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    #[test]
    fn test_extension_permutation_priority_js_beats_ts() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "src/utils.js", "export {};\n");
        write(tmp.path(), "src/utils.ts", "export {};\n");
        let files = vec![
            make_file("f-utils-js", "src/utils.js"),
            make_file("f-utils-ts", "src/utils.ts"),
        ];
        let ctx = JsResolverContext::new(&files, tmp.path());
        let id = ctx
            .resolve("./utils", "src/main.js")
            .expect("should resolve");
        assert_eq!(id, "f-utils-js", ".js must win over .ts (fixed priority)");
    }

    #[test]
    fn test_directory_index_fallback() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "src/utils/index.ts", "export {};\n");
        let files = vec![make_file("f-idx", "src/utils/index.ts")];
        let ctx = JsResolverContext::new(&files, tmp.path());
        let id = ctx
            .resolve("./utils", "src/main.js")
            .expect("directory index must resolve");
        assert_eq!(id, "f-idx");
    }

    #[test]
    fn test_unindexed_file_on_disk_yields_no_entity() {
        // index.* on disk but not in grouping → no entity id (no guess).
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "src/utils/index.ts", "export {};\n");
        let ctx = JsResolverContext::new(&[], tmp.path());
        assert_eq!(ctx.resolve("./utils", "src/main.js"), None);
    }

    #[test]
    fn test_tsconfig_paths_alias_wildcard() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "tsconfig.json",
            r#"{"compilerOptions": {"paths": {"@/*": ["src/*"]}}}"#,
        );
        write(tmp.path(), "src/deep/util.ts", "export {};\n");
        let files = vec![make_file("f-util", "src/deep/util.ts")];
        let ctx = JsResolverContext::new(&files, tmp.path());
        let id = ctx
            .resolve("@/deep/util", "src/main.ts")
            .expect("alias must resolve");
        assert_eq!(id, "f-util");
    }

    #[test]
    fn test_tsconfig_wins_over_jsconfig() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "tsconfig.json",
            r#"{"compilerOptions": {"paths": {"@/*": ["src/*"]}}}"#,
        );
        write(
            tmp.path(),
            "jsconfig.json",
            r#"{"compilerOptions": {"paths": {"@/*": ["wrong/*"]}}}"#,
        );
        write(tmp.path(), "src/util.ts", "export {};\n");
        write(tmp.path(), "wrong/util.ts", "export {};\n");
        let files = vec![
            make_file("f-right", "src/util.ts"),
            make_file("f-wrong", "wrong/util.ts"),
        ];
        let ctx = JsResolverContext::new(&files, tmp.path());
        let id = ctx
            .resolve("@/util", "src/main.ts")
            .expect("tsconfig alias must win");
        assert_eq!(id, "f-right", "tsconfig.json must beat jsconfig.json");
    }

    #[test]
    fn test_jsconfig_used_only_when_tsconfig_absent() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "jsconfig.json",
            r#"{"compilerOptions": {"paths": {"~/x": ["js/only.ts"]}}}"#,
        );
        write(tmp.path(), "js/only.ts", "export {};\n");
        let files = vec![make_file("f-js", "js/only.ts")];
        let ctx = JsResolverContext::new(&files, tmp.path());
        assert_eq!(
            ctx.resolve("~/x", "js/main.js"),
            Some("f-js"),
            "jsconfig.json must apply when tsconfig.json is absent"
        );
    }

    #[test]
    fn test_workspace_member_resolution() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "package.json",
            r#"{"name": "root", "workspaces": ["packages/*"]}"#,
        );
        write(
            tmp.path(),
            "packages/alpha/package.json",
            r#"{"name": "alpha"}"#,
        );
        write(tmp.path(), "packages/alpha/util.ts", "export {};\n");
        write(
            tmp.path(),
            "packages/beta/package.json",
            r#"{"name": "beta"}"#,
        );
        write(tmp.path(), "packages/beta/sub/deep.ts", "export {};\n");

        let files = vec![
            make_file("f-alpha-util", "packages/alpha/util.ts"),
            make_file("f-beta-deep", "packages/beta/sub/deep.ts"),
        ];
        let ctx = JsResolverContext::new(&files, tmp.path());
        let id = ctx
            .resolve("beta/sub/deep", "packages/alpha/main.ts")
            .expect("workspace member must resolve");
        assert_eq!(id, "f-beta-deep");
    }

    #[test]
    fn test_unresolved_counters_split_internal_external() {
        let tmp = TempDir::new().unwrap();
        let files = vec![make_file("f-a", "src/a.js")];
        let ctx = JsResolverContext::new(&files, tmp.path());

        let mut counts = UnresolvedCounts::default();
        assert!(ctx.resolve("fs", "src/a.js").is_none());
        UnresolvedKind::External.apply(&mut counts);
        assert!(ctx.resolve("./missing", "src/a.js").is_none());
        UnresolvedKind::Internal.apply(&mut counts);
        assert_eq!(
            counts,
            UnresolvedCounts {
                internal: 1,
                external: 1
            }
        );
    }

    #[test]
    fn test_relative_path_normalization() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "src/a/b/target.js", "export {};\n");
        let files = vec![make_file("f-t", "src/a/b/target.js")];
        let ctx = JsResolverContext::new(&files, tmp.path());
        // "./a/../a/b/./target" from src/x.js → src/a/b/target.js
        let id = ctx
            .resolve("./a/../a/b/./target", "src/x.js")
            .expect("normalized relative path must resolve");
        assert_eq!(id, "f-t");
    }

    #[test]
    fn test_no_entity_id_for_unindexed_disk_file() {
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "orphan.js", "export {};\n");
        let ctx = JsResolverContext::new(&[], tmp.path());
        assert_eq!(ctx.resolve("./orphan", "main.js"), None);
    }

    #[test]
    fn test_join_relative_and_normalize() {
        assert_eq!(join_relative("src/a/b.js", "./c.js"), "src/a/c.js");
        assert_eq!(join_relative("src/a/b.js", "../c.js"), "src/c.js");
        assert_eq!(join_relative("src/a/b.js", "../../c.js"), "c.js");
        assert_eq!(join_relative("top.js", "./c.js"), "c.js");
        assert_eq!(normalize_posix_path("a/./b/../c"), "a/c");
        assert_eq!(normalize_posix_path("a//b"), "a/b");
    }

    #[test]
    fn test_exact_key_alias_no_wildcard() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "tsconfig.json",
            r#"{"compilerOptions": {"paths": {"@shared": ["shared/index.ts"]}}}"#,
        );
        write(tmp.path(), "shared/index.ts", "export {};\n");
        let files = vec![make_file("f-shared", "shared/index.ts")];
        let ctx = JsResolverContext::new(&files, tmp.path());
        assert_eq!(ctx.resolve("@shared", "src/main.ts"), Some("f-shared"));
    }

    #[test]
    fn test_multi_target_alias_first_winning_target_wins() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "tsconfig.json",
            r#"{"compilerOptions": {"paths": {"x": ["missing/x.js", "present/x.js"]}}}"#,
        );
        write(tmp.path(), "present/x.js", "export {};\n");
        let files = vec![make_file("f-x", "present/x.js")];
        let ctx = JsResolverContext::new(&files, tmp.path());
        // First target is missing → falls through to the second target.
        assert_eq!(ctx.resolve("x", "src/main.ts"), Some("f-x"));
    }

    #[test]
    fn test_bare_workspace_package_resolves_to_member_index() {
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "package.json",
            r#"{"name": "root", "workspaces": ["pkgs/*"]}"#,
        );
        write(
            tmp.path(),
            "pkgs/alpha/package.json",
            r#"{"name": "alpha"}"#,
        );
        write(tmp.path(), "pkgs/alpha/index.js", "export {};\n");
        let files = vec![make_file("f-alpha-idx", "pkgs/alpha/index.js")];
        let ctx = JsResolverContext::new(&files, tmp.path());
        assert_eq!(ctx.resolve("alpha", "src/main.ts"), Some("f-alpha-idx"));
    }

    #[test]
    fn test_specifier_not_mangled_by_rust_colon_stripping() {
        // JS specifiers contain no :: — this pins that the JS resolver is a
        // separate path (no rfind("::") behaviour applies).
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "src/dep.js", "export {};\n");
        let files = vec![make_file("f-dep", "src/dep.js")];
        let ctx = JsResolverContext::new(&files, tmp.path());
        assert_eq!(ctx.resolve("./dep", "src/main.js"), Some("f-dep"));
    }
}
