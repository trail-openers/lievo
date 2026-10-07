//! Shared repository registration (issue #29). One entry point,
//! [`register`], used by both the MCP `resolve_or_register` routine and the
//! CLI `add-repo` handler. The two paths previously diverged (MCP: path-only
//! + blind suffixing; CLI: get-or-create-by-name); this module unifies them
//! in a neutral location shared by both. `crate::mcp::repo_resolution`
//! delegates here and re-exports `find_repo_by_path` for compatibility.
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
/// `resolve_or_register` routine and the CLI `add-repo` handler call this;
/// the two paths previously diverged (MCP: path-only + blind suffixing;
/// CLI: get-or-create-by-name). This function unifies them.
///
/// Lookup order (issue #29 acceptance criteria):
/// 1. **Path match** — a repo already registered at this path (via
///    `find_repo_by_path`'s canonicalisation, defined in this module).
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
///    `<repo>@<dir>` (the new checkout is never attached to the first one's
///    project by name or identity), with a deterministic suffix if that name
///    is already taken, never a move.
/// 4. **Fresh registration** — no path, no identity match: a new project
///    named `<dir>` if free, `<owner>/<repo>` if `<dir>` is taken by a
///    different identity, and — for repos with no derivable identity — the
///    existing `name-2`, `name-3`, … suffix (issue #29's resolved open
///    question).
///
/// Explicit-name conflict (owned here, issue #29): the check fires only
/// when BOTH the request has a derivable identity AND the named project
/// holds a repo whose stored `git_url` is non-NULL and differs from that
/// identity — then the name is "taken by a different identity"
/// (`InvalidInput`). A stored NULL `git_url`, or a request with no
/// derivable identity, never conflicts.
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

    // Load the repository's own configuration here (rather than trusting a
    // caller-supplied default) so the `identity:` override in
    // `<repo_root>/.lievo/config.yaml` is honored identically by MCP and CLI.
    let config = crate::config::RepoConfig::load_or_default(request.repo_root);

    // Identity is derived once here and threaded through (no second call).
    let identity = derive_identity(request.repo_root, &config);
    let identity: Option<&str> = identity.as_deref();

    // Step 1: path match.
    if let Some(mut repo) = find_repo_by_path(storage, request.repo_root)? {
        let project = storage
            .get_project_by_id(&repo.project_id)?
            .ok_or_else(|| {
                LievoError::InvalidInput(format!("project missing for repo {repo:?}"))
            })?;
        if let Some(key) = identity
            && backfill_git_url(storage, &repo, Some(key))?
        {
            repo.git_url = Some(key.to_string());
        }
        return Ok(ResolvedRepo { project, repo });
    }

    // Explicit-name conflict rule (owned by register, issue #29): an
    // explicit project name that exists and holds a repo with a stored
    // non-NULL `git_url` that differs from this request's derived identity
    // is a hard error. A stored NULL `git_url` is not an identity, so it
    // cannot conflict. When the request itself has no derivable identity
    // (`None`), the conflict check is skipped entirely (no re-key).
    if let Some(explicit) = request.project_name
        && let Some(identity) = identity
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
    if let Some(identity) = identity {
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
                sanitize(&dir_name),
                sanitize(identity),
                sanitize(&vanished.local_path),
            );
            return register_fresh(
                storage,
                repo_root_str,
                request.project_name,
                &dir_name,
                Some(identity),
            );
        }

        if has_live {
            // Step 3: identity match among live paths — distinct project
            // named `<repo>@<dir>` (with a deterministic suffix if taken).
            let repo_short = repo_short_name(identity);
            let base = format!("{repo_short}@{dir_name}");
            let candidate = pick_available_project_name(storage, &base, Some(identity))?;
            let project = storage.create_project(&candidate, None)?;
            let repo = storage.add_repo(&project.id, &sanitize(&dir_name), repo_root_str)?;
            storage.set_repo_git_url(&repo.id, identity)?;
            let repo = storage.get_repo(&repo.id)?.ok_or_else(|| {
                LievoError::InvalidInput(format!("repo row missing after insert: {}", repo.id))
            })?;
            return Ok(ResolvedRepo { project, repo });
        }
    }

    // Step 4: fresh registration.
    register_fresh(
        storage,
        repo_root_str,
        request.project_name,
        &dir_name,
        identity,
    )
}

