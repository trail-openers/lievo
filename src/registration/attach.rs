//! Create-or-attach a project and add the repository row (issue #29). The one
//! place that creates projects for a registration, so the orphan-cleanup
//! logic exists exactly once.

use super::{ResolvedRepo, sanitize};
use crate::LievoError;
use crate::model::Repository;
use crate::storage::Storage;

/// Attach `repo_root_str` to the project named `name`, creating that project
/// only if the name is free. `name` must already be resolved by the caller
/// (free, or a name the caller has confirmed may be reused).
///
/// Resolution is explicit (`get_project` first, `create_project` only when the
/// name is free), so the get-or-create fallback inside `create_project` is
/// never relied on. If this call creates the project and the following repo
/// insert fails, the project is deleted best-effort (`delete_project` also
/// removes any repo rows in the same transaction) and the original error is
/// returned. A project that pre-existed is never touched.
pub(super) fn attach_or_create(
    storage: &dyn Storage,
    name: &str,
    repo_root_str: &str,
    dir_name: &str,
    identity: Option<&str>,
) -> crate::Result<ResolvedRepo> {
    let existing = storage.get_project(name)?;
    let created = existing.is_none();
    let project = match existing {
        Some(project) => project,
        None => storage.create_project(name, None)?,
    };
    match add_repo_row(storage, &project.id, dir_name, repo_root_str, identity) {
        Ok(repo) => Ok(ResolvedRepo { project, repo }),
        Err(e) => {
            if created {
                delete_orphan_project(storage, name, &project.id);
            }
            Err(e)
        }
    }
}

/// Add the repo row under its raw directory name (stored unchanged; only
/// print boundaries sanitize), stamp the identity, and re-read. The re-read is
/// necessary because the row returned by `add_repo` is a hand-built struct that
/// never carries `git_url`, so we reflect the stamp from storage.
fn add_repo_row(
    storage: &dyn Storage,
    project_id: &str,
    dir_name: &str,
    repo_root_str: &str,
    identity: Option<&str>,
) -> crate::Result<Repository> {
    let repo = storage.add_repo(project_id, dir_name, repo_root_str)?;
    if let Some(key) = identity {
        storage.set_repo_git_url(&repo.id, key)?;
    }
    storage.get_repo(&repo.id)?.ok_or_else(|| {
        LievoError::InvalidInput(format!("repo row missing after insert: {}", repo.id))
    })
}

/// Best-effort removal of a project this call created but could not populate.
/// The caller returns its own original error; a failed delete only leaves an
/// orphan, so it is reported as a one-line sanitized notice instead.
fn delete_orphan_project(storage: &dyn Storage, name: &str, id: &str) {
    if let Err(e) = storage.delete_project(id) {
        eprintln!(
            "left orphaned project '{}' ({}) after failed registration: cleanup failed: {}",
            sanitize(name),
            sanitize(id),
            sanitize(&e.to_string()),
        );
    }
}
