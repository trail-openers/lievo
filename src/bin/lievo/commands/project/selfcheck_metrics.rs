// Pure metric + gate functions for `lievo admin selfcheck` (issue #715).
// No storage/I/O beyond the injected closures — unit-testable with plain
// slices/structs. Split from `selfcheck_ops.rs` (AGENTS.md §6).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use lievo::model::CodeUnit;
use lievo::retrieval::retrieval_eval::normalise_path;
use lievo::retrieval::tool_trait::Tool;
use lievo::{LievoError, Result};

pub use super::selfcheck_false_zero::{
    SectionReport, SelfcheckThresholds, detect_false_zero_callers, gate_false_zero_callers,
    gate_payload_bytes, gate_retrieval_probes, on_disk_grep_importer_count, skipped_section,
};

// The edge-correctness gate moved to `selfcheck_edge_split.rs` (issue #777 —
// the rate is computed over import-resolved edges only and the detail string
// names the call-based/unverifiable category). Call sites import it directly
// from that module (see `selfcheck_ops` / `selfcheck_ops_tests`).

// ---------------------------------------------------------------------------
// Section (a): edge-correctness
// ---------------------------------------------------------------------------

/// One sampled import edge re-verified against an independent resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeSample {
    pub source_file: String,
    pub recorded_target: String,
    /// None = the independent resolver could not resolve the specifier at all.
    pub resolved_target: Option<String>,
    pub wrong: bool,
}

/// Deterministically sample up to `n` (source, target) import edges,
/// ordered so the sample is reproducible across runs (no RNG/seed).
pub fn sample_edges(resolved_by_path: &[(String, String)], n: usize) -> Vec<(String, String)> {
    let mut sorted = resolved_by_path.to_vec();
    sorted.sort();
    sorted.dedup();
    let limit = if n == 0 { usize::MAX } else { n };
    sorted.into_iter().take(limit).collect()
}

/// Re-verify each sampled edge: does an independent resolver land on the
/// recorded target? An edge is "wrong" only when the resolver resolves at
/// least one specifier but never to the recorded target. Unresolvable
/// specifiers (aliases, workspace, external) are not evidence either way.
pub fn verify_edges(
    sampled: &[(String, String)],
    code_units: &[CodeUnit],
    known_paths: &HashSet<String>,
    repo_name: &str,
    logical_to_physical: &HashMap<String, String>,
) -> Vec<EdgeSample> {
    let mut specifiers_by_file: HashMap<&str, Vec<&str>> = HashMap::new();
    for unit in code_units {
        for import in &unit.imports {
            specifiers_by_file
                .entry(unit.file.as_str())
                .or_default()
                .push(import.as_str());
        }
    }

    sampled
        .iter()
        .map(|(src, tgt)| {
            let base_candidates: HashSet<String> = specifiers_by_file
                .get(src.as_str())
                .into_iter()
                .flatten()
                .filter_map(|spec| {
                    independent_resolve(src, spec, known_paths, repo_name, logical_to_physical)
                })
                .collect();
            // Re-export case (issue #724): a `mod.rs` file may also resolve
            // to its direct-child .rs files, so those children join the
            // candidate set — the production resolver records the edge
            // against the child where the symbol is actually defined. Only
            // `mod.rs` files have re-export semantics; flat `.rs` files do
            // not.
            let mut candidates = base_candidates.clone();
            let is_mod_rs = tgt.ends_with("/mod.rs");
            // The recorded target is a direct-child file re-exported via its
            // parent's mod.rs (the mod.rs sibling resolves to the same module).
            let is_reexport_child = tgt.rsplit_once('/').is_some_and(|(dir, base)| {
                base.strip_suffix(".rs").is_some_and(|stem| {
                    let parent = dir.rsplit('/').next().unwrap_or(dir);
                    base_candidates.contains(&format!("src/{parent}/{stem}.rs"))
                })
            });
            if (is_mod_rs || is_reexport_child)
                && let Some(dir) = std::path::Path::new(tgt)
                    .parent()
                    .map(|d| d.to_string_lossy().as_ref().to_string())
            {
                candidates.extend(expand_mod_rs_children(&dir, tgt, known_paths));
            }
            for resolved in &base_candidates {
                if resolved.ends_with("/mod.rs")
                    && let Some(dir) = std::path::Path::new(resolved)
                        .parent()
                        .map(|d| d.to_string_lossy().as_ref().to_string())
                {
                    candidates.extend(expand_mod_rs_children(&dir, resolved, known_paths));
                }
            }
            let resolved_target = candidates.iter().find(|c| *c == tgt).cloned().or_else(|| {
                if candidates.is_empty() {
                    None
                } else {
                    candidates.iter().next().cloned()
                }
            });
            let wrong = !candidates.is_empty() && !candidates.contains(tgt);
            EdgeSample {
                source_file: src.clone(),
                recorded_target: tgt.clone(),
                resolved_target,
                wrong,
            }
        })
        .collect()
}

