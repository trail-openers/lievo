// Search tool implementation.

use serde_json::{Value, json};

const MAX_SEARCH_WORDS: usize = 10;

use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

use super::super::SearchEntitiesTool;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Truncate a string to `max` chars, appending "..." if truncated.
pub(super) fn truncate(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    // Take first max-3 chars and append "..."
    let take_count = max.saturating_sub(3);
    if take_count == 0 {
        return "...".to_string();
    }
    chars[0..take_count].iter().collect::<String>() + "..."
}

// `lock_storage!` and `should_exclude_entity` are the ONE shared definition
// (issue #712 finding #7 — this file previously carried its own private
// copy of the macro, and a separate copy of the filter, alongside
// `tools_explore.rs`/`tools_explore_scope.rs`). Re-exported under this
// module's existing names so `tools_relationship.rs`, `tools_entity.rs`,
// `tools_directory.rs`, `tools_doc.rs`, `tools_function.rs`, and
// `tools_query.rs` (which import both from `super::tools_search`) need no
// changes.
pub(super) use crate::retrieval::explore_common::lock_storage;
pub(crate) use crate::retrieval::explore_common::should_exclude_entity;

// ---------------------------------------------------------------------------
// SearchEntitiesTool Helper Functions
// ---------------------------------------------------------------------------

/// Determine the appropriate index warning message based on repository coverage.
fn index_coverage_warning(
    repos: &[crate::model::Repository],
    partial_coverage: bool,
    has_any_index: bool,
) -> Option<String> {
    // partial_coverage requires !repos.is_empty(), so these branches are mutually exclusive
    debug_assert!(!(partial_coverage && repos.is_empty()));
    if partial_coverage {
        Some("Semantic search enabled for indexed repositories only. Consider running 'lievo refresh' on all repos for full coverage.".to_string())
    } else if repos.is_empty() {
        Some("No repositories added. Run 'lievo admin add-repo' to add a repository, then 'lievo refresh' to enable semantic code search.".to_string())
    } else if !has_any_index {
        Some(
            "Vector index not found. Run 'lievo refresh' to enable semantic code search."
                .to_string(),
        )
    } else {
        None
    }
}

