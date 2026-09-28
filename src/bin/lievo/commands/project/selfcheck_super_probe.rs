// Structural super:: probe (issue #744) — genuinely INDEPENDENT evidence
// for Rust `use super::…` sites, read from the source tree itself.
//
// The existing edge-correctness section is circular for this class:
// `independent_resolve` is crate-root-anchored and returns `None` for
// `super::` specifiers exactly as the production resolver did before
// #742, so `verify_edges` excludes no-evidence samples — the gate cannot
// tell "correctly no edge" from "resolver silently dropped the edge".
// This probe breaks the circularity by asserting a FACT ABOUT THE SOURCE
// TREE: for a `use super::X` in file F, walk up from F's module position
// `hops` levels (one per leading `super` segment) to the referenced
// module, then check whether that module's file provides the referenced
// name.
//
// Independence is STRUCTURALLY enforced: this module imports neither
// `independent_resolve` (selfcheck_metrics) nor the production
// `resolve_rust_relative` (relationship_helpers) — the walk below is
// re-derived from the file path alone, sharing no resolver code.
//
// Verdicts per site (issue #744 corrected rule — 2026-09-11):
//   - `use super::*` (glob): INDETERMINATE. A glob names no module, so
//     there is nothing to confirm; it must never be a failure (this alone
//     is the `#[cfg(test)] mod tests { use super::*; }` class that produced
//     the 1652 bogus pre-fix failures).
//   - `use super::X` where the target file declares `mod X;` (any
//     visibility) → CONFIRMED.
//   - `use super::X` where the target file declares ANY item named X
//     (`fn`/`struct`/`enum`/`trait`/`type`/`const`/`static`/`union`/
//     `macro_rules! X`, or re-exports X via a `use`/`pub use` path ending
//     in `X`) → CONFIRMED. Referring to a function or type in the parent
//     module is completely legitimate Rust.
//   - `use super::X` where the target file exists but contains NO
//     reference to X at all → FAILURE. The real signal: the import points
//     at something the target does not provide.
//   - Target cannot be determined (crate root, target file absent on
//     disk) → INDETERMINATE.
//   - Grouped `use super::{a, b}` → each member classified independently.
//
// Test-file hygiene (defect 2): the pre-read set applies the shared
// `is_test_like_file` rule (with the crate root exempt — rule c flags
// `lib.rs` for the false-zero section's 0-importer treatment, but the
// root is a valid probe target), and `super::` sites located IN test-like
// files are excluded from the probe's site list — matching the
// false-zero-callers section's behaviour.
//
// O(files): each candidate target file is read at most once (the
// pre-read declaration map), so the probe does not multiply the
// real-corpus CI step's runtime.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use lievo::analysis::rust_mod_parse::{find_item_names, is_test_like_file, parse_mod_decls};
use lievo::model::CodeUnit;

use super::selfcheck_false_zero::{SectionReport, SelfcheckThresholds};
use super::selfcheck_super_probe_sites::collect_super_sites;

/// Per-site results of the structural probe.
#[derive(Debug, Clone, Default)]
pub struct SuperProbeResults {
    /// Sites the target file provides (a `mod X;` declaration or any
    /// item named X).
    pub confirmed: usize,
    /// Sites the target file does NOT provide the referenced name — this
    /// count (alone) drives the gate.
    pub failures: usize,
    /// Glob sites (`use super::*`) plus sites whose target cannot be
    /// determined from the tree — visible in the detail, never a failure.
    pub indeterminate: usize,
    /// The first few failed sites (for the diagnostic detail string).
    pub failed_sites: Vec<String>,
}

/// Evaluate the super:: structural-probe gate: the gate fails when the
/// confirmed-failure count exceeds the threshold. Indeterminate sites do
/// NOT contribute to the gate (issue #744 operator decision).
pub fn gate_super_probe(
    results: &SuperProbeResults,
    thresholds: &SelfcheckThresholds,
) -> SectionReport {
    let passed = results.failures <= thresholds.max_super_probe_failures;
    let sites_desc = if results.failed_sites.is_empty() {
        String::new()
    } else {
        let sites: Vec<&str> = results
            .failed_sites
            .iter()
            .take(5)
            .map(String::as_str)
            .collect();
        format!(" sites={sites:?}")
    };
    SectionReport {
        name: "super_structural_probe",
        passed,
        skipped: false,
        skip_reason: None,
        detail: format!(
            "confirmed={} failures={} indeterminate={} threshold<= {}{}",
            results.confirmed,
            results.failures,
            results.indeterminate,
            thresholds.max_super_probe_failures,
            sites_desc
        ),
    }
}

