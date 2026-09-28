// Private helper functions for RelationshipBuilder.
// Not part of the public API.

use crate::model::{CodeUnit, Entity};
use std::collections::{HashMap, HashSet};

/// Resolve a relative Rust module specifier (`super::…` / `self::…`) against
/// the set of known repo-relative file paths (issue #742 task-a).
///
/// `importing_file` is the repo-relative path of the file containing the
/// `use` statement; `spec` is the raw specifier (the extractor passes the
/// `use_declaration` argument field verbatim). `known_paths` must be the
/// entity paths verbatim (repo-relative, "/" separators).
///
/// Module-tree walk (identical shape to the selfcheck verifier's super:: arm
/// — both must agree, see the operator decision on production/verifier
/// agreement):
///
/// - the importing file's own module: sibling form `src/P.rs` → `P`; mod.rs
///   form `src/P/mod.rs` → `P`; `src/lib.rs` → the crate root.
/// - each leading `super::` segment (counted on the ORIGINAL specifier —
///   `super::super::x` counts as two) walks one level up; walking past the
///   crate root returns None (no underflow, no bogus edge).
/// - `self::` stays at the importing file's own module.
/// - the resolved module's file entity is whichever of `src/{mod}.rs` (sibling
///   form, probed FIRST) or `src/{mod}/mod.rs` (mod.rs form) exists in
///   `known_paths`; if neither exists the specifier is unresolved.
///
/// `use super::*` records one edge to the parent module file — no per-member
/// expansion, no dropping.
///
/// Returns the target's repo-relative file path (the caller maps path →
/// entity id); `None` means unresolvable (crate root, missing parent file).
/// Public re-export of the production `super::`/`self::` module-tree walk
/// (issue #742 task-a) for the selfcheck verifier's agreement test (issue
/// #742 task-b). The verifier's `super::` arm in `selfcheck_metrics.rs`
/// must produce the identical target as this function for the same import,
/// or the selfcheck gate flags the new super:: edges as wrong and the CI
/// threshold ratchet regresses. See `resolve_rust_relative` for the full
/// contract (sibling-first probing, leading-`super`-counting, crate-root
/// underflow → None).
#[doc(hidden)]
pub fn resolve_rust_relative_for_agreement(
    importing_file: &str,
    spec: &str,
    known_paths: &HashSet<&str>,
) -> Option<String> {
    resolve_rust_relative(importing_file, spec, known_paths)
}

pub(crate) fn resolve_rust_relative(
    importing_file: &str,
    spec: &str,
    known_paths: &HashSet<&str>,
) -> Option<String> {
    // `hops`: super:: walks up `hops` levels, self:: stays put. Leading
    // `super` segments are counted on the ORIGINAL specifier — the off-by-one
    // defect from the prior attempt stripped one, then counted the remainder,
    // yielding 1 for `super::super::X`. The `let`-else lets the `Option`
    // returned by `strip_prefix` type-enforce the guard: no `.unwrap()`, so a
    // future arm reorder can no longer panic — a non-matching prefix yields
    // `None` and bails cleanly.
    let hops = match spec {
        s if s.starts_with("super::") => {
            let _rest = s.strip_prefix("super::"); // type-enforced: arm guard guarantees Some
            let mut supers = 0;
            for seg in s.split("::") {
                if seg == "super" {
                    supers += 1;
                } else {
                    break;
                }
            }
            supers
        }
        s if s.starts_with("self::") => {
            let _rest = s.strip_prefix("self::");
            0
        }
        _ => return None, // neither super:: nor self:: — not a relative specifier
    };

    let mut importing_module = importing_module_segments(importing_file);
    for _ in 0..hops {
        if importing_module.is_empty() {
            // super:: at (or above) the crate root: no parent to walk to.
            return None;
        }
        importing_module.pop();
    }

    // The trailing segments (after the leading super's/self) name a SYMBOL
    // in the resolved module, not a sub-module path. So we resolve the parent
    // module itself; the trailing symbol segments are unused for path
    // resolution.
    let target: Vec<&str> = importing_module.iter().map(String::as_str).collect();
    let dot_path = target.join("::");
    if dot_path.is_empty() {
        // Resolved to the crate root itself.
        let cand = "src/lib.rs".to_string();
        return known_paths.contains(cand.as_str()).then_some(cand);
    }
    // Sibling form probed first, then mod.rs form — deterministic and
    // identical in production and verifier.
    let as_path = dot_path.replace("::", "/");
    let candidates = [format!("src/{as_path}.rs"), format!("src/{as_path}/mod.rs")];
    candidates
        .into_iter()
        .find(|c| known_paths.contains(c.as_str()))
}