/// Attempt to load a semantic searcher. Returns (Option<SemanticSearcher>, Option<warning_message>).
/// Falls back gracefully on any failure — caller receives a warning but not an error.
pub(super) fn try_load_semantic_searcher(
    index_path: &str,
) -> (
    Option<Box<dyn crate::retrieval::semantic_searcher::SemanticSearcher>>,
    Option<String>,
) {
    // Load UsearchSearcher (tree-sitter based semantic search)
    let path = std::path::Path::new(index_path);
    match crate::retrieval::usearch_searcher::UsearchSearcher::load(path) {
        Ok(searcher) => {
            tracing::info!("Loaded UsearchSearcher from {}", index_path);
            (Some(Box::new(searcher)), None)
        }
        Err(e) => {
            tracing::warn!(
                "Failed to load usearch index: {}. Falling back to name/path search.",
                e
            );
            (
                None,
                Some("Semantic search unavailable: vector index could not be loaded. Results shown are from exact name/path matching only — use a literal entity name (e.g. 'MemoryStore') rather than a descriptive phrase (e.g. 'memory storage') for best results. Run 'lievo refresh' to build the vector index for full semantic search (it is rebuilt automatically when the index is missing or was built with a different embedding model)".to_string()),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// SearchEntitiesTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for SearchEntitiesTool<S> {
    fn name(&self) -> &str {
        "search_entities"
    }

    fn description(&self) -> &str {
        "Search for entities by name or keyword. Set semantic=true to use tree-sitter-based semantic code search (intelligently finds related code even without exact name matches). Falls back to name/path matching if no vector index exists — run 'lievo refresh' to build the index."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword or name fragment to search, e.g. 'auth', 'UserService', 'src/api'. Returns matching entities with their IDs."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results (default 10)"
                },
                "semantic": {
                    "type": "boolean",
                    "description": "Whether to use semantic search (tree-sitter-based). When true, uses intelligent code search instead of simple name/path matching. Default false."
                },
                "tier": {
                    "type": "string",
                    "description": "Optional entity tier filter. Valid values: 'function', 'file', 'module', 'subsystem'."
                }
            },
            "required": ["query"]
        })
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let query = input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::LievoError::InvalidInput("missing 'query'".into()))?;

        // Validate query is not empty or whitespace-only
        let q = query.trim();
        if q.is_empty() {
            return Ok(json!({"error": "query cannot be empty"}).to_string());
        }

        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v.clamp(1, 30) as usize)
            .unwrap_or(10);

        let semantic = input
            .get("semantic")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Parse optional tier filter and validate it
        let tier = input.get("tier").and_then(|v| v.as_str());
        if let Some(t) = tier {
            match t {
                "function" | "file" | "module" | "subsystem" => {}
                _ => {
                    return Ok(json!({
                        "error": format!(
                            "invalid tier '{}': valid values are 'function', 'file', 'module', 'subsystem'",
                            t
                        )
                    })
                    .to_string());
                }
            }
        }

        if semantic {
            // Semantic mode: direct vector-index search

            // Step 1: Find first repo with a valid vector_index_path (with lock)
            // ensure_potion_code_model() may block on first call (~150MB download)
            let (vector_index_path, index_warning) = {
                let guard = lock_storage!(self.ctx.storage);
                let repos = guard.list_repos(&self.ctx.project_id)?;

                // Single-pass iteration: collect index info and coverage status
                let mut index_path: Option<String> = None;
                // Initialized to true; short-circuit in partial_coverage makes this safe for empty repos
                let mut has_all_indexed = true;
                let mut has_any_index = false;

                for repo in &repos {
                    let has_index = repo.index_path.is_some();
                    if has_index {
                        has_any_index = true;
                        if index_path.is_none() {
                            index_path = repo.index_path.clone();
                        }
                    } else {
                        has_all_indexed = false;
                    }
                }

                let partial_coverage = !repos.is_empty() && !has_all_indexed && has_any_index;
                let warning = index_coverage_warning(&repos, partial_coverage, has_any_index);

                (index_path, warning)
            };

            // Step 2: Drop the lock before expensive operations (model load, index load)
            let (searcher, load_warning) = match vector_index_path.as_ref() {
                Some(index_path) => try_load_semantic_searcher(index_path),
                None => (None, None),
            };

            // Step 3: Merge warnings: load_warning OR index_warning (prefer load_warning as it's more specific)
            let effective_warning = load_warning.or(index_warning);

            // Step 4: Execute search. Without a usable searcher (no index,
            // or the index failed to load) the tool returns a success
            // response with the warning and zero rows — not an error.
            let results = match searcher {
                Some(searcher) => {
                    // The storage lock is NOT held across the model encode /
                    // ANN search (step 2 dropped it); acquire it only for the
                    // entity listing used to resolve hit paths to entities.
                    let guard = lock_storage!(self.ctx.storage);
                    let raw = searcher.search(query, limit.saturating_mul(2))?;
                    let entities = guard.list_entities(&self.ctx.project_id, None)?;
                    drop(guard);
                    crate::retrieval::resolve_semantic_hits(raw, &entities, limit)
                }
                None => Vec::new(),
            };

            // Step 5: Convert results and add warning if applicable
            let matches: Vec<Value> = results
                .into_iter()
                .filter(|r| {
                    // Exclude entities matching the output directory
                    !should_exclude_entity(r.path.as_deref(), &self.ctx.output_dir)
                })
                .map(|r| {
                    json!({
                        "entity_id": r.entity_id,
                        "name": r.name,
                        "path": r.path,
                        "tier": r.tier,
                        "score": r.score,
                        "source": format!("{:?}", r.source),
                        "snippet": r.snippet
                    })
                })
                .collect();

            // Build response: results array + optional warning
            let mut response = json!({
                "results": matches
            });

            // Add warning to top level if present
            if let Some(warning) = effective_warning {
                response["warning"] = json!(warning);
            }

            Ok(response.to_string())
        } else {
            // Non-semantic mode: exact name/path string matching (no summary clause)

            // Validate minimum word length to prevent runaway single-char matches
            let query_lower = query.to_lowercase();
            let words: Vec<&str> = query_lower.split_whitespace().collect();
            let filtered_words: Vec<&str> =
                words.iter().copied().filter(|w| w.len() >= 2).collect();
            // Edge case: when query is "a b" (both words < 2 chars), filtered_words is empty
            if filtered_words.is_empty() {
                return Ok(
                    json!({"error": "query words must be at least 2 characters"}).to_string(),
                );
            }
            // Truncate to MAX_SEARCH_WORDS to prevent DoS from excessive word counts
            let filtered_words: Vec<&str> = filtered_words
                .iter()
                .copied()
                .take(MAX_SEARCH_WORDS)
                .collect();

            let guard = lock_storage!(self.ctx.storage);
            let entities = guard.search_entities_by_name(
                &self.ctx.project_id,
                &filtered_words,
                limit,
                tier,
            )?;
            let matches: Vec<Value> = entities
                .into_iter()
                .filter(|e| !should_exclude_entity(e.path.as_deref(), &self.ctx.output_dir))
                .map(|e| {
                    json!({
                        "entity_id": e.id,
                        "name": e.name,
                        "path": e.path,
                        "tier": e.tier.to_string(),
                        "score": 1.0,
                        "source": "ExactMatch",
                        "snippet": e.summary.as_deref().filter(|s| !s.is_empty()).or(e.path.as_deref()).unwrap_or(&e.name)
                    })
                })
                .collect();

            // Build response with optional warning for empty results
            let mut response = json!({
                "results": matches
            });

            // Add warning if no results found with appropriate guidance
            if matches.is_empty() {
                // Only check if entities exist when results are empty (lazy evaluation)
                // This avoids an extra DB query on the happy path (results found)
                let has_entities = !guard.list_entities(&self.ctx.project_id, None)?.is_empty();
                if has_entities {
                    // Entities exist but query didn't match — suggest semantic search
                    response["warning"] = json!(
                        "No matching entities found. Try semantic mode (semantic=true) for more flexible search, or use a different search term."
                    );
                } else {
                    // No entities indexed at all — guide them to refresh
                    response["warning"] = json!(
                        "No entities indexed. Run 'lievo refresh <project>' to index your project first."
                    );
                }
            }

            Ok(response.to_string())
        }
    }
}

#[cfg(test)]
#[path = "tools_search_helpers_tests.rs"]
mod helpers_tests;

#[cfg(test)]
#[path = "tools_search_validation_tests/mod.rs"]
mod validation_tests;

#[cfg(test)]
#[path = "tools_search_multiword_tests.rs"]
mod multiword_tests;

#[cfg(test)]
#[path = "tools_search_tier_filter_tests.rs"]
mod tier_filter_tests;

#[cfg(test)]
#[path = "tools_search_warning_tests.rs"]
mod warning_tests;

#[cfg(test)]
#[path = "tools_search_truncation_tests.rs"]
mod truncation_tests;
