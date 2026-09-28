use super::*;
use crate::model::{EntityTier, Insight};
use std::path::Path;
use tempfile::TempDir;

fn open_temp() -> (Lievo, TempDir) {
    let dir = TempDir::new().unwrap();
    let lievo = Lievo::open_at(dir.path().join("test.db").as_path()).unwrap();
    (lievo, dir)
}

/// Compile-time assertion: Lievo must implement Send + Sync.
#[test]
fn test_lievo_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Lievo>();
}

#[test]
fn test_open_at_create_and_list_projects() {
    let (lievo, _dir) = open_temp();
    lievo.create_project("alpha").unwrap();
    lievo.create_project("beta").unwrap();
    let projects = lievo.list_projects().unwrap();
    assert_eq!(projects.len(), 2);
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
}

#[test]
fn test_project_lookup_by_name() {
    let (lievo, _dir) = open_temp();
    lievo.create_project("my-project").unwrap();
    let found = lievo.project("my-project").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "my-project");
}

#[test]
fn test_project_lookup_missing_returns_none() {
    let (lievo, _dir) = open_temp();
    let found = lievo.project("does-not-exist").unwrap();
    assert!(found.is_none());
}

#[test]
fn test_add_repo_and_list_repos() {
    let (lievo, _dir) = open_temp();
    let project = lievo.create_project("proj").unwrap();
    lievo.add_repo(&project.id, "repo1", "/local/path").unwrap();
    let repos = lievo.list_repos(&project.id).unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].name, "repo1");
    assert_eq!(repos[0].local_path, "/local/path");
    assert_eq!(repos[0].project_id, project.id);
}

#[test]
fn test_query_methods_return_empty_on_fresh_db() {
    let (lievo, _dir) = open_temp();
    let project = lievo.create_project("fresh").unwrap();
    assert!(lievo.subsystems(&project.id).unwrap().is_empty());
    assert!(lievo.list_repos(&project.id).unwrap().is_empty());
    assert!(lievo.entity("nonexistent").unwrap().is_none());
    assert!(lievo.hotspots(&project.id, 10).unwrap().is_empty());
}

#[test]
fn test_modules_in_empty_subsystem() {
    let (lievo, _dir) = open_temp();
    let project = lievo.create_project("proj").unwrap();
    let repo = lievo.add_repo(&project.id, "r", "/p").unwrap();
    insert_entity(
        &lievo,
        "subsys-1",
        &project.id,
        &repo.id,
        EntityTier::Subsystem,
        None,
    );
    assert!(lievo.modules_in("subsys-1").unwrap().is_empty());
}

#[test]
fn test_files_in_empty_module() {
    let (lievo, _dir) = open_temp();
    let project = lievo.create_project("proj").unwrap();
    let repo = lievo.add_repo(&project.id, "r", "/p").unwrap();
    insert_entity(
        &lievo,
        "module-1",
        &project.id,
        &repo.id,
        EntityTier::Module,
        None,
    );
    assert!(lievo.files_in("module-1").unwrap().is_empty());
}

#[test]
fn test_dependency_methods_return_error_for_missing_entity() {
    let (lievo, _dir) = open_temp();
    assert!(lievo.dependencies_of("nonexistent").is_err());
    assert!(lievo.dependents_of("nonexistent").is_err());
}

#[test]
fn test_hotspots_sorted_by_complexity() {
    let (lievo, _dir) = open_temp();
    let project = lievo.create_project("proj").unwrap();
    let repo = lievo.add_repo(&project.id, "r", "/p").unwrap();
    insert_entity_with_complexity(&lievo, "e1", &project.id, &repo.id, 5.0);
    insert_entity_with_complexity(&lievo, "e2", &project.id, &repo.id, 10.0);
    let hotspots = lievo.hotspots(&project.id, 2).unwrap();
    assert_eq!(hotspots[0].id, "e2");
    assert_eq!(hotspots[1].id, "e1");
}

