use super::*;
use crate::model::EdgeProvenance;

#[test]
fn test_open_in_memory() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.create_project("test", None).unwrap();
    let projects = storage.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "test");
}

#[test]
fn test_project_crud() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage
        .create_project("test", Some("test description"))
        .unwrap();
    let project = storage.get_project("test").unwrap().unwrap();
    assert_eq!(project.name, "test");
    assert_eq!(project.description.as_deref(), Some("test description"));
}

/// Regression test for issue #493: calling create_project with the same name
/// twice must return the EXISTING project with the same ID, not generate a new one.
#[test]
fn test_create_project_idempotent_returns_existing_id() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let first = storage.create_project("myproj", None).unwrap();
    assert_eq!(first.name, "myproj");

    // Calling create_project again with the same name must return the SAME project
    let second = storage.create_project("myproj", None).unwrap();
    assert_eq!(
        second.id, first.id,
        "second call with same name must return existing project with same ID"
    );
    assert_eq!(second.name, "myproj");

    // Verify only one project exists in the database
    let all = storage.list_projects().unwrap();
    assert_eq!(all.len(), 1, "only one project should exist");
    assert_eq!(all[0].id, first.id);
}

#[test]
fn test_get_project_by_id() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("myproj", None).unwrap();
    let found = storage.get_project_by_id(&project.id).unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "myproj");
    let missing = storage.get_project_by_id("nonexistent-id").unwrap();
    assert!(missing.is_none());
}

#[test]
fn test_repo_crud() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/path/to/repo1")
        .unwrap();
    let repos = storage.list_repos(&project.id).unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].name, "repo1");
}

/// `count_entities` counts every stored entity of a repository via a COUNT
/// query (issue #875), ignoring other repositories' entities.
#[test]
fn test_count_entities_counts_only_the_given_repo() {
    use crate::model::{Entity, EntityTier};

    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();
    let repo_a = storage
        .add_repo(&project.id, "repo-a", "/path/to/repo-a")
        .unwrap();
    let repo_b = storage
        .add_repo(&project.id, "repo-b", "/path/to/repo-b")
        .unwrap();

    let entity = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo_a.id.clone()),
        tier: EntityTier::Function,
        parent_id: None,
        name: "alpha".to_string(),
        path: Some("a.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&entity).unwrap();

    let other = Entity {
        repo_id: Some(repo_b.id.clone()),
        path: Some("b.rs".to_string()),
        name: "beta".to_string(),
        id: "e2".to_string(),
        ..entity.clone()
    };
    storage.upsert_entity(&other).unwrap();

    assert_eq!(storage.count_entities(&repo_a.id).unwrap(), 1);
    assert_eq!(storage.count_entities(&repo_b.id).unwrap(), 1);
    assert_eq!(storage.count_entities("no-such-repo").unwrap(), 0);
}

/// Upsert must preserve the original created_at when the same entity id is
/// inserted a second time with a different timestamp.
#[test]
fn test_upsert_entity_preserves_created_at() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();

    let original_ts = "2020-01-01T00:00:00Z".to_string();
    let entity = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: original_ts.clone(),
        updated_at: original_ts.clone(),
    };
    storage.upsert_entity(&entity).unwrap();

    // Upsert again with a newer timestamp and changed name
    let later_ts = "2025-06-01T00:00:00Z".to_string();
    let updated = Entity {
        name: "main_renamed.rs".to_string(),
        created_at: later_ts.clone(),
        updated_at: later_ts.clone(),
        ..entity
    };
    storage.upsert_entity(&updated).unwrap();

    let fetched = storage.get_entity("e1").unwrap().unwrap();
    assert_eq!(
        fetched.created_at, original_ts,
        "created_at must not change on upsert"
    );
    assert_eq!(fetched.name, "main_renamed.rs", "name must be updated");
    assert_ne!(
        fetched.updated_at, original_ts,
        "updated_at must change on upsert"
    );
}

