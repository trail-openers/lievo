//! Batch file-list retrieval mode for `lievo_explore` (issue #741).
//!
//! A non-empty `files` array short-circuits the word-match and scope-listing
//! paths: the caller passes all needed repo-relative paths in ONE call and the
//! response packs as many files as fit under the 24K output cap, in requested
//! order. The remainder is surfaced through the EXISTING continuation-pointer
//! mechanism (returned/total/next) — `next` names the exact remaining paths
//! so the follow-up call carries them verbatim.
//!
//! Rules (issue #741 edge cases):
//!   - dedup: duplicate paths collapse, first-occurrence order preserved
//!   - invalid paths (traversal or not in the index) never fail the batch —
//!     they are reported in `not_found` and the rest still returns
//!   - deterministic: identical request → identical set in requested order
//!   - a single oversized file is never dropped silently: it goes first, shed
//!     depth, then truncate its source on a whole-line boundary, and only as a
//!     last resort drop the source and return the lean symbol (issue #838)
//!
//! Packing budget (issue #838):
//!   - the first requested file may keep guaranteed representation, but it is
//!     budgeted like any other: its lean probe is measured, so one large file
//!     with include_depth=true can no longer consume the whole 24K budget and
//!     starve every other requested file
//!   - per-file depth (call_paths/blast_radius) is probed as a LEAN shape and
//!     is only built for a file once it is chosen — a file that fails the fit
//!     check never has its depth constructed
//!   - when a chosen file's full-depth payload would overflow, its depth is shed
//!     (depth is dropped before a whole file is dropped), and the depth-shed
//!     files are demoted in order until the batch fits: 4 lean files beat 1 deep
//!     file, because the agent's next turn is what costs tokens
//!
//! `files` honours `include_source`/`include_depth` exactly like word-match:
//! include_source=false → tier-1 lean map (no source bodies, small bodies
//! inlined, large bodies → stored summary); include_source=true → verbatim
//! line-numbered source per file.

use std::path::Component;

use serde_json::{Map, Value};

use crate::model::{Entity, EntityTier};
use crate::retrieval::explore_cap::cap_response;
use crate::retrieval::explore_common::lock_storage;
use crate::retrieval::tools::ToolContext;
use crate::retrieval::tools_explore::{
    MAX_EXPLORE_OUTPUT_CHARS, build_symbol, safe_read_file_in_repo,
};
use crate::retrieval::tools_explore_files_asm::{
    FilesResponseCtx, files_probe, files_response, single_fits,
};
use crate::storage::Storage;

/// Entry point: read `files`/`include_source`/`include_depth` from the call
/// input and, when a non-empty `files` array is present, run the batch
/// file-list path instead of scope/word-match. Called at the top of
/// `ExploreTool::call` (before the empty-query warning, the scope listing,
/// and the word match). Returns `None` when `files` is absent or empty,
/// signaling the caller to fall through unchanged.
pub(crate) fn maybe_files_batch<S: Storage>(
    ctx: &std::sync::Arc<ToolContext<S>>,
    input: &Value,
) -> Option<crate::Result<String>> {
    let paths: Vec<String> = input
        .get("files")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .filter(|paths: &Vec<String>| !paths.is_empty())?;

    let include_source = input
        .get("include_source")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let include_depth = input
        .get("include_depth")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    Some(files_batch(ctx, &paths, include_source, include_depth))
}