#[test]
fn test_open_at_empty_path_returns_error() {
    let result = Lievo::open_at(Path::new(""));
    assert!(matches!(
        result,
        Err(crate::LievoError::InvalidProjectId(_))
    ));
}

#[test]
fn test_add_repo_empty_project_id_returns_error() {
    let (lievo, _dir) = open_temp();
    let result = lievo.add_repo("", "repo", "/path");
    assert!(matches!(
        result,
        Err(crate::LievoError::InvalidProjectId(_))
    ));
}

#[test]
fn test_add_repo_whitespace_project_id_returns_error() {
    let (lievo, _dir) = open_temp();
    let result = lievo.add_repo("   ", "repo", "/path");
    assert!(matches!(
        result,
        Err(crate::LievoError::InvalidProjectId(_))
    ));
}

#[test]
fn test_list_repos_empty_project_id_returns_error() {
    let (lievo, _dir) = open_temp();
    let result = lievo.list_repos("");
    assert!(matches!(
        result,
        Err(crate::LievoError::InvalidProjectId(_))
    ));
}

#[test]
fn test_subsystems_empty_project_id_returns_error() {
    let (lievo, _dir) = open_temp();
    let result = lievo.subsystems("");
    assert!(matches!(
        result,
        Err(crate::LievoError::InvalidProjectId(_))
    ));
}

#[test]
fn test_hotspots_empty_project_id_returns_error() {
    let (lievo, _dir) = open_temp();
    let result = lievo.hotspots("", 10);
    assert!(matches!(
        result,
        Err(crate::LievoError::InvalidProjectId(_))
    ));
}

#[test]
fn test_link_repo_moves_repo_to_new_project() {
    let (lievo, _dir) = open_temp();
    let proj_a = lievo.create_project("proj-a").unwrap();
    let proj_b = lievo.create_project("proj-b").unwrap();
    let repo = lievo.add_repo(&proj_a.id, "my-repo", "/tmp/r").unwrap();
    // Insert an entity belonging to the repo (and proj_a)
    insert_entity(
        &lievo,
        "entity-1",
        &proj_a.id,
        &repo.id,
        EntityTier::Subsystem,
        None,
    );
    lievo.link_repo(&proj_b.id, &repo.id).unwrap();
    // Repo moved to proj_b
    assert!(lievo.list_repos(&proj_a.id).unwrap().is_empty());
    let repos_b = lievo.list_repos(&proj_b.id).unwrap();
    assert_eq!(repos_b.len(), 1);
    assert_eq!(repos_b[0].id, repo.id);
    // Entities are deleted (not moved) when repo changes projects.
    // Entity IDs are deterministically baked as {project_id}:{repo_name}:{tier}:{path},
    // so stale entities must be deleted and regenerated on refresh.
    assert!(lievo.subsystems(&proj_a.id).unwrap().is_empty());
    assert!(lievo.subsystems(&proj_b.id).unwrap().is_empty());
}

#[test]
fn test_link_repo_empty_project_id_returns_error() {
    let (lievo, _dir) = open_temp();
    let result = lievo.link_repo("", "some-repo-id");
    assert!(matches!(
        result,
        Err(crate::LievoError::InvalidProjectId(_))
    ));
}

#[test]
fn test_link_repo_nonexistent_repo_returns_error() {
    let (lievo, _dir) = open_temp();
    let proj = lievo.create_project("proj").unwrap();
    let result = lievo.link_repo(&proj.id, "nonexistent-repo");
    assert!(matches!(result, Err(crate::LievoError::RepoNotFound(_))));
}

#[test]
fn test_link_repo_empty_repo_id_returns_error() {
    let (lievo, _dir) = open_temp();
    let proj = lievo.create_project("proj").unwrap();
    let result = lievo.link_repo(&proj.id, "");
    assert!(matches!(result, Err(crate::LievoError::InvalidInput(_))));
}

