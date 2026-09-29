use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::error::LievoError;
use crate::model::*;
use crate::storage::Storage;
use crate::storage::queries as q;
use crate::storage::schema;

#[path = "sqlite_ops.rs"]
pub mod sqlite_ops;

#[path = "sqlite_delete.rs"]
mod sqlite_delete;

#[path = "sqlite_project.rs"]
mod sqlite_project;

#[path = "sqlite_ops_entity_by_path_tests.rs"]
#[cfg(test)]
mod sqlite_ops_entity_by_path_tests;

pub struct SqliteStorage {
    conn: Connection,
}

// SqliteStorage intentionally does not implement Clone.
// Opening a second SQLite connection to the same file is problematic and can fail.
// The correct pattern is to share a single connection via Arc<Mutex<SqliteStorage>>.

impl SqliteStorage {
    /// Open the default database for this process, applying `LIEVO_DB` (a
    /// full file path) as an override when present — used by integration
    /// tests to point the binary at an isolated, per-test database. When
    /// `LIEVO_DB` is unset, falls back to `~/.lievo/lievo.db`.
    pub fn open() -> Result<Self> {
        let db_path = match std::env::var("LIEVO_DB") {
            Ok(p) if !p.is_empty() => PathBuf::from(p),
            _ => {
                let home = dirs::home_dir().ok_or_else(|| {
                    LievoError::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Home directory not found",
                    ))
                })?;
                let lievo_dir = home.join(".lievo");
                fs::create_dir_all(&lievo_dir)?;
                lievo_dir.join("lievo.db")
            }
        };
        Self::open_at(&db_path)
    }

    pub fn open_at<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        schema::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub(crate) fn now() -> String {
        use chrono::Utc;
        Utc::now().to_rfc3339()
    }
}

/// Convert a String parse error into a rusqlite error (for use inside query_map closures).
pub(crate) fn parse_err(e: String) -> rusqlite::Error {
    rusqlite::Error::InvalidParameterName(e)
}

impl Storage for SqliteStorage {
    // ---- Project (delegated to sqlite_project) ----

    fn create_project(&self, name: &str, description: Option<&str>) -> Result<Project> {
        sqlite_project::create_project(&self.conn, &Self::now(), name, description)
    }

    fn get_project(&self, name: &str) -> Result<Option<Project>> {
        sqlite_project::get_project(&self.conn, name)
    }

    fn get_project_by_id(&self, project_id: &str) -> Result<Option<Project>> {
        sqlite_project::get_project_by_id(&self.conn, project_id)
    }

    fn list_projects(&self) -> Result<Vec<Project>> {
        sqlite_project::list_projects(&self.conn)
    }

    fn delete_project(&self, project_id: &str) -> Result<crate::storage::DeleteStats> {
        sqlite_delete::delete_project(&self.conn, project_id)
    }

    fn add_output_dir(&self, project_id: &str, dir: &str) -> Result<()> {
        sqlite_project::add_output_dir(&self.conn, project_id, dir)
    }

    fn get_output_dirs(&self, project_id: &str) -> Result<Vec<String>> {
        sqlite_project::get_output_dirs(&self.conn, project_id)
    }

    // ---- Repository ----

