//! Symbol-name matching for `lievo_explore` query mode.
//!
//! Query mode previously matched only File-tier entities by name/path, so a
//! query naming a function or type (e.g. `sum_of_squares`) returned a
//! zero-match warning even though the symbol was indexed — and the warning
//! pointed agents at `search_entities`, which is hidden behind
//! `LIEVO_MCP_TOOLS`.
//!
//! This module adds symbol-name admission on top of the existing file
//! matching:
//!
//!   - Storage-side prefilter (`Storage::symbols_matching_names`): a SQL
//!     `LIKE` scan over the narrow (repo_id, path, name) projection of the
//!     non-file tiers, so a large index does not enter the process per call.
//!     Each candidate carries its own repo_id so symbols resolve against the
//!     repo they live in.
//!   - Rust-side confirmation with the SAME token matcher files use
//!     (`query_tokenizer::word_matches` on `query_words`), so symbol
//!     admission and file admission can never diverge.
//!   - Resolution to containing files via `entity_ids_for_paths` (one
//!     batched lookup per repo, using that repo's confirmed symbols only).
//!
//! Ranking (requirement: an exact symbol-name hit outranks a file path token
//! and a symbol-name prefix): the containing file's final score is the MAX of
//! the existing `score_file_entity` result and a symbol score of
//! (exact word match × 4, prefix match × 2). The file-name/path scoring
//! (`score_file_entity`) is otherwise unchanged, so existing ranking tests
//! keep passing; the symbol channel only ever raises a score, and only for
//! files admitted through a confirmed symbol.

use std::collections::{HashMap, HashSet};

use crate::model::{Entity, EntityTier};
use crate::retrieval::explore_common::should_exclude_entity;
use crate::retrieval::query_tokenizer::word_matches;
use crate::retrieval::tools::ToolContext;
use crate::retrieval::tools_explore::query_words;
use crate::storage::Storage;

/// Score contribution for a file whose symbol name EQUALS a query word.
/// Above the file-name hit (2) so it outranks every file-channel score.
pub(crate) const EXACT_SYMBOL_SCORE: i32 = 4;
/// Score contribution for a file whose symbol name is a token-prefix match
/// for a query word. Above a path-token hit (1), at a name hit (2).
pub(crate) const PREFIX_SYMBOL_SCORE: i32 = 2;

/// Reason text appended when the symbol channel outranks the file channel
/// and the symbol name EQUALS a query word.
pub(crate) const EXACT_SYMBOL_REASON: &str = "; symbol: exact name match";
/// Reason text appended when the symbol channel outranks the file channel
/// and the symbol name is a token-prefix match for a query word.
pub(crate) const PREFIX_SYMBOL_REASON: &str = "; symbol: name prefix match";

/// The entity tiers the symbol channel prefilter covers (everything that
/// carries a name and lives inside a file). Kept in one place: the SQL
/// prefilter keeps `tier <> 'file'` (the inverse of the file tier) and the
/// trait default filters by this set — both must stay in sync, so the SQL
/// side carries a comment pointing here.
pub const SYMBOL_TIERS: [EntityTier; 3] = [
    EntityTier::Function,
    EntityTier::Module,
    EntityTier::Subsystem,
];

/// Upper bound on prefilter rows returned per call. Exact-name matches sort
/// first (`ORDER BY (LOWER(name) = …) DESC`), so a common word cannot push an
/// exact match out of the window unless there are more than this many exact
/// matches.
pub const SYMBOL_PREFILTER_LIMIT: usize = 2000;

/// A symbol-tier entity returned by the storage-side prefilter (the narrow
/// `repo_id, path, name` projection — no summary or metrics blobs). `repo_id`
/// is `Option<String>` because the projection column is nullable for
/// entities stored outside a repo.
#[derive(Debug, Clone)]
pub struct SymbolCandidate {
    pub repo_id: Option<String>,
    pub path: Option<String>,
    pub name: String,
}

/// Run the storage-side prefilter. An empty word set (all stop words) yields
/// an empty vector without a SQL call; a storage error degrades to the file
/// channel only (never an error out of query mode) — the degraded call is
/// logged via `tracing` (no stdout; MCP stdio).
fn prefilter_symbols<S: Storage>(
    storage: &S,
    project_id: &str,
    query: &str,
) -> Vec<SymbolCandidate> {
    let words = query_words(query);
    if words.is_empty() {
        return Vec::new();
    }
    // The trait takes plain lowercase words; the SQL-side LIKE pattern
    // formatting lives in the SqliteStorage override (single construction
    // site there).
    storage
        .symbols_matching_names(project_id, &words)
        .unwrap_or_else(|err| {
            tracing::warn!("lievo_explore symbol prefilter degraded to file channel only: {err}");
            Vec::new()
        })
}

/// Confirm a prefiltered candidate with the token matcher: the lowercased
/// symbol name must have a query word that is a token-boundary prefix of a
/// name token (the same predicate that admits files). Returns
/// `(is_exact, is_prefix)`; `None` when no query word hits the name.
fn symbol_name_hits(name: &str, words: &[String]) -> Option<(bool, bool)> {
    let name_lc = name.to_lowercase();
    let mut exact = false;
    let mut prefix = false;
    for w in words {
        if word_matches(&name_lc, w) {
            prefix = true;
        }
        if name_lc == *w {
            exact = true;
        }
    }
    if exact || prefix {
        Some((exact, prefix))
    } else {
        None
    }
}

