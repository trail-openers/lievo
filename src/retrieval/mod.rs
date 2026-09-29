pub mod doc_discovery;
mod doc_discovery_helpers;
pub(super) mod doc_parsers;
pub mod explore_cap;
pub(crate) mod explore_common;
pub(crate) mod explore_ranking;

pub mod metrics;
pub mod model_cache;
pub mod project_boundary;
pub(crate) mod query_tokenizer;
pub mod retrieval_eval;
pub mod semantic_searcher;
pub mod tool_trait;
pub mod tools;
pub mod tools_explore;
pub(crate) mod tools_explore_blast;
pub(crate) mod tools_explore_bundle;
pub(crate) mod tools_explore_files;
pub(crate) mod tools_explore_files_asm;
pub(crate) mod tools_explore_format;
pub(crate) mod tools_explore_in_progress;
#[cfg(test)]
#[path = "tools_explore_match_tests.rs"]
mod tools_explore_match_tests;
pub(crate) mod tools_explore_scope;
pub(crate) mod tools_explore_symbols;
pub mod usearch_searcher;

#[cfg(test)]
pub mod test_helpers;

use serde::{Deserialize, Serialize};

/// The retrieval channels that can produce a search result: an exact
/// name/path match, or a semantic code hit from the vector index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RetrievalSource {
    ExactMatch,
    SemanticCode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub entity_id: String,
    pub name: String,
    pub path: Option<String>,
    pub snippet: String,
    pub score: f32,
    pub source: RetrievalSource,
    pub tier: String,
}

/// Run a semantic (vector-index) search and map the raw hits onto stored
/// entities.
///
/// Both semantic call sites (`lievo query entities --semantic` and the
/// `search_entities` MCP tool's `semantic=true` branch) must use identical
/// post-processing, so the logic lives here rather than being duplicated:
///
/// - over-fetch `limit * 2` raw hits from the searcher, because hits that
///   resolve to no stored entity are dropped below — requesting exactly
///   `limit` would let drops under-fill the final list;
/// - resolve each hit's path to a stored entity, dropping hits whose path
///   maps to no entity BEFORE truncating to `limit`, so synthetic or
///   unmappable hits never displace real entities (an empty stored path
///   never matches: `path.ends_with("")` would be true for every hit);
/// - return the survivors in the searcher's score order (score descending,
///   entity_id ascending tie-break — the searcher's absolute score values
///   are not reliable, but its relative ordering is).
pub fn resolve_semantic_hits(
    raw: Vec<SearchResult>,
    entities: &[crate::model::Entity],
    limit: usize,
) -> Vec<SearchResult> {
    let mut resolved: Vec<_> = raw
        .into_iter()
        .filter_map(|mut r| {
            let path = r.path.as_ref()?;
            let entity = entities.iter().find(|e| {
                e.path
                    .as_deref()
                    .map(|p| !p.is_empty() && (p == path || path.ends_with(p) || p.ends_with(path)))
                    .unwrap_or(false)
            })?;
            r.entity_id = entity.id.clone();
            r.tier = entity.tier.to_string();
            Some(r)
        })
        .collect();

    resolved.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.entity_id.cmp(&b.entity_id))
    });
    resolved.truncate(limit);
    resolved
}