/// Independent re-resolution of a raw import specifier to a repo-relative
/// path, using ONLY filesystem-shape reasoning against the known corpus path
/// set — never the crate's `import_map`/`bare_name_map`/`JsResolverContext`.
///
/// Deliberately narrow: relative JS/TS specifiers resolved by extension
/// permutation; Rust `crate::`/`{repo_name}::` module paths by `::` → `/`
/// with `.rs`/`/mod.rs` permutation, trailing-symbol fallback, and `#[path]`
/// alias lookup (issue #732); bare-local `use X::…` specifiers resolve
/// against the crate root (issue #732 round-5); anything else returns
/// `None`. `pub(crate)` so tests can unit-test the alias seam directly.
pub(crate) fn independent_resolve(
    source_file: &str,
    specifier: &str,
    known_paths: &HashSet<String>,
    repo_name: &str,
    logical_to_physical: &HashMap<String, String>,
) -> Option<String> {
    const JS_EXTS: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"];

    if specifier.starts_with("./") || specifier.starts_with("../") {
        let base = Path::new(source_file).parent().unwrap_or(Path::new(""));
        let joined = normalise_relative(base, specifier);
        for ext in JS_EXTS {
            let candidate = format!("{joined}{ext}");
            if known_paths.contains(&candidate) {
                return Some(candidate);
            }
        }
        for ext in JS_EXTS {
            let candidate = format!("{joined}/index{ext}");
            if known_paths.contains(&candidate) {
                return Some(candidate);
            }
        }
        if known_paths.contains(&joined) {
            return Some(joined);
        }
        return None;
    }

    // Relative Rust specifiers: `super::`/`self::` walk the module tree from
    // the IMPORTING FILE's module (issue #742 task-b). The walk must produce
    // the identical target as the production resolver
    // (`resolve_rust_relative`, issue #742 task-a) — probe sibling form
    // `src/{mod}.rs` before `src/{mod}/mod.rs`, count ALL leading `super`/
    // `self` segments on the original specifier, and return None (empty
    // candidate set → no evidence, not wrong) when the walk reaches the
    // crate root or the parent's module file is missing.
    if specifier.starts_with("super::") || specifier.starts_with("self::") {
        return resolve_relative_super_self(
            source_file,
            specifier,
            known_paths,
            logical_to_physical,
        );
    }

    // Rust specifiers: `crate::X`, `{repo}::X`, or bare-local `use X::…`
    // (bare-local resolves against the crate root — issue #732 round-5).
    // The module path is the specifier with any crate prefix stripped; the
    // resolution loop tests literal `.rs`/`/mod.rs`, the `#[path]` alias map,
    // and trailing-symbol fallback (the last segment may name a symbol, not a
    // file — issue #724).
    let crate_prefix = format!("{repo_name}::");
    let module_path = specifier
        .strip_prefix("crate::")
        .or_else(|| specifier.strip_prefix(&crate_prefix))
        .map(str::to_string)
        .unwrap_or_else(|| specifier.to_string());
    let mut prefix = module_path.as_str();
    loop {
        let as_path = prefix.replace("::", "/");
        let literal = [format!("src/{as_path}.rs"), format!("src/{as_path}/mod.rs")]
            .into_iter()
            .find(|c| known_paths.contains(c));
        if let Some(candidate) = literal {
            return Some(candidate);
        }
        // `#[path]` alias map: the literal location may not exist while the
        // module is declared at a divergent physical file (issue #732). The
        // alias target must exist in the known corpus — an alias whose
        // physical file is absent (e.g. a `#[cfg(test)]`-gated test module)
        // would otherwise be mistaken for a resolved-but-mismatched edge.
        if let Some(alias) = logical_to_physical.get(prefix).cloned()
            && known_paths.contains(alias.as_str())
        {
            return Some(alias);
        }
        if let Some((stripped, _)) = prefix.rsplit_once("::") {
            prefix = stripped;
        } else {
            break;
        }
    }
    None
}

