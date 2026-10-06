// ExploreTool (issues #680/#682/#711).
//
// Two-tier progressive disclosure:
//   Tier 1 (default) — per-symbol map: name, kind, qualified_path, signature,
//     one-line summary, call paths, blast radius. No source bodies.
//   Tier 2 (include_source=true) — verbatim line-numbered source.
//
// Issue #711: file tier is relevance-ranked (name hits > path hits, stable
// path tiebreak) with a width cap binding before symbol-building, per-file
// score + reason, and not-shown/completeness disclosures.
//
// All output is hard-capped at 24K chars (UTF-8-char-boundary safe) with a
// truncation signal and a countable continuation pointer. The tool composes
// the same storage primitives as search/read tools; it never spawns apfel on
// the hot path — it consumes the already-stored Entity.summary column only.

use serde_json::{Value, json};

use crate::model::Entity;
use crate::retrieval::explore_common::{continuation_pointer, lock_storage};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::{ExploreTool, ToolContext};
use crate::retrieval::tools_explore_bundle::maybe_bundle_listing;
use crate::retrieval::tools_explore_files::maybe_files_batch;
use crate::retrieval::tools_explore_in_progress::indexing_in_progress_response;
use crate::retrieval::tools_explore_symbols::matching_entities;
use crate::storage::Storage;

/// Hard output cap (chars) for the tool's serialized response (issue #680).
pub const MAX_EXPLORE_OUTPUT_CHARS: usize = 24_000;

/// Default width cap: matched file entities returned when `max_files` is absent.
pub const DEFAULT_MAX_FILES: usize = 8;

/// Hard ceiling for `max_files`, mirroring search_entities' limit clamp.
pub const MAX_MAX_FILES: usize = 30;

use crate::retrieval::explore_cap::cap_response;
use crate::retrieval::explore_ranking::rank_and_cut_with_symbols;
pub use crate::retrieval::tools_explore_format::{
    SMALL_BODY_THRESHOLD_CHARS, is_small_body, line_numbered_source,
};
use crate::retrieval::tools_explore_scope;

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Filter the query into the word set used for both admission and scoring.
/// Each whitespace token is kept whole, lowercased, deduped, and must be
/// length >= 2. Common English stop-words carried by natural-language queries
/// (issue #837) are dropped here: an all-stop-word query therefore returns an
/// empty word set, and the existing empty-words early return in
/// `matching_entities` degrades gracefully instead of matching everything.
pub(crate) fn query_words(query: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    query
        .to_lowercase()
        .split_whitespace()
        .filter(|w| w.len() >= 2)
        .filter(|w| !crate::retrieval::query_tokenizer::is_stop_word(w))
        .filter(|w| seen.insert(w.to_string()))
        .take(10)
        .map(String::from)
        .collect()
}

// Storage-side helpers

/// Read a repo-relative path to string, containing path traversal and
/// escaping the repo root the same way ReadFileTool does.
pub(crate) fn safe_read_file_in_repo(
    repo_path: &std::path::Path,
    rel_path: &str,
) -> Option<String> {
    use std::path::Component;
    let p = std::path::Path::new(rel_path);
    for component in p.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return None;
        }
    }
    if repo_path.as_os_str().is_empty() {
        return None;
    }
    let full = repo_path.join(rel_path);
    let canonical_repo = repo_path.canonicalize().ok()?;
    let canonical_full = full.canonicalize().ok()?;
    if !canonical_full.starts_with(&canonical_repo) {
        return None;
    }
    std::fs::read_to_string(canonical_full).ok()
}

use crate::retrieval::tools_explore_blast::{
    call_paths_and_blast_radius, file_function_entity_id, lean_dependents_hint,
};

// Symbol mapping (tier 1 / tier 2 per matched file)

