//! Zero-config repository resolution and auto-registration for `lievo mcp`
//! (issue #863), shared by `serve()` (MCP) and `lievo doctor` (#865).
//! Order: LIEVO_PROJECT_DIR > CLAUDE_PROJECT_DIR > cwd; canonicalised and
//! walked up to the git work-tree root via `git2::Repository::discover`.
//!
//! Issue #29: the registration entry point (`register`) is shared by both the
//! MCP `resolve_or_register` and the CLI `add-repo` handler (via
//! `lievo::mcp::repo_resolution::register`). The lookup order is path
//! match → identity match among vanished paths (register fresh until move
//! handling from issue #30 lands) → identity match among live paths
//! (register a distinct `<repo>@<dir>` project) → fresh registration.

use std::path::{Path, PathBuf};

use crate::LievoError;
use crate::identity::derive_identity;
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

/// A request to register a repository, shared by both MCP
/// `resolve_or_register` and the CLI `add-repo` (issue #29: one shared
/// registration function). `project_name` is `None` for the MCP entry point
/// (which has no explicit project argument) and `Some(name)` for a CLI
/// invocation with an explicit project name. `config` carries the optional
/// `identity:` override; a default `RepoConfig` derives identity purely from
/// the repository's `origin` remote.
pub struct RegistrationRequest<'a> {
    /// The git work-tree root to register, already validated to exist.
    pub repo_root: &'a Path,
    /// Explicit project name (CLI `--project`), or `None` to derive one.
    pub project_name: Option<&'a str>,
    /// The repository's configuration (identity override, etc.).
    pub config: &'a crate::config::RepoConfig,
}

/// Shared registration entry point (issue #29). Both the MCP
/// `resolve_or_register` routine and the CLI `add-repo` handler call this;
/// the two paths previously diverged (MCP: path-only + blind suffixing;
/// CLI: get-or-create-by-name). This function unifies them.
///
/// Lookup order (issue #29 acceptance criteria):
/// 1. **Path match** — a repo already registered at this path (via
///    `find_repo_by_path`'s canonicalisation). If that row's `git_url` is
///    `NULL`, backfill it from the derived identity (when derivable); a
///    derived key that already belongs to another repo logs the overlap and
///    path match wins (no merge, no move). A changed on-disk origin on a
///    path-matched row does NOT re-key: the stored `git_url` is
///    authoritative.
/// 2. **Identity match among vanished paths** — a stored repo row with the
///    same identity whose `local_path` no longer exists. Hands off to move
///    handling (issue #30); until that lands, register fresh with a
///    user-facing notice naming the vanished path.
/// 3. **Identity match among live paths** — a live checkout of the same
///    identity is already registered. Registers a distinct project named
///    `<repo>@<dir>` (with a deterministic suffix if that name is already
///    taken), never a move.
/// 4. **Fresh registration** — no path, no identity match: a new project
///    named `<dir>` if free, `<owner>/<repo>` if `<dir>` is taken by a
///    different identity, and — for repos with no derivable identity — the
///    existing `name-2`, `name-3`, … suffix (issue #29's resolved open
///    question).
///
/// Returns the project and the (possibly new, possibly existing) repository
/// row.
pub fn register(
    storage: &dyn Storage,
    request: &RegistrationRequest,
) -> crate::Result<ResolvedRepo> {
    // Step 1: path match.
    if let Some(repo) = find_repo_by_path(storage, request.repo_root)? {
        let project = storage
            .get_project_by_id(&repo.project_id)?
            .ok_or_else(|| {
                LievoError::InvalidInput(format!("project missing for repo {repo:?}"))
            })?;
        backfill_git_url(storage, &repo, request)?;
        return Ok(ResolvedRepo { project, repo });
    }

    let dir_name = repo_dir_name(request.repo_root);
    let identity = derive_identity(request.repo_root, request.config);

    // Steps 2 & 3: identity lookup (only when a derivable identity exists).
    if let Some(identity) = identity.as_deref() {
        let matches = storage.find_repos_by_git_url(identity)?;
        let first_vanished = matches
            .iter()
            .find(|r| !path_exists(&r.local_path))
            .cloned();
        let has_live = matches.iter().any(|r| path_exists(&r.local_path));

        if let Some(vanished) = first_vanished {
            // Step 2: identity match among vanished paths — hand off to move
            // handling (issue #30). Until that lands, register fresh with a
            // user-facing notice naming the vanished path.
            eprintln!(
                "registered '{}' as a fresh project (identity '{}' also appears at vanished path '{}'); move handling for re-cloned checkouts is a follow-up",
                dir_name, identity, vanished.local_path,
            );
            return register_fresh(storage, request, &dir_name, Some(identity));
        }

        if has_live {
            // Step 3: identity match among live paths — distinct project
            // named `<repo>@<dir>` (with a deterministic suffix if taken).
            let repo_short = repo_short_name(identity);
            let base = format!("{repo_short}@{dir_name}");
            let candidate = pick_available_project_name(storage, &base, Some(identity))?;
            let project = storage.create_project(&candidate, None)?;
            let repo = storage.add_repo(
                &project.id,
                &dir_name,
                request.repo_root.to_str().unwrap_or(""),
            )?;
            if let Err(e) = storage.set_repo_git_url(&repo.id, identity) {
                tracing::warn!("failed to stamp identity on repo {}: {e}", repo.id);
            }
            return Ok(ResolvedRepo { project, repo });
        }
    }

    // Step 4: fresh registration.
    register_fresh(storage, request, &dir_name, identity.as_deref())
}