/// A structural re-persist (incoming summary = None) must NOT wipe a previously
/// stored summary. Issue #674: UPSERT_ENTITY used to unconditionally overwrite
/// summary/summary_commit with the incoming (NULL) values, so every structural
/// analysis pass destroyed previously-written summaries. This test fails on main.
#[test]
fn test_upsert_entity_null_incoming_summary_preserves_stored_summary() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();

    let stored = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: None,
        name: "my_fn".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: Some("apfel text".to_string()),
        summary_commit: Some("commit-abc".to_string()),
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&stored).unwrap();

    // Structural re-persist: same id, a new name/metrics but NO summary to offer.
    let re_persist = Entity {
        name: "my_fn_renamed".to_string(),
        metrics_json: Some("{\"lines\":42}".to_string()),
        summary: None,
        summary_commit: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2025-06-01T00:00:00Z".to_string(),
        ..stored
    };
    storage.upsert_entity(&re_persist).unwrap();

    let fetched = storage.get_entity("e1").unwrap().unwrap();
    // The stored summary and its commit marker must survive the NULL incoming row.
    assert_eq!(
        fetched.summary,
        Some("apfel text".to_string()),
        "stored summary must survive a structural re-persist that offers no summary"
    );
    assert_eq!(
        fetched.summary_commit,
        Some("commit-abc".to_string()),
        "summary_commit must survive a structural re-persist that offers no summary"
    );
    // Non-summary columns DO update unconditionally.
    assert_eq!(
        fetched.name, "my_fn_renamed",
        "name must update on re-persist"
    );
    assert_eq!(
        fetched.metrics_json,
        Some("{\"lines\":42}".to_string()),
        "metrics_json must update on re-persist"
    );
}

/// Refill still wins: upserting an entity WITH a real (non-NULL) summary must
/// overwrite a previously stored summary. Guarantees the COALESCE fix does not
/// prevent legitimate re-summarization from taking effect.
#[test]
fn test_upsert_entity_some_incoming_summary_overwrites_stored() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();

    let stored = Entity {
        id: "e1".to_string(),
        project_id: project.id.clone(),
        repo_id: None,
        tier: EntityTier::Function,
        parent_id: None,
        name: "my_fn".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: Some("old summary".to_string()),
        summary_commit: Some("old-commit".to_string()),
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&stored).unwrap();

    let refill = Entity {
        summary: Some("new apfel summary".to_string()),
        summary_commit: Some("new-commit".to_string()),
        ..stored
    };
    storage.upsert_entity(&refill).unwrap();

    let fetched = storage.get_entity("e1").unwrap().unwrap();
    assert_eq!(
        fetched.summary,
        Some("new apfel summary".to_string()),
        "a real incoming summary must overwrite the stored one"
    );
    assert_eq!(
        fetched.summary_commit,
        Some("new-commit".to_string()),
        "a real incoming summary_commit must overwrite the stored one"
    );
}

/// A fresh insert (id never seen before) must take the incoming summary as-is;
/// a NULL summary on a fresh insert stays NULL and is therefore counted as missing.
#[test]
fn test_upsert_entity_fresh_insert_null_summary_stays_null() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();
    let repo = storage.add_repo(&project.id, "repo", "/tmp/repo").unwrap();

    let fresh = Entity {
        id: "fn-fresh".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::Function,
        parent_id: None,
        name: "fresh_fn".to_string(),
        path: Some("src/fresh.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&fresh).unwrap();

    let fetched = storage.get_entity("fn-fresh").unwrap().unwrap();
    assert_eq!(fetched.summary, None, "fresh NULL summary stays NULL");
    // A NULL-summary function tier entity counts as missing.
    assert_eq!(
        storage.count_missing_summaries(&repo.id).unwrap(),
        1,
        "a fresh NULL-summary function must be counted as missing"
    );
}

/// Upsert must preserve the original detected_at for insights.
#[test]
fn test_upsert_insight_preserves_detected_at() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();

    let original_ts = "2020-01-01T00:00:00Z".to_string();
    let insight = Insight {
        id: "ins1".to_string(),
        project_id: project.id.clone(),
        category: "hotspot".to_string(),
        severity: Some("high".to_string()),
        title: "Original title".to_string(),
        description: None,
        entity_ids_json: None,
        detected_at: original_ts.clone(),
        still_valid: true,
    };
    storage.upsert_insight(&insight).unwrap();

    // Upsert again with a different detected_at and updated title
    let later_ts = "2025-06-01T00:00:00Z".to_string();
    let updated = Insight {
        title: "Updated title".to_string(),
        detected_at: later_ts.clone(),
        ..insight
    };
    storage.upsert_insight(&updated).unwrap();

    let fetched = storage
        .list_insights(&project.id, None, None, 10)
        .unwrap()
        .into_iter()
        .find(|i| i.id == "ins1")
        .unwrap();
    assert_eq!(
        fetched.detected_at, original_ts,
        "detected_at must not change on upsert"
    );
    assert_eq!(fetched.title, "Updated title", "title must be updated");
}

