// Integration tests for CLI project management commands.
// These test the command handler logic via the library APIs (SqliteStorage),
// since we cannot easily invoke the binary with stdin/stdout in integration tests.
// Behavioral coverage: create-project, add-repo, link-repo, list-projects, list-repos, status, info.

mod bin_common;

use lievo::model::{Entity, EntityTier};
use lievo::output::OutputFormat;
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn in_memory_storage() -> SqliteStorage {
    SqliteStorage::open_in_memory().expect("in-memory storage must open")
}

fn temp_git_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let repo_path = dir.path();
    git2::Repository::init(repo_path).expect("git init");
    dir
}

// ---------------------------------------------------------------------------
// create-project
// ---------------------------------------------------------------------------

#[test]
fn test_create_project_returns_project_with_name() {
    let storage = in_memory_storage();
    let project = storage.create_project("my-project", None).unwrap();
    assert_eq!(project.name, "my-project");
    assert!(!project.id.is_empty());
    assert!(!project.created_at.is_empty());
}

#[test]
fn test_create_project_with_description() {
    let storage = in_memory_storage();
    let project = storage
        .create_project("proj", Some("A test project"))
        .unwrap();
    assert_eq!(project.description.as_deref(), Some("A test project"));
}

#[test]
fn test_create_project_idempotent_returns_existing() {
    let storage = in_memory_storage();
    let first = storage.create_project("dup", None).unwrap();
    // Calling create_project again with the same name returns the EXISTING project (not an error).
    // This ensures project_id remains stable across refreshes (issue #493).
    let second = storage.create_project("dup", None).unwrap();
    assert_eq!(
        second.id, first.id,
        "second call with same name must return existing project with same ID"
    );
    assert_eq!(second.name, "dup");
    // Verify only one project exists
    let all = storage.list_projects().unwrap();
    assert_eq!(all.len(), 1, "only one project should exist");
    assert_eq!(all[0].id, first.id);
}

// ---------------------------------------------------------------------------
// list-projects
// ---------------------------------------------------------------------------

#[test]
fn test_list_projects_empty() {
    let storage = in_memory_storage();
    let projects = storage.list_projects().unwrap();
    assert!(projects.is_empty());
}

#[test]
fn test_list_projects_returns_all() {
    let storage = in_memory_storage();
    storage.create_project("alpha", None).unwrap();
    storage.create_project("beta", None).unwrap();
    let projects = storage.list_projects().unwrap();
    assert_eq!(projects.len(), 2);
    let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
}

#[test]
fn test_list_projects_includes_created_at() {
    let storage = in_memory_storage();
    storage.create_project("dated", None).unwrap();
    let projects = storage.list_projects().unwrap();
    assert!(!projects[0].created_at.is_empty());
}

// ---------------------------------------------------------------------------
// add-repo (validates path exists and is a git repo)
// ---------------------------------------------------------------------------

#[test]
fn test_add_repo_to_existing_project() {
    let storage = in_memory_storage();
    let project = storage.create_project("proj", None).unwrap();
    let git_dir = temp_git_repo();
    let path = git_dir.path().to_str().unwrap();

    let repo = storage.add_repo(&project.id, "repo1", path).unwrap();
    assert_eq!(repo.name, "repo1");
    assert_eq!(repo.local_path, path);
    assert_eq!(repo.project_id, project.id);
}

#[test]
fn test_add_repo_path_must_exist_as_git_repo() {
    // Validate that a non-git path would be rejected at the CLI layer.
    // We test the git2 check directly here.
    let result = git2::Repository::open("/this/path/does/not/exist/at/all");
    assert!(result.is_err(), "non-existent path must not be a git repo");
}

#[test]
fn test_add_repo_non_git_dir_fails_git2_check() {
    let dir = TempDir::new().unwrap();
    let result = git2::Repository::open(dir.path());
    assert!(
        result.is_err(),
        "directory without .git must fail git2::Repository::open"
    );
}

