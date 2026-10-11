//! Explicit project-name policy for registration (issue #29).

use super::sanitize;
use crate::LievoError;
use crate::model::Repository;
use crate::storage::Storage;

/// Errors with `InvalidInput` when any repo in `repos` has a non-NULL stored
/// `git_url` different from `identity` (an identity conflict). A NULL stored
/// `git_url` or a request without an identity is never a conflict. Shared by
/// the step-2 check and the attach step, so a project created concurrently
/// after step 2 is still checked.
pub(super) fn ensure_no_identity_conflict(
    repos: &[Repository],
    explicit: &str,
    identity: Option<&str>,
) -> crate::Result<()> {
    let Some(identity) = identity else {
        return Ok(());
    };
    // Keys are stored lowercase; compare case-insensitively against the derived identity.
    let conflicting = repos.iter().any(|r| {
        r.git_url
            .as_deref()
            .is_some_and(|stored| !stored.eq_ignore_ascii_case(identity))
    });
    if conflicting {
        return Err(LievoError::InvalidInput(format!(
            "project name '{}' is held by a repository with a different identity (identity conflict). \
             Choose a different project name or register this path under its own project.",
            sanitize(explicit)
        )));
    }
    Ok(())
}

/// Checks an explicit project name against the request's derived identity.
///
/// Errors with `InvalidInput` when the named project holds a repo whose stored
/// non-NULL `git_url` differs from `identity` (an identity conflict). A NULL
/// stored `git_url`, a missing project, or a request without an identity is
/// never a conflict. When the named project already holds `identity` at
/// another path, a notice is printed and the registration proceeds.
pub(super) fn check_explicit_name(
    storage: &dyn Storage,
    explicit: &str,
    identity: Option<&str>,
) -> crate::Result<()> {
    let Some(identity) = identity else {
        return Ok(());
    };
    let Some(project) = storage.get_project(explicit)? else {
        return Ok(());
    };
    let repos = storage.list_repos(&project.id)?;
    ensure_no_identity_conflict(&repos, explicit, Some(identity))?;
    // Keys are stored lowercase; compare case-insensitively against the derived identity.
    if let Some(same) = repos.iter().find(|r| {
        r.git_url
            .as_deref()
            .is_some_and(|stored| stored.eq_ignore_ascii_case(identity))
    }) {
        eprintln!(
            "attached to existing project '{}', which already tracks the same remote at '{}'",
            sanitize(explicit),
            sanitize(&same.local_path),
        );
    }
    Ok(())
}