/// Independent module-tree walk for `super::`/`self::` Rust specifiers
/// (issue #742 task-b). The producing side is the production resolver
/// `resolve_rust_relative` in `src/analysis/relationship_helpers.rs`; this
/// arm must stay in lock-step with it (operator decision: production and
/// verifier must agree, else the new edges count as wrong and the ratchet
/// regresses).
///
/// - The importing file's own module: sibling form `src/P.rs` → `P`, mod.rs
///   form `src/P/mod.rs` → `P`, `src/lib.rs` / `src/bin/<x>.rs` → the crate
///   root (empty segment list).
/// - Each leading `super` segment (counted on the ORIGINAL specifier) walks
///   one level up; `self` stays put. Walking past the crate root returns
///   None — no underflow, no self-edge, no bogus edge.
/// - The target module file is whichever of `src/{mod}.rs` (probed FIRST) or
///   `src/{mod}/mod.rs` exists in `known_paths`; a resolved crate root
///   resolves to `src/lib.rs`. The same `#[path]` alias lookup the `crate::`
///   arm uses then runs against the walked module path (a parent module file
///   may sit at a `#[path]`-divergent physical location — issue #732's
///   alias seam applies here too; without it the verifier would miss an
///   edge that production resolves, silently regressing the ratchet).
///   Neither location exists → None.
fn resolve_relative_super_self(
    source_file: &str,
    specifier: &str,
    known_paths: &HashSet<String>,
    logical_to_physical: &HashMap<String, String>,
) -> Option<String> {
    // (hops, _rest): count leading `super`/`self` segments on the original
    // specifier (`super::super::X` → 2). The walk itself is guarded by the
    // arm's gate (only `super::`/`self::` prefixed specifiers reach here),
    // so the remaining segments are irrelevant for the hop count — a
    // trailing `::super` symbol is counted, and that is benign (a missing
    // grandparent yields no literal and no alias, i.e. no false evidence).
    let hops = specifier
        .split("::")
        .take_while(|seg| **seg == *"super" || **seg == *"self")
        .filter(|seg| **seg == *"super")
        .count();

    let mut modules: Vec<&str> = importing_module_segments(source_file);
    for _ in 0..hops {
        if modules.is_empty() {
            // super:: at (or above) the crate root: empty candidate set.
            return None;
        }
        modules.pop();
    }

    let dot_path = modules.join("::");
    if dot_path.is_empty() {
        // The resolved module IS the crate root (e.g. `self::` from a root
        // file). `src/lib.rs` is the canonical crate-root file.
        return known_paths
            .contains("src/lib.rs")
            .then(|| "src/lib.rs".to_string());
    }
    // Mirror the production resolver's documented `src/` assumption: a
    // module path is only mapped to a physical location when the IMPORTING
    // file itself lives under `src/` — a non-src importer has no physical
    // home for the walked module (no evidence). Production and verifier
    // must keep this agreement; the src/ assumption itself is a known
    // simplification (tracked, not part of this fix).
    if !source_file.starts_with("src/") {
        return None;
    }
    let as_path = dot_path.replace("::", "/");
    if let Some(candidate) = [format!("src/{as_path}.rs"), format!("src/{as_path}/mod.rs")]
        .into_iter()
        .find(|candidate| known_paths.contains(candidate))
    {
        return Some(candidate);
    }
    // `#[path]` alias lookup (mirrors the crate:: arm): the walked module's
    // literal location is absent, but the alias map may record a divergent
    // physical file. Unlike the crate:: arm's prefix fallback, the super arm
    // consults EXACTLY the walked path (no fallback to shorter prefixes):
    // `super::x` names the walked module itself, and a shorter prefix may
    // alias a different module (the same reason the literal probe is
    // sibling-first and single, not a prefix loop).
    logical_to_physical
        .get(&dot_path)
        .filter(|alias| known_paths.contains(alias.as_str()))
        .cloned()
}

/// The importing file's own module as a segment list (the crate root for
/// `src/lib.rs`; `mod.rs` files contribute their parent directory). Mirrors
/// `importing_module_segments` in `src/analysis/relationship_helpers.rs`
/// (production side, issue #742 task-a) — the two must stay identical or
/// production/verifier agreement breaks.
fn importing_module_segments(importing_file: &str) -> Vec<&str> {
    let without_src = importing_file
        .strip_prefix("src/")
        .unwrap_or(importing_file);
    // The `mod` marker is `mod.rs` — the extension must be stripped before
    // the trailing-segment test, otherwise the marker never matches and
    // every `mod.rs` file resolves to a bogus `<parent>::mod` module.
    let stem = without_src.strip_suffix(".rs").unwrap_or(without_src);
    let segments: Vec<&str> = stem.split('/').filter(|s| !s.is_empty()).collect();
    // A `mod.rs` file IS its parent directory's module — drop the trailing
    // `mod` segment ONLY when the last segment is exactly `mod` (the
    // production-side copy in `relationship_helpers.rs` must stay identical;
    // string-suffix stripping would also wrongly mangle `a/bmod.rs`).
    let trimmed = if segments.last() == Some(&"mod") {
        &segments[..segments.len() - 1]
    } else {
        &segments[..]
    };
    if trimmed.len() == 1 && trimmed[0] == "lib" {
        return Vec::new();
    }
    trimmed.to_vec()
}

