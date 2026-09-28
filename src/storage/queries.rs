// SQL query constants for SqliteStorage implementation
// All parameterized queries stored as constants for reuse and clarity

// Project queries
pub const CREATE_PROJECT: &str = r#"
INSERT INTO projects (id, name, description, created_at, updated_at)
VALUES (?1, ?2, ?3, ?4, ?5)
"#;

pub const GET_PROJECT: &str = r#"
SELECT id, name, description, output_dirs, created_at, updated_at
FROM projects
WHERE name = ?1
"#;

pub const GET_PROJECT_BY_ID: &str = r#"
SELECT id, name, description, output_dirs, created_at, updated_at
FROM projects
WHERE id = ?1
"#;

pub const LIST_PROJECTS: &str = r#"
SELECT id, name, description, output_dirs, created_at, updated_at
FROM projects
ORDER BY created_at DESC
"#;

// Repository queries
pub const CREATE_REPO: &str = r#"
INSERT INTO repositories (id, project_id, name, local_path, git_url, default_branch, created_at, updated_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
"#;

pub const GET_REPO: &str = r#"
SELECT id, project_id, name, git_url, local_path, default_branch, last_analyzed_commit, index_path, created_at, updated_at, summarization_unconfigured
FROM repositories
WHERE id = ?1
"#;

pub const LIST_REPOS: &str = r#"
SELECT id, project_id, name, git_url, local_path, default_branch, last_analyzed_commit, index_path, created_at, updated_at, summarization_unconfigured
FROM repositories
WHERE project_id = ?1
ORDER BY created_at DESC
"#;

pub const UPDATE_REPOSITORY_UNCONFIGURED_MARKER: &str = r#"
UPDATE repositories
SET summarization_unconfigured = ?1, updated_at = ?2
WHERE id = ?3
"#;

pub const UPDATE_REPO_INDEX_PATH: &str = r#"
UPDATE repositories
SET index_path = ?1, updated_at = ?2
WHERE id = ?3
"#;

pub const UPDATE_REPO_LAST_COMMIT: &str = r#"
UPDATE repositories
SET last_analyzed_commit = ?1, updated_at = ?2
WHERE id = ?3
"#;

pub const RECORD_REPO_UNRESOLVED_COUNTS: &str = r#"
UPDATE repositories
SET unresolved_internal = ?1, unresolved_external = ?2, updated_at = ?3
WHERE id = ?4
"#;

pub const UPDATE_REPO_PROJECT: &str = r#"
UPDATE repositories
SET project_id = ?1, updated_at = ?2
WHERE id = ?3
"#;

pub const UPDATE_ENTITIES_REPO_PROJECT: &str = r#"
UPDATE entities
SET project_id = ?1, updated_at = ?2
WHERE repo_id = ?3
"#;

// Entity queries
pub const UPSERT_ENTITY: &str = r#"
INSERT INTO entities (id, project_id, repo_id, tier, parent_id, name, path, language, summary, summary_commit, metrics_json, created_at, updated_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
ON CONFLICT(id) DO UPDATE SET
    project_id = excluded.project_id,
    repo_id = excluded.repo_id,
    tier = excluded.tier,
    parent_id = excluded.parent_id,
    name = excluded.name,
    path = excluded.path,
    language = excluded.language,
    summary = COALESCE(excluded.summary, summary),
    summary_commit = COALESCE(excluded.summary_commit, summary_commit),
    metrics_json = excluded.metrics_json,
    updated_at = excluded.updated_at
"#;

// Clear an entity's stored summary in place. A dedicated statement (mirroring
// CLEAR_REPO_SUMMARIES) is required because UPSERT_ENTITY now COALESCEs the summary
// columns: a NULL incoming summary means "no summary to offer" (structural re-persist)
// and must NOT overwrite a stored summary. The only path that wants to actively NULL a
// single entity's summary (lievo summarize --file) therefore uses this UPDATE.
pub const CLEAR_ENTITY_SUMMARY: &str = r#"
UPDATE entities
SET summary = NULL, summary_commit = NULL
WHERE id = ?1
"#;

pub const GET_ENTITY: &str = r#"
SELECT id, project_id, repo_id, tier, parent_id, name, path, language, summary, summary_commit, metrics_json, created_at, updated_at
FROM entities
WHERE id = ?1
"#;

pub const LIST_ENTITIES: &str = r#"
SELECT id, project_id, repo_id, tier, parent_id, name, path, language, summary, summary_commit, metrics_json, created_at, updated_at
FROM entities
WHERE project_id = ?1 AND (?2 IS NULL OR tier = ?2)
ORDER BY name ASC
"#;

pub const ENTITIES_BY_REPO: &str = r#"
SELECT id, project_id, repo_id, tier, parent_id, name, path, language, summary, summary_commit, metrics_json, created_at, updated_at
FROM entities
WHERE repo_id = ?1 AND (?2 IS NULL OR tier = ?2)
ORDER BY path ASC
"#;

pub const ENTITIES_BY_PARENT: &str = r#"
SELECT id, project_id, repo_id, tier, parent_id, name, path, language, summary, summary_commit, metrics_json, created_at, updated_at
FROM entities
WHERE parent_id = ?1
ORDER BY name ASC
"#;

pub const ENTITY_BY_PATH: &str = r#"
SELECT id, project_id, repo_id, tier, parent_id, name, path, language, summary, summary_commit, metrics_json, created_at, updated_at
FROM entities
WHERE repo_id = ?1 AND path = ?2
"#;

pub const DELETE_ENTITIES_BY_REPO: &str = r#"
DELETE FROM entities
WHERE repo_id = ?1
"#;

// Relationship queries
pub const UPSERT_RELATIONSHIP: &str = r#"
INSERT INTO relationships (source_id, target_id, rel_type, weight, evidence_json, provenance)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(source_id, target_id, rel_type) DO UPDATE SET
    weight = excluded.weight,
    evidence_json = excluded.evidence_json,
    provenance = excluded.provenance
"#;

pub const RELATIONSHIPS_FROM: &str = r#"
SELECT r.source_id, r.target_id, r.rel_type, r.weight, r.evidence_json, r.provenance,
       e.id, e.project_id, e.repo_id, e.tier, e.parent_id, e.name, e.path, e.language, e.summary, e.summary_commit, e.metrics_json, e.created_at, e.updated_at
FROM relationships r
JOIN entities e ON r.target_id = e.id
WHERE r.source_id = ?1
"#;

pub const RELATIONSHIPS_TO: &str = r#"
SELECT r.source_id, r.target_id, r.rel_type, r.weight, r.evidence_json, r.provenance,
       e.id, e.project_id, e.repo_id, e.tier, e.parent_id, e.name, e.path, e.language, e.summary, e.summary_commit, e.metrics_json, e.created_at, e.updated_at
FROM relationships r
JOIN entities e ON r.source_id = e.id
WHERE r.target_id = ?1
"#;

pub const DELETE_RELATIONSHIPS_BY_SOURCE: &str = r#"
DELETE FROM relationships
WHERE source_id = ?1
"#;

pub const DELETE_RELATIONSHIPS_BY_REPO: &str = r#"
DELETE FROM relationships
WHERE source_id IN (SELECT id FROM entities WHERE repo_id = ?1)
   OR target_id IN (SELECT id FROM entities WHERE repo_id = ?1)
"#;

// Insight queries
pub const UPSERT_INSIGHT: &str = r#"
INSERT INTO insights (id, project_id, category, severity, title, description, entity_ids_json, detected_at, still_valid)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT(id) DO UPDATE SET
    project_id = excluded.project_id,
    category = excluded.category,
    severity = excluded.severity,
    title = excluded.title,
    description = excluded.description,
    entity_ids_json = excluded.entity_ids_json,
    still_valid = excluded.still_valid
"#;

pub const LIST_INSIGHTS: &str = r#"
SELECT id, project_id, category, severity, title, description, entity_ids_json, detected_at, still_valid
FROM insights
WHERE project_id = ?1 AND (?2 IS NULL OR category = ?2) AND (?3 IS NULL OR severity = ?3) AND still_valid = 1
ORDER BY detected_at DESC
LIMIT ?4
"#;

pub const INVALIDATE_INSIGHTS: &str = r#"
UPDATE insights
SET still_valid = 0
WHERE project_id = ?1
"#;

// Convention queries
pub const UPSERT_CONVENTION: &str = r#"
INSERT INTO conventions (id, project_id, category, title, description, example_code, confidence, entity_ids_json, detected_at, still_valid)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
ON CONFLICT(id) DO UPDATE SET
    project_id = excluded.project_id,
    category = excluded.category,
    title = excluded.title,
    description = excluded.description,
    example_code = excluded.example_code,
    confidence = excluded.confidence,
    entity_ids_json = excluded.entity_ids_json,
    still_valid = excluded.still_valid
"#;

pub const LIST_CONVENTIONS: &str = r#"
SELECT id, project_id, category, title, description, example_code, confidence, entity_ids_json, detected_at, still_valid
FROM conventions
WHERE project_id = ?1 AND (?2 IS NULL OR category = ?2)
ORDER BY detected_at DESC
"#;

// Analysis run queries
pub const CREATE_ANALYSIS_RUN: &str = r#"
INSERT INTO analysis_runs (id, repo_id, commit_hash, status)
VALUES (?1, ?2, ?3, ?4)
"#;

pub const UPDATE_ANALYSIS_RUN: &str = r#"
UPDATE analysis_runs
SET files_analyzed = ?1, files_changed = ?2, entities_upserted = ?3, relationships_upserted = ?4,
    duration_ms = ?5, status = ?6, completed_at = ?7
WHERE id = ?8
"#;

// File hash queries
pub const GET_FILE_HASH: &str = r#"
SELECT content_hash
FROM file_hashes
WHERE repo_id = ?1 AND file_path = ?2
"#;

pub const UPSERT_FILE_HASH: &str = r#"
INSERT INTO file_hashes (repo_id, file_path, content_hash, last_analyzed)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(repo_id, file_path) DO UPDATE SET
    content_hash = excluded.content_hash,
    last_analyzed = excluded.last_analyzed
"#;

pub const GET_ALL_FILE_HASHES: &str = r#"
SELECT file_path, content_hash
FROM file_hashes
WHERE repo_id = ?1
"#;

// Project deletion queries
pub const LIST_REPO_IDS_FOR_PROJECT: &str = r#"
SELECT id FROM repositories WHERE project_id = ?1
"#;

pub const DELETE_RELATIONSHIPS_BY_PROJECT: &str = r#"
DELETE FROM relationships
WHERE source_id IN (SELECT id FROM entities WHERE project_id = ?1)
   OR target_id IN (SELECT id FROM entities WHERE project_id = ?1)
"#;

pub const DELETE_ANALYSIS_RUNS_BY_REPO: &str = r#"
DELETE FROM analysis_runs WHERE repo_id = ?1
"#;

pub const DELETE_FILE_HASHES_BY_REPO: &str = r#"
DELETE FROM file_hashes WHERE repo_id = ?1
"#;

pub const DELETE_FILE_HASH: &str = r#"
DELETE FROM file_hashes
WHERE repo_id = ?1 AND file_path = ?2
"#;

pub const DELETE_INSIGHTS_BY_PROJECT: &str = r#"
DELETE FROM insights WHERE project_id = ?1
"#;

pub const DELETE_CONVENTIONS_BY_PROJECT: &str = r#"
DELETE FROM conventions WHERE project_id = ?1
"#;

pub const DELETE_REPOS_BY_PROJECT: &str = r#"
DELETE FROM repositories WHERE project_id = ?1
"#;

pub const DELETE_REPO: &str = r#"
DELETE FROM repositories WHERE id = ?1
"#;

pub const DELETE_PROJECT_BY_ID: &str = r#"
DELETE FROM projects WHERE id = ?1
"#;

// Reconciliation queries
pub const LIST_FILE_ENTITIES_BY_REPO: &str = r#"
SELECT id, path FROM entities
WHERE repo_id = ?1 AND tier = 'file' AND path IS NOT NULL
"#;

pub const LIST_MODULES_BY_REPO: &str = r#"
SELECT id FROM entities
WHERE repo_id = ?1 AND tier = 'module'
"#;

pub const LIST_SUBSYSTEMS_BY_REPO: &str = r#"
SELECT id FROM entities
WHERE repo_id = ?1 AND tier = 'subsystem'
"#;

pub const DELETE_RELATIONSHIPS_BY_TARGET: &str = r#"
DELETE FROM relationships
WHERE target_id = ?1
"#;

pub const DELETE_ENTITY_BY_ID: &str = r#"
DELETE FROM entities WHERE id = ?1
"#;

pub const CLEAR_ALL_SUMMARIES: &str = r#"
UPDATE entities
SET summary = NULL, summary_commit = NULL, updated_at = ?1
WHERE project_id = ?2
"#;

pub const CLEAR_REPO_SUMMARIES: &str = r#"
UPDATE entities
SET summary = NULL, summary_commit = NULL, updated_at = ?1
WHERE repo_id = ?2
"#;

/// Count entities with NULL summary in a repository (function-tier only, the
/// summarization target). Used to detect incomplete summarization and trigger
/// resume.
///
/// The `NOT LOWER(name) LIKE 'test_%'` predicate here must stay in sync with
/// `crate::summarization::pipeline::is_test_entity` (used by the summarization
/// pipeline and the entity-creation filter in `function_preservation.rs`) —
/// both exclude test-prefixed functions from the summarization population.
/// See issue #649.
pub const COUNT_MISSING_SUMMARIES: &str = r#"
SELECT COUNT(*) FROM entities
WHERE repo_id = ?1 AND tier = 'function' AND summary IS NULL
  AND NOT LOWER(name) LIKE 'test_%'
"#;

/// Count all entities stored for a repository (issue #875: `lievo doctor`
/// reports the count without loading the whole table).
pub const COUNT_ENTITIES: &str = r#"
SELECT COUNT(*) FROM entities
WHERE repo_id = ?1
"#;

/// Count entities with NULL summary in a repository at one tier (issue #793).
/// The function tier mirrors `COUNT_MISSING_SUMMARIES` exactly (the value
/// stays byte-comparable across the per-tier change); the module and
/// subsystem tiers feed the refresh report's per-tier coverage.
///
/// The file tier is NOT served by this SQL string: `SqliteStorage`
/// intentionally inherits the trait default for that tier, which counts
/// from `entities_by_repo` using the same Rust test-file predicate the
/// pipeline uses (`is_test_file_path`). A SQL approximation (issue #793
/// review, finding 2) diverged on filenames like `foo_test.rs` and created
/// a second source of truth for the same coverage number.
///
/// Returns `None` for an unknown tier (issue #793 review, finding 3): the
/// tier is matched to one of four fixed query strings, never interpolated
/// into the query text, so there is no SQL-injection sink in the tier value.
pub fn count_missing_summaries_sql(tier: &str) -> Option<String> {
    // Exhaustive match: no `format!`-based interpolation of the tier into
    // the query text — an unknown tier is rejected by the caller instead of
    // reaching the SQL layer (issue #793 review, finding 3).
    Some(match tier {
        "function" => "\nSELECT COUNT(*) FROM entities\nWHERE repo_id = ?1 AND tier = 'function' AND summary IS NULL\n  AND NOT LOWER(name) LIKE 'test_%'\n".to_string(),
        "module" => "\nSELECT COUNT(*) FROM entities\nWHERE repo_id = ?1 AND tier = 'module' AND summary IS NULL\n".to_string(),
        "subsystem" => "\nSELECT COUNT(*) FROM entities\nWHERE repo_id = ?1 AND tier = 'subsystem' AND summary IS NULL\n".to_string(),
        _ => return None,
    })
}

/// Build a parameterized entity search query for N search words.
/// Returns SQL string where params are:
///   ?1 = project_id
///   ?2..?N+1 = one lowercase word per AND block
///   ?N+2 = limit
pub fn build_entity_search_query(word_count: usize) -> String {
    build_entity_search_query_with_tier(word_count, None)
}

/// Build a parameterized entity search query for N search words with optional tier filter.
/// Returns SQL string where params are:
///   ?1 = project_id
///   ?2..?N+1 = one lowercase word per AND block
///   ?N+2 = limit
/// If tier is Some, an additional AND tier = ? clause is appended.
pub fn build_entity_search_query_with_tier(word_count: usize, tier: Option<&str>) -> String {
    let mut sql = r#"
SELECT id, project_id, repo_id, tier, parent_id, name, path, language,
       summary, summary_commit, metrics_json, created_at, updated_at
FROM entities
WHERE project_id = ?1
"#
    .to_string();

    // Add one block per word for OR logic
    // Collect per-word clauses first, then join with OR and wrap in AND
    let word_clauses: Vec<String> = (0..word_count)
        .map(|i| {
            let param_num = i + 2;
            // Each word uses a single parameter bound once — SQLite allows a parameter
            // to appear multiple times in a query (it is evaluated once and reused).
            // The parameter vector pushes ONE string per word, not two.
            format!(
                "(LOWER(name) LIKE '%' || ?{param_num} || '%' OR LOWER(path) LIKE '%' || ?{param_num} || '%')"
            )
        })
        .collect();

    // Add the word match clauses with OR logic
    if !word_clauses.is_empty() {
        sql.push_str(&format!("  AND ({})\n", word_clauses.join("\n    OR ")));
    }

    // Add tier filter if provided
    if tier.is_some() {
        sql.push_str("  AND tier = ?\n");
    }

    // Add ORDER BY and LIMIT
    let limit_param = word_count + 2;
    sql.push_str(&format!(
        r#"ORDER BY name ASC
LIMIT ?{}
"#,
        limit_param
    ));

    sql
}
