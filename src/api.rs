// Lievo — top-level public API facade.
//
// Composes SqliteStorage into a single entry point. Each method is a thin
// delegation to storage or an inline equivalent of the QueryEngine logic.

use std::path::Path;
use std::sync::Mutex;

use crate::Result;
use crate::analysis::insights::InsightDetector;
use crate::model::{Entity, EntityTier, Insight, Project, Relationship, Repository};
use crate::retrieval::project_boundary::same_project;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

/// Top-level entry point for the lievo knowledge bank.
///
/// Wraps `SqliteStorage` in a `Mutex` so that `Lievo` is `Send + Sync`.
/// If another thread panics while holding the storage lock, the mutex
/// becomes poisoned. All methods convert poison to [`LievoError::DatabaseLocked`].
pub struct Lievo {
    storage: Mutex<SqliteStorage>,
}

impl Lievo {
    /// Open the default database at `~/.lievo/lievo.db`.
    pub fn open() -> Result<Self> {
        Ok(Self {
            storage: Mutex::new(SqliteStorage::open()?),
        })
    }

    /// Open a database at a custom path (useful for testing).
    ///
    /// Returns [`LievoError::InvalidProjectId`] if `db_path` is empty.
    pub fn open_at(db_path: &Path) -> Result<Self> {
        if db_path.as_os_str().is_empty() {
            return Err(crate::LievoError::InvalidProjectId(
                "db_path cannot be empty".to_string(),
            ));
        }
        Ok(Self {
            storage: Mutex::new(SqliteStorage::open_at(db_path)?),
        })
    }

