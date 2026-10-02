// SemanticSearcher trait — abstraction over semantic code search implementations.
// Currently implemented by UsearchSearcher (embedding-based (model2vec + usearch));
// enables future replacements without changing retrieval logic.

use super::SearchResult;

/// Trait for semantic code search.
///
/// This trait provides a clean abstraction over code search implementations,
/// allowing implementations to be swapped (e.g. model2vec + usearch) without
/// changing retrieval logic.
pub trait SemanticSearcher {
    /// Search for code units matching the query.
    ///
    /// # Arguments
    /// * `query` - A natural language or code snippet query
    /// * `limit` - Maximum number of results to return
    ///
    /// # Returns
    /// A Vec of SearchResult structs ranked by relevance
    fn search(&self, query: &str, limit: usize) -> crate::Result<Vec<SearchResult>>;
}
