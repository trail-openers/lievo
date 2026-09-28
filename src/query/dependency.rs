// Dependency query methods: forward deps, reverse deps, and impact analysis.
use crate::LievoError;
use crate::model::Entity;
use crate::retrieval::project_boundary::same_project;
use crate::storage::Storage;
use std::collections::HashSet;

/// Unresolved-import resolution signal for a queried repo.
///
/// `internal` is `None` when the repo has no recorded count (pre-#681 index);
/// it is `Some(n)` once #681's side-channel counter has been persisted. This
/// split lets callers tell "0 dependents, fully resolved" from "0 dependents
/// but N internal imports never resolved" — the second must not be read as
/// dead code. External (bare-package / non-crate) imports never drive a
/// caveat: they fail resolution by design, so they are reported separately and
/// excluded from `caveat_active`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionSignal {
    /// Count of internal (relative/alias, in-workspace) imports that failed
    /// resolution in this repo. `None` = not recorded (pre-#681).
    pub unresolved_internal: Option<u64>,
    /// Count of external (bare-package, non-crate) imports that failed
    /// resolution. Never drives a caveat. `None` = not recorded.
    pub unresolved_external: Option<u64>,
}

impl ResolutionSignal {
    /// Build a signal from `Storage::get_unresolved_counts` (`None` degrades
    /// to the pre-#681 "unknown" shape — never a false full-coverage claim).
    pub fn from_counts(counts: Option<(u64, u64)>) -> Self {
        match counts {
            Some((internal, external)) => Self {
                unresolved_internal: Some(internal),
                unresolved_external: Some(external),
            },
            None => Self {
                unresolved_internal: None,
                unresolved_external: None,
            },
        }
    }

    /// True when unresolved *internal* imports are recorded and non-zero — the
    /// condition under which "absence of dependents" may reflect failed
    /// resolution rather than genuine dead code. Pre-#681 (`None`) and
    /// external-only counts never fire the caveat.
    pub fn caveat_active(&self) -> bool {
        self.unresolved_internal.map(|n| n > 0).unwrap_or(false)
    }
}

/// Impact report for a set of changed files: lists all transitively affected entities.
///
/// Includes module/subsystem dependents and function-level callers when Function entities exist.
///
/// The repo-scoped unresolved-import signal is NOT part of this struct — it is
/// exposed through the sibling accessor [`impact_resolution_for_repo`] so this
/// type stays a pure entity-listing carrier and the resolution signal can be
/// surfaced independently in response payloads (the #690 amendment keeps it
/// additive; embedding it here would force every existing `ImpactReport`
/// construction site to change).
#[derive(Debug)]
pub struct ImpactReport {
    pub changed_files: Vec<Entity>,
    pub affected_modules: Vec<Entity>,
    pub affected_subsystems: Vec<Entity>,
    pub downstream_dependents: Vec<Entity>,
    pub affected_functions: Vec<Entity>,
}

/// Repo-scoped unresolved-import signal for an impact query: the same shape
/// [`dependents_of_with_resolution`] exposes, but for a bare repo id so the
/// CLI and MCP layers can read it without threading state through
/// [`ImpactReport`].
pub fn impact_resolution_for_repo(storage: &dyn Storage, repo_id: &str) -> ResolutionSignal {
    resolution_signal_for_repo(storage, repo_id)
}

/// Dependency result with the repo-scoped resolution signal attached, so the
/// response can distinguish "0 dependents, fully resolved" from "0 dependents
/// but N internal imports unresolved".
#[derive(Debug)]
pub struct DependencyResult {
    pub relationships: Vec<(crate::model::Relationship, Entity)>,
    pub resolution: ResolutionSignal,
}

/// Read the repo-scoped unresolved-import count from storage and shape it into
/// a [`ResolutionSignal`] for the given entity's repo. Entities with no
/// `repo_id` (or a repo with no recorded count) degrade to the pre-#681
/// "unknown" shape — never a false full-coverage claim.
fn resolution_signal_for_entity(storage: &dyn Storage, entity: &Entity) -> ResolutionSignal {
    let Some(repo_id) = entity.repo_id.as_deref() else {
        return ResolutionSignal::from_counts(None);
    };
    ResolutionSignal::from_counts(storage.get_unresolved_counts(repo_id))
}

