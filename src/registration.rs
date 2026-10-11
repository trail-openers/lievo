//! Shared repository registration (issue #29). One entry point,
//! [`register`], used by both the MCP `resolve_or_register` routine and the
//! CLI `add-repo` handler. The two paths previously diverged (MCP: path-only
//! + blind suffixing; CLI: get-or-create-by-name); this module unifies them
//! in a neutral location shared by both. `crate::mcp::repo_resolution`
//! delegates here and re-exports `find_repo_by_path` for compatibility.
//!
//! Submodules: `lookup` (path match, liveness, identity backfill), `naming`
//! (project-name selection), `attach` (create-or-attach a project and add the
//! repo row, with orphan cleanup).
//!
//! `register` owns the explicit-project-name conflict rule: an explicit name
//! that exists and holds a repo whose stored non-NULL `git_url` differs from
//! this request's derived identity is `InvalidInput` ("taken by a different
//! identity"); a NULL stored `git_url` is not a conflict.

use std::path::Path;

mod attach;
mod lookup;
mod naming;

use crate::LievoError;
use crate::config::RepoConfig;
use crate::identity::derive_identity;
use crate::storage::Storage;
use attach::attach_or_create;
use lookup::{backfill_git_url, path_exists};
use naming::{
    pick_available_project_name, pick_fresh_project_name, repo_dir_name, repo_short_name,
};

pub use lookup::find_repo_by_path;

/// A project and its repository (issue #29: the shared registration result).
#[derive(Debug, Clone)]
pub struct ResolvedRepo {
    pub project: crate::model::Project,
    pub repo: crate::model::Repository,
}

/// A request to register a repository, shared by both MCP
/// `resolve_or_register` and the CLI `add-repo` (issue #29: one shared
/// registration function). `project_name` is `None` for the MCP entry point
/// (which has no explicit project argument) and `Some(name)` for a CLI
/// invocation with an explicit project name. The repository's configuration
/// (including the optional `identity:` override) is loaded from
/// `<repo_root>/.lievo/config.yaml` by [`register`] itself, so no caller
/// can accidentally pass a default config that ignores the override.
pub struct RegistrationRequest<'a> {
    /// The git work-tree root to register, already validated to exist.
    pub repo_root: &'a Path,
    /// Explicit project name (CLI `--project`), or `None` to derive one.
    pub project_name: Option<&'a str>,
}

