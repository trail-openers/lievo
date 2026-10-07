use super::*;
use lievo::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use lievo::output::OutputFormat;
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;
use std::path::Path;
use tempfile::TempDir;

fn storage() -> SqliteStorage {
    SqliteStorage::open_in_memory().unwrap()
}

fn git_dir() -> TempDir {
    let dir = TempDir::new().unwrap();
    git2::Repository::init(dir.path()).unwrap();
    dir
}

/// A tempdir git repo with an origin remote set to `url` (issue #29:
/// identity-bearing add-repo coverage; the plain `git_dir` helper creates
/// repos with NO remote, exercising the no-identity path).
fn git_dir_with_remote(url: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    repo.config()
        .unwrap()
        .set_str("remote.origin.url", url)
        .unwrap();
    dir
}

/// Write `.lievo/config.yaml` into a temp repo root (issue #29: identity
/// override injection for registration tests).
fn write_identity_config(root: &Path, identity: &str) {
    let lievo_dir = root.join(".lievo");
    std::fs::create_dir_all(&lievo_dir).unwrap();
    std::fs::write(
        lievo_dir.join("config.yaml"),
        format!("identity: {identity}\n"),
    )
    .unwrap();
}

/// Issue #29 (lens fix): a repo whose `.lievo/config.yaml` sets `identity:`
/// must get that identity through the CLI `add-repo` path as well, proving
/// the config is loaded by the shared `register` rather than passed in.
#[test]
fn test_add_repo_handler_identity_override_from_repo_config_applies() {
    let s = storage();
    // No origin remote: identity is ONLY derivable from the config override.
    let dir = git_dir();
    write_identity_config(dir.path(), "github.com/override-org/override-cli");

    add_repo(&s, dir.path(), None).unwrap();
    let projects = s.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    let repos = s.list_repos(&projects[0].id).unwrap();
    assert_eq!(
        repos[0].git_url.as_deref(),
        Some("github.com/override-org/override-cli"),
        "identity override from .lievo/config.yaml must be honored by the CLI path"
    );
}

#[test]
fn test_create_project_handler_succeeds() {
    let s = storage();
    create_project(&s, "test-proj").unwrap();
    let projects = s.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "test-proj");
}

#[test]
fn test_list_projects_handler_empty() {
    let s = storage();
    list_projects(&s).unwrap();
}

#[test]
fn test_add_repo_handler_auto_creates_project() {
    let s = storage();
    let dir = git_dir();
    add_repo(&s, dir.path(), None).unwrap();
    let projects = s.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
}

#[test]
fn test_add_repo_handler_auto_creates_project_identity_bearing() {
    // Identity-bearing repo: project named after the dir, git_url stamped
    // (issue #29: shared registration via the MCP path).
    let s = storage();
    let dir = git_dir_with_remote("git@github.com:alice/myrepo");
    add_repo(&s, dir.path(), None).unwrap();
    let projects = s.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(
        projects[0].name,
        dir.path().file_name().unwrap().to_string_lossy()
    );
    let repos = s.list_repos(&projects[0].id).unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(
        repos[0].git_url.as_deref(),
        Some("github.com/alice/myrepo"),
        "identity-bearing registration must stamp git_url"
    );
}

#[test]
fn test_add_repo_handler_uses_explicit_project() {
    let s = storage();
    s.create_project("explicit-proj", None).unwrap();
    let dir = git_dir();
    add_repo(&s, dir.path(), Some("explicit-proj")).unwrap();
    let projects = s.list_projects().unwrap();
    assert_eq!(projects.len(), 1, "must not create duplicate project");
}

#[test]
fn test_add_repo_handler_invalid_path_fails() {
    let s = storage();
    let result = add_repo(&s, Path::new("/nonexistent/path"), None);
    assert!(result.is_err());
}

#[test]
fn test_add_repo_handler_non_git_dir_fails() {
    let s = storage();
    let dir = TempDir::new().unwrap();
    let result = add_repo(&s, dir.path(), None);
    assert!(result.is_err());
}