/// Upsert must preserve the original detected_at for conventions.
#[test]
fn test_upsert_convention_preserves_detected_at() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("proj", None).unwrap();

    let original_ts = "2020-01-01T00:00:00Z".to_string();
    let convention = Convention {
        id: "conv1".to_string(),
        project_id: project.id.clone(),
        category: "naming".to_string(),
        title: "Original convention".to_string(),
        description: None,
        example_code: None,
        confidence: 0.8,
        entity_ids_json: None,
        detected_at: original_ts.clone(),
        still_valid: true,
    };
    storage.upsert_convention(&convention).unwrap();

    // Upsert again with a different detected_at and updated title
    let later_ts = "2025-06-01T00:00:00Z".to_string();
    let updated = Convention {
        title: "Updated convention".to_string(),
        detected_at: later_ts.clone(),
        ..convention
    };
    storage.upsert_convention(&updated).unwrap();

    let fetched = storage
        .list_conventions(&project.id, None)
        .unwrap()
        .into_iter()
        .find(|c| c.id == "conv1")
        .unwrap();
    assert_eq!(
        fetched.detected_at, original_ts,
        "detected_at must not change on upsert"
    );
    assert_eq!(fetched.title, "Updated convention", "title must be updated");
}

/// Multi-repo safety test: clearing summaries for repo A must not affect repo B.
/// Regression test for issue #485.
#[test]
fn test_clear_repo_summaries_does_not_affect_other_repos() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("test", None).unwrap();

    // Create two repos
    let repo_a = storage
        .add_repo(&project.id, "repo-a", "/path/to/repo-a")
        .unwrap();
    let repo_b = storage
        .add_repo(&project.id, "repo-b", "/path/to/repo-b")
        .unwrap();

    // Add entities to both repos with summaries
    let entity_a = Entity {
        id: "entity-a".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo_a.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "file-a.rs".to_string(),
        path: Some("src/file-a.rs".to_string()),
        language: Some("rust".to_string()),
        summary: Some("Summary for repo A".to_string()),
        summary_commit: Some("commit-a".to_string()),
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };

    let entity_b = Entity {
        id: "entity-b".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo_b.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "file-b.rs".to_string(),
        path: Some("src/file-b.rs".to_string()),
        language: Some("rust".to_string()),
        summary: Some("Summary for repo B".to_string()),
        summary_commit: Some("commit-b".to_string()),
        metrics_json: None,
        created_at: "2020-01-01T00:00:00Z".to_string(),
        updated_at: "2020-01-01T00:00:00Z".to_string(),
    };

    storage.upsert_entity(&entity_a).unwrap();
    storage.upsert_entity(&entity_b).unwrap();

    // Verify both summaries exist
    let fetched_a = storage.get_entity("entity-a").unwrap().unwrap();
    let fetched_b = storage.get_entity("entity-b").unwrap().unwrap();
    assert_eq!(fetched_a.summary, Some("Summary for repo A".to_string()));
    assert_eq!(fetched_b.summary, Some("Summary for repo B".to_string()));

    // Clear summaries only for repo A
    storage.clear_repo_summaries(&repo_a.id).unwrap();

    // Verify repo A summaries are cleared but repo B summaries remain
    let fetched_a_after = storage.get_entity("entity-a").unwrap().unwrap();
    let fetched_b_after = storage.get_entity("entity-b").unwrap().unwrap();

    assert_eq!(
        fetched_a_after.summary, None,
        "repo A summaries must be cleared"
    );
    assert_eq!(
        fetched_a_after.summary_commit, None,
        "repo A summary_commit must be cleared"
    );
    assert_eq!(
        fetched_b_after.summary,
        Some("Summary for repo B".to_string()),
        "repo B summaries must NOT be affected"
    );
    assert_eq!(
        fetched_b_after.summary_commit,
        Some("commit-b".to_string()),
        "repo B summary_commit must NOT be affected"
    );
}

