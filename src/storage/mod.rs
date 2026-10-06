// Storage layer abstraction and implementations
// Includes: Storage trait, SQLite implementation, schema, queries
pub mod identity_queries;
pub mod queries;
pub mod reconcile;
pub mod schema;
pub mod sqlite;
mod sqlite_ref_impl;
use crate::extraction::function_preservation::is_test_file_path;
use crate::model::{
    AnalysisRun, Convention, Entity, EntityTier, Insight, Project, Relationship, Repository,
};

/// Statistics returned by `delete_project` and `delete_repo` describing what was removed.
#[derive(Debug, Default)]
pub struct DeleteStats {
    pub repos_deleted: u64,
    pub entities_deleted: u64,
    pub relationships_deleted: u64,
    pub insights_deleted: u64,
    pub conventions_deleted: u64,
    pub index_dirs_removed: u64,
}

/// Storage trait abstracts database operations for all entity types.
/// Implementations provide synchronous database access with proper error handling.
pub trait Storage {
    // Project CRUD
    fn create_project(&self, name: &str, description: Option<&str>) -> crate::Result<Project>;
    fn get_project(&self, name: &str) -> crate::Result<Option<Project>>;
    fn get_project_by_id(&self, project_id: &str) -> crate::Result<Option<Project>>;
    fn list_projects(&self) -> crate::Result<Vec<Project>>;
    fn delete_project(&self, project_id: &str) -> crate::Result<DeleteStats>;
    fn add_output_dir(&self, project_id: &str, dir: &str) -> crate::Result<()>;
    fn get_output_dirs(&self, project_id: &str) -> crate::Result<Vec<String>>;

    // Repository CRUD
    fn add_repo(&self, project_id: &str, name: &str, local_path: &str)
    -> crate::Result<Repository>;
    fn get_repo(&self, repo_id: &str) -> crate::Result<Option<Repository>>;
    fn list_repos(&self, project_id: &str) -> crate::Result<Vec<Repository>>;
    fn update_repo_index_path(&self, repo_id: &str, path: &str) -> crate::Result<()>;
    fn update_repo_last_commit(&self, repo_id: &str, commit: &str) -> crate::Result<()>;
    /// Record the repo-wide unresolved-import counter (issue #856) on the
    /// repository row. Written unconditionally by the pipeline — zeros
    /// included, so a fully-resolved fresh index reads as `Some((0, 0))`
    /// rather than `None` (not recorded).
    ///
    /// Note: the trait groups identity writes (set_repo_git_url /
    /// find_repos_by_git_url / update_repo_local_path) here with the
    /// record_* family, while the SqliteStorage impl groups them with the
    /// other UPDATE_* methods — the two orderings are intentionally not
    /// aligned.
    fn record_unresolved_counts(
        &self,
        repo_id: &str,
        internal: u64,
        external: u64,
    ) -> crate::Result<()> {
        let _ = (repo_id, internal, external);
        Ok(())
    }
    /// Set the normalized git_url identity key on a repository row.
    ///
    /// `key` MUST already be a normalized git-remote identity (see issue #24
    /// sub-issue 1 for the normalization format); implementations persist it
    /// opaque, with no well-formedness validation.
    ///
    /// Default is a no-op so test doubles that don't
    /// model git_url keep compiling; `SqliteStorage` overrides with a real
    /// UPDATE that returns `LievoError::RepoNotFound` on 0 rows.
    fn set_repo_git_url(&self, _repo_id: &str, _key: &str) -> crate::Result<()> {
        Ok(())
    }

    /// Find all repositories registered under a normalized git_url key.
    ///
    /// `key` MUST already be a normalized git-remote identity (see issue #24
    /// sub-issue 1); implementations persist it opaque, with no validation.
    ///
    /// Returns a `Vec` (not `Option`) because the same normalized remote can
    /// be registered at several local_paths (checkouts) across projects.
    /// Default returns an empty vec so test doubles keep compiling.
    fn find_repos_by_git_url(&self, _key: &str) -> crate::Result<Vec<Repository>> {
        Ok(Vec::new())
    }

