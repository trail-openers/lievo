//! Subsystem-bundle mode for `lievo_explore` (issue #743).
//!
//! An explicit `bundle` parameter (the scope prefix to bundle) selects
//! subsystem-bundle mode: ONE call, under the 24K cap, returns
//!   - per-file verbatim line-numbered source for the packed files (always
//!     tier-2 — dense-by-construction is the point; operator decision #3),
//!   - the intra-scope Calls/Imports edges among the packed files,
//!   - `not_shown_files`: an array of paths naming every omitted file,
//!   - `completeness`: a structured object `{complete, omitted_files,
//!     omitted_edges}` telling the agent it can stop when complete.
//!
//! Operator decisions binding here (issue #743 comment):
//!   - the selector is a single OPTIONAL STRING named `bundle` (not a bool,
//!     not an enum): presence selects bundle mode, absence leaves every
//!     existing behaviour byte-identical
//!   - only intra-scope edges (BOTH endpoints mapped to packed files) are
//!     returned; edges leaving the scope are counted in `omitted_edges`,
//!     never invented
//!   - a missing/unreadable requested file produces a named
//!     `not_shown_files` entry, never a silent empty source (decision #7)
//!   - a single oversized file reuses `cap_response`'s post-hoc truncation
//!     (decision #6), never dropped
//!
//! `bundle` short-circuits everything: `maybe_bundle_listing` is called
//! FIRST in `ExploreTool::call`, before files / scope / word-match, so a
//! bundle request never falls through into another mode.

use std::collections::{HashMap, HashSet};
use std::path::Component;

use serde_json::{Value, json};

use crate::model::{Entity, EntityTier, RelType};
use crate::retrieval::explore_cap::cap_response;
use crate::retrieval::explore_common::{lock_storage, should_exclude_entity};
use crate::retrieval::tools::ToolContext;
use crate::retrieval::tools_explore::{
    MAX_EXPLORE_OUTPUT_CHARS, line_numbered_source, safe_read_file_in_repo,
};
use crate::storage::Storage;

/// Entry point: read `bundle` from the call input and, when it is present
/// and non-blank, run the subsystem-bundle path instead of files / scope /
/// word-match. Returns `None` when `bundle` is absent (or blank), signaling
/// the caller to fall through unchanged — a query-only or scope-only call
/// therefore never activates bundle mode (issue #743 operator decision #8).
pub(crate) fn maybe_bundle_listing<S: Storage>(
    ctx: &std::sync::Arc<ToolContext<S>>,
    input: &Value,
) -> Option<crate::Result<String>> {
    let scope = input.get("bundle").and_then(|v| v.as_str())?;
    if scope.trim().is_empty() {
        return None;
    }
    Some(bundle_listing(ctx, scope))
}

