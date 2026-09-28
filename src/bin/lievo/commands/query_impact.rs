// Impact analysis command handler — `lievo query impact`.
//
// Resolves repo-relative file paths to stored entity paths, then runs
// `dependency::impact_analysis` and prints the report in the requested
// OutputFormat.

use lievo::output::{OutputFormat, format_impact_human, format_impact_json};
use lievo::query::dependency;
use lievo::storage::Storage;
use lievo::{LievoError, Result};

use lievo::project_resolution::resolve_project_id;

pub fn impact(
    storage: &dyn Storage,
    project_name: Option<&str>,
    file_paths: &[String],
    format: OutputFormat,
) -> Result<()> {
    let project_id = resolve_project_id(storage, project_name)?;
    let (repo_id, normalized_paths) = find_repo_for_files(storage, &project_id, file_paths)?;
    let path_refs: Vec<&str> = normalized_paths.iter().map(|s| s.as_str()).collect();
    let report = dependency::impact_analysis(storage, &repo_id, &path_refs)?;
    let signal = dependency::impact_resolution_for_repo(storage, &repo_id);

    match format {
        OutputFormat::Human => println!("{}", format_impact_human(&report, &signal)),
        OutputFormat::Json => format_impact_json(&report, &signal, &mut std::io::stdout())?,
    }
    Ok(())
}

/// Find the repo ID that contains at least one of the provided file paths.
/// Returns (repo_id, resolved_paths) where each path is the form actually
/// stored on the entity.
///
/// Entity paths are stored as absolute paths (joined against the repo's
/// canonical local path), so repo-relative input (with or without a "./"
/// prefix) must be resolved against the repo root before lookup. The
/// absolute form is tried first; a repo-root-prefixed form covers repos
/// registered under a symlinked path.
fn find_repo_for_files(
    storage: &dyn Storage,
    project_id: &str,
    file_paths: &[String],
) -> Result<(String, Vec<String>)> {
    for repo in storage.list_repos(project_id)? {
        let candidates = candidate_paths(&repo.local_path, file_paths);
        let mut matched = 0usize;
        let mut resolved: Vec<String> = Vec::with_capacity(file_paths.len());
        for path_candidates in candidates {
            for candidate in path_candidates {
                if storage
                    .entity_by_path(&repo.id, candidate.as_str())?
                    .is_some()
                {
                    matched += 1;
                    resolved.push(candidate.clone());
                    break;
                }
            }
        }
        if matched > 0 {
            return Ok((repo.id, resolved));
        }
    }
    Err(LievoError::PathNotFound(format!(
        "no entity found for paths: {}",
        file_paths.join(", ")
    )))
}

