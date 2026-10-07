//! Shared repository registration (issue #29). One entry point,
//! [`register`], used by both the MCP `resolve_or_register` routine (via
//! `crate::mcp::repo_resolution`) and the CLI `add-repo` handler (via
//! `lievo::registration::register`). The two paths previously diverged
//! (MCP: path-only + blind suffixing; CLI: get-or-create-by-name); this
//! module unifies them in a neutral location shared by both.
//!
//! `register` owns the explicit-project-name conflict rule: an explicit name
//! that exists and holds a repo whose stored non-NULL `git_url` differs from
//! this request's derived identity is `InvalidInput` ("taken by a different
//! identity"); a NULL stored `git_url` is not a conflict.

use std::path::Path;

use crate::LievoError;
use crate::identity::derive_identity;
use crate::model::Repository;
use crate::storage::Storage;

/// A project and its repository (issue #29: the shared registration result).
#[derive(Debug, Clone)]
pub struct ResolvedRepo {
    pub project: crate::model::Project,
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
///    `crate::mcp::repo_resolution::find_repo_by_path`'s canonicalisation).
///    If that row's `git_url` is `NULL`, backfill it from the derived
///    identity (when derivable); a derived key that already belongs to
///    another repo logs the overlap and path match wins (no merge, no move).
///    A changed on-disk origin on a path-matched row does NOT re-key: the
///    stored `git_url` is authoritative.
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
/// Explicit-name conflict (owned here, issue #29): when the request carries
/// an explicit project name, that name already exists, and a repo in that
/// project stores a non-NULL `git_url` that differs from this request's
/// derived identity, the name is "taken by a different identity" —
/// `InvalidInput`. A NULL stored `git_url` is not a conflict.
///
/// Returns the project and the (possibly new, possibly existing) repository
/// row.
pub fn register(
    storage: &dyn Storage,
    request: &RegistrationRequest,
) -> crate::Result<ResolvedRepo> {
    // Step 1: path match.
    if let Some(repo) = crate::mcp::repo_resolution::find_repo_by_path(storage, request.repo_root)?
    {
        let project = storage
            .get_project_by_id(&repo.project_id)?
            .ok_or_else(|| {
                LievoError::InvalidInput(format!("project missing for repo {repo:?}"))
            })?;
        backfill_git_url(storage, &repo, request)?;
        return Ok(ResolvedRepo { project, repo });
    }

    // Identity is derived once here and threaded through (no second call).
    let identity = derive_identity(request.repo_root, request.config);

    // Explicit-name conflict rule (owned by register, issue #29): an
    // explicit project name that exists and holds a repo with a stored
    // non-NULL `git_url` that differs from this request's derived identity
    // is a hard error. A stored NULL `git_url` is not an identity, so it
    // cannot conflict. When the request itself has no derivable identity
    // (`None`), the conflict check is skipped entirely (no re-key).
    if let Some(explicit) = request.project_name
        && let Some(identity) = identity.as_deref()
        && let Some(project) = storage.get_project(explicit)?
    {
        let conflicting = storage.list_repos(&project.id)?.iter().any(|r| {
            r.git_url
                .as_deref()
                .is_some_and(|stored| stored != identity)
        });
        if conflicting {
            return Err(LievoError::InvalidInput(format!(
                "project name '{explicit}' is already registered at a different path (identity conflict). \
                 Choose a different project name or register this path under its own project."
            )));
        }
    }

    let dir_name = repo_dir_name(request.repo_root);

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
                dir_name,
                identity,
                sanitize(&vanished.local_path),
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
            storage.set_repo_git_url(&repo.id, identity)?;
            return Ok(ResolvedRepo { project, repo });
        }
    }

    // Step 4: fresh registration.
    register_fresh(storage, request, &dir_name, identity.as_deref())
}

/// Backfill `git_url` on a path-matched row when it is NULL and a derivable
/// identity exists. A changed on-disk origin on a path-matched row does NOT
/// re-key: stored `git_url` governs identity, so a non-NULL stored value is
/// left untouched. The identity key is passed in (derived once in
/// `register`) rather than re-derived.
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
    let other = find_other_repo_by_git_url(storage, &key, &repo.id)?;
    if let Some(other) = other {
        // Derived key already belongs to another repo. Log the overlap and
        // let path match win: no merge, no move, no backfill.
        eprintln!(
            "identity overlap for path-matched repo '{}' ({}): derived identity '{}' is already owned by repo '{}' in project '{}'; path match wins, not merging",
            repo.name,
            repo.id,
            sanitize(&key),
            other.id,
            other.project_id,
        );
        return Ok(());
    }
    storage.set_repo_git_url(&repo.id, &key)?;
    Ok(())
}

/// Find a repo OTHER than `except_id` that owns the given identity key.
/// Named to not shadow `Storage::find_repos_by_git_url` (the full-vec
/// cross-project lookup).
fn find_other_repo_by_git_url(
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
    if let Some(key) = identity {
        storage.set_repo_git_url(&repo.id, key)?;
    }
    Ok(ResolvedRepo { project, repo })
}

/// Resolve the project name for a fresh registration.
///
/// Precedence (issue #29 naming rules):
/// - CLI explicit project name: used as-is when free; the taken-by-different-
///   identity collision is caught in `register` before reaching here, so we
///   trust it here.
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
                let repos = storage.list_repos(&p.id)?;
                let same_identity = identity
                    .is_some_and(|id| repos.iter().any(|r| r.git_url.as_deref() == Some(id)));
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

/// Strip control characters from a value that will be interpolated into a
/// user-facing stderr notice (terminal-safety: a hostile identity key or
/// stored path containing ANSI/escape sequences would otherwise be written
/// to the terminal raw).
fn sanitize(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).collect()
}