// ---------------------------------------------------------------------------
// The walk: file-tree based, no resolver involvement.
// ---------------------------------------------------------------------------

/// The importing file's LOGICAL module as a segment list. The `#[path]`
/// alias map (physical→logical, issue #758) is consulted first: when the
/// file has an alias entry, the map's logical path IS the module (it is the
/// structural fact of where the file's `mod` declaration lives — the same
/// category of fact as the declaration parser the probe already reuses,
/// never a resolver). When the map has no entry, the path is re-derived
/// from the file path alone (the probe's structural independence is
/// preserved: no resolver code is shared). Empty list = the file IS a
/// crate root (`src/lib.rs`); a `mod.rs` file is its parent directory's
/// module.
fn importing_module_segments(
    importing_file: &str,
    physical_to_logical: &HashMap<String, String>,
) -> Vec<String> {
    if let Some(logical) = physical_to_logical.get(importing_file) {
        return logical
            .split("::")
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    let without_src = importing_file
        .strip_prefix("src/")
        .unwrap_or(importing_file);
    let stem = without_src.strip_suffix(".rs").unwrap_or(without_src);
    let segments: Vec<String> = stem
        .split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    // A `mod.rs` file IS its parent directory's module — drop the trailing
    // `mod` segment ONLY when the last segment is exactly `mod`.
    let is_mod = segments.last().map(|s| s == "mod").unwrap_or(false);
    let trimmed: &[String] = if is_mod {
        &segments[..segments.len() - 1]
    } else {
        &segments[..]
    };
    if trimmed.len() == 1 && trimmed[0].eq("lib") {
        return Vec::new();
    }
    trimmed.to_vec()
}

/// The physical parent-directory file for an importing file — the file that
/// physically declares (or would declare) the importing file as a sibling
/// module. `src/analysis/mod.rs` for `src/analysis/x.rs`. `None` for
/// top-level files (no directory parent under `src/`).
fn physical_parent_file(importing_file: &str, known_paths: &HashSet<&str>) -> Option<String> {
    let without_src = importing_file.strip_prefix("src/")?;
    let without_rs = without_src.strip_suffix(".rs").unwrap_or(without_src);
    let dir = without_rs.rsplit_once('/')?;
    let dir_str = dir.0;
    [format!("src/{dir_str}.rs"), format!("src/{dir_str}/mod.rs")]
        .into_iter()
        .find(|c| known_paths.contains(c.as_str()))
}

/// Resolve a walked module path (segment list) to its physical file:
/// `src/{path}.rs` (probed first) or `src/{path}/mod.rs`, with a `#[path]`
/// alias-map fallback (logical→physical, issue #758): when neither literal
/// is in the scanned corpus, the alias map may know where the module
/// actually lives (a `#[path]`-relocated module's physical file). `None`
/// when neither the literals nor the alias map yield a file (covers simply
/// missing files — indeterminate, not failures).
fn module_file(
    segments: &[String],
    known_paths: &HashSet<&str>,
    logical_to_physical: &HashMap<String, String>,
) -> Option<String> {
    let as_path = segments.join("/");
    let candidates = [format!("src/{as_path}.rs"), format!("src/{as_path}/mod.rs")];
    for candidate in &candidates {
        if known_paths.contains(candidate.as_str()) {
            return Some(candidate.clone());
        }
    }
    // `#[path]` alias fallback: the logical module path (segment list) may
    // have a divergent physical file the literals above miss. The map
    // answers "where is this module's file", never "is the import correct"
    // — the declaration test still reads the returned file directly.
    logical_to_physical.get(&as_path).and_then(|phys| {
        known_paths
            .contains(phys.as_str())
            .then(|| phys.to_string())
    })
}

/// The crate root file for the importing file's crate: `src/lib.rs` for
/// lib crates; `src/bin/<name>.rs` or `src/bin/<name>/main.rs` for bin
/// crates. `None` when the root cannot be determined from the tree.
fn crate_root_file(importing_file: &str, known_paths: &HashSet<&str>) -> Option<String> {
    if importing_file.starts_with("src/bin/") {
        // Bin crate: the root is the `src/bin/<name>` entry the file sits
        // under (flat `src/bin/<name>.rs` or directory `src/bin/<name>/`).
        let rest = importing_file.strip_prefix("src/bin/")?;
        let name = rest.split('/').next()?;
        for candidate in [
            format!("src/bin/{name}.rs"),
            format!("src/bin/{name}/main.rs"),
        ] {
            if known_paths.contains(candidate.as_str()) {
                return Some(candidate);
            }
        }
        return None;
    }
    if known_paths.contains("src/lib.rs") {
        return Some("src/lib.rs".to_string());
    }
    None
}

/// True when `file` is the crate root of its crate (the file that has no
/// parent module). `src/lib.rs` for lib crates; `src/bin/<name>.rs` (flat,
/// no sub-path) for bin crates. The crate root is exempt from the
/// `is_test_like_file` check (rule c flags `lib.rs` for the false-zero
/// section's 0-importer treatment, but the root is a valid probe target).
fn is_crate_root(file: &str) -> bool {
    if file == "src/lib.rs" {
        return true;
    }
    if let Some(rest) = file.strip_prefix("src/bin/") {
        // Flat `src/bin/<name>.rs` (no sub-path): the bin crate root.
        rest.ends_with(".rs") && !rest.contains('/')
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// The probe proper: O(files) — each target file is read at most once.
// ---------------------------------------------------------------------------

/// Run the structural probe over all Rust `use super::…` sites.
///
/// `code_units` is the repo's extracted code units (all languages — the
/// probe filters to Rust itself); `all_files` is the scanned-file set;
/// `repo_root` is the repository root; `logical_to_physical` is the
/// `#[path]` alias map (logical→physical, from the same scan the edge
/// section uses — the map is DATA the other section already computed,
/// not a resolver the probe calls); `physical_to_logical` is the map's
/// inverse (issue #758) — the probe keys it by the importing file's
/// PHYSICAL path to get the file's logical module path (the map's
/// forward half is keyed the other way and cannot answer that question).
///
/// Independence (issue #758 operator decision): the alias map is NOT
/// resolver output — it is a structural fact parsed from `mod` declarations
/// in the source tree, exactly the same category as the declaration parser
/// this module already reuses (`rust_mod_parse`). The map is consulted ONLY
/// to locate which file the `super::` walk lands on; it never decides
/// whether an import is correct (that remains a direct read of the target
/// file's declarations). This module imports neither `resolve_import`
/// nor `independent_resolve` — the import list at the top of this file is
/// the structural check.
pub fn probe_super_sites(
    code_units: &[CodeUnit],
    all_files: &[String],
    repo_root: &Path,
    logical_to_physical: &HashMap<String, String>,
    physical_to_logical: &HashMap<String, String>,
) -> SuperProbeResults {
    let known_files: HashSet<&str> = all_files.iter().map(String::as_str).collect();
    let known_paths = &known_files;

    // Collect sites and count include!d sites (issue #788: their logical
    // module is the including parent's, not the directory the walk reads,
    // so they degrade to indeterminate — never a failure).
    let (sites, include_d_sites) = collect_super_sites(code_units, all_files, repo_root);

    // Resolve each site's target file: the file the `super::` specifier
    // points to. Walk up `hops` levels from the importing file's LOGICAL
    // module (alias map first, directory layout as fallback — issue #758);
    // the target file is the file of the walked module. A site is
    // indeterminate when the target file cannot be located.
    //
    // Target-location refinement (issue #758 operator decision 4, framed
    // as target LOCATION, not a change to the classification rule): for a
    // non-relocated file whose walked (directory-derived) module file does
    // not exist, the target is indeterminate — the directory walk has no
    // file to read. This catches the `include!`d-file class (files
    // pulled in via `include!` whose directory parent does not declare
    // them — the alias map structurally cannot cover `include!`): such a
    // site degrades to indeterminate instead of a false failure.
    let mut resolved: Vec<Option<String>> = Vec::new();
    for (file, _module_name, hops) in &sites {
        let file_str = file.as_str();
        let modules = importing_module_segments(file_str, physical_to_logical);
        let reloc = physical_to_logical.contains_key(file_str);
        let target = if *hops > modules.len() {
            // Walk passes the crate root: no target to walk up to.
            None
        } else {
            let walked_segments = &modules[..modules.len() - *hops];
            if walked_segments.is_empty() {
                // The walk landed on the crate root itself (e.g.
                // `use super::x` in `src/x.rs`): the root file is the
                // target — a determinate target, so its file is checked
                // for the declaration.
                crate_root_file(file_str, known_paths)
            } else if file_str.starts_with("src/") {
                // Walked module file: literal candidates first, then the
                // `#[path]` alias map (the map may know the divergent
                // physical file of a relocated module).
                let walked = walked_segments.to_vec();
                match module_file(&walked, known_paths, logical_to_physical) {
                    Some(t) => Some(t),
                    None => {
                        // Directory walk found no file: relocated sites
                        // stay indeterminate (the alias map had no entry
                        // either); non-relocated sites refine to the
                        // physical parent directory file (the `include!`
                        // class — the file that physically contains the
                        // `include!` of the importing file).
                        if reloc {
                            None
                        } else {
                            physical_parent_file(file_str, known_paths)
                        }
                    }
                }
            } else {
                None
            }
        };
        resolved.push(target);
    }

    // Pre-read the target files (each at most once) into the declaration
    // map: target file -> provided names (mod declarations union item
    // names). Test-like target files are excluded (defect 2); the crate
    // root is exempt.
    let mut target_files: Vec<String> = Vec::new();
    for target in resolved.iter() {
        let Some(tf) = target else { continue };
        if !known_paths.contains(tf.as_str()) {
            continue;
        }
        let is_root = is_crate_root(tf);
        if !is_root && is_test_like_file(tf) {
            continue;
        }
        if !target_files.contains(tf) {
            target_files.push(tf.clone());
        }
    }
    let mut decl_map: HashMap<&str, HashSet<String>> = HashMap::new();
    for tf in &target_files {
        let full = repo_root.join(tf);
        let content = match std::fs::read_to_string(&full) {
            Ok(c) => c,
            Err(_) => {
                decl_map.insert(tf.as_str(), HashSet::new());
                continue;
            }
        };
        // A target file "provides" X when it declares `mod X;` OR any
        // item named X (the corrected rule). The item scan is line-based
        // (`find_item_names` in rust_mod_parse — the shared declaration
        // parser, not a resolver).
        let mut provided: HashSet<String> = parse_mod_decls(&content)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        for name in find_item_names(&content) {
            provided.insert(name);
        }
        decl_map.insert(tf.as_str(), provided);
    }

    // Evaluate each site: a glob is indeterminate by the corrected rule;
    // otherwise the site is confirmed when its target file provides the
    // referenced name (`mod X;` or any item named X) — the target file
    // existing is a precondition, a missing target file makes the site
    // indeterminate, not a failure. A target that IS the crate root is
    // indeterminate (the root has no parent module to assert the
    // declaration against).
    let mut results = SuperProbeResults::default();
    // Include!d sites were excluded from the site list up front (issue
    // #788) — their directory walk reads the wrong module, so they count
    // as indeterminate, never as failures.
    results.indeterminate += include_d_sites;
    for ((file, module_name, _hops), target) in sites.iter().zip(resolved.iter()) {
        if *module_name == "*" {
            // Glob: names no module — nothing to confirm, never a failure.
            results.indeterminate += 1;
            continue;
        }
        let Some(target_file) = target else {
            // The walked module has no literal file: `#[path]` relocation
            // or a broken tree — indeterminate (never a failure).
            results.indeterminate += 1;
            continue;
        };
        if !known_paths.contains(target_file.as_str()) {
            results.indeterminate += 1;
            continue;
        }
        if is_crate_root(target_file) {
            // The target IS the crate root: no parent module to assert the
            // declaration against — indeterminate.
            results.indeterminate += 1;
            continue;
        }
        if is_test_like_file(target_file) {
            // Test-like target file (excluded from the pre-read set,
            // defect 2): the declaration cannot be read — indeterminate.
            results.indeterminate += 1;
            continue;
        }
        let provided = decl_map
            .get(target_file.as_str())
            .cloned()
            .unwrap_or_default();
        if provided.iter().any(|p| p == module_name) {
            results.confirmed += 1;
        } else {
            results.failures += 1;
            if results.failed_sites.len() < 5 {
                results
                    .failed_sites
                    .push(format!("{file}: super::{module_name}"));
            }
        }
    }

    results
}

#[cfg(test)]
#[path = "selfcheck_super_probe_tests.rs"]
mod selfcheck_super_probe_tests;