/// Match file entities against the query words (name or path) AND confirm
/// symbol-tier entities by name, resolving each confirmed symbol to its
/// containing file via `entity_ids_for_paths`.
///
/// Returns `(matched_files, symbol_score)`: `matched_files` is the
/// de-duplicated union of direct file matches and containing files of
/// confirmed symbols (admission order: direct file matches first);
/// `symbol_score` maps a file entity id to its symbol-channel score, which
/// the caller applies as `max(file score, symbol score)`. Output-dir paths
/// are excluded everywhere; symbols with no path (module/subsystem
/// groupings) cannot resolve to a file and are dropped.
pub(crate) fn matching_entities<S: Storage>(
    storage: &S,
    ctx: &ToolContext<S>,
    query: &str,
) -> (Vec<Entity>, HashMap<String, i32>) {
    let words = query_words(query);
    if words.is_empty() {
        return (Vec::new(), HashMap::new());
    }

    // --- File channel (unchanged admission) ---
    let direct = storage
        .list_entities(&ctx.project_id, Some(EntityTier::File))
        .unwrap_or_default()
        .into_iter()
        .filter(|e| !should_exclude_entity(e.path.as_deref(), &ctx.output_dir))
        .filter(|e| {
            let name = e.name.to_lowercase();
            let path = e.path.as_deref().unwrap_or("").to_lowercase();
            words
                .iter()
                .any(|w| word_matches(&name, w) || word_matches(&path, w))
        });

    // --- Symbol channel ---
    // Each candidate carries its own repo from the prefilter projection, so
    // a symbol in repo B is found even when repo A (listed first) has a file
    // at the same relative path — confirmed symbols are grouped by repo_id
    // and `entity_ids_for_paths` is called once per repo with that repo's
    // own paths.
    let confirmed: Vec<SymbolCandidate> = prefilter_symbols(storage, &ctx.project_id, query)
        .into_iter()
        .filter(|c| {
            c.path.is_some() && c.repo_id.is_some() && symbol_name_hits(&c.name, &words).is_some()
        })
        .collect();

    let mut symbol_score: HashMap<String, i32> = HashMap::new();
    let mut seen_file_ids: HashSet<String> = HashSet::new();
    let mut containing_file_ids: Vec<String> = Vec::new();
    if !confirmed.is_empty() {
        // Group confirmed symbols by their own repo_id; distinct paths per
        // repo (sorted for a stable lookup).
        let mut paths_by_repo: HashMap<String, Vec<String>> = HashMap::new();
        let mut symbols_by_repo: HashMap<String, Vec<&SymbolCandidate>> = HashMap::new();
        for c in &confirmed {
            let Some(repo_id) = c.repo_id.clone() else {
                continue;
            };
            let Some(path) = c.path.clone() else {
                continue;
            };
            symbols_by_repo.entry(repo_id.clone()).or_default().push(c);
            paths_by_repo.entry(repo_id).or_default().push(path);
        }
        for paths in paths_by_repo.values_mut() {
            paths.sort();
            paths.dedup();
        }
        for (repo_id, repo_paths) in &paths_by_repo {
            let repo_paths_slice: Vec<&str> = repo_paths.iter().map(String::as_str).collect();
            let id_by_path = match storage.entity_ids_for_paths(repo_id, &repo_paths_slice) {
                Ok(map) => map,
                Err(err) => {
                    tracing::warn!(
                        "lievo_explore symbol resolution degraded for repo {repo_id}: {err}"
                    );
                    continue;
                }
            };
            let Some(symbols) = symbols_by_repo.get(repo_id) else {
                continue;
            };
            for c in symbols {
                let Some(path) = c.path.as_deref() else {
                    continue;
                };
                let Some(file_id) = id_by_path.get(path) else {
                    continue;
                };
                let Some(hits) = symbol_name_hits(&c.name, &words) else {
                    continue;
                };
                let score = if hits.0 {
                    EXACT_SYMBOL_SCORE
                } else {
                    PREFIX_SYMBOL_SCORE
                };
                // A file with both an exact and a prefix symbol takes the
                // exact score.
                match symbol_score.get_mut(file_id) {
                    Some(existing) => {
                        if score > *existing {
                            *existing = score;
                        }
                    }
                    None => {
                        symbol_score.insert(file_id.clone(), score);
                    }
                }
                // De-duplicate containing files while preserving first-seen
                // (admission) order.
                if seen_file_ids.insert(file_id.clone()) {
                    containing_file_ids.push(file_id.clone());
                }
            }
        }
    }

    // --- Merge, de-duplicate, keep admission order ---
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut matched: Vec<Entity> = direct.filter(|e| seen_ids.insert(e.id.clone())).collect();

    for file_id in &containing_file_ids {
        let Some(entity) = storage.get_entity(file_id).unwrap_or_else(|err| {
            // Storage degraded for one file: log, keep the rest, never
            // surface an error out of query mode.
            tracing::debug!("lievo_explore skipping containing file {file_id}: {err}");
            None
        }) else {
            // Ok(None): the file entity is gone (stale prefilter row) — skip
            // silently.
            continue;
        };
        if entity.tier == EntityTier::File
            && !should_exclude_entity(entity.path.as_deref(), &ctx.output_dir)
            && seen_ids.insert(entity.id.clone())
        {
            matched.push(entity);
        }
    }
    (matched, symbol_score)
}

#[cfg(test)]
#[path = "tools_explore_symbols_tests.rs"]
mod tests;