#[test]
fn test_add_repo_handler_bare_repo_fails() {
    let s = storage();
    let dir = TempDir::new().unwrap();
    git2::Repository::init_bare(dir.path()).unwrap();
    let result = add_repo(&s, dir.path(), None);
    assert!(
        result.is_err(),
        "bare repositories must be rejected by add_repo"
    );
}

#[test]
fn test_list_repos_handler_no_project_filter() {
    let s = storage();
    let p = s.create_project("p", None).unwrap();
    let dir = git_dir();
    s.add_repo(&p.id, "r", dir.path().to_str().unwrap())
        .unwrap();
    list_repos(&s, None).unwrap();
}

#[test]
fn test_status_handler_no_project_filter() {
    let s = storage();
    status(&s, None, OutputFormat::Human).unwrap();
}

#[test]
fn test_status_handler_with_project() {
    let s = storage();
    s.create_project("proj-a", None).unwrap();
    status(&s, Some("proj-a"), OutputFormat::Human).unwrap();
}

#[test]
fn test_status_handler_unknown_project_fails() {
    let s = storage();
    let result = status(&s, Some("does-not-exist"), OutputFormat::Human);
    assert!(result.is_err());
}

#[test]
fn test_status_handler_json_format() {
    let s = storage();
    s.create_project("proj-json", None).unwrap();
    // JSON format must succeed; actual output goes to stdout.
    status(&s, Some("proj-json"), OutputFormat::Json).unwrap();
}

#[test]
fn test_info_handler_in_memory() {
    let s = storage();
    s.create_project("p1", None).unwrap();
    let p = std::path::PathBuf::from(":memory:");
    info(&s, &p, OutputFormat::Human).unwrap();
}

#[test]
fn test_info_handler_json_format() {
    let s = storage();
    s.create_project("p1", None).unwrap();
    let p = std::path::PathBuf::from(":memory:");
    info(&s, &p, OutputFormat::Json).unwrap();
}

#[test]
fn test_link_repo_handler_already_linked_succeeds() {
    let s = storage();
    let proj = s.create_project("proj", None).unwrap();
    let dir = git_dir();
    let repo = s
        .add_repo(&proj.id, "r", dir.path().to_str().unwrap())
        .unwrap();
    // Re-linking to the same project must succeed (idempotent).
    link_repo(&s, "proj", &repo.id, OutputFormat::Human).unwrap();
}

#[test]
fn test_link_repo_moves_repo_between_projects() {
    let s = storage();
    let proj_a = s.create_project("proj-a", None).unwrap();
    let proj_b = s.create_project("proj-b", None).unwrap();
    let dir = git_dir();
    let repo = s
        .add_repo(&proj_a.id, "r", dir.path().to_str().unwrap())
        .unwrap();

    // Link repo (in proj-a) to proj-b — must succeed and move the repo.
    link_repo(&s, "proj-b", &repo.id, OutputFormat::Human).unwrap();

    let updated = s.get_repo(&repo.id).unwrap().unwrap();
    assert_eq!(
        updated.project_id, proj_b.id,
        "repo.project_id must be updated to proj-b"
    );

    // Verify list_repos reflects the move.
    let repos_a = s.list_repos(&proj_a.id).unwrap();
    assert!(repos_a.is_empty(), "proj-a must have no repos after move");
    let repos_b = s.list_repos(&proj_b.id).unwrap();
    assert_eq!(repos_b.len(), 1);
    assert_eq!(repos_b[0].id, repo.id);
}

#[test]
fn test_link_repo_handler_unknown_project_fails() {
    let s = storage();
    let result = link_repo(&s, "ghost-project", "some-repo-id", OutputFormat::Human);
    assert!(result.is_err());
}

#[test]
fn test_link_repo_handler_unknown_repo_fails() {
    let s = storage();
    s.create_project("proj", None).unwrap();
    let result = link_repo(&s, "proj", "nonexistent-repo-id", OutputFormat::Human);
    assert!(result.is_err());
}