/// Build the per-symbol entry for a matched file entity. Tier 1 carries no
/// source body (except for small bodies — Complexity Trap); tier 2 carries
/// verbatim line-numbered source in `source`. When `include_depth` is false,
/// the call_paths/blast_radius keys are omitted entirely (and the tier-2
/// relationship-scan storage reads are skipped — issue #723); instead a cheap
/// hop-0 dependent count is disclosed as a `completeness` hint (issue #767),
/// so a lean response is self-describing about the impact data it withheld.
pub(crate) fn build_symbol(
    storage: &dyn Storage,
    ctx: &ToolContext<impl Storage>,
    file: &Entity,
    include_source: bool,
    include_depth: bool,
) -> Value {
    let body = safe_read_file_in_repo(&ctx.repo_path, file.path.as_deref().unwrap_or(""));

    let is_small = is_small_body(body.as_deref());

    // `entity_id` is intentionally absent from the per-symbol payload
    // (issue #834): for a File-tier symbol it is derivable as
    // `{project_id}:{repo_name}:file:{normalized_path}` from `qualified_path`
    // and adds no signal the caller cannot reconstruct. No internal reader
    // round-trips this field into a storage call (blast/scope payloads are
    // terminal, and the cap stub in `explore_cap::cap_response` reads it as
    // `Option`, eliding a missing key to JSON null rather than failing).
    let mut symbol = json!({
        "name": file.name,
        "kind": "file",
        "qualified_path": file.path,
        "language": file.language,
    });
    if include_depth {
        let (call_paths, blast_radius, call_path_errors, blast_radius_errors, blast_complete) =
            call_paths_and_blast_radius(storage, file);
        symbol["call_paths"] = json!(call_paths);
        symbol["blast_radius"] = json!(blast_radius);
        // Per-symbol completeness signal (issue #854): explicit — true when
        // the two-level traversal finished without truncation, false when the
        // cap bound or a storage error truncated the closure (the omission is
        // quantified in blast_radius_errors). Absent only on symbols that
        // carry no depth (lean / depth shed under the 24K cap).
        symbol["blast_radius_complete"] = json!(blast_complete);
        if !call_path_errors.is_empty() {
            symbol["call_path_errors"] = json!(call_path_errors);
        }
        if !blast_radius_errors.is_empty() {
            symbol["blast_radius_errors"] = json!(blast_radius_errors);
        }
    } else if let Some(hint) = lean_dependents_hint(storage, file) {
        symbol["completeness"] = json!(hint);
    }

    // Signature: stored on the Function-tier entity of the file (via its
    // metrics_json) when available; otherwise the first line of the file body
    // (a crude hint). No tree-sitter lookup on the hot path.
    let signature = match file_function_entity_id(storage, file) {
        Some(fid) => {
            let fn_entity = storage.get_entity(&fid).ok().flatten();
            fn_entity
                .as_ref()
                .and_then(|f| {
                    f.metrics_json
                        .as_deref()
                        .and_then(|m| serde_json::from_str::<Value>(m).ok())
                        .and_then(|m| {
                            m.get("signature")
                                .and_then(|s| s.as_str())
                                .map(String::from)
                        })
                })
                .or_else(|| {
                    body.as_deref()
                        .and_then(|b| b.lines().next())
                        .map(|l| l.trim().to_string())
                })
        }
        None => body
            .as_deref()
            .and_then(|b| b.lines().next())
            .map(|l| l.trim().to_string()),
    };
    if let Some(sig) = signature {
        symbol["signature"] = json!(sig);
    }

    // Complexity Trap: small bodies inline, large bodies get a stored summary.
    if include_source {
        match &body {
            Some(b) => {
                symbol["source"] = json!(line_numbered_source(b));
                symbol["source_truncated"] = json!(false);
            }
            None => {
                symbol["source"] = json!(String::new());
                symbol["source_truncated"] = json!(false);
            }
        }
    } else if is_small {
        // Tier 1: body inline, NO summary field (Complexity Trap).
        symbol["source"] = json!(line_numbered_source(body.as_deref().unwrap_or("")));
        symbol["small_body_inline"] = json!(true);
    } else if let Some(stored) = file.summary.as_deref()
        && !stored.is_empty()
    {
        symbol["summary"] = json!(stored);
    }

    symbol
}