#[test]
fn test_link_repo_nonexistent_project_returns_error() {
    let (lievo, _dir) = open_temp();
    let proj = lievo.create_project("proj").unwrap();
    let repo = lievo.add_repo(&proj.id, "r", "/p").unwrap();
    let result = lievo.link_repo("nonexistent-project-id", &repo.id);
    assert!(matches!(result, Err(crate::LievoError::ProjectNotFound(_))));
}

#[test]
fn test_entity_by_path() {
    let (lievo, _dir) = open_temp();
    let project = lievo.create_project("proj").unwrap();
    let repo = lievo.add_repo(&project.id, "r", "/p").unwrap();
    let now = "2024-01-01T00:00:00Z".to_string();
    let entity = Entity {
        id: "file-x".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "file-x".to_string(),
        path: Some("src/foo.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.clone(),
        updated_at: now,
    };
    lievo.storage_for_test().upsert_entity(&entity).unwrap();
    let found = lievo.entity_by_path(&repo.id, "src/foo.rs").unwrap();
    assert_eq!(found.unwrap().id, "file-x");
    let miss = lievo.entity_by_path(&repo.id, "nonexistent.rs").unwrap();
    assert!(miss.is_none());
}

#[test]
fn test_entity_by_path_normalizes_dotslash_prefix() {
    let (lievo, _dir) = open_temp();
    let project = lievo.create_project("proj").unwrap();
    let repo = lievo.add_repo(&project.id, "r", "/p").unwrap();
    let now = "2024-01-01T00:00:00Z".to_string();
    let entity = Entity {
        id: "file-y".to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "file-y".to_string(),
        path: Some("src/bar.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.clone(),
        updated_at: now,
    };
    lievo.storage_for_test().upsert_entity(&entity).unwrap();
    // Lookup with `./` prefix should find the entity stored without it
    let found = lievo.entity_by_path(&repo.id, "./src/bar.rs").unwrap();
    assert_eq!(found.unwrap().id, "file-y");
}

#[test]
fn test_entity_by_path_empty_repo_id_returns_error() {
    let (lievo, _dir) = open_temp();
    let result = lievo.entity_by_path("", "src/foo.rs");
    assert!(matches!(result, Err(crate::LievoError::InvalidInput(_))));
}

#[test]
fn test_entity_by_path_empty_path_returns_error() {
    let (lievo, _dir) = open_temp();
    let result = lievo.entity_by_path("some-repo-id", "");
    assert!(matches!(result, Err(crate::LievoError::InvalidInput(_))));
}

#[test]
fn test_insights_empty_project_id_returns_error() {
    let (lievo, _dir) = open_temp();
    let result = lievo.insights("");
    assert!(matches!(
        result,
        Err(crate::LievoError::InvalidProjectId(_))
    ));
}

#[test]
fn test_insights_fresh_project_returns_empty() {
    let (lievo, _dir) = open_temp();
    let project = lievo.create_project("fresh").unwrap();
    let insights: Vec<Insight> = lievo.insights(&project.id).unwrap();
    assert!(insights.is_empty());
}

fn insert_entity(
    lievo: &Lievo,
    id: &str,
    project_id: &str,
    repo_id: &str,
    tier: EntityTier,
    parent_id: Option<&str>,
) {
    let now = "2024-01-01T00:00:00Z".to_string();
    let entity = Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier,
        parent_id: parent_id.map(|s| s.to_string()),
        name: id.to_string(),
        path: None,
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.clone(),
        updated_at: now,
    };
    lievo.storage_for_test().upsert_entity(&entity).unwrap();
}

fn insert_entity_with_complexity(
    lievo: &Lievo,
    id: &str,
    project_id: &str,
    repo_id: &str,
    complexity: f64,
) {
    let now = "2024-01-01T00:00:00Z".to_string();
    let entity = Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: id.to_string(),
        path: Some(format!("{id}.rs")),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: Some(format!(r#"{{"complexity_max": {complexity}}}"#)),
        created_at: now.clone(),
        updated_at: now,
    };
    lievo.storage_for_test().upsert_entity(&entity).unwrap();
}