/// Build lookup candidates for each input path: the path as given (minus a
/// leading "./"), then the absolute form joined against the repo root, and
/// finally the repo-root-prefixed form.
fn candidate_paths(repo_local_path: &str, file_paths: &[String]) -> Vec<Vec<String>> {
    let root = std::path::Path::new(repo_local_path);
    file_paths
        .iter()
        .map(|path| {
            let stripped = path.strip_prefix("./").unwrap_or(path.as_str());
            let mut candidates: Vec<String> = vec![path.clone()];
            if stripped != path {
                candidates.push(stripped.to_string());
            }
            for form in [path.as_str(), stripped] {
                if let Some(p) = form.strip_prefix('/') {
                    // Absolute input: try as given (done), and the repo-root
                    // prefixed form covers repos registered under a symlink.
                    let prefixed = root.join(p);
                    let s = prefixed.to_string_lossy();
                    if !candidates.iter().any(|c| c == s.as_ref()) {
                        candidates.push(s.into_owned());
                    }
                } else {
                    // Relative input: the stored path is absolute, so the
                    // joined form is what must match.
                    let joined = root.join(form);
                    let s = joined.to_string_lossy();
                    if !candidates.iter().any(|c| c == s.as_ref()) {
                        candidates.push(s.into_owned());
                    }
                }
            }
            candidates
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use lievo::model::{Entity, EntityTier};
    use lievo::storage::sqlite::SqliteStorage;

    fn insert_entity(storage: &dyn Storage, project_id: &str, repo_id: &str, id: &str, path: &str) {
        let now = "2024-01-01T00:00:00Z".to_string();
        let entity = Entity {
            id: id.to_string(),
            project_id: project_id.to_string(),
            repo_id: Some(repo_id.to_string()),
            tier: EntityTier::File,
            parent_id: None,
            name: id.to_string(),
            path: Some(path.to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: now.clone(),
            updated_at: now,
        };
        storage.upsert_entity(&entity).unwrap();
    }

    /// Record the repo-wide unresolved-import counts on the repository row's
    /// columns (#856) so `get_unresolved_counts` returns `Some((internal,
    /// external))` for `repo_id`.
    fn seed_unresolved_counts(
        storage: &dyn Storage,
        _project_id: &str,
        repo_id: &str,
        _repo_name: &str,
        internal: u64,
        external: u64,
    ) {
        storage
            .record_unresolved_counts(repo_id, internal, external)
            .unwrap();
    }

    #[test]
    fn test_impact_single_project_auto_resolves() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("single-proj", None).unwrap();
        let repo = storage.add_repo(&project.id, "repo", "/path").unwrap();
        insert_entity(&storage, &project.id, &repo.id, "file-1", "src/file1.rs");

        // Should succeed without --project (only one project exists)
        let result = impact(
            &storage,
            None,
            &["src/file1.rs".to_string()],
            OutputFormat::Human,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_impact_with_project_scopes_resolution() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let proj1 = storage.create_project("proj-1", None).unwrap();
        let proj2 = storage.create_project("proj-2", None).unwrap();
        let repo1 = storage.add_repo(&proj1.id, "repo1", "/path1").unwrap();
        let repo2 = storage.add_repo(&proj2.id, "repo2", "/path2").unwrap();

        // Same path in both projects
        insert_entity(&storage, &proj1.id, &repo1.id, "file-1", "src/common.rs");
        insert_entity(&storage, &proj2.id, &repo2.id, "file-2", "src/common.rs");

        // With --project, should find the correct entity
        let result = impact(
            &storage,
            Some("proj-1"),
            &["src/common.rs".to_string()],
            OutputFormat::Human,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_impact_stored_path_resolved_by_suffix_matches_repo_root() {
        // Regression for #644: entity paths are stored in the ACTUAL stored
        // format — absolute, joined against the repo's canonical local path.
        // Both bare repo-relative and "./"-prefixed inputs must resolve.
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test-proj", None).unwrap();
        let repo = storage
            .add_repo(&project.id, "repo", "/tmp/example-repo")
            .unwrap();
        insert_entity(
            &storage,
            &project.id,
            &repo.id,
            "file-1",
            "/tmp/example-repo/src/storage/sqlite.rs",
        );

        let bare = impact(
            &storage,
            Some("test-proj"),
            &["src/storage/sqlite.rs".to_string()],
            OutputFormat::Human,
        );
        assert!(
            bare.is_ok(),
            "bare repo-relative path failed: {:?}",
            bare.err()
        );

        let dotslash = impact(
            &storage,
            Some("test-proj"),
            &["./src/storage/sqlite.rs".to_string()],
            OutputFormat::Human,
        );
        assert!(
            dotslash.is_ok(),
            "./-prefixed path failed: {:?}",
            dotslash.err()
        );

        // Absolute path continues to work.
        let absolute = impact(
            &storage,
            Some("test-proj"),
            &["/tmp/example-repo/src/storage/sqlite.rs".to_string()],
            OutputFormat::Human,
        );
        assert!(absolute.is_ok());
    }

    #[test]
    fn test_impact_normalizes_dotslash_prefix() {
        // Pins the exact-match candidate path: when the entity is stored with
        // a relative path, the "./"-prefixed input still resolves via the
        // stripped exact-match candidate (first entry in candidate_paths).
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test-proj", None).unwrap();
        let repo = storage.add_repo(&project.id, "repo", "/path").unwrap();
        insert_entity(&storage, &project.id, &repo.id, "file-1", "src/main.rs");

        let result = impact(
            &storage,
            Some("test-proj"),
            &["./src/main.rs".to_string()],
            OutputFormat::Human,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_impact_bare_relative_path_resolves() {
        // Pins the exact-match candidate path: a bare relative input that
        // matches a relative stored path resolves directly (no join needed).
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test-proj", None).unwrap();
        let repo = storage.add_repo(&project.id, "repo", "/path").unwrap();
        insert_entity(&storage, &project.id, &repo.id, "file-1", "lib/core.rs");

        let result = impact(
            &storage,
            Some("test-proj"),
            &["lib/core.rs".to_string()],
            OutputFormat::Human,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_impact_multiple_projects_without_project_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage.create_project("proj-1", None).unwrap();
        storage.create_project("proj-2", None).unwrap();

        // Should error when multiple projects exist and --project is not specified
        let result = impact(
            &storage,
            None,
            &["src/file.rs".to_string()],
            OutputFormat::Human,
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("specify --project"), "got: {msg}");
    }

    #[test]
    fn test_impact_unknown_path_returns_error() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test-proj", None).unwrap();
        storage.add_repo(&project.id, "repo", "/path").unwrap();

        // Should error when path has no entity
        let result = impact(
            &storage,
            Some("test-proj"),
            &["nonexistent.rs".to_string()],
            OutputFormat::Human,
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("no entity found for paths"), "got: {msg}");
        // The message must not double-wrap the entity-not-found phrasing
        // (issue #645: previously `Entity 'no entity found for paths: …' not found`).
        assert!(!msg.contains("not found for paths"), "got: {msg}");
        assert!(!msg.starts_with("Entity '"), "got: {msg}");
    }

    // --- #690: impact_resolution_for_repo ---

    #[test]
    fn test_impact_resolution_for_repo_returns_pre681_when_no_counts_recorded() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let repo = storage.add_repo(&project.id, "myrepo", "/path").unwrap();
        insert_entity(&storage, &project.id, &repo.id, "f1", "src/main.rs");
        // No seed: get_unresolved_counts returns None -> pre-#681 shape.
        let sig = dependency::impact_resolution_for_repo(&storage, &repo.id);
        assert!(
            sig.unresolved_internal.is_none(),
            "pre-#681: unresolved_internal must be None"
        );
        assert!(
            sig.unresolved_external.is_none(),
            "pre-#681: unresolved_external must be None"
        );
        assert!(!sig.caveat_active());
    }

    #[test]
    fn test_impact_resolution_for_repo_returns_some_when_counts_present() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let repo = storage.add_repo(&project.id, "myrepo", "/path").unwrap();
        insert_entity(&storage, &project.id, &repo.id, "f1", "src/main.rs");
        seed_unresolved_counts(&storage, &project.id, &repo.id, "myrepo", 3, 7);
        let sig = dependency::impact_resolution_for_repo(&storage, &repo.id);
        assert_eq!(sig.unresolved_internal, Some(3));
        assert_eq!(sig.unresolved_external, Some(7));
        assert!(sig.caveat_active());
    }

    #[test]
    fn test_impact_resolution_for_repo_zero_internal_is_not_caveat_active() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let repo = storage.add_repo(&project.id, "myrepo", "/path").unwrap();
        insert_entity(&storage, &project.id, &repo.id, "f1", "src/main.rs");
        seed_unresolved_counts(&storage, &project.id, &repo.id, "myrepo", 0, 5);
        let sig = dependency::impact_resolution_for_repo(&storage, &repo.id);
        assert_eq!(sig.unresolved_internal, Some(0));
        assert_eq!(sig.unresolved_external, Some(5));
        assert!(
            !sig.caveat_active(),
            "zero internal + external-only must NOT fire caveat"
        );
    }
}