/// Resolve + pack the deduplicated path list under the 24K cap.
fn files_batch<S: Storage>(
    ctx: &ToolContext<S>,
    paths: &[String],
    include_source: bool,
    include_depth: bool,
) -> crate::Result<String> {
    // Dedup: first-occurrence order preserved.
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<&String> = Vec::new();
    for p in paths {
        if seen.insert(p.as_str()) {
            unique.push(p);
        }
    }

    // Resolution: the index is the source of truth for a file's repo-relative
    // path (entity.path). Paths that are traversal attempts, not in the
    // index, or unreadable from disk are reported in `not_found` — they
    // never fail the batch.
    let guard = lock_storage!(ctx.storage);
    let all_files = guard
        .list_entities(&ctx.project_id, Some(EntityTier::File))
        .unwrap_or_default();
    drop(guard);

    let total = unique.len();
    let mut resolved: Vec<(Entity, String)> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();
    for p in unique {
        if is_path_traversal(p) {
            not_found.push(p.clone());
            continue;
        }
        match find_indexed(&all_files, p) {
            Some(entity) => {
                let path = entity.path.as_deref().unwrap_or_default().to_string();
                match safe_read_file_in_repo(&ctx.repo_path, &path) {
                    Some(body) => resolved.push((entity.clone(), body)),
                    None => not_found.push(p.clone()),
                }
            }
            None => not_found.push(p.clone()),
        }
    }

    // Cap-bound packing, in requested order (deterministic). Per-file budget:
    // every resolved file is probed at its LEAN payload (identity, signature,
    // and source/summary — no call_paths/blast_radius). The FIRST file is
    // always taken (guaranteed representation — an oversized single file is
    // never dropped silently; the ladder below trims it), but it is still
    // budgeted like any other, so a single large file with
    // include_depth=true can no longer consume the whole 24K budget and starve
    // every other requested file. A later file that fails the lean fit check
    // is dropped for budget reasons (never returned, and its depth is never
    // built — no relationship-scan work spent on it). The leftover budget from
    // small files is reusable by remaining files, so a batch is not
    // under-filled when some files are small.
    let guard = lock_storage!(ctx.storage);
    let mut chosen: Vec<(Entity, String, Value)> = Vec::new();
    let mut remainder_paths: Vec<String> = Vec::new();
    let items: Vec<(Entity, String)> = resolved;
    for (i, (entity, body)) in items.into_iter().enumerate() {
        let lean = build_symbol(&*guard, ctx, &entity, include_source, false);
        if i == 0 {
            // First file: guaranteed representation (never dropped), even if
            // its lean payload alone exceeds the cap — the ladder trims it.
            chosen.push((entity, body, lean));
            continue;
        }
        let mut probe_symbols: Vec<Value> = chosen.iter().map(|c| c.2.clone()).collect();
        probe_symbols.push(lean.clone());
        let probe = files_probe(&probe_symbols, &remainder_paths, total, 0);
        if probe.to_string().chars().count() <= MAX_EXPLORE_OUTPUT_CHARS {
            chosen.push((entity, body, lean));
        } else {
            remainder_paths.push(entity.path.as_deref().unwrap_or("").to_string());
        }
    }
    drop(guard);

    // Depth is built ONLY for chosen files, after they clear the lean fit
    // check (issue #838 decision 5: a file that fails the fit check never has
    // its call_paths/blast_radius constructed). The loop runs for EVERY chosen
    // file count — including a single resolved file, which is the shape a
    // multi-path request takes when exactly one path resolves (issue #848:
    // the old `n > 1` gate made include_depth=true a silent no-op for n == 1,
    // returning the lean symbol byte-identically).
    //
    // Depth-shed order (issue #838, unchanged): if a file's full-depth build
    // overflows the cap, its depth is shed before whole files are dropped —
    // 4 lean files beat 1 deep file, because the agent's next turn is what
    // costs tokens. A file that fails its depth build is reverted to its
    // LEAN symbol and the next file tries with the leftover budget
    // (revert-and-continue, unchanged from the pre-#848 loop). The loop now
    // runs to the end for every chosen file (issue #848: no early `break`
    // that skipped depth for the remaining files), and the shed state of
    // each file is tracked explicitly in `depth_shed` (issue #848 decision
    // #3), so the withheld hint is applied by flag, never by
    // string-matching a remedy.
    // shed under the cap (built, overflowed, reverted). The state is tracked
    // explicitly in `depth_shed` (issue #848 decision #3) — never a remedy
    // string-match — and feeds the withheld-hint rewrite and the
    // top-level completeness gate below.
    let guard = lock_storage!(ctx.storage);
    let n = chosen.len();
    let mut depth_shed = vec![false; n];
    let mut depth_withheld = false;
    for i in 0..n {
        let entity = chosen[i].0.clone();
        let full = build_symbol(&*guard, ctx, &entity, include_source, include_depth);
        chosen[i].2 = full.clone();
        let symbols: Vec<Value> = chosen.iter().map(|c| c.2.clone()).collect();
        let probe = files_probe(&symbols, &remainder_paths, total, 0);
        if probe.to_string().chars().count() <= MAX_EXPLORE_OUTPUT_CHARS {
            // The batch fits so far: keep building depth for the remaining
            // chosen files (issue #848: every requested file gets its depth
            // built; the shed pass below trims what overflows the cap).
            continue;
        }
        // Full-depth overflows: shed this file's depth (revert to its lean
        // payload) and let the next file try with the leftover budget.
        let mut reverted = build_symbol(&*guard, ctx, &entity, include_source, false);
        if include_depth {
            depth_shed[i] = true;
            depth_withheld = true;
            // The per-symbol hint must say the depth was WITHHELD under the
            // cap (decision #4) — the caller already passed include_depth,
            // so the lean "pass include_depth=true" remedy is a lie.
            if let Some(hint) = reverted
                .get_mut("completeness")
                .and_then(Value::as_object_mut)
            {
                hint.insert(
                    "remedy".to_string(),
                    Value::String(
                        "depth withheld under the 24K output cap — request this file alone with include_depth=true"
                            .to_string(),
                    ),
                );
            }
        }
        chosen[i].2 = reverted;
    }
    drop(guard);

    // The shed state is resolved explicitly (issue #848 decision #3): the
    // overflow loop above sets `depth_shed[i]` when it reverted file i, and
    // when the single-file ladder below sheds step (a) it returns the flag.
    // No post-hoc string-matching of remedies and no recomputed dependent
    // counts.
    let depth_requested = include_depth;

    let symbols: Vec<Value> = chosen.iter().map(|c| c.2.clone()).collect();

    // Any remaining overflow (e.g. a single oversized file whose lean payload
    // alone exceeds the cap) is resolved by the single-file ladder below,
    // before the response is assembled.
    let (mut symbols, source_truncated, ladder_depth_shed) = ladder_overflow(&symbols);
    if depth_requested && ladder_depth_shed {
        // The single oversized file's depth was requested but never
        // delivered: rewrite its per-symbol hint to the depth-withheld
        // remedy (decision #4), and the flag folds into the completeness
        // gate below.
        if !symbols.is_empty()
            && let Some(hint) = symbols[0]
                .get_mut("completeness")
                .and_then(Value::as_object_mut)
        {
            hint.insert(
                    "remedy".to_string(),
                    Value::String(
                        "depth withheld under the 24K output cap — request this file alone with include_depth=true"
                            .to_string(),
                    ),
                );
        }
        depth_withheld = true;
    }

    let not_shown_files: Vec<String> = remainder_paths.clone();
    // `complete` must mean the agent can stop: when depth was requested and a
    // returned symbol's depth was shed under the cap, the response is
    // incomplete even though every requested file is present (issue #848).
    // Gated on `include_depth` so an ordinary lean request (never asked for
    // depth) keeps reporting complete:true.
    let complete =
        not_shown_files.is_empty() && !source_truncated && !(depth_requested && depth_withheld);
    let omitted_files = not_shown_files.len();

    let response = files_response(&FilesResponseCtx {
        symbols: &symbols,
        not_shown_files: &not_shown_files,
        not_found: &not_found,
        total,
        include_source,
        complete,
        omitted_files,
    });

    let serialized = response.to_string();
    if serialized.chars().count() > MAX_EXPLORE_OUTPUT_CHARS
        && let Some(capped) = cap_response(&response)
    {
        return Ok(capped);
    }
    Ok(serialized)
}