/// Read the repo-scoped resolution signal for a bare repo id.
fn resolution_signal_for_repo(storage: &dyn Storage, repo_id: &str) -> ResolutionSignal {
    ResolutionSignal::from_counts(storage.get_unresolved_counts(repo_id))
}

/// Return all outgoing relationships from `entity_id` with resolved target entities.
///
/// Returns `EntityNotFound` if the entity does not exist.
pub fn dependencies_of(
    storage: &dyn Storage,
    entity_id: &str,
) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
    let entity = require_entity(storage, entity_id)?;
    // Project boundary (issue #764): the relationships table has no
    // project_id column, so an outgoing edge can point at a foreign entity
    // when multiple projects share one database — filter against the
    // entity's own project.
    let project_id = &entity.project_id;
    let mut results = storage.relationships_from(entity_id)?;
    results.retain(|(_, target)| same_project(project_id, &target.project_id));
    results.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    Ok(results)
}

/// Like [`dependencies_of`] but also attaches the entity's repo-scoped
/// resolution signal, so callers can tell a true empty result from an
/// unresolved-import one.
pub fn dependencies_of_with_resolution(
    storage: &dyn Storage,
    entity_id: &str,
) -> crate::Result<DependencyResult> {
    let entity = storage
        .get_entity(entity_id)?
        .ok_or_else(|| LievoError::EntityNotFound(entity_id.to_string()))?;
    let resolution = resolution_signal_for_entity(storage, &entity);
    let relationships = dependencies_of(storage, entity_id)?;
    Ok(DependencyResult {
        relationships,
        resolution,
    })
}

/// Return all incoming relationships to `entity_id` with resolved source entities.
///
/// Returns `EntityNotFound` if the entity does not exist.
pub fn dependents_of(
    storage: &dyn Storage,
    entity_id: &str,
) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
    let entity = require_entity(storage, entity_id)?;
    // Project boundary (issue #764): the relationships table has no
    // project_id column, so an incoming edge can originate from a foreign
    // entity when multiple projects share one database — filter against the
    // entity's own project.
    let project_id = &entity.project_id;
    let mut results = storage.relationships_to(entity_id)?;
    results.retain(|(_, source)| same_project(project_id, &source.project_id));
    results.sort_by(|a, b| a.1.id.cmp(&b.1.id));
    Ok(results)
}

/// Like [`dependents_of`] but also attaches the entity's repo-scoped
/// resolution signal, so callers can tell "0 dependents, fully resolved" from
/// "0 dependents but N internal imports unresolved" (which must not be read
/// as dead code).
pub fn dependents_of_with_resolution(
    storage: &dyn Storage,
    entity_id: &str,
) -> crate::Result<DependencyResult> {
    let entity = storage
        .get_entity(entity_id)?
        .ok_or_else(|| LievoError::EntityNotFound(entity_id.to_string()))?;
    let resolution = resolution_signal_for_entity(storage, &entity);
    let relationships = dependents_of(storage, entity_id)?;
    Ok(DependencyResult {
        relationships,
        resolution,
    })
}