#[test]
fn test_add_repo_auto_creates_project_from_dirname() {
    // Simulate the auto-create-project logic: derive project name from last path component.
    let storage = in_memory_storage();
    let git_dir = temp_git_repo();
    let path = git_dir.path();

    // The directory name becomes the project name when --project is omitted.
    let dir_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .expect("temp dir has a name");

    // Auto-create project if not found.
    let existing = storage.get_project(dir_name).unwrap();
    let project = match existing {
        Some(p) => p,
        None => storage.create_project(dir_name, None).unwrap(),
    };

    let repo = storage
        .add_repo(&project.id, dir_name, path.to_str().unwrap())
        .unwrap();
    assert_eq!(repo.project_id, project.id);
}

// ---------------------------------------------------------------------------
// link-repo
// ---------------------------------------------------------------------------

#[test]
fn test_link_repo_by_repo_id_is_tracked_via_project() {
    // link-repo associates a repo to a project — since our storage adds repos
    // directly with a project_id, we verify the association is visible via list_repos.
    let storage = in_memory_storage();
    let p1 = storage.create_project("proj1", None).unwrap();
    let git_dir = temp_git_repo();
    let path = git_dir.path().to_str().unwrap();

    let repo = storage.add_repo(&p1.id, "repo-a", path).unwrap();
    let repos = storage.list_repos(&p1.id).unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].id, repo.id);
}

// ---------------------------------------------------------------------------
// list-repos
// ---------------------------------------------------------------------------

#[test]
fn test_list_repos_for_project() {
    let storage = in_memory_storage();
    let project = storage.create_project("proj", None).unwrap();

    let git1 = temp_git_repo();
    let git2 = temp_git_repo();

    storage
        .add_repo(&project.id, "r1", git1.path().to_str().unwrap())
        .unwrap();
    storage
        .add_repo(&project.id, "r2", git2.path().to_str().unwrap())
        .unwrap();

    let repos = storage.list_repos(&project.id).unwrap();
    assert_eq!(repos.len(), 2);
}

#[test]
fn test_list_repos_empty_for_new_project() {
    let storage = in_memory_storage();
    let project = storage.create_project("empty-proj", None).unwrap();
    let repos = storage.list_repos(&project.id).unwrap();
    assert!(repos.is_empty());
}

// ---------------------------------------------------------------------------
// status — last_analyzed_commit reflects analysis state
// ---------------------------------------------------------------------------

#[test]
fn test_status_repo_not_analyzed_has_no_commit() {
    let storage = in_memory_storage();
    let project = storage.create_project("proj", None).unwrap();
    let git_dir = temp_git_repo();
    let repo = storage
        .add_repo(&project.id, "repo", git_dir.path().to_str().unwrap())
        .unwrap();

    // Without any analysis run, last_analyzed_commit is None.
    assert!(
        repo.last_analyzed_commit.is_none(),
        "new repo must report no analysis"
    );
}

#[test]
fn test_status_repo_after_analysis_has_commit() {
    let storage = in_memory_storage();
    let project = storage.create_project("proj", None).unwrap();
    let git_dir = temp_git_repo();
    let repo = storage
        .add_repo(&project.id, "repo", git_dir.path().to_str().unwrap())
        .unwrap();

    storage
        .update_repo_last_commit(&repo.id, "abc123def")
        .unwrap();

    let fetched = storage.get_repo(&repo.id).unwrap().unwrap();
    assert_eq!(fetched.last_analyzed_commit.as_deref(), Some("abc123def"));
}