/// The importing file's own module as a segment list (the crate root for
/// `src/lib.rs`; `mod.rs` files contribute their parent directory).
fn importing_module_segments(importing_file: &str) -> Vec<String> {
    let without_src = importing_file
        .strip_prefix("src/")
        .unwrap_or(importing_file);
    let mut segments: Vec<String> = strip_rs_extension(without_src)
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    // A `mod.rs` file IS its parent directory's module — pop the trailing
    // `mod` segment ONLY when it is exactly `mod` (a sibling form such as
    // `a/bmod.rs` keeps its `bmod` segment; a directory literally named
    // `mod` (`a/b/mod/leaf.rs`) is only ever reached when `leaf` is the
    // last segment, so the pop is unambiguous).
    if segments.last().is_some_and(|s| s == "mod") {
        segments.pop();
    }
    if segments.len() == 1 && segments[0] == "lib" {
        Vec::new()
    } else {
        segments
    }
}

/// Build a map from import path variants to file entity ID.
pub(super) fn build_import_map(
    files: &[Entity],
    project_id: &str,
    repo_name: &str,
) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();

    for file in files {
        let file_path = match &file.path {
            Some(p) => p.as_str(),
            None => continue,
        };

        // Raw path as key (for relative import resolution)
        map.insert(file_path.to_string(), file.id.clone());

        // Derive Rust module-style keys by stripping common extensions and converting / to ::
        let rust_module_keys = rust_module_keys_for_path(file_path, repo_name, project_id);
        for key in rust_module_keys {
            map.entry(key).or_insert_with(|| file.id.clone());
        }
    }

    map
}

/// Normalize a file path to repo-relative form.
///
/// If the path is already relative (doesn't start with `/`), return it as-is.
/// If the path is absolute (starts with `/`), find the repo root portion by
/// looking for the repo name in the path, and return the portion after it.
/// Falls back to finding `/src/` if the repo name is not found.
///
/// This ensures `rust_module_keys_for_path` works correctly regardless of
/// whether entity paths are stored as absolute or repo-relative (issue #681
/// hard prerequisite: the resolver must operate on repo-relative paths on
/// both sides).
fn normalize_to_repo_relative<'a>(file_path: &'a str, repo_name: &str) -> &'a str {
    if !file_path.starts_with('/') {
        return file_path;
    }

    // Absolute path: look for the repo name in the path
    let repo_marker = format!("/{repo_name}/");
    if let Some(pos) = file_path.find(&repo_marker) {
        return &file_path[pos + repo_marker.len()..];
    }

    // Fallback: look for /src/ and use everything after it
    if let Some(pos) = file_path.rfind("/src/") {
        return &file_path[pos + 1..];
    }

    // Last resort: use the path as-is (will produce garbage keys but won't panic)
    file_path
}

/// Generates Rust module import key variants for a file path.
///
/// Only registers the leaf name when it is prefixed with `crate::` or `repo::`.
/// Bare leaf names are omitted to avoid false positives from external crates.
///
/// Examples for `src/utils/mod.rs` in repo `myrepo`:
/// - "crate::utils"      (crate-relative)
/// - "myrepo::utils"     (repo-qualified)
pub(super) fn rust_module_keys_for_path(
    file_path: &str,
    repo_name: &str,
    _project_id: &str,
) -> Vec<String> {
    let mut keys = Vec::new();

    // Normalize to repo-relative form (handles both absolute and relative paths)
    let normalized = normalize_to_repo_relative(file_path, repo_name);
    let without_src = normalized.strip_prefix("src/").unwrap_or(normalized);
    let without_ext = strip_rs_extension(without_src);

    // Handle mod.rs → parent directory (strip "/mod" suffix)
    let module_path = if let Some(stripped) = without_ext.strip_suffix("/mod") {
        stripped
    } else {
        without_ext
    };

    if !module_path.is_empty() {
        // Issue 2 fix: removed bare module_path key to prevent short-leaf false positives.
        // Only register crate:: and repo:: qualified forms.
        let dot_path = module_path.replace('/', "::");
        keys.push(format!("crate::{}", dot_path));
        keys.push(format!("{}::{}", repo_name, dot_path));
    }

    keys
}

pub(super) fn strip_rs_extension(path: &str) -> &str {
    for ext in [".rs", ".py", ".ts", ".js"] {
        if let Some(s) = path.strip_suffix(ext) {
            return s;
        }
    }
    path
}

