use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct SearchEntitiesParams {
    pub(super) query: String,
    pub(super) limit: Option<u64>,
    #[serde(default)]
    pub(super) semantic: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct GetEntityParams {
    pub(super) entity_id: String,
    #[serde(default)]
    pub(super) include_children: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct ListRelationshipsParams {
    pub(super) entity_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct GetConventionsParams {
    pub(super) category: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct GetInsightsParams {
    pub(super) category: Option<String>,
    pub(super) severity: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct ReadFileParams {
    pub(super) entity_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct ListDirectoryParams {
    pub(super) path: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct ReadProjectDocParams {
    pub(super) path: String,
    pub(super) format: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct GetFunctionParams {
    pub(super) entity_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct GetImpactParams {
    pub(super) files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct GetExecutionFlowsParams {
    pub(super) entry_point: Option<String>,
    pub(super) max_depth: Option<u64>,
    pub(super) limit: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct GetHotspotsParams {
    #[serde(default = "default_limit")]
    pub(super) limit: u64,
    #[serde(default = "default_tier")]
    pub(super) tier: String,
}

fn default_limit() -> u64 {
    10
}

fn default_tier() -> String {
    "file".to_string()
}

// Parameters for the `lievo_explore` tool (issue #680/#682/#741).
//
// Used by the `lievo_explore` registration in `tools.rs`, which composes
// `retrieval::tools::ExploreTool`. Note: `///` doc comments on this struct or
// its fields are serialized into the wire input schema (issue #850) — the
// model reads them every turn, so keep this a plain `//` comment.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(super) struct ExploreParams {
    /// Keyword or name fragment to word-match against indexed file entities
    /// by name or path (e.g. 'auth'). Not used when `files` is passed.
    #[serde(default)]
    pub(super) query: String,
    /// Repo-relative file paths to fetch in ONE batched call (pack all needed
    /// paths in this call rather than one call per file). When present,
    /// short-circuits `query` word-match and `scope` listing.
    pub(super) files: Option<Vec<String>>,
    /// Maximum number of file entities to include in the response.
    /// Default 8, max 30.
    #[serde(default = "default_max_files")]
    pub(super) max_files: u32,
    /// When true, returns verbatim line-numbered source for matched symbols
    /// (tier 2). Default false (tier 1: map only).
    #[serde(default)]
    pub(super) include_source: bool,
    /// Optional repo-relative directory prefix (e.g. "src/retrieval"). When
    /// set, switches from word-match search to scope-membership listing: the
    /// sorted, repo-relative indexed files under that prefix. This is the sole
    /// trigger for scope mode; a path-like `query` alone never triggers it.
    // No `#[serde(default)]` here (unlike `max_files`/`include_source` above)
    // is deliberate: `Option<T>` fields already deserialize a missing key to
    // `None` without one — a `///` doc comment would leak that implementation
    // note into the wire schema (issue #850).
    pub(super) scope: Option<String>,
    /// Zero-based offset into the sorted scope listing, for paging through
    /// scopes whose file count exceeds `max_files`. Ignored when `scope` is
    /// absent.
    pub(super) offset: Option<u32>,
    /// When true, includes `call_paths` (outgoing and incoming call edges) and
    /// `blast_radius` (incoming dependents only) on each symbol — the opt-in
    /// for blast-radius/impact questions. Default false (lean response).
    /// Ignored in scope mode. blast_radius entries carry a `hop` level (0 =
    /// direct dependent, 1 = second level) and are capped per file, nearest
    /// first. blast_radius_complete is true when the closure is the complete
    /// two-level reverse dependency set and its files need not be
    /// re-queried for depth.
    #[serde(default)]
    pub(super) include_depth: bool,
    /// Repo-relative directory prefix to select subsystem-bundle mode: one
    /// call under the 24K output cap returns the verbatim line-numbered source
    /// for the packed files, the intra-scope Calls/Imports edges between them,
    /// a `not_shown_files` list, and a structured `completeness` field
    /// {complete, omitted_files, omitted_edges}. Presence (any value) is the
    /// sole trigger for bundle mode — it short-circuits `query`, `files` and
    /// `scope`.
    pub(super) bundle: Option<String>,
}

fn default_max_files() -> u32 {
    8
}

/// Hand-written `Default` for `ExploreParams` (issue #756).
///
/// `#[derive(Default)]` is deliberately NOT used here: `max_files` is a
/// non-`Option` `u32` that carries `#[serde(default = "default_max_files")]`
/// (which yields 8). A derived `Default` would give `u32::default()` = 0,
/// silently flipping the tier-1/tier-2 path for every test initializer that
/// previously passed `max_files: 8` explicitly. The hand-written impl mirrors
/// the serde defaults field-for-field:
///
/// - field with `#[serde(default = "f")]`  → `Default` uses `f()`
/// - field with bare `#[serde(default)]`    → `Default` uses the type default
/// - `Option<T>` field                      → `Default` is `None`
///
/// A corresponding `#[cfg(test)]` test in `explore_wire_default_tests.rs`
/// deserializes `{}` and asserts the result equals `ExploreParams::default()`,
/// so the two cannot drift apart.
impl Default for ExploreParams {
    fn default() -> Self {
        Self {
            // `#[serde(default)]` on a `String` — the type default.
            query: String::default(),
            // `Option<Vec<String>>` — no serde attribute; missing key → `None`.
            files: None,
            // `#[serde(default = "f")]` → `default_max_files()`.
            max_files: default_max_files(),
            // `#[serde(default)]` on a `bool` — the type default.
            include_source: bool::default(),
            // `Option<String>` — no serde attribute; missing key → `None`.
            scope: None,
            // `Option<u32>` — no serde attribute; missing key → `None`.
            offset: None,
            // `#[serde(default)]` on a `bool` — the type default.
            include_depth: bool::default(),
            // `Option<String>` — no serde attribute; missing key → `None`.
            bundle: None,
        }
    }
}

#[cfg(test)]
mod params_tests {
    use super::ExploreParams;

    /// Issue #756: deserialize an empty JSON object into `ExploreParams` via
    /// serde and assert the result equals `ExploreParams::default()`. This
    /// makes serde's defaults and Rust's `Default` structurally unable to
    /// drift apart — if a future serde attribute changes a field's default,
    /// or a `Default` impl is changed, this test catches the divergence.
    #[test]
    fn explore_params_default_matches_serde_defaults_for_empty_object() {
        let deserialized: ExploreParams = serde_json::from_str("{}").unwrap();
        let defaulted = ExploreParams::default();

        // Field-by-field comparison: `ExploreParams` does not derive
        // `PartialEq` (that would alter the schemars-derived schema's
        // trait bounds and risk changing the advertised input schema). The
        // field-by-field asserts are structurally equivalent to an
        // `assert_eq!` and leave the production struct untouched.
        assert_eq!(deserialized.query, defaulted.query);
        assert_eq!(deserialized.files, defaulted.files);
        assert_eq!(deserialized.max_files, defaulted.max_files);
        assert_eq!(deserialized.include_source, defaulted.include_source);
        assert_eq!(deserialized.scope, defaulted.scope);
        assert_eq!(deserialized.offset, defaulted.offset);
        assert_eq!(deserialized.include_depth, defaulted.include_depth);
        assert_eq!(deserialized.bundle, defaulted.bundle);
    }
}