#[test]
fn test_link_repo_json_format_already_linked() {
    let s = storage();
    let proj = s.create_project("proj", None).unwrap();
    let dir = git_dir();
    let repo = s
        .add_repo(&proj.id, "r", dir.path().to_str().unwrap())
        .unwrap();
    link_repo(&s, "proj", &repo.id, OutputFormat::Json).unwrap();
}

// ---------------------------------------------------------------------------
// Coverage handler tests (issue #679)
// ---------------------------------------------------------------------------

/// Temp git repo containing two Rust files with a single resolved cross-file
/// import, so the gate + JSON path run on real extraction output.
fn coverage_fixture_dir() -> TempDir {
    let dir = TempDir::new().unwrap();
    let git = git2::Repository::init(dir.path()).unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("a.rs"),
        "mod b;\nuse crate::b::beta;\nfn alpha() -> i32 { beta() }\n",
    )
    .unwrap();
    std::fs::write(src.join("b.rs"), "pub fn beta() -> i32 { 1 }\n").unwrap();
    // Stage + commit so the repo has a HEAD commit (the in-memory storage
    // path does not require it, but keeps the fixture realistic).
    let mut index = git.index().expect("index");
    index
        .add_all(["**"], git2::IndexAddOption::DEFAULT, None)
        .expect("stage");
    index.write().expect("write index");
    let tree = git
        .find_tree(index.write_tree().expect("tree"))
        .expect("tree obj");
    let sig = git2::Signature::now("test", "t@t").expect("sig");
    let _ = git
        .commit(Some("refs/heads/main"), &sig, &sig, "fixture", &tree, &[])
        .expect("commit");
    dir
}

#[test]
fn test_coverage_handler_unknown_project_fails() {
    let s = storage();
    let result = coverage(
        &s,
        Some("does-not-exist"),
        None,
        false,
        10,
        OutputFormat::Human,
    );
    assert!(result.is_err());
}

#[test]
fn test_coverage_handler_json_output_shape() {
    let s = storage();
    s.create_project("cov-proj", None).unwrap();
    // No repos registered: the command must still succeed and emit the
    // JSON envelope (empty languages + gates, gate_failed: false).
    coverage(&s, Some("cov-proj"), None, false, 10, OutputFormat::Json).unwrap();
}

#[test]
fn test_coverage_handler_gate_without_violation_succeeds() {
    let s = storage();
    s.create_project("cov-gate", None).unwrap();
    let dir = coverage_fixture_dir();
    let proj = s.get_project("cov-gate").unwrap().expect("project");
    s.add_repo(&proj.id, "cov", dir.path().to_str().unwrap())
        .unwrap();
    // Two Rust files, one resolved import edge, no 1-char callees, fan-out 1
    // per name: the gate must pass.
    coverage(&s, Some("cov-gate"), None, true, 10, OutputFormat::Human).unwrap();
}

#[test]
fn test_coverage_handler_fan_out_gate_fails() {
    let s = storage();
    s.create_project("cov-fo", None).unwrap();
    let dir = coverage_fixture_dir();
    let proj = s.get_project("cov-fo").unwrap().expect("project");
    s.add_repo(&proj.id, "cov", dir.path().to_str().unwrap())
        .unwrap();
    // 11 same-named function units → fan-out 11 > threshold 10 → gate fails.
    let src = dir.path().join("src");
    for i in 0..11 {
        std::fs::write(
            src.join(format!("f{i}.rs")),
            format!("fn main() {{ println!(\"f{i}\") }}\n"),
        )
        .unwrap();
    }
    let result = coverage(&s, Some("cov-fo"), None, true, 10, OutputFormat::Human);
    assert!(result.is_err(), "fan-out gate must fail the command");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("gate"), "error must name the gate: {msg}");
    // Without --gate the same repo only reports and succeeds.
    coverage(&s, Some("cov-fo"), None, false, 10, OutputFormat::Human).unwrap();
}