/// Bundle the indexed files under `scope` into one response: packed per-file
/// tier-2 source, the intra-scope Calls/Imports edges, a `not_shown_files`
/// list and a structured `completeness` field — all under the 24K cap.
fn bundle_listing<S: Storage>(ctx: &ToolContext<S>, scope: &str) -> crate::Result<String> {
    // Scope selection reuses the scope-mode prefix rules (slash-anchored
    // containment, traversal rejection, `.`/blank normalization); the helpers
    // are private to that module, so the minimal equivalent is kept local
    // rather than widening its visibility (no cross-module coupling).
    let normalized = normalize_scope(scope);
    if normalized.is_empty() || is_path_traversal(scope) {
        return Ok(json!({
            "warning": format!(
                "Invalid bundle scope '{scope}': an empty scope or a path-traversal prefix is not allowed. \n                 Pass a repo-relative directory prefix, e.g. bundle='src/retrieval'."
            ),
        })
        .to_string());
    }

    let guard = lock_storage!(ctx.storage);
    let all_files = guard.list_entities(&ctx.project_id, Some(EntityTier::File))?;
    drop(guard);

    // Deterministic scope order: lexicographic repo-relative path (the
    // scope-mode order), so packing order is stable for an identical request.
    let scope_slash = format!("{normalized}/");
    let in_scope: Vec<&Entity> = all_files
        .iter()
        .filter(|e| {
            e.path
                .as_ref()
                .map(|p| {
                    (p == normalized || p.starts_with(&scope_slash))
                        && !should_exclude_entity(Some(p), &ctx.output_dir)
                })
                .unwrap_or(false)
        })
        .collect();
    let mut in_scope: Vec<&Entity> = in_scope;
    in_scope.sort_by(|a, b| {
        a.path
            .as_deref()
            .unwrap_or("")
            .cmp(b.path.as_deref().unwrap_or(""))
    });

    // Single scan of the graph (not O(files^2)): `file_for_entity` maps
    // every entity id inside the scope — file entities AND their Function-
    // tier children (parent_id = file id) — up to the scope file's path, so
    // a function→function edge is "intra-scope" iff both endpoints map to
    // scope files. One `list_all_relationships` call feeds both the intra-
    // and out-of-scope edge accounting.
    let guard = lock_storage!(ctx.storage);
    let mut file_for_entity: HashMap<String, String> = HashMap::new();
    for f in &in_scope {
        let path = f.path.clone().unwrap_or_default();
        file_for_entity.insert(f.id.clone(), path.clone());
        if let Ok(children) = guard.entities_by_parent(&f.id) {
            for c in &children {
                file_for_entity.insert(c.id.clone(), path.clone());
            }
        }
    }
    let all_rels = guard.list_all_relationships(&ctx.project_id)?;
    drop(guard);

    // (a) both endpoints map to two DIFFERENT scope files → intra-scope
    //       edge (deduped file pairs — function→function edges between two
    //       files appear several times);
    //   (b) both endpoints map to the SAME scope file → intra-file edge:
    //       below the file level, neither shown nor counted as omitted;
    //   (c) one endpoint maps to a scope file, the other does NOT map (it
    //       is outside the scope) → omitted edge.
    let scope_ids: HashSet<&str> = in_scope.iter().map(|e| e.id.as_str()).collect();
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
    let mut omitted_edges: usize = 0;
    for r in &all_rels {
        if !matches!(r.rel_type, RelType::Calls | RelType::Imports) {
            continue;
        }
        let src_mapped = file_for_entity.get(r.source_id.as_str());
        let tgt_mapped = file_for_entity.get(r.target_id.as_str());
        match (src_mapped, tgt_mapped) {
            (Some(a), Some(b)) => {
                if a == b {
                    continue; // case (b): intra-file
                }
                if seen_pairs.insert((a.clone(), b.clone())) {
                    edges.push((a.clone(), b.clone()));
                }
            }
            _ => {
                // One or both endpoints outside the scope map: the edge is
                // counted as omitted only when at least one endpoint touches
                // the scope (a scope file itself or a scope-mapped function).
                let touches_scope = scope_ids.contains(r.source_id.as_str())
                    || scope_ids.contains(r.target_id.as_str())
                    || file_for_entity.contains_key(r.source_id.as_str())
                    || file_for_entity.contains_key(r.target_id.as_str());
                if touches_scope {
                    omitted_edges += 1;
                }
            }
        }
    }

    // Packing, scope order, disclosure reserved BEFORE packing (mirrors the
    // #741 files-mode probe, extended with the bundle's disclosure shape so
    // its serialized size is measured too): the first file is always taken
    // (an oversized single file is truncated via the existing `cap_response`
    // mechanism, never dropped); every later file fits only while the
    // fully-disclosure-serialized response stays under the 24K cap.
    let total = in_scope.len();
    let built: Vec<BundleFile> = in_scope
        .iter()
        .map(|e| {
            let path = e.path.clone().unwrap_or_default();
            let body = safe_read_file_in_repo(&ctx.repo_path, &path);
            let source = match body.as_deref() {
                Some(b) => line_numbered_source(b),
                None => String::new(),
            };
            BundleFile {
                path,
                source,
                missing: body.is_none(),
            }
        })
        .collect();

    let mut chosen: Vec<usize> = Vec::new();
    for (i, entry) in built.iter().enumerate() {
        let mut probe_symbols: Vec<Value> = chosen
            .iter()
            .map(|&j| bundle_symbol(&built[j], false))
            .collect();
        probe_symbols.push(bundle_symbol(entry, entry.missing));
        let fits = if chosen.is_empty() {
            true
        } else {
            let probe = bundle_probe(&built, &chosen, i, &edges, total, omitted_edges);
            probe.to_string().chars().count() <= MAX_EXPLORE_OUTPUT_CHARS
        };
        if fits {
            chosen.push(i);
        }
    }

    // Packed file symbols: verbatim tier-2 line-numbered source. A packed
    // path whose on-disk source is unreadable is still represented (empty
    // source + flag) AND lands in not_shown_files — a named omission, never
    // a silent drop (decision #7). A genuinely empty file (present, 0 bytes)
    // carries empty source with no flag and is NOT an omission.
    let mut symbols: Vec<Value> = Vec::new();
    let mut not_shown_files: Vec<String> = Vec::new();
    for (i, entry) in built.iter().enumerate() {
        if !chosen.contains(&i) {
            not_shown_files.push(entry.path.clone());
            continue;
        }
        if entry.missing {
            symbols.push(bundle_symbol(entry, true));
            not_shown_files.push(entry.path.clone());
        } else {
            symbols.push(bundle_symbol(entry, false));
        }
    }
    let returned = symbols.len();
    // A missing-on-disk file is represented (flagged) AND named in
    // not_shown_files; it therefore counts as omitted in the structured
    // completeness (operator decision #7), on top of the pack-omitted files.
    let pack_omitted = total.saturating_sub(returned);
    let missing_count = not_shown_files
        .iter()
        .filter(|p| built.iter().any(|f| f.path.as_str() == **p && f.missing))
        .count();
    let omitted_files = pack_omitted + missing_count;
    let chosen_paths_set: HashSet<&str> = chosen.iter().map(|&i| built[i].path.as_str()).collect();

    // Edges are shown only when BOTH endpoint files are packed; the rest are
    // counted as omitted (deduped intra-scope pairs + out-of-scope edges).
    let shown_edges: Vec<Value> = edges
        .iter()
        .filter(|(a, b)| {
            chosen_paths_set.contains(a.as_str()) && chosen_paths_set.contains(b.as_str())
        })
        .map(|(a, b)| json!({ "source": a, "target": b }))
        .collect();
    let shown_set: HashSet<(&str, &str)> = shown_edges
        .iter()
        .map(|v| {
            (
                v["source"].as_str().unwrap_or(""),
                v["target"].as_str().unwrap_or(""),
            )
        })
        .collect();
    let omitted_edges_final = edges
        .iter()
        .filter(|(a, b)| !shown_set.contains(&(a.as_str(), b.as_str())))
        .count()
        + omitted_edges;

    let response = json!({
        "symbols": symbols,
        "edges": shown_edges,
        "not_shown_files": not_shown_files,
        "returned": returned,
        "total": total,
        "completeness": {
            "complete": not_shown_files.is_empty(),
            "omitted_files": omitted_files,
            "omitted_edges": omitted_edges_final,
        },
    });

    let serialized = response.to_string();
    if serialized.chars().count() > MAX_EXPLORE_OUTPUT_CHARS
        && let Some(capped) = cap_response(&response)
    {
        return Ok(capped);
    }
    Ok(serialized)
}

