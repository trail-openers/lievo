use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::InterceptingMcpServer;
use super::repo_resolution;
use super::tools::LievoMcpServer;
use crate::project_resolution::resolve_project_id;
use crate::refresh::start_index_if_needed;
use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use rmcp::ServiceExt;

/// When the project has 2+ repos, keep today's clear error (issue #863
/// decision 7: serve a pre-existing multi-repo project only if it cannot
/// leak other repos' entities — project-boundary isolation is #764, and the
/// current storage has no repo-scoped entity queries, so the error is the
/// only safe behaviour). For 0 repos, return an empty path; the tool layer
/// returns guidance (see `ToolContext::zero_repo_guidance`).
fn resolve_repo_path(storage: &dyn Storage, project_id: &str) -> crate::Result<PathBuf> {
    let repos = storage.list_repos(project_id)?;
    match repos.len() {
        0 => Ok(PathBuf::new()),
        1 => Ok(PathBuf::from(&repos[0].local_path)),
        _ => Err(crate::LievoError::InvalidInput(
            "project has multiple repos; MCP server requires a single-repo project".to_string(),
        )),
    }
}

/// Resolve the project, repository path, and zero-repo guidance for a
/// `serve()` invocation. Explicit PROJECT keeps today's exact behaviour
/// (issue #863 binding decision 3): a multi-repo project still errors with
/// `resolve_repo_path`, including the zero-repos case (the zero-repos
/// guidance path, binding decision 4, is reserved for the no-argument
/// launch inside a git repo, which always auto-registers). An absent
/// PROJECT resolves the repo the server was launched in and auto-registers
/// it if unknown (decision 1), falling back to guidance when the launch
/// directory is not in a git repo (decision 4).
fn resolve_serve_context(
    storage: &dyn Storage,
    project_name: Option<&str>,
) -> crate::Result<(Option<String>, PathBuf, Option<String>)> {
    match project_name {
        Some(name) => {
            let project_id = resolve_project_id(storage, Some(name))?;
            let repo_path = resolve_repo_path(storage, &project_id)?;
            Ok((Some(project_id), repo_path, None))
        }
        None => match repo_resolution::resolve_project_root() {
            Ok(resolved) => {
                let r = repo_resolution::resolve_or_register(storage, &resolved.root)?;
                let repo_path = PathBuf::from(r.repo.local_path.clone());
                Ok((Some(r.project.id), repo_path, None))
            }
            Err(e) => {
                eprintln!(
                    "lievo mcp: {} is not inside a git repository — starting anyway, tools return guidance",
                    e.checked.display()
                );
                let guidance = format!(
                    "The directory {dir} (checked for a git repository) is not inside a git repository. lievo works inside a git repository: cd into your repository and start `lievo mcp` again, or register one with `lievo admin add-repo <path>`.",
                    dir = e.checked.display()
                );
                Ok((None, PathBuf::new(), Some(guidance)))
            }
        },
    }
}

fn resolve_output_dir(storage: &dyn Storage, project_id: &str) -> crate::Result<Option<String>> {
    Ok(storage.get_output_dirs(project_id)?.into_iter().next())
}

pub fn serve(project_name: Option<&str>) -> crate::Result<()> {
    let storage = SqliteStorage::open()?;
    let (project_id, repo_path, zero_repo_guidance) =
        resolve_serve_context(&storage, project_name)?;
    let output_dir = project_id
        .as_deref()
        .map(|id| resolve_output_dir(&storage, id))
        .transpose()?
        .flatten();

    // Issue #864: after the repo is resolved (project_id + repo_path),
    // decide whether a background index is needed — never indexed → first
    // full index, stale → incremental refresh, fresh → nothing. The
    // decision is made synchronously (fast, <10ms, using the storage the
    // server just opened) so the MCP initialize handshake is NOT blocked;
    // the actual work runs on a background std::thread (started inside
    // `start_index_if_needed`, which also acquires the per-repo cross-
    // process lock so concurrent `lievo mcp` sessions on the same repo do
    // not index twice). A `None` return means either the index is current
    // or another session is already indexing — both are fine to proceed.
    let should_index = project_id
        .as_ref()
        .map(|pid| {
            !repo_path.as_os_str().is_empty() && start_index_if_needed(pid, &repo_path).is_some()
        })
        .unwrap_or(false);
    if should_index {
        eprintln!(
            "lievo mcp: background index started for project '{}'",
            project_id.as_ref().unwrap()
        );
    }

    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project_id.unwrap_or_default(),
        repo_path,
        output_dir,
        zero_repo_guidance,
    });

    let server = InterceptingMcpServer::new(LievoMcpServer::new(ctx));
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| crate::LievoError::InvalidInput(format!("tokio runtime: {e}")))?;

    runtime.block_on(async move {
        let service = server
            .serve(rmcp::transport::io::stdio())
            .await
            .map_err(|e| crate::LievoError::InvalidInput(format!("mcp initialize error: {e}")))?;
        // Intentionally ignored: service handle not needed after spawn
        let _ = service
            .waiting()
            .await
            .map_err(|e| crate::LievoError::InvalidInput(format!("mcp server error: {e}")))?;
        Ok::<(), crate::LievoError>(())
    })?;

    Ok(())
}