/// Single-oversized-file ladder (issue #838 decision 1): when the packed
/// symbols alone still exceed the cap, shed the single file's depth, then
/// truncate its `source` at a whole-line boundary (keeping the symbol
/// well-formed), and only as a last resort drop `source` and return the lean
/// symbol. Never splices serialized JSON text. Returns the trimmed symbols,
/// whether any returned file's source was truncated or shed (which demotes
/// `completeness.complete` to false), and whether the ladder shed step (a)
/// removed a built depth payload (issue #848 decision #3: feeds the
/// explicit shed flag, never a remedy string-match).
fn ladder_overflow(symbols: &[Value]) -> (Vec<Value>, bool, bool) {
    let serialized = Value::Array(symbols.to_vec()).to_string();
    if serialized.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS {
        return (symbols.to_vec(), false, false);
    }
    // Only the single-file case is laddered here (multi-file overflow is
    // handled by the depth-shed + per-file budget above). For a single
    // oversized file, walk the ladder on that one symbol.
    let out: Vec<Value> = symbols.to_vec();
    if out.len() != 1 {
        return (out, false, false);
    }
    let sym = out[0].clone();
    // (a) shed depth: drop call_paths/blast_radius (+ error fields).
    let mut shed = sym.clone();
    for key in [
        "call_paths",
        "blast_radius",
        "call_path_errors",
        "blast_radius_errors",
    ] {
        shed.as_object_mut().unwrap().remove(key);
    }
    let depth_removed = sym.get("blast_radius").is_some() || sym.get("call_paths").is_some();
    if single_fits(&shed) {
        return (vec![shed], false, depth_removed);
    }
    // (b) truncate `source` at a whole-line boundary, keeping the symbol
    //     well-formed. The source is line-numbered ("N\t...\n" per line), so
    //     a whole-line cut is a clean boundary.
    let obj = shed.as_object_mut().unwrap();
    let src_owned = obj.get("source").and_then(Value::as_str).map(String::from);
    if let Some(src) = src_owned {
        let target = max_source_chars(&Value::Object(obj.clone()));
        if target > 0
            && let Some(cut) = last_line_boundary(&src, target)
        {
            let truncated: String = src.chars().take(cut).collect();
            obj.insert("source".to_string(), Value::String(truncated));
            obj.insert("source_truncated".to_string(), Value::Bool(true));
            let candidate = Value::Object(obj.clone());
            if single_fits(&candidate) {
                return (vec![candidate], true, depth_removed);
            }
        }
    }
    // (c) drop `source` entirely, return the lean symbol.
    let mut lean = shed.clone();
    if let Some(obj) = lean.as_object_mut() {
        obj.remove("source");
        obj.remove("source_truncated");
    }
    let c_depth_removed = lean.get("blast_radius").is_some() || lean.get("call_paths").is_some();
    (vec![lean], true, c_depth_removed)
}

