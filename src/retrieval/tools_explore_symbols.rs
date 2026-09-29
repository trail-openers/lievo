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
//!     `LIKE` scan over the narrow (path, name) projection of the non-file
//!     tiers (`build_symbol_name_prefilter_query`), so a large index does not
//!     enter the process per call.
//!   - Rust-side confirmation with the SAME token matcher files use
//!     (`query_tokenizer::word_matches` on `query_words`), so symbol
//!     admission and file admission can never diverge.
//!   - Resolution to containing files via `entity_ids_for_paths` (one
//!     batched lookup per repo).
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

/// A symbol-tier entity returned by the storage-side prefilter (the narrow
/// `path, name` projection — no id, summary, or metrics blobs).
#[derive(Debug, Clone)]
pub struct SymbolCandidate {
    pub path: Option<String>,
    pub name: String,
}

/// Run the storage-side prefilter. An empty word set (all stop words) yields
/// an empty vector without a SQL call; a storage error degrades to the file
/// channel only (never an error out of query mode).
fn prefilter_symbols<S: Storage>(
    storage: &S,
    project_id: &str,
    query: &str,
) -> Vec<SymbolCandidate> {
    let words = query_words(query);
    if words.is_empty() {
        return Vec::new();
    }
    // Params: project_id first, then one `%word%` LIKE term per query word —
    // the exact shape `build_symbol_name_prefilter_query(words.len())`
    // documents. Kept in one place so the param contract has a single
    // construction site (see symbol_prefilter_params).
    let strings = symbol_prefilter_params(project_id, &words).unwrap_or_default();
    storage.symbols_matching_names(&strings).unwrap_or_default()
}

/// Build the prefilter param vector as (project_id, formatted word LIKE
/// terms). Pure helper so the param shape has a single construction site
/// with a test.
pub(crate) fn symbol_prefilter_params(project_id: &str, words: &[String]) -> Option<Vec<String>> {
    if words.is_empty() {
        return None;
    }
    Some(
        std::iter::once(project_id.to_string())
            .chain(words.iter().map(|w| format!("%{w}%")))
            .collect(),
    )
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
    let confirmed: Vec<SymbolCandidate> = prefilter_symbols(storage, &ctx.project_id, query)
        .into_iter()
        .filter(|c| c.path.is_some() && symbol_name_hits(&c.name, &words).is_some())
        .collect();

    let mut symbol_score: HashMap<String, i32> = HashMap::new();
    let mut containing_file_ids: Vec<String> = Vec::new();
    if !confirmed.is_empty() {
        let mut distinct_paths: Vec<&str> =
            confirmed.iter().filter_map(|c| c.path.as_deref()).collect();
        distinct_paths.sort_unstable();
        distinct_paths.dedup();

        let repos = storage.list_repos(&ctx.project_id).unwrap_or_default();

        // Batched lookup via `entity_ids_for_paths`.
        for repo in &repos {
            let id_by_path = storage
                .entity_ids_for_paths(&repo.id, &distinct_paths)
                .unwrap_or_default();
            if !id_by_path.is_empty() {
                for c in &confirmed {
                    let Some(path) = c.path.as_deref() else {
                        continue;
                    };
                    let Some(file_id) = id_by_path.get(path) else {
                        continue;
                    };
                    let exact = symbol_name_hits(&c.name, &words).is_some_and(|(e, _)| e);
                    match symbol_score.entry(file_id.clone()) {
                        std::collections::hash_map::Entry::Vacant(_) => {
                            symbol_score.insert(
                                file_id.clone(),
                                if exact {
                                    EXACT_SYMBOL_SCORE
                                } else {
                                    PREFIX_SYMBOL_SCORE
                                },
                            );
                            if !containing_file_ids.iter().any(|x| x == file_id) {
                                containing_file_ids.push(file_id.clone());
                            }
                        }
                        std::collections::hash_map::Entry::Occupied(mut occ) => {
                            if exact && *occ.get() < EXACT_SYMBOL_SCORE {
                                *occ.get_mut() = EXACT_SYMBOL_SCORE;
                            }
                        }
                    }
                }
                break;
            }
        }
    }

    // --- Merge, de-duplicate, keep admission order ---
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut matched: Vec<Entity> = direct.filter(|e| seen_ids.insert(e.id.clone())).collect();

    for file_id in &containing_file_ids {
        if let Some(entity) = storage.get_entity(file_id).ok().flatten()
            && entity.tier == EntityTier::File
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