// NOTE: serve() cannot be directly tested because it requires mocking
// rmcp::transport::io::stdio(), which would require an external mocking library.
// All setup logic (project resolution, repo validation, output dir selection) is
// thoroughly tested via resolve_project_id, resolve_repo_path, and resolve_output_dir.
// The async transport initialization error path would only be triggered in production
// by actual stdio transport failures, which are not testable without mocks.

#[cfg(test)]
mod tests {
    use super::{resolve_output_dir, resolve_repo_path};

    use crate::project_resolution::resolve_project_id;
    use crate::storage::Storage;
    use crate::storage::sqlite::SqliteStorage;

    #[test]
    fn resolve_repo_path_errors_when_project_has_multiple_repos() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        storage
            .add_repo(&project.id, "repo-a", "/tmp/repo-a")
            .unwrap();
        storage
            .add_repo(&project.id, "repo-b", "/tmp/repo-b")
            .unwrap();

        let err = resolve_repo_path(&storage, &project.id).unwrap_err();

        assert!(
            err.to_string()
                .contains("MCP server requires a single-repo project")
        );
    }

    #[test]
    fn resolve_repo_path_succeeds_with_single_repo() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        storage
            .add_repo(&project.id, "repo-a", "/path/to/repo")
            .unwrap();

        let result = resolve_repo_path(&storage, &project.id).unwrap();

        assert_eq!(result.to_string_lossy(), "/path/to/repo");
    }

    #[test]
    fn resolve_repo_path_returns_empty_when_no_repos() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();

        let result = resolve_repo_path(&storage, &project.id).unwrap();

        assert_eq!(result.to_string_lossy(), "");
    }

    #[test]
    fn resolve_output_dir_returns_none_when_no_output_dirs() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();

        let result = resolve_output_dir(&storage, &project.id).unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn resolve_output_dir_returns_first_when_single_output_dir() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        storage
            .add_output_dir(&project.id, "/path/to/output")
            .unwrap();

        let result = resolve_output_dir(&storage, &project.id).unwrap();

        assert_eq!(result, Some("/path/to/output".to_string()));
    }

    #[test]
    fn resolve_output_dir_returns_first_when_multiple_output_dirs() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        storage.add_output_dir(&project.id, "/path/one").unwrap();
        storage.add_output_dir(&project.id, "/path/two").unwrap();

        let result = resolve_output_dir(&storage, &project.id).unwrap();

        // resolve_output_dir returns the first element from the iterator deterministically
        assert_eq!(result, Some("/path/one".to_string()));
    }

    #[test]
    fn resolve_project_id_errors_when_no_projects_exist() {
        let storage = SqliteStorage::open_in_memory().unwrap();

        let err = resolve_project_id(&storage, None).unwrap_err();

        assert!(err.to_string().contains("no projects exist"));
    }

    #[test]
    fn resolve_project_id_succeeds_with_single_project_no_name() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();

        let result = resolve_project_id(&storage, None).unwrap();

        assert_eq!(result, project.id);
    }

    #[test]
    fn resolve_project_id_errors_when_named_project_not_found() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage.create_project("existing", None).unwrap();

        let err = resolve_project_id(&storage, Some("nonexistent")).unwrap_err();

        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn resolve_project_id_succeeds_with_matching_project_name() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("myproject", None).unwrap();

        let result = resolve_project_id(&storage, Some("myproject")).unwrap();

        assert_eq!(result, project.id);
    }

    #[test]
    fn serve_validates_single_repo_success_path() {
        // Tests the validation chain for the single-repo success path:
        // project resolution -> repo validation -> output dir selection.
        // The serve() function will fail at transport initialization (which
        // requires mocking stdio), but we verify the pre-transport validation succeeds.
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test-proj", None).unwrap();
        storage.add_repo(&project.id, "repo", "/tmp/repo").unwrap();
        storage.add_output_dir(&project.id, "/tmp/output").unwrap();

        // All helper functions used by serve() should succeed with valid inputs
        let resolved_project = resolve_project_id(&storage, None).unwrap();
        let resolved_repo = resolve_repo_path(&storage, &resolved_project).unwrap();
        let resolved_output = resolve_output_dir(&storage, &resolved_project).unwrap();

        assert_eq!(resolved_project, project.id);
        assert_eq!(resolved_repo.to_string_lossy(), "/tmp/repo");
        assert_eq!(resolved_output, Some("/tmp/output".to_string()));
    }
}