/// Backfill `git_url` on a path-matched row when it is NULL and a derivable
/// identity exists. A changed on-disk origin on a path-matched row does NOT
/// re-key: stored `git_url` governs identity, so a non-NULL stored value is
/// left untouched.
fn backfill_git_url(
    storage: &dyn Storage,
    repo: &Repository,
    request: &RegistrationRequest,
) -> crate::Result<()> {
    if repo.git_url.is_some() {
        return Ok(());
    }
    let Some(key) = derive_identity(request.repo_root, request.config) else {
        return Ok(());
    };
    let other = find_repos_by_git_url(storage, &key, &repo.id)?;
    if let Some(other) = other {
        // Derived key already belongs to another repo. Log the overlap and
        // let path match win: no merge, no move, no backfill.
        eprintln!(
            "identity overlap for path-matched repo '{}' ({}): derived identity '{}' is already owned by repo '{}' in project '{}'; path match wins, not merging",
            repo.name, repo.id, key, other.id, other.project_id,
        );
        return Ok(());
    }
    storage.set_repo_git_url(&repo.id, &key)?;
    Ok(())
}

/// Find a repo OTHER than `except_id` that owns the given identity key.
fn find_repos_by_git_url(
    storage: &dyn Storage,
    key: &str,
    except_id: &str,
) -> crate::Result<Option<Repository>> {
    for r in storage.find_repos_by_git_url(key)? {
        if r.id != except_id {
            return Ok(Some(r));
        }
    }
    Ok(None)
}

/// Shared fresh-registration path: create the project, add the repo row, and
/// (if derivable) stamp the git_url. Used for step 2 (identity match among
/// vanished paths) and step 4 (no match).
fn register_fresh(
    storage: &dyn Storage,
    request: &RegistrationRequest,
    dir_name: &str,
    identity: Option<&str>,
) -> crate::Result<ResolvedRepo> {
    let project_name = pick_fresh_project_name(storage, request, dir_name, identity)?;
    let project = storage.create_project(&project_name, None)?;
    let repo = storage.add_repo(
        &project.id,
        dir_name,
        request.repo_root.to_str().unwrap_or(""),
    )?;
    if let Some(key) = identity
        && let Err(e) = storage.set_repo_git_url(&repo.id, key)
    {
        tracing::warn!("failed to stamp identity on repo {}: {e}", repo.id);
    }
    Ok(ResolvedRepo { project, repo })
}

