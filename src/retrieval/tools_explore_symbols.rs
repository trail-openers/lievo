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
//!     batched lookup per repo), with a fallback to `get_entity` using the
//!     conventional file-id format for test mocks that don't implement the
//!     batched lookup.
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

        // Primary path: batched lookup via `entity_ids_for_paths`.
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

        // Fallback: resolve via conventional file id
        // `{project_id}:{repo_name}:file:{path}`. This path is taken when
        // `entity_ids_for_paths` returned empty for every repo (e.g. test
        // mocks that don't implement the batched lookup).
        if containing_file_ids.is_empty() && !repos.is_empty() {
            for c in &confirmed {
                let Some(path) = c.path.as_deref() else {
                    continue;
                };
                let repo_name = repos[0].name.as_str();
                let file_id = format!("{}:{repo_name}:file:{}", ctx.project_id, path);
                if let Some(entity) = storage.get_entity(&file_id).ok().flatten()
                    && entity.tier == EntityTier::File
                    && !should_exclude_entity(entity.path.as_deref(), &ctx.output_dir)
                {
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
                            containing_file_ids.push(file_id.clone());
                        }
                        std::collections::hash_map::Entry::Occupied(mut occ) => {
                            if exact && *occ.get() < EXACT_SYMBOL_SCORE {
                                *occ.get_mut() = EXACT_SYMBOL_SCORE;
                            }
                        }
                    }
                }
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
mod tests {
    use super::*;

    #[test]
    fn symbol_prefilter_params_empty_words_is_none() {
        assert!(symbol_prefilter_params("p", &[]).is_none());
    }

    #[test]
    fn symbol_prefilter_params_project_id_first_then_like_terms() {
        let words = vec!["auth".to_string(), "sum_of_squares".to_string()];
        let params = symbol_prefilter_params("p1", &words).unwrap();
        assert_eq!(params, vec!["p1", "%auth%", "%sum_of_squares%"]);
    }

    #[test]
    fn symbol_name_hits_exact_and_prefix_and_miss() {
        let words = vec!["sum_of_squares".to_string(), "auth".to_string()];
        // Exact: name equals a query word.
        assert_eq!(
            symbol_name_hits("Sum_of_Squares", &words),
            Some((true, true))
        );
        // Prefix: query word "auth" is a token prefix of "authentication".
        assert_eq!(
            symbol_name_hits("authentication", &words),
            Some((false, true))
        );
        // Miss: no query word is a token prefix of any name token.
        assert_eq!(symbol_name_hits("validity", &words), None);
    }