/// Analyse impact of changing `file_paths` within `repo_id`.
///
/// Walks: file → parent module → parent subsystem, then collects reverse
/// dependencies (dependents) of each module and subsystem up to 2 levels deep.
/// Also traces function-level callers when Function entities exist in changed files.
/// All result vecs are deduplicated and sorted by entity ID for determinism.
pub fn impact_analysis(
    storage: &dyn Storage,
    repo_id: &str,
    file_paths: &[&str],
) -> crate::Result<ImpactReport> {
    let mut file_ids: HashSet<String> = HashSet::new();
    let mut module_ids: HashSet<String> = HashSet::new();
    let mut subsystem_ids: HashSet<String> = HashSet::new();
    let mut downstream_ids: HashSet<String> = HashSet::new();
    let mut affected_function_ids: HashSet<String> = HashSet::new();

    // Project boundary (issue #764): the impact walk is scoped to one repo,
    // so the project is derived from the first changed file. The reverse
    // dependency walk (below) must not surface entities from a different
    // project sharing the same database.

    // Resolve file entities and walk up to parent module and subsystem.
    let mut changed_files: Vec<Entity> = Vec::new();
    let mut derived_project_id: Option<String> = None;
    for path in file_paths {
        let entity = storage
            .entity_by_path(repo_id, path)?
            .ok_or_else(|| LievoError::EntityNotFound(path.to_string()))?;
        if derived_project_id.is_none() {
            derived_project_id = Some(entity.project_id.clone());
        }
        if file_ids.insert(entity.id.clone()) {
            changed_files.push(entity.clone());
        }
        if let Some(module_id) = &entity.parent_id
            && module_ids.insert(module_id.clone())
            && let Some(module_entity) = storage.get_entity(module_id)?
            && let Some(subsys_id) = &module_entity.parent_id
        {
            subsystem_ids.insert(subsys_id.clone());
        }
    }

    // Level 1: dependents of affected modules and subsystems.
    // `project_id` is the project of the first changed file; it is always
    // `Some` for real entities but `None` is tolerated (no project filter).
    let is_same_project = |dep_project: &str| match derived_project_id.as_deref() {
        Some(p) => same_project(p, dep_project),
        None => true,
    };
    let mut level1_ids: HashSet<String> = HashSet::new();
    for id in module_ids.iter().chain(subsystem_ids.iter()) {
        for (_, dep) in storage.relationships_to(id)? {
            if !is_same_project(&dep.project_id)
                || module_ids.contains(&dep.id)
                || subsystem_ids.contains(&dep.id)
                || file_ids.contains(&dep.id)
            {
                continue;
            }
            level1_ids.insert(dep.id.clone());
            downstream_ids.insert(dep.id);
        }
    }

    // Level 2: dependents of level-1 dependents.
    for l1_id in &level1_ids {
        for (_, dep) in storage.relationships_to(l1_id)? {
            if !is_same_project(&dep.project_id)
                || module_ids.contains(&dep.id)
                || subsystem_ids.contains(&dep.id)
                || file_ids.contains(&dep.id)
                || level1_ids.contains(&dep.id)
            {
                continue;
            }
            downstream_ids.insert(dep.id);
        }
    }

    // Function-level tracing: find all functions in changed files, then find callers.
    for file_id in &file_ids {
        let functions = storage.entities_by_parent(file_id)?;
        for func in functions {
            // Find callers of this function via reverse Calls relationships.
            for (rel, caller) in storage.relationships_to(&func.id)? {
                if rel.rel_type == crate::model::RelType::Calls
                    && caller.tier == crate::model::EntityTier::Function
                {
                    // Project boundary (issue #764): a cross-project caller
                    // is not part of this repo's impact set.
                    if is_same_project(&caller.project_id) {
                        affected_function_ids.insert(caller.id.clone());
                    }
                }
            }
        }
    }

    let affected_modules = resolve_sorted(storage, &module_ids)?;
    let affected_subsystems = resolve_sorted(storage, &subsystem_ids)?;
    let downstream_dependents = resolve_sorted(storage, &downstream_ids)?;
    let affected_functions = resolve_sorted(storage, &affected_function_ids)?;
    changed_files.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(ImpactReport {
        changed_files,
        affected_modules,
        affected_subsystems,
        downstream_dependents,
        affected_functions,
    })
}

/// Verify entity exists and return it; `EntityNotFound` if it does not.
///
/// The caller needs the entity's `project_id` to scope its relationship
/// traversal (issue #764).
fn require_entity(storage: &dyn Storage, entity_id: &str) -> crate::Result<Entity> {
    storage
        .get_entity(entity_id)?
        .ok_or_else(|| LievoError::EntityNotFound(entity_id.to_string()))
}