    fn add_repo(&self, project_id: &str, name: &str, local_path: &str) -> Result<Repository> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = Self::now();
        self.conn.execute(
            q::CREATE_REPO,
            (
                &id,
                project_id,
                name,
                local_path,
                None::<&str>,
                "main",
                &now,
                &now,
            ),
        )?;
        Ok(Repository {
            id: id.clone(),
            project_id: project_id.to_string(),
            name: name.to_string(),
            git_url: None,
            local_path: local_path.to_string(),
            default_branch: "main".to_string(),
            last_analyzed_commit: None,
            index_path: None,
            summarization_unconfigured: None,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    fn get_repo(&self, repo_id: &str) -> Result<Option<Repository>> {
        let mut stmt = self.conn.prepare_cached(q::GET_REPO)?;
        let mut rows = stmt.query([repo_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(Repository {
                id: row.get(0)?,
                project_id: row.get(1)?,
                name: row.get(2)?,
                git_url: row.get(3)?,
                local_path: row.get(4)?,
                default_branch: row.get(5)?,
                last_analyzed_commit: row.get(6)?,
                index_path: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                summarization_unconfigured: row.get(10)?,
            })),
            None => Ok(None),
        }
    }

    fn list_repos(&self, project_id: &str) -> Result<Vec<Repository>> {
        let mut stmt = self.conn.prepare_cached(q::LIST_REPOS)?;
        let rows = stmt.query_map([project_id], |row| {
            Ok(Repository {
                id: row.get(0)?,
                project_id: row.get(1)?,
                name: row.get(2)?,
                git_url: row.get(3)?,
                local_path: row.get(4)?,
                default_branch: row.get(5)?,
                last_analyzed_commit: row.get(6)?,
                index_path: row.get(7)?,
                created_at: row.get(8)?,
                updated_at: row.get(9)?,
                summarization_unconfigured: row.get(10)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    fn update_repo_index_path(&self, repo_id: &str, path: &str) -> Result<()> {
        let now = Self::now();
        self.conn
            .execute(q::UPDATE_REPO_INDEX_PATH, (path, &now, repo_id))?;
        Ok(())
    }

    fn update_repo_last_commit(&self, repo_id: &str, commit: &str) -> Result<()> {
        let now = Self::now();
        self.conn
            .execute(q::UPDATE_REPO_LAST_COMMIT, (commit, &now, repo_id))?;
        Ok(())
    }

    fn record_unresolved_counts(&self, repo_id: &str, internal: u64, external: u64) -> Result<()> {
        let now = Self::now();
        let rows = self.conn.execute(
            q::RECORD_REPO_UNRESOLVED_COUNTS,
            (internal as i64, external as i64, &now, repo_id),
        )?;
        if rows == 0 {
            return Err(crate::LievoError::RepoNotFound(repo_id.to_string()));
        }
        Ok(())
    }

    fn update_repository_unconfigured_marker(
        &self,
        repo_id: &str,
        fingerprint: Option<&str>,
    ) -> Result<()> {
        let now = Self::now();
        let rows = self.conn.execute(
            q::UPDATE_REPOSITORY_UNCONFIGURED_MARKER,
            (fingerprint, &now, repo_id),
        )?;
        if rows == 0 {
            return Err(crate::LievoError::RepoNotFound(repo_id.to_string()));
        }
        Ok(())
    }

    fn update_repo_project(&self, repo_id: &str, project_id: &str) -> Result<()> {
        let now = Self::now();
        let tx = self.conn.unchecked_transaction()?;
        let rows = tx.execute(q::UPDATE_REPO_PROJECT, (project_id, &now, repo_id))?;
        if rows == 0 {
            return Err(crate::LievoError::RepoNotFound(repo_id.to_string()));
        }
        // Delete all entities for this repo instead of updating their project_id.
        // Entity IDs are deterministically baked as {project_id}:{repo_name}:{tier}:{path}
        // at extraction time, so moving a repo to a new project requires regenerating all
        // entity IDs. Deleting and letting refresh re-analyze is simpler than rewriting
        // entity IDs in place. Relationships cascade delete via ON DELETE CASCADE.
        tx.execute(q::DELETE_RELATIONSHIPS_BY_REPO, [repo_id])?;
        tx.execute(q::DELETE_ENTITIES_BY_REPO, [repo_id])?;
        tx.commit()?;
        Ok(())
    }

    fn delete_repo(&self, repo_id: &str) -> Result<crate::storage::DeleteStats> {
        sqlite_delete::delete_repo(&self.conn, repo_id)
    }

    // ---- Entity (delegated to sqlite_ops) ----

    fn upsert_entity(&self, entity: &Entity) -> Result<()> {
        sqlite_ops::upsert_entity(&self.conn, entity, &Self::now())
    }

    fn clear_entity_summary(&self, entity_id: &str) -> Result<()> {
        let _ = sqlite_ops::clear_entity_summary(&self.conn, entity_id)?;
        Ok(())
    }

    fn get_entity(&self, entity_id: &str) -> Result<Option<Entity>> {
        sqlite_ops::get_entity(&self.conn, entity_id)
    }

    fn list_entities(&self, project_id: &str, tier: Option<EntityTier>) -> Result<Vec<Entity>> {
        sqlite_ops::list_entities(&self.conn, project_id, tier)
    }

    fn symbols_matching_names(
        &self,
        params: &[String],
    ) -> Result<Vec<crate::retrieval::tools_explore_symbols::SymbolCandidate>> {
        let word_count = params.len().saturating_sub(1);
        let Some(sql) = crate::storage::queries::build_symbol_name_prefilter_query(word_count)
        else {
            return Ok(Vec::new());
        };
        let param_refs: Vec<&dyn rusqlite::ToSql> =
            params.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(param_refs.as_slice(), |row| {
            Ok(crate::retrieval::tools_explore_symbols::SymbolCandidate {
                path: row.get(0)?,
                name: row.get(1)?,
            })
        })?;
        sqlite_ops::collect_rows(rows)
    }

    fn search_entities_by_name(
        &self,
        project_id: &str,
        words: &[&str],
        limit: usize,
        tier: Option<&str>,
    ) -> Result<Vec<Entity>> {
        sqlite_ops::search_entities_by_name(&self.conn, project_id, words, limit, tier)
    }

    fn entities_by_repo(&self, repo_id: &str, tier: Option<EntityTier>) -> Result<Vec<Entity>> {
        sqlite_ops::entities_by_repo(&self.conn, repo_id, tier)
    }

    fn count_entities(&self, repo_id: &str) -> Result<u64> {
        let count: i64 = self
            .conn
            .query_row(q::COUNT_ENTITIES, [repo_id], |row| row.get(0))?;
        Ok(count as u64)
    }

    fn get_unresolved_counts(&self, repo_id: &str) -> Option<(u64, u64)> {
        sqlite_ops::get_unresolved_counts(&self.conn, repo_id)
            .ok()
            .flatten()
    }

    fn entities_by_parent(&self, parent_id: &str) -> Result<Vec<Entity>> {
        sqlite_ops::entities_by_parent(&self.conn, parent_id)
    }

    fn entity_by_path(&self, repo_id: &str, path: &str) -> Result<Option<Entity>> {
        sqlite_ops::entity_by_path(&self.conn, repo_id, path)
    }

    fn entity_ids_for_paths(
        &self,
        repo_id: &str,
        paths: &[&str],
    ) -> Result<std::collections::HashMap<String, String>> {
        sqlite_ops::entity_ids_for_paths(&self.conn, repo_id, paths)
    }

    fn entity_by_path_projectwide(&self, project_id: &str, path: &str) -> Result<Vec<Entity>> {
        sqlite_ops::entity_by_path_projectwide(&self.conn, project_id, path)
    }

    fn delete_entities_by_repo(&self, repo_id: &str) -> Result<u64> {
        sqlite_ops::delete_entities_by_repo(&self.conn, repo_id)
    }

    fn delete_entities_by_paths(&self, repo_id: &str, exclude_paths: &[String]) -> Result<u64> {
        sqlite_ops::delete_entities_by_paths(&self.conn, repo_id, exclude_paths)
    }

    // ---- Relationship (delegated to sqlite_ops) ----

    fn upsert_relationship(&self, rel: &Relationship) -> Result<()> {
        sqlite_ops::upsert_relationship(&self.conn, rel)
    }

    fn relationships_from(&self, source_id: &str) -> Result<Vec<(Relationship, Entity)>> {
        sqlite_ops::relationships_from(&self.conn, source_id)
    }

    fn list_all_relationships(&self, project_id: &str) -> Result<Vec<Relationship>> {
        sqlite_ops::list_all_relationships(&self.conn, project_id)
    }

    fn relationships_to(&self, target_id: &str) -> Result<Vec<(Relationship, Entity)>> {
        sqlite_ops::relationships_to(&self.conn, target_id)
    }

    fn delete_relationships_by_source(&self, source_id: &str) -> Result<u64> {
        sqlite_ops::delete_relationships_by_source(&self.conn, source_id)
    }

    // ---- Insight (delegated to sqlite_ops) ----

    fn upsert_insight(&self, insight: &Insight) -> Result<()> {
        sqlite_ops::upsert_insight(&self.conn, insight)
    }

    fn list_insights(
        &self,
        project_id: &str,
        category: Option<&str>,
        severity: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Insight>> {
        sqlite_ops::list_insights(&self.conn, project_id, category, severity, limit)
    }

    fn invalidate_insights(&self, project_id: &str) -> Result<()> {
        sqlite_ops::invalidate_insights(&self.conn, project_id)
    }

    // ---- Convention (delegated to sqlite_ops) ----

    fn upsert_convention(&self, convention: &Convention) -> Result<()> {
        sqlite_ops::upsert_convention(&self.conn, convention)
    }

    fn list_conventions(
        &self,
        project_id: &str,
        category: Option<&str>,
    ) -> Result<Vec<Convention>> {
        sqlite_ops::list_conventions(&self.conn, project_id, category)
    }

    // ---- Analysis run & file hash (delegated to sqlite_ops) ----

    fn create_analysis_run(&self, repo_id: &str, commit_hash: &str) -> Result<AnalysisRun> {
        sqlite_ops::create_analysis_run(&self.conn, repo_id, commit_hash)
    }

    fn update_analysis_run(&self, run: &AnalysisRun) -> Result<()> {
        sqlite_ops::update_analysis_run(&self.conn, run)
    }

    fn get_file_hash(&self, repo_id: &str, file_path: &str) -> Result<Option<String>> {
        sqlite_ops::get_file_hash(&self.conn, repo_id, file_path)
    }

    fn upsert_file_hash(&self, repo_id: &str, file_path: &str, content_hash: &str) -> Result<()> {
        sqlite_ops::upsert_file_hash(&self.conn, repo_id, file_path, content_hash, &Self::now())
    }

    fn get_all_file_hashes(
        &self,
        repo_id: &str,
    ) -> Result<std::collections::HashMap<String, String>> {
        sqlite_ops::get_all_file_hashes(&self.conn, repo_id)
    }

    fn delete_file_hash(&self, repo_id: &str, file_path: &str) -> Result<()> {
        sqlite_ops::delete_file_hash(&self.conn, repo_id, file_path)
    }

    fn persist_analysis_batch(
        &self,
        entities: &[&Entity],
        relationships: &[Relationship],
        run: &AnalysisRun,
        repo_id: &str,
        last_commit: &str,
    ) -> crate::Result<(i64, i64)> {
        sqlite_ops::persist_analysis_batch(
            &self.conn,
            entities,
            relationships,
            run,
            repo_id,
            last_commit,
            &Self::now(),
        )
    }

    fn reconcile_entities(
        &self,
        project_id: &str,
        repo_id: &str,
        repo_path: &std::path::Path,
    ) -> crate::Result<crate::storage::reconcile::ReconcileStats> {
        crate::storage::reconcile::reconcile_entities(&self.conn, project_id, repo_id, repo_path)
    }

    fn clear_all_summaries(&self, project_id: &str) -> crate::Result<()> {
        self.conn
            .execute(q::CLEAR_ALL_SUMMARIES, (&Self::now(), project_id))?;
        Ok(())
    }

    fn clear_repo_summaries(&self, repo_id: &str) -> crate::Result<()> {
        self.conn
            .execute(q::CLEAR_REPO_SUMMARIES, (&Self::now(), repo_id))?;
        Ok(())
    }

    fn count_missing_summaries(&self, repo_id: &str) -> crate::Result<u64> {
        let count: i64 = self
            .conn
            .query_row(q::COUNT_MISSING_SUMMARIES, [repo_id], |row| row.get(0))?;
        Ok(count as u64)
    }

    // `count_missing_summaries_tier` intentionally does NOT override the
    // default implementation: the default counts from `entities_by_repo`
    // using the same Rust test-file predicate the pipeline uses
    // (`is_test_entity` for function names, `is_test_file_path` for file
    // paths). An SQL approximation (issue #793 review, finding 2) diverged
    // on filenames like `foo_test.rs` and created a second source of truth
    // for the same coverage number.
}

#[path = "sqlite_tests.rs"]
#[cfg(test)]
mod sqlite_tests;