    /// Relocate a repository: update both `local_path` and `index_path`
    /// atomically in a single UPDATE statement, plus bump `updated_at`.
    ///
    /// The two columns are written in one statement so no intermediate
    /// state leaves `local_path` moved while `index_path` still points at
    /// the old location. This is the ONLY write path that relocates a repo —
    /// `update_repo_index_path` is for initial index setup and never moves
    /// one. `new_path` / `new_index_path` MUST be canonical absolute paths.
    /// Default is a no-op so test doubles keep
    /// compiling; `SqliteStorage` overrides with a real UPDATE that
    /// returns `LievoError::RepoNotFound` on 0 rows.
    fn update_repo_local_path(
        &self,
        _repo_id: &str,
        _new_path: &str,
        _new_index_path: Option<&str>,
    ) -> crate::Result<()> {
        Ok(())
    }

    fn update_repo_project(&self, repo_id: &str, project_id: &str) -> crate::Result<()>;
    /// Set the enabled-but-unconfigured marker for a repo to a config
    /// fingerprint (or `None` to clear it). Issue #788: lets `is_stale`
    /// skip re-entering the full pipeline for a repo whose summarization gate
    /// is enabled but has no usable backend, until a new commit arrives or the
    /// config changes.
    ///
    /// Default is a no-op so test mock implementations that don't model the
    /// column keep compiling; the production `SqliteStorage` overrides it.
    fn update_repository_unconfigured_marker(
        &self,
        _repo_id: &str,
        _fingerprint: Option<&str>,
    ) -> crate::Result<()> {
        Ok(())
    }
    fn delete_repo(&self, repo_id: &str) -> crate::Result<DeleteStats>;

    // Entity CRUD
    fn upsert_entity(&self, entity: &Entity) -> crate::Result<()>;
    /// Actively clear an entity's stored `summary` and `summary_commit`.
    /// Unlike a NULL summary passed to `upsert_entity` (which means "no summary to
    /// offer" and is preserved via COALESCE), this is the explicit clear intent used
    /// by `lievo summarize --file` to invalidate a stale summary for re-summarization.
    fn clear_entity_summary(&self, entity_id: &str) -> crate::Result<()>;
    fn get_entity(&self, entity_id: &str) -> crate::Result<Option<Entity>>;
    fn list_entities(
        &self,
        project_id: &str,
        tier: Option<EntityTier>,
    ) -> crate::Result<Vec<Entity>>;
    /// Storage-side prefilter for `lievo_explore` symbol-name matching:
    /// fetch only symbol-tier entities (the non-file tiers — see
    /// `crate::retrieval::tools_explore_symbols::SYMBOL_TIERS`) whose
    /// lowercase name contains ANY plain lowercase query word (OR
    /// semantics). `words` must be plain lowercase words — no `%` wildcards;
    /// the implementation owns LIKE pattern formatting. `SqliteStorage`
    /// overrides with the indexed `LIKE` query (narrow repo_id/path/name
    /// projection); the default filters `list_entities` in Rust so test
    /// mocks that don't model the SQL projection still compile.
    fn symbols_matching_names(
        &self,
        project_id: &str,
        words: &[String],
    ) -> crate::Result<Vec<crate::retrieval::tools_explore_symbols::SymbolCandidate>> {
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let tiers = crate::retrieval::tools_explore_symbols::SYMBOL_TIERS;
        let mut out: Vec<crate::retrieval::tools_explore_symbols::SymbolCandidate> = Vec::new();
        for tier in tiers {
            for e in self.list_entities(project_id, Some(tier))? {
                let name_lc = e.name.to_lowercase();
                if words
                    .iter()
                    .any(|w| !w.is_empty() && name_lc.contains(w.as_str()))
                {
                    out.push(crate::retrieval::tools_explore_symbols::SymbolCandidate {
                        repo_id: e.repo_id.clone(),
                        path: e.path,
                        name: e.name,
                    });
                }
            }
        }
        Ok(out)
    }
    /// Search entities by name/path substring.
    /// Matches entities where ANY search word appears in name or path (OR semantics).
    /// `words` must be non-empty — passing an empty slice returns an InvalidInput error.
    /// `tier` optionally filters by EntityTier (e.g., "function", "file", "module", "subsystem").
    fn search_entities_by_name(
        &self,
        project_id: &str,
        words: &[&str],
        limit: usize,
        tier: Option<&str>,
    ) -> crate::Result<Vec<Entity>>;
    fn entities_by_repo(
        &self,
        repo_id: &str,
        tier: Option<EntityTier>,
    ) -> crate::Result<Vec<Entity>>;
    fn entities_by_parent(&self, parent_id: &str) -> crate::Result<Vec<Entity>>;
    fn entity_by_path(&self, repo_id: &str, path: &str) -> crate::Result<Option<Entity>>;
    /// Batch lookup of File-tier entity ids by path for one repository.
    ///
    /// The default scans `entities_by_repo` in Rust (File tier only, exact
    /// path match) so test storages that model the entity table compile
    /// without a dedicated implementation; `SqliteStorage` overrides it with
    /// an indexed `WHERE path IN (...)` query (chunked to respect SQLite's
    /// bind-variable limit).
    fn entity_ids_for_paths(
        &self,
        repo_id: &str,
        paths: &[&str],
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        let wanted: std::collections::HashSet<&str> = paths.iter().copied().collect();
        let mut map = std::collections::HashMap::new();
        for e in self.entities_by_repo(repo_id, Some(EntityTier::File))? {
            if let Some(p) = e.path.as_deref()
                && wanted.contains(p)
            {
                map.insert(p.to_string(), e.id);
            }
        }
        Ok(map)
    }
    fn entity_by_path_projectwide(
        &self,
        project_id: &str,
        path: &str,
    ) -> crate::Result<Vec<Entity>>;
    /// Count all entities stored for a repository via a `COUNT` query
    /// (issue #875: used by `lievo doctor`, which must not load the whole
    /// table to report a number).
    ///
    /// The default implementation counts from `entities_by_repo` so test
    /// mocks that already implement `entities_by_repo` compile without a
    /// second implementation; `SqliteStorage` overrides it with a `COUNT`
    /// query.
    fn count_entities(&self, repo_id: &str) -> crate::Result<u64> {
        Ok(self.entities_by_repo(repo_id, None)?.len() as u64)
    }
    fn delete_entities_by_repo(&self, repo_id: &str) -> crate::Result<u64>;
    /// Delete entities whose paths match excluded patterns.
    /// Used during `analyze --full` to clean up old self-referential output directory entities.
    fn delete_entities_by_paths(
        &self,
        repo_id: &str,
        exclude_paths: &[String],
    ) -> crate::Result<u64>;