/// Resolve a set of entity IDs into a sorted `Vec<Entity>`, skipping missing ones.
///
/// Entities that no longer exist (deleted between relationship creation and query)
/// are silently skipped. Storage errors are propagated.
fn resolve_sorted(storage: &dyn Storage, ids: &HashSet<String>) -> crate::Result<Vec<Entity>> {
    let mut sorted_ids: Vec<_> = ids.iter().collect();
    sorted_ids.sort();
    let mut entities = Vec::new();
    for id in sorted_ids {
        if let Some(entity) = storage.get_entity(id)? {
            entities.push(entity);
        }
        // None means entity was deleted between relationship creation and query — skip silently
    }
    Ok(entities)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
    use crate::storage::sqlite::SqliteStorage;

    /// Two-project fixture for cross-project boundary tests.
    fn two_project_storage() -> (SqliteStorage, String, String) {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let pa = storage.create_project("dp-a", None).unwrap();
        let pb = storage.create_project("dp-b", None).unwrap();
        let _repo_a = storage.add_repo(&pa.id, "repoA", "/tmp/repoA").unwrap();
        let _repo_b = storage.add_repo(&pb.id, "repoB", "/tmp/repoB").unwrap();
        (storage, pa.id, pb.id)
    }

    fn make_entity(id: &str, project_id: &str, repo_id: Option<&str>, name: &str) -> Entity {
        Entity {
            id: id.to_string(),
            project_id: project_id.to_string(),
            repo_id: repo_id.map(|s| s.to_string()),
            tier: EntityTier::Module,
            parent_id: None,
            name: name.to_string(),
            path: Some(name.to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn dependencies_of_does_not_leak_across_project_boundaries() {
        let (storage, pa_id, pb_id) = two_project_storage();
        let repo_a = storage
            .list_repos(&pa_id)
            .unwrap()
            .first()
            .unwrap()
            .id
            .clone();
        let repo_b = storage
            .list_repos(&pb_id)
            .unwrap()
            .first()
            .unwrap()
            .id
            .clone();

        let mod_a = make_entity("pa:mod", &pa_id, Some(&repo_a), "mod_a");
        let foreign = make_entity("pb:file", &pb_id, Some(&repo_b), "foreign");
        for e in [&mod_a, &foreign] {
            storage.upsert_entity(e).unwrap();
        }
        // Cross-project edge: foreign (projB) → mod_a (projA).
        storage
            .upsert_relationship(&Relationship {
                source_id: "pb:file".into(),
                target_id: "pa:mod".into(),
                rel_type: RelType::DependsOn,
                weight: 1.0,
                evidence_json: None,
                provenance: EdgeProvenance::Heuristic,
            })
            .unwrap();

        // mod_a's dependents: the cross-project foreign file must not appear.
        let dependents = dependents_of(&storage, "pa:mod").unwrap();
        let leaked = dependents.iter().any(|(_, e)| e.id == "pb:file");
        assert!(
            !leaked,
            "cross-project entity must not appear in dependents_of: {dependents:?}"
        );

        // foreign's dependencies: the cross-project mod_a must not appear.
        let deps = dependencies_of(&storage, "pb:file").unwrap();
        let leaked2 = deps.iter().any(|(_, e)| e.id == "pa:mod");
        assert!(
            !leaked2,
            "cross-project entity must not appear in dependencies_of: {deps:?}"
        );
    }

    #[test]
    fn impact_analysis_does_not_leak_across_project_boundaries() {
        let (storage, pa_id, pb_id) = two_project_storage();
        let repo_a = storage
            .list_repos(&pa_id)
            .unwrap()
            .first()
            .unwrap()
            .id
            .clone();
        let repo_b = storage
            .list_repos(&pb_id)
            .unwrap()
            .first()
            .unwrap()
            .id
            .clone();

        // Project A: module + file (the file is the "changed" file).
        let mod_a = make_entity("pa:mod", &pa_id, Some(&repo_a), "mod_a");
        let file_a = Entity {
            id: "pa:file:main.rs".into(),
            project_id: pa_id.clone(),
            repo_id: Some(repo_a.clone()),
            tier: EntityTier::File,
            parent_id: Some("pa:mod".into()),
            name: "main.rs".into(),
            path: Some("src/main.rs".into()),
            language: Some("rust".into()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        };
        // Project B: a file with a cross-project DependsOn edge to mod_a.
        let foreign = make_entity("pb:file:foreign.rs", &pb_id, Some(&repo_b), "foreign");
        for e in [&mod_a, &file_a, &foreign] {
            storage.upsert_entity(e).unwrap();
        }
        // Cross-project edge: foreign (projB) → mod_a (projA).
        storage
            .upsert_relationship(&Relationship {
                source_id: "pb:file:foreign.rs".into(),
                target_id: "pa:mod".into(),
                rel_type: RelType::DependsOn,
                weight: 1.0,
                evidence_json: None,
                provenance: EdgeProvenance::Heuristic,
            })
            .unwrap();

        let report = impact_analysis(&storage, &repo_a, &["src/main.rs"]).unwrap();
        let leaked = report
            .downstream_dependents
            .iter()
            .any(|e| e.id == "pb:file:foreign.rs");
        assert!(
            !leaked,
            "cross-project entity must not appear in impact_analysis downstream: {report:?}"
        );
    }
}