#[test]
fn test_status_json_reports_per_repo_summary_coverage() {
    // Issue #648: admin status must surface per-repo summary coverage
    // (seeded 3 functions: 2 summarized + 1 NULL) via a JSON field.
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("lievo.db");
    let storage = SqliteStorage::open_at(&db_path).expect("storage must open");
    let project = storage.create_project("cov-proj", None).unwrap();
    let git_dir = temp_git_repo();
    let repo = storage
        .add_repo(&project.id, "repo", git_dir.path().to_str().unwrap())
        .unwrap();

    let now = "2024-01-01T00:00:00Z";
    let make_fn = |id: &str, name: &str, summary: Option<&str>| Entity {
        id: id.to_string(),
        project_id: project.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::Function,
        parent_id: None,
        name: name.to_string(),
        path: None,
        language: None,
        summary: summary.map(String::from),
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    storage
        .upsert_entity(&make_fn("fn-1", "alpha", Some("does alpha things")))
        .unwrap();
    storage
        .upsert_entity(&make_fn("fn-2", "beta", None))
        .unwrap();
    storage
        .upsert_entity(&make_fn("fn-3", "test_helper", Some("a test")))
        .unwrap();

    // The status command computes coverage by filtering function-tier entities
    // to non-`test_` names (matching COUNT_MISSING_SUMMARIES). Assert that
    // filter logic on the exact data the command reads, and that the existing
    // count_missing_summaries query agrees.
    let functions = storage
        .entities_by_repo(&repo.id, Some(EntityTier::Function))
        .unwrap();
    let non_test: Vec<&Entity> = functions
        .iter()
        .filter(|e| !e.name.to_ascii_lowercase().starts_with("test_"))
        .collect();
    let missing = non_test.iter().filter(|e| e.summary.is_none()).count();
    assert_eq!(functions.len(), 3, "3 function entities seeded");
    assert_eq!(non_test.len(), 2, "coverage must exclude test_ names");
    assert_eq!(missing, 1, "1 non-test function must be missing a summary");
    assert_eq!(
        storage.count_missing_summaries(&repo.id).unwrap(),
        missing as u64,
        "count_missing_summaries must agree with the status coverage filter"
    );

    // Build the same JSON line the status command emits and assert the
    // summary_coverage field is present with the correct values.
    let coverage = [(&repo as &lievo::model::Repository, 2u64, 1u64)];
    use bin_common::{StatusStats, format_status_json};

    let stats = StatusStats {
        project_name: "cov-proj",
        repo_count: 1,
        last_commit: None,
        total_entities: 3,
        subsystem_count: 0,
        module_count: 0,
        file_count: 0,
        rel_count: 0,
        repo_coverage: &coverage,
    };
    let line = format_status_json(&stats);
    let json: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
    let coverage_arr = json
        .get("summary_coverage")
        .and_then(|c| c.as_array())
        .expect("summary_coverage field must be present");
    assert_eq!(coverage_arr.len(), 1, "one repo in coverage list");
    assert_eq!(
        coverage_arr[0].get("repo").and_then(|v| v.as_str()),
        Some("repo")
    );
    assert_eq!(
        coverage_arr[0]
            .get("total_functions")
            .and_then(|v| v.as_u64()),
        Some(2)
    );
    assert_eq!(
        coverage_arr[0].get("summarized").and_then(|v| v.as_u64()),
        Some(1)
    );
    assert_eq!(
        coverage_arr[0]
            .get("missing_summaries")
            .and_then(|v| v.as_u64()),
        Some(1)
    );

    // The status command itself must run cleanly against this project.
    bin_common::status(&storage, Some("cov-proj"), OutputFormat::Json)
        .expect("status must succeed");
}

// ---------------------------------------------------------------------------
// info — database stats
// ---------------------------------------------------------------------------

#[test]
fn test_info_project_count_and_repo_count() {
    let storage = in_memory_storage();
    storage.create_project("p1", None).unwrap();
    let p2 = storage.create_project("p2", None).unwrap();
    let git_dir = temp_git_repo();
    storage
        .add_repo(&p2.id, "r1", git_dir.path().to_str().unwrap())
        .unwrap();

    let projects = storage.list_projects().unwrap();
    assert_eq!(projects.len(), 2, "info must report 2 projects");

    let total_repos: usize = projects
        .iter()
        .map(|p| storage.list_repos(&p.id).unwrap().len())
        .sum();
    assert_eq!(
        total_repos, 1,
        "info must report 1 repo across all projects"
    );
}

// ---------------------------------------------------------------------------
// Error display — human-readable messages
// ---------------------------------------------------------------------------

#[test]
fn test_invalid_repo_path_error_is_human_readable() {
    let err = lievo::LievoError::InvalidRepoPath("/bad/path".to_string());
    let msg = err.to_string();
    assert!(
        msg.contains("/bad/path"),
        "error message must include the path: {msg}"
    );
    assert!(
        !msg.contains("InvalidRepoPath"),
        "error message must not expose enum variant name: {msg}"
    );
}

#[test]
fn test_project_not_found_error_is_human_readable() {
    let err = lievo::LievoError::ProjectNotFound("my-proj".to_string());
    let msg = err.to_string();
    assert!(
        msg.contains("my-proj"),
        "error message must include the project name: {msg}"
    );
}
