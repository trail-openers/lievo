//! Zero-config repository resolution and auto-registration for `lievo mcp`
//! (issue #863), shared by `serve()` (MCP) and `lievo doctor` (#865).
//! Order: LIEVO_PROJECT_DIR > CLAUDE_PROJECT_DIR > cwd; canonicalised and
//! walked up to the git work-tree root via `git2::Repository::discover`.

use std::path::{Path, PathBuf};

use crate::LievoError;
use crate::model::{Project, Repository};
use crate::storage::Storage;

/// Where the resolved repository root came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoRootSource {
    /// The `LIEVO_PROJECT_DIR` environment variable.
    LievoProjectDir,
    /// The `CLAUDE_PROJECT_DIR` environment variable.
    ClaudeProjectDir,
    /// The process working directory.
    CurrentDir,
}

impl RepoRootSource {
    /// Short label used in stderr diagnostics.
    pub fn as_label(self) -> &'static str {
        match self {
            RepoRootSource::LievoProjectDir => "LIEVO_PROJECT_DIR",
            RepoRootSource::ClaudeProjectDir => "CLAUDE_PROJECT_DIR",
            RepoRootSource::CurrentDir => "cwd",
        }
    }
}

/// A resolved git work-tree root and the source that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRepoRoot {
    pub root: PathBuf,
    pub source: RepoRootSource,
}

/// The resolved directory is not inside a git repository. Carries the
/// directory that was checked so guidance can name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotInGitRepo {
    pub checked: PathBuf,
    pub source: RepoRootSource,
}

/// Pick the directory: LIEVO_PROJECT_DIR > CLAUDE_PROJECT_DIR > `start_dir`.
fn source_dir(start_dir: &Path) -> (PathBuf, RepoRootSource) {
    if let Ok(dir) = std::env::var("LIEVO_PROJECT_DIR")
        && !dir.is_empty()
    {
        return (PathBuf::from(dir), RepoRootSource::LievoProjectDir);
    }
    if let Ok(dir) = std::env::var("CLAUDE_PROJECT_DIR")
        && !dir.is_empty()
    {
        return (PathBuf::from(dir), RepoRootSource::ClaudeProjectDir);
    }
    (start_dir.to_path_buf(), RepoRootSource::CurrentDir)
}

/// Find the git work-tree root for `dir` (git2::Repository::discover).
fn find_git_root(dir: &Path) -> Result<PathBuf, NotInGitRepo> {
    git2::Repository::discover(dir)
        .map(|repo| {
            repo.workdir()
                .map(PathBuf::from)
                .unwrap_or_else(|| dir.to_path_buf())
        })
        .map_err(|_| NotInGitRepo {
            checked: dir.to_path_buf(),
            source: RepoRootSource::CurrentDir,
        })
}

/// Resolve the repo root: LIEVO_PROJECT_DIR > CLAUDE_PROJECT_DIR > the
/// process working directory, canonicalised, walked up to the git root.
/// Env-mutating tests hold `test_env_lock`. Callers log the source (e.g.
/// `serve` does; `lievo doctor` reports it instead).
pub fn resolve_project_root() -> Result<ResolvedRepoRoot, NotInGitRepo> {
    let start_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_project_root_from(&start_dir)
}

/// Resolve the repo root from an explicit start directory. Order:
/// LIEVO_PROJECT_DIR, then CLAUDE_PROJECT_DIR, then `start_dir` (env vars
/// still take precedence), canonicalised, walked up to the git root.
/// Used by `lievo mcp` (via `resolve_project_root`) and `lievo doctor`
/// (with an optional PATH arg).
pub fn resolve_project_root_from(start_dir: &Path) -> Result<ResolvedRepoRoot, NotInGitRepo> {
    let (dir, source) = source_dir(start_dir);
    let canonical = match dir.canonicalize() {
        Ok(p) => p,
        // A source directory that does not exist cannot be inside a git
        // repository: report the (non-canonical) directory as checked.
        Err(_) => {
            return Err(NotInGitRepo {
                checked: dir,
                source,
            });
        }
    };
    match find_git_root(&canonical) {
        Ok(root) => Ok(ResolvedRepoRoot { root, source }),
        Err(mut e) => {
            e.checked = canonical.clone();
            e.source = source;
            Err(e)
        }
    }
}

/// Find a registered repo whose canonical `local_path` matches `path`,
/// across ALL projects (issue #863 decision 1).
pub fn find_repo_by_path(storage: &dyn Storage, path: &Path) -> crate::Result<Option<Repository>> {
    let target = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    for project in storage.list_projects()? {
        for repo in storage.list_repos(&project.id)? {
            let stored = Path::new(&repo.local_path);
            let canon = stored.canonicalize().ok();
            if stored == path
                || stored == target
                || canon.as_deref() == Some(path)
                || canon.as_deref() == Some(target.as_path())
            {
                return Ok(Some(repo));
            }
        }
    }
    Ok(None)
}

/// A project and its repository.
#[derive(Debug, Clone)]
pub struct ResolvedRepo {
    pub project: Project,
    pub repo: Repository,
}

/// Find (or create) the project that owns `repo_root` (issue #863 binding
/// decision 1: a library-side routine, never the binary-local `add_repo`
/// get-or-create-by-name path). Registered: the project already containing a
/// repo at this path. Unregistered: a fresh single-repo project named after
/// the directory, with a deterministic collision suffix (`name`, `name-2`,
/// …). Never attaches to an existing project by name.
pub fn resolve_or_register(storage: &dyn Storage, repo_root: &Path) -> crate::Result<ResolvedRepo> {
    if let Some(repo) = find_repo_by_path(storage, repo_root)? {
        let project = storage
            .get_project_by_id(&repo.project_id)?
            .ok_or_else(|| {
                LievoError::InvalidInput(format!("project missing for repo {repo:?}"))
            })?;
        return Ok(ResolvedRepo { project, repo });
    }

    let dir_name = repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "repo".to_string());

    let mut candidate = dir_name.clone();
    let mut suffix = 2;
    while storage.get_project(&candidate)?.is_some() {
        candidate = format!("{dir_name}-{suffix}");
        suffix += 1;
    }

    let project = storage.create_project(&candidate, None)?;
    let repo = storage.add_repo(&project.id, &dir_name, repo_root.to_str().unwrap_or(""))?;
    Ok(ResolvedRepo { project, repo })
}

#[cfg(test)]
#[path = "repo_resolution_tests.rs"]
pub(crate) mod tests;