/// Build a map from function name to the file entity ID that defines it.
///
/// Issue 3 fix: functions defined by more than one file are ambiguous and
/// are omitted entirely — they must not produce edges.
///
/// Counts unique FILES per function name (not code units) — multiple units
/// in the same file defining the same name are unambiguous.
pub(super) fn build_fn_map<'a>(
    code_units: &'a [CodeUnit],
    files: &'a [Entity],
) -> HashMap<&'a str, &'a str> {
    let path_to_id: HashMap<&str, &str> = files
        .iter()
        .filter_map(|f| f.path.as_deref().map(|p| (p, f.id.as_str())))
        .collect();

    // Count unique files per function name (not units — multiple units in same file are unambiguous).
    let mut name_files: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut qname_files: HashMap<&str, HashSet<&str>> = HashMap::new();

    for unit in code_units {
        if path_to_id.contains_key(unit.file.as_str()) {
            name_files
                .entry(unit.name.as_str())
                .or_default()
                .insert(unit.file.as_str());
            qname_files
                .entry(unit.qualified_name.as_str())
                .or_default()
                .insert(unit.file.as_str());
        }
    }

    // Only register unambiguous (defined-by-exactly-one-file) names.
    let mut map: HashMap<&str, &str> = HashMap::new();
    for unit in code_units {
        if let Some(file_id) = path_to_id.get(unit.file.as_str()) {
            if name_files.get(unit.name.as_str()).map_or(0, |s| s.len()) == 1 {
                map.entry(unit.name.as_str()).or_insert(file_id);
            }
            if qname_files
                .get(unit.qualified_name.as_str())
                .map_or(0, |s| s.len())
                == 1
            {
                map.entry(unit.qualified_name.as_str()).or_insert(file_id);
            }
        }
    }
    map
}

/// Resolve an import string to a file entity ID.
///
/// The optional `importing_file` is the repo-relative path of the file
/// carrying the `use` statement; it is needed for the relative-specifier
/// walk (`super::` / `self::`, issue #742 task-a). When it is `None`,
/// relative specifiers stay unresolved (the pre-#742 behaviour — the
/// call-site-contract seam).
pub(super) fn resolve_import_for<'a>(
    import: &str,
    import_map: &'a HashMap<String, String>,
    importing_file: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(id) = import_map.get(import) {
        return Some(id.as_str());
    }

    // Relative Rust specifiers: walk the module tree from the importing
    // file's module (super::/self::). The target path is mapped back through
    // the import map's raw-path keys, so a file the grouping did not index
    // can never produce an edge to a nonexistent entity.
    if let Some(importing_file) = importing_file
        && (import.starts_with("super::") || import.starts_with("self::"))
    {
        let known_paths: HashSet<&str> = import_map
            .keys()
            .filter(|k| k.ends_with(".rs"))
            .map(String::as_str)
            .collect();
        if let Some(path) = resolve_rust_relative(importing_file, import, &known_paths)
            && let Some(id) = import_map.get(&path)
        {
            return Some(id.as_str());
        }
        // A relative specifier that walks to a missing file is not an
        // external import — it simply has no edge.
        return None;
    }

    // Try progressively shorter prefixes (strip trailing ::* segments)
    let mut candidate = import;
    while let Some(pos) = candidate.rfind("::") {
        candidate = &candidate[..pos];
        if let Some(id) = import_map.get(candidate) {
            return Some(id.as_str());
        }
    }

    None
}

/// Build a child→parent ID map from a slice of entities.
pub(super) fn entity_parent_map(entities: &[Entity]) -> HashMap<String, String> {
    entities
        .iter()
        .filter_map(|e| e.parent_id.as_ref().map(|p| (e.id.clone(), p.clone())))
        .collect()
}

/// Aggregate file-level edges up one tier. Input keys: (src, tgt, RelType). Self-edges excluded.
pub(super) fn aggregate_depends_on<K>(
    edge_weights: &HashMap<(String, String, K), u32>,
    child_to_parent: &HashMap<String, String>,
) -> HashMap<(String, String), u32>
where
    K: std::hash::Hash + Eq,
{
    let mut out: HashMap<(String, String), u32> = HashMap::new();
    for ((src, tgt, _), count) in edge_weights {
        if let (Some(parent_src), Some(parent_tgt)) =
            (child_to_parent.get(src), child_to_parent.get(tgt))
            && parent_src != parent_tgt
        {
            let w = out
                .entry((parent_src.clone(), parent_tgt.clone()))
                .or_insert(0);
            *w = w.saturating_add(*count);
        }
    }
    out
}

/// Same as `aggregate_depends_on` but input keys are 2-tuples (no type tag).
pub(super) fn aggregate_flat_depends_on(
    edge_weights: &HashMap<(String, String), u32>,
    child_to_parent: &HashMap<String, String>,
) -> HashMap<(String, String), u32> {
    let mut out: HashMap<(String, String), u32> = HashMap::new();
    for ((src, tgt), count) in edge_weights {
        if let (Some(parent_src), Some(parent_tgt)) =
            (child_to_parent.get(src), child_to_parent.get(tgt))
            && parent_src != parent_tgt
        {
            let w = out
                .entry((parent_src.clone(), parent_tgt.clone()))
                .or_insert(0);
            *w = w.saturating_add(*count);
        }
    }
    out
}

/// Normalize a crate name for cross-repo matching (dashes→underscores, lowercase).
pub(super) fn normalize_crate_name(name: &str) -> String {
    name.to_lowercase().replace('-', "_")
}

#[cfg(test)]
#[path = "relationship_helpers_tests.rs"]
mod relationship_helpers_tests;