#[test]
fn test_json_escape_backslash_and_quote() {
    assert_eq!(json_escape("foo\\bar"), "foo\\\\bar");
    assert_eq!(json_escape("say \"hi\""), "say \\\"hi\\\"");
    assert_eq!(json_escape("normal"), "normal");
    assert_eq!(json_escape("a\\\"b"), "a\\\\\\\"b");
}

#[test]
fn test_delete_existing_project_removes_project() {
    let s = storage();
    create_project(&s, "to-delete").unwrap();
    // Verify it exists.
    let before = s.list_projects().unwrap();
    assert_eq!(before.len(), 1);

    // Force-delete to skip stdin prompt.
    delete_project(&s, "to-delete", true).unwrap();

    let after = s.list_projects().unwrap();
    assert!(after.is_empty(), "project must be gone after delete");
}

#[test]
fn test_delete_nonexistent_project_returns_error() {
    let s = storage();
    let result = delete_project(&s, "ghost", true);
    assert!(
        result.is_err(),
        "deleting a non-existent project must return an error"
    );
}

#[test]
fn test_delete_project_with_repo_removes_repo() {
    let s = storage();
    let proj = s.create_project("proj-with-repo", None).unwrap();
    let dir = git_dir();
    let repo = s
        .add_repo(&proj.id, "r", dir.path().to_str().unwrap())
        .unwrap();

    // Insert two entities so we can verify entities_deleted count.
    let now = "2024-01-01T00:00:00Z".to_string();
    let entity_a = Entity {
        id: "ea-1".to_string(),
        project_id: proj.id.clone(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: "a.rs".to_string(),
        path: Some("src/a.rs".to_string()),
        language: Some("rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let entity_b = Entity {
        id: "eb-1".to_string(),
        name: "b.rs".to_string(),
        path: Some("src/b.rs".to_string()),
        ..entity_a.clone()
    };
    s.upsert_entity(&entity_a).unwrap();
    s.upsert_entity(&entity_b).unwrap();

    // Insert a relationship between the two entities.
    let rel = Relationship {
        source_id: "ea-1".to_string(),
        target_id: "eb-1".to_string(),
        rel_type: RelType::DependsOn,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    s.upsert_relationship(&rel).unwrap();

    // Delete via Storage trait directly to capture DeleteStats.
    let stats = s.delete_project(&proj.id).unwrap();

    // Assert counts are accurate.
    assert_eq!(stats.repos_deleted, 1, "one repo must be deleted");
    assert_eq!(stats.entities_deleted, 2, "two entities must be deleted");
    assert_eq!(
        stats.relationships_deleted, 1,
        "one relationship must be deleted"
    );

    // Both project and repo must be gone.
    assert!(s.get_project("proj-with-repo").unwrap().is_none());
    let projects = s.list_projects().unwrap();
    assert!(projects.is_empty());
}

#[test]
fn test_delete_repo_command_force() {
    let s = storage();
    let proj = s.create_project("proj", None).unwrap();
    let dir = git_dir();
    let repo = s
        .add_repo(&proj.id, "to-delete", dir.path().to_str().unwrap())
        .unwrap();

    // Verify repo exists
    let before = s.list_repos(&proj.id).unwrap();
    assert_eq!(before.len(), 1);

    // Force-deleterepo to skip stdin prompt
    delete_repo(&s, "to-delete", "proj", true).unwrap();

    let after = s.list_repos(&proj.id).unwrap();
    assert!(after.is_empty(), "repo must be gone after delete");

    // Verify repo cannot be found by ID
    let found = s.get_repo(&repo.id).unwrap();
    assert!(found.is_none());
}

#[test]
fn test_delete_repo_command_name_collision() {
    let s = storage();
    let proj = s.create_project("proj", None).unwrap();

    let dir1 = git_dir();
    let dir2 = git_dir();
    s.add_repo(&proj.id, "same-name", dir1.path().to_str().unwrap())
        .unwrap();
    s.add_repo(&proj.id, "same-name", dir2.path().to_str().unwrap())
        .unwrap();

    // Attempting to delete by name when there are duplicates should fail
    let result = delete_repo(&s, "same-name", "proj", true);
    assert!(result.is_err());

    // Verify the error is InvalidInput
    match result {
        Err(lievo::LievoError::InvalidInput(msg)) => {
            assert!(msg.contains("multiple repos"));
            assert!(msg.contains("same-name"));
        }
        _ => panic!("Expected InvalidInput error for name collision"),
    }

    // Both repos should still exist
    let repos = s.list_repos(&proj.id).unwrap();
    assert_eq!(repos.len(), 2);
}

/// Issue #29: an explicit project name that is taken by a DIFFERENT identity
/// exits non-zero with an identity-conflict error, distinct from the
/// idempotent "already registered here" success (issue #29).
#[test]
fn test_add_repo_handler_explicit_project_taken_by_different_identity_fails() {
    use lievo::LievoError;
    let s = storage();
    // Create a project with a repo that has a specific identity.
    let proj = s.create_project("conflict-proj", None).unwrap();
    let dir1 = git_dir_with_remote("git@github.com:owner-a/repo");
    let repo1 = s
        .add_repo(&proj.id, "repo1", dir1.path().to_str().unwrap())
        .unwrap();
    s.set_repo_git_url(&repo1.id, "github.com/owner-a/repo")
        .unwrap();

    // New repo with a DIFFERENT identity at a different path.
    let dir2 = git_dir_with_remote("git@github.com:owner-b/other");
    let result = add_repo(&s, dir2.path(), Some("conflict-proj"));
    // Must fail with InvalidInput (identity conflict), not succeed.
    let err =
        result.expect_err("must fail when explicit project name is taken by a different identity");
    match err {
        LievoError::InvalidInput(msg) => {
            assert!(
                msg.contains("identity conflict"),
                "error must mention identity conflict: {msg}"
            );
            assert!(
                msg.contains("conflict-proj"),
                "error must name the project: {msg}"
            );
        }
        other => panic!("expected InvalidInput, got {:?}", other),
    }

    // The "already registered here" idempotent success path is distinct:
    // re-adding the same path under the same project name must succeed.
    let result_ok = add_repo(&s, dir1.path(), Some("conflict-proj"));
    assert!(
        result_ok.is_ok(),
        "re-adding the same path under the same project must be idempotent"
    );
}

/// CLI explicit project name with a NULL-identity existing repo does NOT
/// conflict (issue #29: a stored NULL `git_url` is not an identity, so it
/// cannot be "a different identity"). The new path is registered as a SECOND
/// repo row in the existing project; the original NULL-identity row is left
/// untouched (no re-keying, no merge).
#[test]
fn test_add_repo_handler_explicit_project_null_identity_no_conflict() {
    let s = storage();
    // Create a project with a repo that has NULL git_url (no identity).
    let proj = s.create_project("null-identity-proj", None).unwrap();
    let dir1 = git_dir();
    let repo1 = s
        .add_repo(&proj.id, "repo1", dir1.path().to_str().unwrap())
        .unwrap();

    // New repo WITH a derivable identity at a different path. The request's
    // derived identity is Some(...), but the stored row has a NULL git_url,
    // so the conflict check (`stored != derived`) is `None != Some(...)` =
    // true → no conflict. The name is free.
    let dir2 = git_dir_with_remote("git@github.com:owner-a/repo");

    // Must succeed: a stored NULL git_url is not "a different identity".
    add_repo(&s, dir2.path(), Some("null-identity-proj")).unwrap();

    // The new path was registered as a second repo in the EXISTING project
    // (not a new project, not an attachment to the old row's path).
    let projects = s.list_projects().unwrap();
    assert_eq!(projects.len(), 1, "must not create a duplicate project");
    let repos = s.list_repos(&proj.id).unwrap();
    assert_eq!(
        repos.len(),
        2,
        "the new path must be registered as a second repo in the existing project"
    );
    let by_path: std::collections::HashMap<String, _> =
        repos.iter().map(|r| (r.local_path.clone(), r)).collect();
    let old = by_path
        .get(dir1.path().to_str().unwrap())
        .unwrap_or_else(|| panic!("original row at dir1 missing from project repos"));
    assert_eq!(
        old.id, repo1.id,
        "the original row must be untouched (not re-keyed)"
    );
    assert_eq!(
        old.git_url, None,
        "the original NULL-identity row's git_url must stay NULL (no re-keying)"
    );
    let new = by_path
        .get(dir2.path().to_str().unwrap())
        .unwrap_or_else(|| panic!("new row at dir2 missing from project repos"));
    assert_eq!(
        new.git_url.as_deref(),
        Some("github.com/owner-a/repo"),
        "the new row must carry its own derived identity"
    );
}

/// Issue #29 (lens fix): an explicit project name whose project already
/// holds a repo with the SAME identity at a different path must still attach
/// (the user explicitly asked for that name), adding a second repo row
/// without touching the original row.
#[test]
fn test_add_repo_handler_explicit_project_same_identity_attaches_and_notices() {
    let s = storage();
    let proj = s.create_project("same-identity-proj", None).unwrap();
    let dir1 = git_dir_with_remote("git@github.com:owner-a/repo");
    let repo1 = s
        .add_repo(&proj.id, "repo1", dir1.path().to_str().unwrap())
        .unwrap();
    s.set_repo_git_url(&repo1.id, "github.com/owner-a/repo")
        .unwrap();

    // A different checkout of the same identity at a different path, added
    // explicitly under the existing project's name.
    let dir2 = git_dir_with_remote("git@github.com:owner-a/repo");
    add_repo(&s, dir2.path(), Some("same-identity-proj")).unwrap();

    let projects = s.list_projects().unwrap();
    assert_eq!(projects.len(), 1, "must not create a duplicate project");
    let repos = s.list_repos(&proj.id).unwrap();
    assert_eq!(
        repos.len(),
        2,
        "a second repo row must attach to the project"
    );

    let by_path: std::collections::HashMap<&str, &lievo::model::Repository> =
        repos.iter().map(|r| (r.local_path.as_str(), r)).collect();
    let old = by_path
        .get(dir1.path().to_str().unwrap())
        .unwrap_or_else(|| panic!("original row at dir1 missing from project repos"));
    assert_eq!(
        old.id, repo1.id,
        "the original row must be untouched (id unchanged)"
    );
    assert_eq!(old.local_path, dir1.path().to_str().unwrap());
    assert_eq!(
        old.git_url.as_deref(),
        Some("github.com/owner-a/repo"),
        "the original row's git_url must be unchanged"
    );
    let new = by_path
        .get(dir2.path().to_str().unwrap())
        .unwrap_or_else(|| panic!("new row at dir2 missing from project repos"));
    assert_ne!(new.id, repo1.id);
    assert_eq!(new.project_id, proj.id);
    assert_eq!(
        new.git_url.as_deref(),
        Some("github.com/owner-a/repo"),
        "the new row must carry the same identity"
    );
}

/// Issue #29 (lens fix): an explicit project name that is empty or
/// whitespace-only is rejected as invalid input up front.
#[test]
fn test_add_repo_handler_explicit_project_empty_name_rejected() {
    let s = storage();
    let dir = git_dir();
    for bad_name in ["", "   ", "\t"] {
        let result = add_repo(&s, dir.path(), Some(bad_name));
        match result {
            Err(lievo::LievoError::InvalidInput(msg)) => assert!(
                msg.contains("project name"),
                "error must name the project-name problem: {msg}"
            ),
            other => panic!("expected InvalidInput for explicit name {bad_name:?}, got {other:?}"),
        }
    }
    // Nothing must have been persisted by the failed attempts.
    assert!(
        s.list_projects().unwrap().is_empty(),
        "no project may be created"
    );
}