/// Regression test for issue #640: link-repo followed by refresh must succeed
/// without UNIQUE constraint errors.
///
/// The bug was that moving a repo to a new project via update_repo_project would
/// UPDATE entities.project_id but not regenerate entity IDs (which are deterministically
/// baked as {project_id}:{repo_name}:{tier}:{path}). On refresh, the pipeline would
/// generate new IDs with the new project_id and attempt to upsert, but the old rows
/// with stale IDs would still exist and collide on the secondary unique index.
///
/// The fix is to DELETE all entities when moving a repo, letting refresh re-analyze
/// everything fresh under the new project with correct IDs.
#[test]
fn test_update_repo_project_deletes_entities_and_prevents_unique_constraint_violation() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Create two projects: project A and project B
    let project_a = storage.create_project("project-a", None).unwrap();
    let project_b = storage.create_project("project-b", None).unwrap();

    // Create a repo in project A and add entities
    let repo = storage
        .add_repo(&project_a.id, "test-repo", "/path/to/repo")
        .unwrap();

    let entity_1 = Entity {
        id: format!("{}:test-repo:module:src/main.rs", project_a.id),
        project_id: project_a.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    let entity_2 = Entity {
        id: format!("{}:test-repo:file:src/utils.rs", project_a.id),
        project_id: project_a.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "utils.rs".to_string(),
        path: Some("src/utils.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-01T00:00:00Z".to_string(),
        updated_at: "2024-01-01T00:00:00Z".to_string(),
    };

    storage.upsert_entity(&entity_1).unwrap();
    storage.upsert_entity(&entity_2).unwrap();

    // Add a relationship
    let rel = Relationship {
        source_id: entity_1.id.clone(),
        target_id: entity_2.id.clone(),
        rel_type: crate::model::RelType::Contains,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    storage.upsert_relationship(&rel).unwrap();

    // Verify entities and relationships exist in project A
    let entities_a = storage.list_entities(&project_a.id, None).unwrap();
    assert_eq!(entities_a.len(), 2, "should have 2 entities in project A");

    let relationships_from_e1 = storage.relationships_from(&entity_1.id).unwrap();
    assert_eq!(relationships_from_e1.len(), 1, "should have 1 relationship");

    // Move the repo to project B (simulating link-repo)
    storage
        .update_repo_project(&repo.id, &project_b.id)
        .unwrap();

    // Verify entities and relationships were deleted (not updated)
    let entities_a_after = storage.list_entities(&project_a.id, None).unwrap();
    assert_eq!(
        entities_a_after.len(),
        0,
        "all entities must be deleted from project A after move"
    );

    // Verify repo is now in project B
    let repo_after = storage.get_repo(&repo.id).unwrap().unwrap();
    assert_eq!(
        repo_after.project_id, project_b.id,
        "repo must be in project B after update"
    );

    // Verify relationships were deleted (cascade via FK)
    let relationships_from_e1_after = storage.relationships_from(&entity_1.id).unwrap();
    assert_eq!(
        relationships_from_e1_after.len(),
        0,
        "relationships must be deleted"
    );

    // Simulate refresh: insert entities with new IDs under project B
    let new_entity_1 = Entity {
        id: format!("{}:test-repo:module:src/main.rs", project_b.id),
        project_id: project_b.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::Module,
        parent_id: None,
        name: "main.rs".to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-02T00:00:00Z".to_string(),
        updated_at: "2024-01-02T00:00:00Z".to_string(),
    };

    let new_entity_2 = Entity {
        id: format!("{}:test-repo:file:src/utils.rs", project_b.id),
        project_id: project_b.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "utils.rs".to_string(),
        path: Some("src/utils.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2024-01-02T00:00:00Z".to_string(),
        updated_at: "2024-01-02T00:00:00Z".to_string(),
    };

    // This should NOT fail with UNIQUE constraint violation
    storage.upsert_entity(&new_entity_1).unwrap();
    storage.upsert_entity(&new_entity_2).unwrap();

    // Verify entities exist under project B
    let entities_b = storage.list_entities(&project_b.id, None).unwrap();
    assert_eq!(entities_b.len(), 2, "should have 2 entities in project B");

    // Verify new relationship can be created
    let new_rel = Relationship {
        source_id: new_entity_1.id.clone(),
        target_id: new_entity_2.id.clone(),
        rel_type: crate::model::RelType::Contains,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    storage
        .upsert_relationship(&new_rel)
        .expect("new relationship must succeed");

    let relationships_from_new_e1 = storage.relationships_from(&new_entity_1.id).unwrap();
    assert_eq!(
        relationships_from_new_e1.len(),
        1,
        "new relationship must exist"
    );
}