    // Relationship CRUD
    fn upsert_relationship(&self, rel: &Relationship) -> crate::Result<()>;
    fn relationships_from(&self, source_id: &str) -> crate::Result<Vec<(Relationship, Entity)>>;

    /// List all relationships for a project using a single database query
    fn list_all_relationships(&self, project_id: &str) -> crate::Result<Vec<Relationship>> {
        // Default implementation: test mocks can rely on this
        // Production SqliteStorage provides its own efficient implementation
        let _ = project_id;
        Ok(Vec::new())
    }
    fn relationships_to(&self, target_id: &str) -> crate::Result<Vec<(Relationship, Entity)>>;
    fn delete_relationships_by_source(&self, source_id: &str) -> crate::Result<u64>;

    // Insight CRUD
    fn upsert_insight(&self, insight: &Insight) -> crate::Result<()>;
    fn list_insights(
        &self,
        project_id: &str,
        category: Option<&str>,
        severity: Option<&str>,
        limit: usize,
    ) -> crate::Result<Vec<Insight>>;
    /// Mark all existing insights for a project as stale before re-detecting.
    fn invalidate_insights(&self, project_id: &str) -> crate::Result<()>;

    // Convention CRUD
    fn upsert_convention(&self, convention: &Convention) -> crate::Result<()>;
    fn list_conventions(
        &self,
        project_id: &str,
        category: Option<&str>,
    ) -> crate::Result<Vec<Convention>>;

    // Analysis run tracking
    fn create_analysis_run(&self, repo_id: &str, commit_hash: &str) -> crate::Result<AnalysisRun>;
    fn update_analysis_run(&self, run: &AnalysisRun) -> crate::Result<()>;