/// Resolve the project name for a fresh registration.
///
/// Precedence (issue #29 naming rules):
/// - CLI explicit project name: used as-is when free; a taken-by-different-
///   identity collision is caught at the CLI layer (`repo_ops::add_repo`)
///   before reaching here, so we trust it here.
/// - Bare `<dir>` if free.
/// - `<owner>/<repo>` (with a deterministic suffix if taken) when the short
///   name is taken by a different identity AND a derivable identity exists.
/// - `name-2`, `name-3`, … for repos with no derivable identity (resolved
///   open question in #29: no-identity registrations keep today's suffixing).
fn pick_fresh_project_name(
    storage: &dyn Storage,
    request: &RegistrationRequest,
    dir_name: &str,
    identity: Option<&str>,
) -> crate::Result<String> {
    if let Some(explicit) = request.project_name
        && !explicit.is_empty()
    {
        return Ok(explicit.to_string());
    }
    if storage.get_project(dir_name)?.is_none() {
        return Ok(dir_name.to_string());
    }
    if let Some(identity) = identity {
        let owner_prefixed = owner_prefixed_name(identity, dir_name);
        return pick_available_project_name(storage, &owner_prefixed, Some(identity));
    }
    let mut candidate = dir_name.to_string();
    let mut suffix = 2;
    while storage.get_project(&candidate)?.is_some() {
        candidate = format!("{dir_name}-{suffix}");
        suffix += 1;
    }
    Ok(candidate)
}

/// Pick a free project name starting from `base`, trying `base-2`,
/// `base-3`, … if needed. If `identity` is `Some` and a candidate of that
/// name already exists owned by a repo with the SAME identity, reuse it
/// (idempotent); otherwise keep suffixing.
fn pick_available_project_name(
    storage: &dyn Storage,
    base: &str,
    identity: Option<&str>,
) -> crate::Result<String> {
    let base = if base.is_empty() {
        "repo".to_string()
    } else {
        base.to_string()
    };
    let mut candidate = base.clone();
    let mut suffix = 2;
    loop {
        let project = storage.get_project(&candidate)?;
        match project {
            None => return Ok(candidate),
            Some(p) => {
                let same_identity = identity.is_some_and(|id| {
                    storage
                        .list_repos(&p.id)
                        .ok()
                        .map(|repos| repos.iter().any(|r| r.git_url.as_deref() == Some(id)))
                        .unwrap_or(false)
                });
                if same_identity {
                    return Ok(candidate);
                }
                candidate = format!("{base}-{suffix}");
                suffix += 1;
            }
        }
    }
}

/// The owner-prefixed name (`<owner>/<repo>`) extracted from an identity key
/// (`"github.com/owner/repo"` → `"owner/repo"`); falls back to `<repo>-2`
/// when the owner cannot be determined.
fn owner_prefixed_name(identity: &str, dir_name: &str) -> String {
    let parts: Vec<&str> = identity.split('/').collect();
    let repo = parts.last().copied().unwrap_or("");
    if let Some(owner) = parts.get(1)
        && !owner.is_empty()
        && !repo.is_empty()
    {
        return format!("{owner}/{repo}");
    }
    format!("{dir_name}-2")
}

/// The directory name for a repo root (used as the repo row's `name`).
fn repo_dir_name(repo_root: &Path) -> String {
    repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "repo".to_string())
}

/// The short repo name extracted from an identity key
/// (`"github.com/owner/repo"` → `"repo"`). Falls back to `dir_name` for
/// malformed keys.
fn repo_short_name(identity: &str) -> String {
    identity
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "repo".to_string())
}

/// Whether the on-disk path at `local_path` still exists. A path that
/// canonicalises or that simply exists on the filesystem counts as live;
/// a vanished path (deleted, moved, unmounted) does not.
fn path_exists(local_path: &str) -> bool {
    Path::new(local_path).exists()
}

/// Find (or create) the project that owns `repo_root` (issue #863 binding
/// decision 1, extended by issue #29 to a shared registration routine used by
/// both MCP and CLI). Delegates to `register` with an explicit project name
/// of `None` and a default (no-identity-override) `RepoConfig`.
pub fn resolve_or_register(storage: &dyn Storage, repo_root: &Path) -> crate::Result<ResolvedRepo> {
    let config = crate::config::RepoConfig::default();
    let request = RegistrationRequest {
        repo_root,
        project_name: None,
        config: &config,
    };
    register(storage, &request)
}

#[cfg(test)]
#[path = "repo_resolution_tests.rs"]
pub(crate) mod tests;