    #[test]
    fn exact_symbol_score_outranks_file_name_and_path() {
        use crate::retrieval::explore_ranking::score_file_entity;
        let words = vec!["sum_of_squares".to_string()];
        let file = Entity {
            id: "p:repo:file:math.rs".to_string(),
            project_id: "p".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "math".to_string(),
            path: Some("src/math.rs".to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let (file_score, _) = score_file_entity(&file, &words);
        assert_eq!(file_score, 0, "file name/path must not mention the symbol");
        assert!(
            EXACT_SYMBOL_SCORE > file_score && EXACT_SYMBOL_SCORE > 2 && PREFIX_SYMBOL_SCORE > 1
        );
    }

    /// A common word like `new` must not return hundreds of files — the
    /// existing max_files / limit caps and continuation (returned/total/next)
    /// must apply to the merged set.
    #[test]
    fn common_word_cap_applies_to_merged_set() {
        use crate::retrieval::tool_trait::Tool;
        use crate::retrieval::tools::{ExploreTool, ToolContext};
        use crate::storage::sqlite::SqliteStorage;
        use serde_json::json;
        use std::sync::{Arc, Mutex};

        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("cap-test", None).unwrap();
        let repo = storage
            .add_repo(&project.id, "repo1", "/tmp/repo1")
            .unwrap();

        // 20 files, each with a function named `new` (a common word).
        for i in 0..20 {
            let path = format!("src/file_{i}.rs");
            let file_id = format!("{}:repo1:file:{}", project.id, path);
            storage
                .upsert_entity(&Entity {
                    id: file_id.clone(),
                    project_id: project.id.clone(),
                    repo_id: Some(repo.id.clone()),
                    tier: EntityTier::File,
                    parent_id: None,
                    name: format!("file_{i}"),
                    path: Some(path.clone()),
                    language: Some("Rust".to_string()),
                    summary: None,
                    summary_commit: None,
                    metrics_json: None,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                })
                .unwrap();
            storage
                .upsert_entity(&Entity {
                    id: format!("{}:repo1:fn:{}:new", project.id, path),
                    project_id: project.id.clone(),
                    repo_id: Some(repo.id.clone()),
                    tier: EntityTier::Function,
                    parent_id: Some(file_id),
                    name: "new".to_string(),
                    path: Some(path),
                    language: Some("Rust".to_string()),
                    summary: None,
                    summary_commit: None,
                    metrics_json: None,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                })
                .unwrap();
        }

        let ctx = Arc::new(ToolContext {
            storage: Arc::new(Mutex::new(storage)),
            project_id: project.id.clone(),
            repo_path: std::path::PathBuf::new(),
            output_dir: None,
            zero_repo_guidance: None,
        });
        let tool = ExploreTool { ctx };
        // Default max_files=8; 20 matching files must be capped.
        let result = tool.call(json!({ "query": "new" })).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let symbols = v["symbols"].as_array().expect("symbols");
        // Must be capped at max_files (default 8), not all 20.
        assert!(
            symbols.len() <= 8,
            "common word 'new' returned {len} files, expected <= 8 (max_files cap); got {symbols:?}",
            len = symbols.len()
        );
        // Continuation must be present when total > returned.
        let has_continuation = v.get("continuation").is_some();
        let total = v
            .get("completeness")
            .and_then(|c| c.as_str())
            .map(|s| !s.is_empty());
        assert!(
            has_continuation || total.is_some(),
            "expected continuation pointer when matches exceed max_files; got: {v}"
        );
    }

    /// Requirement 2: a file whose symbol name exactly matches a query word
    /// must rank above a file matched only by a path token.
    #[test]
    fn exact_symbol_ranks_above_path_token_match() {
        use std::sync::{Arc, Mutex};
        use crate::retrieval::tool_trait::Tool;
        use crate::retrieval::tools::{ExploreTool, ToolContext};
        use crate::storage::sqlite::SqliteStorage;
        use serde_json::json;

        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("rank-test", None).unwrap();
        let repo = storage.add_repo(&project.id, "repo1", "/tmp/repo1").unwrap();

        // File A: path contains the query word "alpha" but no symbol does.
        let file_a_id = format!("{}:repo1:file:src/alpha/beta.rs", project.id);
        storage.upsert_entity(&Entity {
            id: file_a_id.clone(),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "beta".to_string(),
            path: Some("src/alpha/beta.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }).unwrap();

        // File B: path does NOT contain "alpha", but has a function named "alpha".
        let file_b_id = format!("{}:repo1:file:src/gamma.rs", project.id);
        storage.upsert_entity(&Entity {
            id: file_b_id.clone(),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "gamma".to_string(),
            path: Some("src/gamma.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }).unwrap();
        storage.upsert_entity(&Entity {
            id: format!("{}:repo1:fn:src/gamma.rs:alpha", project.id),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::Function,
            parent_id: Some(file_b_id.clone()),
            name: "alpha".to_string(),
            path: Some("src/gamma.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }).unwrap();

        let ctx = Arc::new(ToolContext {
            storage: Arc::new(Mutex::new(storage)),
            project_id: project.id.clone(),
            repo_path: std::path::PathBuf::new(),
            output_dir: None,
            zero_repo_guidance: None,
        });
        let tool = ExploreTool { ctx };
        let result = tool.call(json!({ "query": "alpha" })).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let symbols = v["symbols"].as_array().expect("symbols");
        assert!(symbols.len() >= 2, "expected both files, got: {symbols:?}");
        // File B (symbol exact match, score 4) must rank above File A (path token, score 1).
        let b_idx = symbols.iter().position(|s| s["qualified_path"] == "src/gamma.rs").unwrap();
        let a_idx = symbols.iter().position(|s| s["qualified_path"] == "src/alpha/beta.rs").unwrap();
        assert!(
            b_idx < a_idx,
            "exact symbol match (src/gamma.rs, idx {b_idx}) must rank above path-token match (src/alpha/beta.rs, idx {a_idx}); got: {symbols:?}"
        );
    }

    /// Integration: a query naming a FUNCTION whose name does not appear in
    /// any file name/path must return the containing file (via the symbol
    /// channel), and that file must be present and scored with the symbol
    /// channel active.
    #[test]
    fn query_naming_function_returns_containing_file() {
        use crate::retrieval::tool_trait::Tool;
        use crate::retrieval::tools::{ExploreTool, ToolContext};
        use crate::storage::sqlite::SqliteStorage;
        use serde_json::json;
        use std::sync::{Arc, Mutex};

        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("sym-test", None).unwrap();
        let repo = storage
            .add_repo(&project.id, "repo1", "/tmp/repo1")
            .unwrap();

        // A file whose name and path do NOT contain the symbol name.
        let file_id = format!("{}:repo1:file:src/util.rs", project.id);
        storage
            .upsert_entity(&Entity {
                id: file_id.clone(),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::File,
                parent_id: None,
                name: "util".to_string(),
                path: Some("src/util.rs".to_string()),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .unwrap();
        // A Function-tier entity whose name IS the query, contained in that file.
        storage
            .upsert_entity(&Entity {
                id: format!("{}:repo1:fn:src/util.rs:sum_of_squares", project.id),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::Function,
                parent_id: Some(file_id.clone()),
                name: "sum_of_squares".to_string(),
                path: Some("src/util.rs".to_string()),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .unwrap();

        let ctx = Arc::new(ToolContext {
            storage: Arc::new(Mutex::new(storage)),
            project_id: project.id.clone(),
            repo_path: std::path::PathBuf::new(),
            output_dir: None,
            zero_repo_guidance: None,
        });
        let tool = ExploreTool { ctx };
        let result = tool.call(json!({ "query": "sum_of_squares" })).unwrap();
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let symbols = v["symbols"].as_array().expect("symbols");
        assert!(
            !symbols.is_empty(),
            "expected the containing file, got: {v}"
        );
        // The containing file (src/util.rs) must be returned.
        let has_util = symbols.iter().any(|s| s["qualified_path"] == "src/util.rs");
        assert!(has_util, "containing file src/util.rs missing: {symbols:?}");
        // Score must reflect the exact symbol-name channel (4), not 0.
        let util = symbols
            .iter()
            .find(|s| s["qualified_path"] == "src/util.rs")
            .unwrap();
        assert_eq!(util["score"], EXACT_SYMBOL_SCORE);
    }
}