    /// Acquire the storage lock, converting mutex poison to [`LievoError::DatabaseLocked`].
    fn lock_storage(&self) -> Result<std::sync::MutexGuard<'_, SqliteStorage>> {
        self.storage
            .lock()
            .map_err(|_| crate::LievoError::DatabaseLocked)
    }

    // ---- Freshness & auto-refresh ----

    /// Check whether any repository HEAD has moved past `last_analyzed_commit`.
    ///
    /// Returns `true` as soon as the first stale repo is detected.
    /// Repos whose local path cannot be opened as a git repo are silently skipped.
    pub fn is_stale(&self, project_id: &str) -> Result<bool> {
        let storage = self.lock_storage()?;
        crate::refresh::is_stale(&*storage, project_id, false)
    }

    /// Re-run cheap analysis layers when any repo HEAD has moved.
    ///
    /// If the project is already up to date, returns immediately with `was_stale=false`.
    /// Only runs extraction, relationships, insights and semantic re-indexing —
    /// NOT the expensive LLM summarization layer.
    ///
    /// Uses default refresh options (incremental, no force, no full rebuild).
    /// To control refresh behavior, use `refresh::ensure_fresh()` directly with `RefreshOptions`.
    pub fn ensure_fresh(&self, project_id: &str) -> Result<crate::refresh::RefreshStats> {
        // Check staleness under a brief lock, then drop it before running the pipeline.
        // This avoids holding the mutex across the full analysis pipeline, which can
        // take seconds and would block all other Lievo methods in the meantime.
        let is_stale = {
            let storage = self.lock_storage()?;
            crate::refresh::is_stale(&*storage, project_id, false)?
        };

        if !is_stale {
            return Ok(crate::refresh::RefreshStats {
                was_stale: false,
                ..Default::default()
            });
        }

        // Re-acquire lock for the pipeline. Single lock held at a time — no nesting.
        let storage = self.lock_storage()?;
        let opts = crate::refresh::RefreshOptions::default();
        crate::refresh::ensure_fresh(&*storage, project_id, &opts)
    }

    // ---- Project management ----

    /// Create a new project with the given name.
    pub fn create_project(&self, name: &str) -> Result<Project> {
        self.lock_storage()?.create_project(name, None)
    }

    /// List all projects.
    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.lock_storage()?.list_projects()
    }

    /// Look up a project by name. Returns `None` if not found.
    pub fn project(&self, name: &str) -> Result<Option<Project>> {
        self.lock_storage()?.get_project(name)
    }

    // ---- Repository management ----

    /// Add a repository to a project.
    pub fn add_repo(&self, project_id: &str, name: &str, local_path: &str) -> Result<Repository> {
        if project_id.trim().is_empty() {
            return Err(crate::LievoError::InvalidProjectId(project_id.to_string()));
        }
        self.lock_storage()?.add_repo(project_id, name, local_path)
    }

    /// List all repositories in a project.
    pub fn list_repos(&self, project_id: &str) -> Result<Vec<Repository>> {
        if project_id.trim().is_empty() {
            return Err(crate::LievoError::InvalidProjectId(project_id.to_string()));
        }
        self.lock_storage()?.list_repos(project_id)
    }

    /// Move a repository from its current project to a different project.
    ///
    /// Returns [`LievoError::InvalidInput`] if `repo_id` is empty.
    /// Returns [`LievoError::RepoNotFound`] if `repo_id` does not exist.
    /// Returns [`LievoError::InvalidProjectId`] if `project_id` is empty.
    /// Returns [`LievoError::ProjectNotFound`] if `project_id` does not exist.
    pub fn link_repo(&self, project_id: &str, repo_id: &str) -> Result<()> {
        if project_id.trim().is_empty() {
            return Err(crate::LievoError::InvalidProjectId(project_id.to_string()));
        }
        if repo_id.trim().is_empty() {
            return Err(crate::LievoError::InvalidInput(
                "repo_id cannot be empty".to_string(),
            ));
        }
        let storage = self.lock_storage()?;
        if storage.get_project_by_id(project_id)?.is_none() {
            return Err(crate::LievoError::ProjectNotFound(project_id.to_string()));
        }
        storage.update_repo_project(repo_id, project_id)
    }

    // ---- Entity queries ----

    /// Return all subsystem-tier entities for the given project.
    pub fn subsystems(&self, project_id: &str) -> Result<Vec<Entity>> {
        if project_id.trim().is_empty() {
            return Err(crate::LievoError::InvalidProjectId(project_id.to_string()));
        }
        self.lock_storage()?
            .list_entities(project_id, Some(EntityTier::Subsystem))
    }

    /// Return all module children of the given subsystem entity.
    pub fn modules_in(&self, subsystem_id: &str) -> Result<Vec<Entity>> {
        self.lock_storage()?.entities_by_parent(subsystem_id)
    }

    /// Return all file children of the given module entity.
    pub fn files_in(&self, module_id: &str) -> Result<Vec<Entity>> {
        self.lock_storage()?.entities_by_parent(module_id)
    }

    /// Look up a single entity by its ID.
    pub fn entity(&self, entity_id: &str) -> Result<Option<Entity>> {
        self.lock_storage()?.get_entity(entity_id)
    }

    /// Look up a single entity by its repository and file path.
    ///
    /// Returns [`LievoError::InvalidInput`] if `repo_id` or `path` is empty.
    /// Normalizes the path by stripping a leading `./` prefix so that
    /// `"./src/foo.rs"` and `"src/foo.rs"` resolve to the same entity.
    pub fn entity_by_path(&self, repo_id: &str, path: &str) -> Result<Option<Entity>> {
        if repo_id.trim().is_empty() {
            return Err(crate::LievoError::InvalidInput(
                "repo_id cannot be empty".to_string(),
            ));
        }
        if path.trim().is_empty() {
            return Err(crate::LievoError::InvalidInput(
                "path cannot be empty".to_string(),
            ));
        }
        let normalized = normalize_entity_path(path);
        self.lock_storage()?.entity_by_path(repo_id, &normalized)
    }

    /// Return the top-`limit` entities sorted by complexity descending (hotspots).
    ///
    /// Complexity is read from `metrics_json` as `{"complexity_max": <number>}`.
    /// Entities with no metrics are treated as complexity 0.
    pub fn hotspots(&self, project_id: &str, limit: usize) -> Result<Vec<Entity>> {
        if project_id.trim().is_empty() {
            return Err(crate::LievoError::InvalidProjectId(project_id.to_string()));
        }
        let mut entities = self.lock_storage()?.list_entities(project_id, None)?;
        entities.sort_by(|a, b| {
            let ca = complexity_of(a);
            let cb = complexity_of(b);
            cb.partial_cmp(&ca)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        entities.truncate(limit);
        Ok(entities)
    }

    /// Run all insight detectors for the project and return detected insights.
    ///
    /// Insights are sorted by severity (critical first), then by title.
    /// Previous detections are invalidated before each run.
    pub fn insights(&self, project_id: &str) -> Result<Vec<Insight>> {
        if project_id.trim().is_empty() {
            return Err(crate::LievoError::InvalidProjectId(project_id.to_string()));
        }
        let guard = self.lock_storage()?;
        let repos = guard.list_repos(project_id)?;
        let repo_root = repos.first().map(|r| std::path::Path::new(&r.local_path));
        let detector = match repo_root {
            Some(root) => InsightDetector::new(&*guard, project_id).with_repo_root(root),
            None => InsightDetector::new(&*guard, project_id),
        };
        detector.detect()
    }

    // ---- Relationship queries ----

    /// Return outgoing dependencies of the given entity with resolved target entities.
    ///
    /// Returns `EntityNotFound` if the entity does not exist.
    pub fn dependencies_of(&self, entity_id: &str) -> Result<Vec<(Relationship, Entity)>> {
        let guard = self.lock_storage()?;
        let entity = require_entity(&*guard, entity_id)?;
        // Project boundary (issue #764): an outgoing edge can target a
        // foreign entity when multiple projects share one database.
        let project_id = &entity.project_id;
        let mut results = guard.relationships_from(entity_id)?;
        results.retain(|(_, target)| same_project(project_id, &target.project_id));
        results.sort_by(|a, b| a.1.id.cmp(&b.1.id));
        Ok(results)
    }

    /// Return incoming dependents of the given entity with resolved source entities.
    ///
    /// Returns `EntityNotFound` if the entity does not exist.
    pub fn dependents_of(&self, entity_id: &str) -> Result<Vec<(Relationship, Entity)>> {
        let guard = self.lock_storage()?;
        let entity = require_entity(&*guard, entity_id)?;
        // Project boundary (issue #764): an incoming edge can originate from
        // a foreign entity when multiple projects share one database.
        let project_id = &entity.project_id;
        let mut results = guard.relationships_to(entity_id)?;
        results.retain(|(_, source)| same_project(project_id, &source.project_id));
        results.sort_by(|a, b| a.1.id.cmp(&b.1.id));
        Ok(results)
    }

    #[cfg(test)]
    pub(crate) fn storage_for_test(
        &self,
    ) -> std::sync::MutexGuard<'_, crate::storage::sqlite::SqliteStorage> {
        self.storage.lock().unwrap()
    }
}

use crate::query::entity_queries::complexity_of;

/// Verify entity exists and return it; `EntityNotFound` if it does not.
///
/// The caller needs the entity's `project_id` to scope its relationship
/// traversal (issue #764).
fn require_entity(storage: &dyn Storage, entity_id: &str) -> Result<Entity> {
    storage
        .get_entity(entity_id)?
        .ok_or_else(|| crate::LievoError::EntityNotFound(entity_id.to_string()))
}

/// Normalize a file path for entity lookup.
/// Strips a leading `./` prefix so that `"./src/foo.rs"` and `"src/foo.rs"` match.
fn normalize_entity_path(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("./") {
        stripped.to_string()
    } else {
        path.to_string()
    }
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
