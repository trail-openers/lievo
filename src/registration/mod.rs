//! Shared repository registration (issue #29). One entry point,
//! [`register`], used by both the MCP `resolve_or_register` routine and the
//! CLI `add-repo` handler. The two paths previously diverged (MCP: path-only
//! + blind suffixing; CLI: get-or-create-by-name); this module unifies them
//! in a neutral location shared by both. `crate::mcp::repo_resolution`
//! delegates here and re-exports `find_repo_by_path` for compatibility.
//!
//! Submodules: `lookup` (path match, liveness, identity backfill), `naming`
//! (candidate project names), `policy` (the explicit-name identity conflict),
//! `attach` (create-or-attach a project, the same-identity reuse rule, the
//! bounded re-pick, and orphan cleanup).

use std::path::Path;

mod attach;
mod lookup;
mod naming;
mod policy;
#[cfg(test)]
mod tests;

use crate::LievoError;
use crate::config::RepoConfig;
use crate::extraction::lievo_data_dir;
use crate::identity::derive_identity;
use crate::refresh::relocate::{self, relocate_checkout};
use crate::storage::Storage;
use attach::{Checkout, NameSource, attach_picked};
use lookup::{backfill_git_url, path_exists};
use naming::{pick_fresh_name, pick_name, repo_dir_name, repo_short_name};
use policy::check_explicit_name;

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
/// 1. **Path match** — a repo already stored at this path, in its raw or
///    canonical form, across all projects (`find_repo_by_path`). Its project
///    is returned even if an explicit name asks for another project; that
///    ignored name is reported on stderr. A NULL stored `git_url` is
///    backfilled from the derived identity unless another repo already owns
///    that key (path match wins, no merge, no move). A non-NULL stored
///    `git_url` is authoritative: a changed on-disk origin never re-keys.
/// 2. **Explicit-name check** (explicit name only) — `check_explicit_name`
///    fails with `InvalidInput` when the named project holds a repo with a
///    different identity, and prints a notice when it already tracks this one.
/// 3. **Live same-identity twin** (auto name only) — another stored repo with
///    the same identity whose `local_path` still exists. The checkout joins
///    `<repo>@<dir>`, reusing that project when it already holds this
///    identity, otherwise the next free suffix. It is never moved and never
///    attached to the twin's own project.
///
/// 3b. **Moved checkout** (auto name, no live twin): a same-identity repo whose
///    stored path has vanished is re-keyed to this path (`relocate_checkout`).
///    The same project and repo row are reused, with no reindex. If the move
///    cannot be done safely, it falls through to step 4. Trust note: identity
///    may come from a `.lievo/config.yaml` `identity:` override, so an override
///    claiming a vanished identity takes over that project's row, index and
///    entity ids.
/// 4. **Fresh registration** — the explicit name as given, or for an auto name
///    the first free of `<dir>`; then, with an identity, `<owner>/<repo>`,
///    `<owner>/<repo>-2`, `-3`, …; without one, `<dir>-2`, `-3`, …. Auto naming
///    always succeeds. A vanished same-identity project is never reattached
///    here.
///
/// An explicit name skips step 3b and registers fresh, as before.
///
/// Attaching (`attach::attach_picked`): an auto name is reused only when its
/// project already holds this identity at a live checkout. If a concurrent
/// registration claimed the picked name for another identity, the name is
/// re-picked until one attaches. An explicit name is never re-picked.
///
/// The explicit name is trimmed once and must not be empty. A stored NULL
/// `git_url`, or a request with no derivable identity, never conflicts.
///
/// Returns the project and the (possibly new, possibly existing) repository
/// row.
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
    // An invalid config warns and falls back to defaults (issue #28), so a bad
    // file never fails registration; identity then comes from the origin.
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
        if let Some(explicit) = project_name
            && explicit != project.name
        {
            eprintln!(
                "'{}' is already registered in project '{}'; ignoring --project '{}'",
                sanitize(repo_root_str),
                sanitize(&project.name),
                sanitize(explicit),
            );
        }
        backfill_git_url(storage, &repo, identity_ref)?;
        let repo = storage.get_repo(&repo.id)?.ok_or_else(|| {
            LievoError::InvalidInput(format!("repo row missing after re-read: {}", repo.id))
        })?;
        return Ok(ResolvedRepo { project, repo });
    }

    // Step 2: explicit-name identity check.
    if let Some(explicit) = project_name {
        check_explicit_name(storage, explicit, identity_ref)?;
    }

    let dir_name = repo_dir_name(request.repo_root);
    let checkout = Checkout {
        root: repo_root_str,
        dir_name: &dir_name,
        identity: identity_ref,
    };

    // Step 3: live same-identity twin. Only a stored path that definitely
    // vanished (`Ok(false)`) is non-live; see `path_exists`.
    if let Some(identity) = identity_ref
        && project_name.is_none()
    {
        let matches = storage.find_repos_by_git_url(identity)?;
        if matches.iter().any(|r| path_exists(&r.local_path)) {
            let base = format!("{}@{dir_name}", repo_short_name(identity));
            return attach_picked(
                storage,
                NameSource::Auto,
                |st: &dyn Storage| pick_name(st, &base, identity),
                &checkout,
            );
        }
        // Step 3b (issue #30): no live twin, so a vanished checkout of this
        // identity may have moved here. Re-key its row instead of registering
        // a fresh project. A failed move falls through to step 4.
        if let Some(vanished) = matches.first() {
            let attempt = match lievo_data_dir() {
                Ok(data_dir) => relocate_checkout(storage, vanished, request.repo_root, &data_dir)?,
                Err(e) => relocate::fall_back(
                    &vanished.local_path,
                    &request.repo_root.display().to_string(),
                    &e.to_string(),
                )?,
            };
            if let Some(moved) = attempt {
                let project = storage
                    .get_project_by_id(&moved.project_id)?
                    .ok_or_else(|| {
                        LievoError::InvalidInput(format!("project missing for repo {}", moved.id))
                    })?;
                return Ok(ResolvedRepo {
                    project,
                    repo: moved,
                });
            }
        }
    }

    // Step 4: fresh registration.
    match project_name {
        Some(explicit) => attach_picked(
            storage,
            NameSource::Explicit,
            |_: &dyn Storage| Ok(explicit.to_string()),
            &checkout,
        ),
        None => attach_picked(
            storage,
            NameSource::Auto,
            |st: &dyn Storage| pick_fresh_name(st, &dir_name, identity_ref),
            &checkout,
        ),
    }
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
        .filter(|c| {
            !(c.is_control()
                || matches!(
                    c,
                    '\u{200B}'..='\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2028}'
                        | '\u{2029}'
                        | '\u{2060}'..='\u{2064}'
                        | '\u{2066}'..='\u{2069}'
                        | '\u{FEFF}'
                ))
        })
        .take(200)
        .collect()
}