/// One file of the packed bundle: its repo-relative path, its line-numbered
/// source (empty when the on-disk file is missing), and whether the on-disk
/// file is absent (`missing` → the symbol carries a `missing_on_disk` flag
/// and the path lands in not_shown_files; a present-but-empty file does not).
struct BundleFile {
    path: String,
    source: String,
    missing: bool,
}

/// Per-file symbol for the bundle response: verbatim tier-2 source; a
/// missing-on-disk file keeps an empty source plus a flag (never dropped).
fn bundle_symbol(file: &BundleFile, missing: bool) -> Value {
    if missing {
        json!({
            "kind": "file",
            "qualified_path": file.path,
            "source": "",
            "missing_on_disk": true,
        })
    } else {
        json!({
            "kind": "file",
            "qualified_path": file.path,
            "source": file.source,
        })
    }
}

/// Fit-probe response for the packing loop: the full disclosure shape with
/// the current `chosen` prefix plus candidate `i`, so the serialized size
/// measured in the loop is a faithful bound on the final response.
fn bundle_probe(
    built: &[BundleFile],
    chosen: &[usize],
    candidate: usize,
    edges: &[(String, String)],
    total: usize,
    omitted_edges_outside: usize,
) -> Value {
    let mut chosen_idx: Vec<usize> = chosen.to_vec();
    chosen_idx.push(candidate);
    let chosen_set: HashSet<usize> = chosen_idx.iter().copied().collect();
    let shown: Vec<(&str, &str)> = edges
        .iter()
        .filter(|(a, b)| {
            chosen_idx.iter().any(|&j| built[j].path == *a)
                && chosen_idx.iter().any(|&j| built[j].path == *b)
        })
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let shown_set: HashSet<(&str, &str)> = shown.iter().copied().collect();
    let omitted_edges = edges
        .iter()
        .filter(|(a, b)| !shown_set.contains(&(a.as_str(), b.as_str())))
        .count()
        + omitted_edges_outside;
    let returned = chosen_idx.len();
    let omitted_files = total.saturating_sub(returned);
    let not_shown_probe: Vec<&str> = built
        .iter()
        .enumerate()
        .filter(|(i, _)| !chosen_set.contains(i))
        .map(|(_, f)| f.path.as_str())
        .collect();
    let symbols: Vec<Value> = chosen_idx
        .iter()
        .map(|&i| bundle_symbol(&built[i], built[i].missing))
        .collect();
    json!({
        "symbols": symbols,
        "edges": shown.iter().map(|(a, b)| json!({ "source": a, "target": b })).collect::<Vec<_>>(),
        "not_shown_files": not_shown_probe,
        "returned": returned,
        "total": total,
        "completeness": {
            "complete": not_shown_probe.is_empty(),
            "omitted_files": omitted_files,
            "omitted_edges": omitted_edges,
        },
    })
}

/// Same normalization as scope mode: trim trailing `/`, strip leading `./`;
/// blank or `.` normalizes to "" (invalid).
fn normalize_scope(scope: &str) -> &str {
    let trimmed = scope.trim().trim_end_matches('/');
    let stripped = trimmed.strip_prefix("./").unwrap_or(trimmed);
    if stripped.is_empty() || stripped == "." {
        ""
    } else {
        stripped
    }
}

/// Traversal guard (mirrors `safe_read_file_in_repo` / scope mode): rejects
/// `..`, absolute paths, and drive prefixes.
fn is_path_traversal(scope: &str) -> bool {
    std::path::Path::new(scope).components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

#[cfg(test)]
#[path = "tools_explore_bundle_tests.rs"]
mod tools_explore_bundle_tests;