    // File hashes for incremental detection
    fn get_file_hash(&self, repo_id: &str, file_path: &str) -> crate::Result<Option<String>>;
    fn upsert_file_hash(
        &self,
        repo_id: &str,
        file_path: &str,
        content_hash: &str,
    ) -> crate::Result<()>;

    /// Get all file hashes for a repository.
    /// Returns a HashMap mapping file_path -> content_hash.
    fn get_all_file_hashes(
        &self,
        repo_id: &str,
    ) -> crate::Result<std::collections::HashMap<String, String>>;

    /// Delete a file hash for a specific file.
    fn delete_file_hash(&self, repo_id: &str, file_path: &str) -> crate::Result<()>;

    /// Read back the repo-wide unresolved-import counter persisted on the
    /// repository row (split internal/external per the #690 amendment,
    /// stored on the repo row since #856). Returns `None` when the repo has
    /// no recorded counts (pre-fix indexes degrade to null — never a false
    /// "full coverage" claim). Test mocks return `None` by default.
    fn get_unresolved_counts(&self, _repo_id: &str) -> Option<(u64, u64)> {
        None
    }

    /// Persist all analysis results atomically in a single transaction.
    /// Returns `(entities_upserted, relationships_upserted)`.
    fn persist_analysis_batch(
        &self,
        entities: &[&Entity],
        relationships: &[Relationship],
        run: &AnalysisRun,
        repo_id: &str,
        last_commit: &str,
    ) -> crate::Result<(i64, i64)>;

    /// Remove entities whose files no longer exist on disk, cascading upward through the hierarchy.
    /// Returns statistics about what was removed.
    fn reconcile_entities(
        &self,
        project_id: &str,
        repo_id: &str,
        repo_path: &std::path::Path,
    ) -> crate::Result<crate::storage::reconcile::ReconcileStats>;

    /// Clear all entity summaries for a project.
    /// Used when `analyze --full` re-extracts entities to remove stale summaries from previous runs.
    fn clear_all_summaries(&self, project_id: &str) -> crate::Result<()>;

    /// Clear entity summaries for a specific repository.
    /// Used when `analyze --full` re-extracts entities to remove stale summaries from a single repo.
    fn clear_repo_summaries(&self, repo_id: &str) -> crate::Result<()>;

    /// Count function entities with NULL summary in a repository.
    /// Used to detect incomplete summarization and trigger resume.
    fn count_missing_summaries(&self, repo_id: &str) -> crate::Result<u64>;

    /// Count entities with NULL summary at one tier in a repository (issue
    /// #793: per-tier coverage in the refresh report).
    ///
    /// The test exclusion is tier-aware, mirroring the pipeline's notion of a
    /// test entity: functions are excluded by name (`is_test_entity`), files
    /// by path (`is_test_file_path`), module and subsystem tiers carry no
    /// test exclusion (entities at those tiers have no file path). The
    /// default implementation counts from `entities_by_repo` so test mocks
    /// that already implement `entities_by_repo` get correct per-tier counts
    /// without a second implementation.
    fn count_missing_summaries_tier(&self, repo_id: &str, tier: &str) -> crate::Result<u64> {
        let tier = tier
            .parse::<EntityTier>()
            .map_err(|_| crate::LievoError::InvalidInput(format!("invalid tier: {tier}")))?;
        let entities = self.entities_by_repo(repo_id, Some(tier))?;
        Ok(entities
            .iter()
            .filter(|e| {
                let missing = e.summary.is_none();
                if !missing {
                    return false;
                }
                if tier == EntityTier::Function {
                    !crate::summarization::pipeline::is_test_entity(&e.name)
                } else if tier == EntityTier::File {
                    e.path
                        .as_deref()
                        .map(|p| !is_test_file_path(p))
                        .unwrap_or(true)
                } else {
                    true
                }
            })
            .count() as u64)
    }
}

// Test modules for SQLite operations
#[cfg(test)]
mod db_test_helpers;
#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod entity_search_tests;
#[cfg(test)]
mod persist_tests;
#[cfg(test)]
mod sqlite_ops_regression_tests;
#[cfg(test)]
mod storage_tests;