/// Join a relative specifier (`./x`, `../x`) onto a base directory using
/// pure string/component arithmetic (no filesystem access — repo-relative
/// paths only).
fn normalise_relative(base: &Path, specifier: &str) -> String {
    let mut parts: Vec<String> = base
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    for seg in specifier.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other.to_string()),
        }
    }
    parts.join("/")
}

/// All known `.rs` files that are DIRECT children of `dir` (no further `/`
/// below it), excluding `excluded` itself.
fn expand_mod_rs_children(dir: &str, excluded: &str, known_paths: &HashSet<String>) -> Vec<String> {
    let prefix = format!("{dir}/");
    known_paths
        .iter()
        .filter(|p| {
            p.starts_with(&prefix)
                && p.ends_with(".rs")
                && p.as_str() != excluded
                && !p[prefix.len()..].contains('/')
        })
        .cloned()
        .collect()
}

/// Wrong-edge rate = wrong / (wrong + correct), excluding "no independent
/// evidence" samples. Returns 0.0 when no sample had independent evidence.
pub fn wrong_edge_rate(samples: &[EdgeSample]) -> f64 {
    let evidenced: Vec<&EdgeSample> = samples
        .iter()
        .filter(|s| s.resolved_target.is_some())
        .collect();
    if evidenced.is_empty() {
        return 0.0;
    }
    let wrong = evidenced.iter().filter(|s| s.wrong).count();
    wrong as f64 / evidenced.len() as f64
}

// ---------------------------------------------------------------------------
// Sections (c)/(d): probe file + per-probe measurement
// ---------------------------------------------------------------------------

/// A single probe: natural-language query → expected repo-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfcheckProbe {
    pub query: String,
    pub expected_path: String,
}

/// Parse a probe file: one `query<TAB>expected_path` pair per non-empty,
/// non-`#`-comment line. Kept intentionally simple (no config-file format
/// per the PM decision: CLI flags only, no config file).
pub fn parse_probes(content: &str) -> Result<Vec<SelfcheckProbe>> {
    let mut probes = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, '\t');
        let query = parts.next().unwrap_or_default().trim();
        let expected_path = parts.next().unwrap_or_default().trim();
        if query.is_empty() || expected_path.is_empty() {
            return Err(LievoError::InvalidInput(format!(
                "probe file line {}: expected 'query<TAB>expected_path', got {:?}",
                i + 1,
                line
            )));
        }
        probes.push(SelfcheckProbe {
            query: query.to_string(),
            expected_path: expected_path.to_string(),
        });
    }
    Ok(probes)
}

/// Per-probe measurement: recall@k (via `ExploreTool::call()`'s returned file
/// paths) and the response's byte length (payload-bytes floor, section d).
#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub query: String,
    pub expected_path: String,
    pub recall: f32,
    pub payload_bytes: usize,
}

/// Drive one probe through `tool.call()` (the full ExploreTool path — storage
/// backed, 24K cap, tier-1 map — so the payload-bytes floor measures what an
/// agent actually receives).
pub fn run_probe(
    tool: &dyn Tool,
    probe: &SelfcheckProbe,
    repo_root: &str,
    k: usize,
) -> Result<ProbeResult> {
    let input = serde_json::json!({ "query": probe.query, "max_files": k });
    let response = tool.call(input)?;
    let payload_bytes = response.len();

    let paths: Vec<String> = serde_json::from_str::<serde_json::Value>(&response)
        .ok()
        .and_then(|v| v.get("symbols").cloned())
        .and_then(|s| s.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.get("qualified_path").and_then(|p| p.as_str()))
        .map(|p| normalise_path(p, repo_root))
        .collect();

    let target: HashSet<String> =
        std::iter::once(normalise_path(&probe.expected_path, repo_root)).collect();
    let recall = lievo::retrieval::metrics::recall_at_k(&paths, &target, k);

    Ok(ProbeResult {
        query: probe.query.clone(),
        expected_path: probe.expected_path.clone(),
        recall,
        payload_bytes,
    })
}

/// Worst-single-probe recall = min over all probes (not mean — the spec's
/// "worst-probe floor" is explicitly a min, so one degenerate probe cannot be
/// masked by several perfect ones).
pub fn worst_probe_recall(results: &[ProbeResult]) -> f32 {
    if results.is_empty() {
        return 0.0;
    }
    results
        .iter()
        .map(|r| r.recall)
        .fold(f32::INFINITY, f32::min)
        .clamp(0.0, 1.0)
}

#[cfg(test)]
#[path = "selfcheck_metrics_tests.rs"]
mod selfcheck_metrics_tests;

#[cfg(test)]
#[path = "selfcheck_metrics_super_tests.rs"]
mod selfcheck_metrics_super_tests;