/// Shared registration entry point (issue #29). Both the MCP
/// `resolve_or_register` routine and the CLI `add-repo` handler call this.
///
/// Lookup order:
/// 1. **Path match** — a repo already stored at this path, in either its raw
///    or canonical form, across all projects (`find_repo_by_path`). Its
///    project is returned. A NULL stored `git_url` is backfilled from the
///    derived identity unless that key already belongs to another repo (the
///    overlap is logged; path match wins, no merge, no move). A changed
///    on-disk origin never re-keys: a non-NULL stored `git_url` is
///    authoritative.
/// 2. **Live same-identity twin** — another stored repo with the same
///    identity whose `local_path` still exists. The new checkout is registered
///    as a distinct project named `<repo>@<dir>` (deterministic suffix if that
///    name is taken). It is never moved and never attached to the twin's
///    project. Skipped when an explicit project name is given.
/// 3. **Fresh registration** — everything else. The project is named `<dir>`
///    if free; `<owner>/<repo>` if `<dir>` is taken by a different identity;
///    and, for repos with no derivable identity, `name-2`, `name-3`, …. A
///    same-identity repo whose stored path has vanished falls through to here
///    until issue #30 adds move handling.
///
/// Explicit project name: when `project_name` is given it is trimmed once and
/// used everywhere. It must not be empty. It is checked against the
/// explicit-name conflict rule (the check fires only when the request has a
/// derivable identity AND the named project holds a repo whose stored non-NULL
/// `git_url` differs from it, which is `InvalidInput` "taken by a different
/// identity"). A stored NULL `git_url`, or a request with no derivable
/// identity, never conflicts. If the named project already tracks the same
/// identity at another path, registration proceeds and a notice is printed.
///
/// Returns the project and the (possibly new, possibly existing) repository
/// row. An explicit project name is only checked when a new registration is
/// created; if the path is already registered, the existing project is
/// returned (path match wins).
pub fn register(
    storage: &dyn Storage,
    request: &RegistrationRequest,
) -> crate::Result<ResolvedRepo> {
    // A non-UTF-8 repo root cannot be stored as `local_path`: reject it as
    // invalid input rather than silently persisting an empty path.
    let repo_root_str = request
        .repo_root
        .to_str()
        .ok_or_else(|| LievoError::InvalidInput("repo root path is not valid UTF-8".to_string()))?;

    // Trim the explicit name once; every later use sees the trimmed value. An
    // empty or whitespace-only name is not a project name.
    let project_name = request.project_name.map(str::trim);
    if project_name.is_some_and(str::is_empty) {
        return Err(LievoError::InvalidInput(
            "project name must not be empty or whitespace".to_string(),
        ));
    }

    // Load the repository's own configuration here (rather than trusting a
    // caller-supplied default) so the `identity:` override in
    // `<repo_root>/.lievo/config.yaml` is honored identically by MCP and CLI.
    let config = RepoConfig::load_or_default(request.repo_root);

    // Identity is derived once here and threaded through (no second call).
    let identity = derive_identity(request.repo_root, &config);
    let identity_ref: Option<&str> = identity.as_deref();

    // Step 1: path match. The row is re-read from storage once, after the
    // backfill attempt (written or not), so the result always reflects the
    // persisted row, not the stale copy returned by the lookup.
    if let Some(repo) = find_repo_by_path(storage, request.repo_root)? {
        let project = storage
            .get_project_by_id(&repo.project_id)?
            .ok_or_else(|| {
                LievoError::InvalidInput(format!("project missing for repo {}", repo.id))
            })?;
        backfill_git_url(storage, &repo, identity_ref)?;
        let repo = storage.get_repo(&repo.id)?.ok_or_else(|| {
            LievoError::InvalidInput(format!("repo row missing after re-read: {}", repo.id))
        })?;
        return Ok(ResolvedRepo { project, repo });
    }

    // Explicit-name conflict rule: an explicit project name that exists and
    // holds a repo with a stored non-NULL `git_url` that differs from this
    // request's derived identity is a hard error. When the request has no
    // derivable identity, the check is skipped entirely (no re-key). When the
    // named project already holds a repo with the SAME identity at a different
    // path, registration still proceeds (the user asked for that name) and a
    // notice makes the duplicate visible.
    if let Some(explicit) = project_name
        && let Some(identity) = identity_ref
        && let Some(project) = storage.get_project(explicit)?
    {
        let repos = storage.list_repos(&project.id)?;
        let conflicting = repos.iter().any(|r| {
            r.git_url
                .as_deref()
                .is_some_and(|stored| stored != identity)
        });
        if conflicting {
            return Err(LievoError::InvalidInput(format!(
                "project name '{}' is already registered at a different path (identity conflict). \
                 Choose a different project name or register this path under its own project.",
                sanitize(explicit)
            )));
        }
        let existing_same = repos
            .iter()
            .find(|r| r.git_url.as_deref() == Some(identity));
        if let Some(existing_same) = existing_same {
            eprintln!(
                "attached to existing project '{}', which already tracks the same remote at '{}'",
                sanitize(explicit),
                sanitize(&existing_same.local_path),
            );
        }
    }

    let dir_name = repo_dir_name(request.repo_root);

    // Step 2: live same-identity twin. Only a stored path that definitely
    // vanished (`Ok(false)`) is non-live; see `path_exists`.
    if let Some(identity) = identity_ref
        && project_name.is_none()
    {
        let matches = storage.find_repos_by_git_url(identity)?;
        if matches.iter().any(|r| path_exists(&r.local_path)) {
            let base = format!("{}@{dir_name}", repo_short_name(identity));
            let candidate = pick_available_project_name(storage, &base, Some(identity))?;
            return attach_or_create(
                storage,
                &candidate,
                repo_root_str,
                &dir_name,
                Some(identity),
            );
        }
    }

    // Step 3: fresh registration.
    let name = pick_fresh_project_name(storage, project_name, &dir_name, identity_ref)?;
    attach_or_create(storage, &name, repo_root_str, &dir_name, identity_ref)
}

/// Strip control characters from a value that will be interpolated into a
/// user-facing stderr notice (terminal-safety: a hostile identity key or
/// stored path containing ANSI/escape sequences would otherwise be written
/// to the terminal raw). Output is also capped at 200 characters so an
/// oversized value cannot flood a notice. Exported so the CLI handlers (which
/// interpolate stored project/repo names, paths and identities into their own
/// notices) sanitize at the same boundary. Stored values are NOT sanitized:
/// only print boundaries are.
pub fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect()
}