// ---------------------------------------------------------------------------
// ExploreTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for ExploreTool<S> {
    fn name(&self) -> &str {
        "explore"
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        // Issue #863 (binding decisions 4 & 5): success-shaped guidance
        // instead of any mode when no repo is being served.
        if let Some(g) = &self.ctx.zero_repo_guidance {
            return Ok(json!({ "symbols": [], "warning": g }).to_string());
        }

        // Issue #864: while a background index is running (any lievo mcp
        // process on this repo, detected via the per-repo cross-process
        // lock), return a success-shaped `indexing: true` response with the
        // trigger and elapsed — and, for a first index with no partial
        // index yet, steer the agent to its built-in tools. Results from a
        // partial (already-present) index are still returned but marked
        // `index_incomplete: true` (#838/#848 semantics).
        if let Some(r) = indexing_in_progress_response(&self.ctx) {
            return r;
        }

        // Bundle mode (#743): an explicit `bundle` scope short-circuits
        // EVERYTHING else (files batch, empty-query warning, scope listing,
        // word-match) — it is a scope selection carrying no meaningful query.
        if let Some(r) = maybe_bundle_listing(&self.ctx, &input) {
            return r;
        }

        // Files-list mode (#741): a non-empty `files` array short-circuits
        // EVERYTHING else (the empty-query warning, scope listing, word-match)
        // — files-only calls carry no query, and the caller passes all needed
        // paths in one call; the response packs as many as fit under the 24K
        // cap, the rest via the existing continuation pointer.
        if let Some(r) = maybe_files_batch(&self.ctx, &input) {
            return r;
        }

        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        let q = query.trim();
        // Empty/missing query is a recoverable condition: success-shaped
        // guidance naming the remediation, not an error (issue #680 P1.2).
        if q.is_empty() {
            return Ok(
                json!({
                    "symbols": [],
                    "warning": "Query is empty. Pass a keyword or name fragment, e.g. lievo_explore(query='auth')."
                })
                .to_string(),
            );
        }

        let include_source = input
            .get("include_source")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let include_depth = input
            .get("include_depth")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let max_files = input
            .get("max_files")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, MAX_MAX_FILES as u64) as usize)
            .unwrap_or(DEFAULT_MAX_FILES);

        // Scope mode (#712): `scope` param is the sole trigger, before word-match.
        if let Some(r) = tools_explore_scope::maybe_scope_listing(&self.ctx, &input, max_files) {
            return r;
        }

        let guard = lock_storage!(self.ctx.storage);
        let (matched, symbol_score) = matching_entities(&*guard, &self.ctx, q);
        let total = matched.len();
        drop(guard);

        if matched.is_empty() {
            let guard = lock_storage!(self.ctx.storage);
            let has_entities = !guard
                .list_entities(&self.ctx.project_id, None)
                .unwrap_or_default()
                .is_empty();
            drop(guard);
            let guidance = if has_entities {
                // Requirement 5: no longer recommend search_entities (hidden
                // behind LIEVO_MCP_TOOLS by default). One neutral message.
                "No matching files or symbols. Try a different name, pass scope='<dir>' to list a directory, or use your built-in search."
            } else {
                // Issue #864: the not-indexed guidance no longer tells the
                // agent to run `lievo refresh` (the agent cannot run lievo
                // commands while the MCP server is running; indexing is
                // automatic in the background). Steer the agent to its
                // built-in tools and a short retry instead.
                "No entities indexed yet. The server indexes the repository automatically in the background when it starts — use your built-in tools for now and retry this call in a moment."
            };
            return Ok(json!({
                "symbols": [],
                "warning": guidance
            })
            .to_string());
        }

        // Width cap binds BEFORE symbol-building (issue #711): rank by
        // relevance (name hits > path hits, path-stable tiebreak) and cut to
        // max_files so no per-file fs I/O or relationship-scan work is spent
        // on files that will not be shown. A confirmed symbol raises the file
        // score (exact symbol-name match > symbol prefix > name > path token).
        let words = query_words(q);
        let (matched_trimmed, not_shown) =
            rank_and_cut_with_symbols(matched, &words, max_files, &symbol_score);
        let returned = matched_trimmed.len();

        let guard = lock_storage!(self.ctx.storage);
        let symbols: Vec<Value> = matched_trimmed
            .iter()
            .map(|(e, score, reason)| {
                let mut symbol = build_symbol(&*guard, &self.ctx, e, include_source, include_depth);
                symbol["score"] = json!(*score);
                symbol["reason"] = json!(*reason);
                symbol
            })
            .collect();
        drop(guard);

        let mut response = json!({
            "symbols": symbols,
        });

        if returned < total {
            // Breadth was truncated: not-shown count + completeness line so an
            // agent can tell truncated breadth from exhausted results. The
            // continuation names lievo_explore itself — `search_entities` is
            // hidden behind LIEVO_MCP_TOOLS by default.
            let next_tool = format!(
                "lievo_explore(query='{}', max_files={})",
                q,
                total.min(MAX_MAX_FILES)
            );
            response["continuation"] = json!(continuation_pointer(returned, total, &next_tool));
            response["not_shown"] = json!(not_shown);
            response["completeness"] =
                json!(format!("showing {returned} of {total} matching files"));
        }

        let serialized = response.to_string();
        if serialized.chars().count() > MAX_EXPLORE_OUTPUT_CHARS
            && let Some(capped) = cap_response(&response)
        {
            return Ok(capped);
        }

        Ok(serialized)
    }
}

// --- Tests ---

#[cfg(test)]
#[path = "tools_explore_tests.rs"]
mod tools_explore_tests;

#[cfg(test)]
#[path = "tools_explore_tier_tests.rs"]
mod tools_explore_tier_tests;

#[cfg(test)]
#[path = "tools_explore_depth_tests.rs"]
mod tools_explore_depth_tests;

#[cfg(test)]
#[path = "tools_explore_blast_tool_tests.rs"]
mod tools_explore_blast_tool_tests;