/// Largest whole-line boundary of `src` (in chars, after the final newline) at
/// or under `target`; `None` if no full line fits.
fn last_line_boundary(src: &str, target: usize) -> Option<usize> {
    src.rfind('\n')
        .map(|b| b + 1)
        .and_then(|b| (b <= target).then_some(b))
}

/// The maximum `source` char length that leaves room for the rest of the
/// single-file response (identity + framing + the source_truncated flag).
fn max_source_chars(sym: &Value) -> usize {
    let map = sym.as_object().unwrap();
    let base_map: Map<String, Value> = map
        .iter()
        .filter(|(k, _)| k.as_str() != "source" && k.as_str() != "source_truncated")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let base = Value::Object(base_map);
    let probe = files_probe(&[base], &[], 1, 0);
    let base_chars = probe.to_string().chars().count();
    // Reserve room for the `source` value's JSON escaping + the flag.
    MAX_EXPLORE_OUTPUT_CHARS
        .saturating_sub(base_chars)
        .saturating_sub(128)
}

/// Same traversal check as `safe_read_file_in_repo` (rejects `..`,
/// absolute paths, and drive prefixes) — applied to the raw `files`
/// entries BEFORE index lookup so a traversal string can never be
/// mistaken for an indexed path.
fn is_path_traversal(path: &str) -> bool {
    std::path::Path::new(path).components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

/// Find the indexed file entity whose repo-relative path equals `requested`
/// (exact match), or whose stored path's file-stem equals the requested
/// name (so `auth` resolves `src/auth.rs` when exactly one file shares the
/// stem — deterministic only for a unique match).
fn find_indexed<'a>(entities: &'a [Entity], requested: &str) -> Option<&'a Entity> {
    if let Some(exact) = entities
        .iter()
        .find(|e| e.path.as_deref() == Some(requested))
    {
        return Some(exact);
    }
    let stem = requested
        .rsplit('/')
        .next()
        .unwrap_or(requested)
        .trim_end_matches(".rs");
    let matches: Vec<&Entity> = entities
        .iter()
        .filter(|e| {
            e.path
                .as_deref()
                .and_then(|p| p.rsplit('/').next())
                .map(|base| base == stem || base == format!("{stem}.rs"))
                .unwrap_or(false)
        })
        .collect();
    (matches.len() == 1).then(|| matches[0])
}

#[cfg(test)]
#[path = "tools_explore_files_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tools_explore_files_depth_tests.rs"]
mod depth_tests;