/// Backfill `git_url` on a path-matched row when it is NULL and a derivable
/// identity exists. A changed on-disk origin on a path-matched row does NOT
/// re-key: stored `git_url` governs identity, so a non-NULL stored value is
/// left untouched. The identity key is passed in (derived once in
/// `register`) rather than re-derived. Returns `true` when the backfill
/// wrote the row's `git_url` (so the caller can reflect it in its result),
/// `false` when there was nothing to backfill.
fn backfill_git_url(
    storage: &dyn Storage,
    repo: &Repository,
    identity: Option<&str>,
) -> crate::Result<bool> {
    if repo.git_url.is_some() {
        return Ok(false);
    }
    let Some(key) = identity else {
        return Ok(false);
    };
    let other = find_other_repo_by_git_url(storage, key, &repo.id)?;
    if let Some(other) = other {
        // Derived key already belongs to another repo. Log the overlap and
        // let path match win: no merge, no move, no backfill.
        eprintln!(
            "identity overlap for path-matched repo '{}' ({}): derived identity '{}' is already owned by repo '{}' in project '{}'; path match wins, not merging",
            repo.name,
            repo.id,
            sanitize(key),
            other.id,
            other.project_id,
        );
        return Ok(false);
    }
    storage.set_repo_git_url(&repo.id, key)?;
    Ok(true)
}

/// Find a registered repo whose canonical `local_path` matches `path`,
/// across ALL projects (issue #863 decision 1). Compares both the raw and
/// the canonicalised form of the stored path against the raw and
/// canonicalised form of `path`, so a symlinked or otherwise
/// non-canonical lookup still matches.
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
    repo_root_str: &str,
    project_name: Option<&str>,
    dir_name: &str,
    identity: Option<&str>,
) -> crate::Result<ResolvedRepo> {
    let project_name = pick_fresh_project_name(storage, project_name, dir_name, identity)?;
    let project = storage.create_project(&project_name, None)?;
    let repo = storage.add_repo(&project.id, &sanitize(dir_name), repo_root_str)?;
    if let Some(key) = identity {
        storage.set_repo_git_url(&repo.id, key)?;
    }
    // Re-read: the row returned by `add_repo` is a hand-built struct that
    // never carries `git_url`, so reflect the stamp from storage.
    let repo = storage.get_repo(&repo.id)?.ok_or_else(|| {
        LievoError::InvalidInput(format!("repo row missing after insert: {}", repo.id))
    })?;
    Ok(ResolvedRepo { project, repo })
}

/// Resolve the project name for a fresh registration.
///
/// Precedence (issue #29 naming rules):
/// - CLI explicit project name: used as-is; the taken-by-different-identity
///   collision is caught in `register` before reaching here, so we trust it
///   here. `create_project` resolves the name to the existing project (no
///   duplicate row), and a second repo row may attach to it.
/// - Bare `<dir>` if free.
/// - `<owner>/<repo>` (with a deterministic suffix if taken) when the short
///   name is taken by a different identity AND a derivable identity exists.
/// - `name-2`, `name-3`, … for repos with no derivable identity (resolved
///   open question in #29: no-identity registrations keep today's suffixing).
fn pick_fresh_project_name(
    storage: &dyn Storage,
    explicit_name: Option<&str>,
    dir_name: &str,
    identity: Option<&str>,
) -> crate::Result<String> {
    if let Some(explicit) = explicit_name
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
    pick_available_project_name(storage, dir_name, None)
}

/// Pick a free project name starting from `base`, trying `base-2`,
/// `base-3`, … if needed. If `identity` is `Some` and a candidate of that
/// name already exists owned by a repo with the SAME identity, reuse it
/// (idempotent, issue #29: an already-taken suffix slot carrying the same
/// identity is attached to rather than re-suffixed); otherwise keep
/// suffixing.
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
/// (`"github.com/owner/repo"` → `"owner/repo"`); falls back to `<dir>-2`
/// (the directory name, `dir_name`) when the owner cannot be determined.
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
