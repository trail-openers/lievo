//! Create-or-attach a project and add the repository row (issue #29). The one
//! place that creates projects for a registration, so the orphan-cleanup
//! logic and the same-identity reuse rule exist exactly once.

use super::lookup::project_holds_identity;
use super::{ResolvedRepo, sanitize};
use crate::LievoError;
use crate::model::Repository;
use crate::storage::Storage;

/// Picked names tried before a registration gives up. More than one attempt is
/// only needed when a concurrent registration claims a picked name first.
const MAX_PICK_ATTEMPTS: usize = 3;

/// The checkout being registered.
pub(super) struct Checkout<'a> {
    /// Repo root, stored as the repo row's `local_path`.
    pub root: &'a str,
    /// Directory name, stored unchanged as the repo row's `name`.
    pub dir_name: &'a str,
    /// Derived identity, stamped as the repo row's `git_url` when present.
    pub identity: Option<&'a str>,
}

/// Where the project name came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NameSource {
    /// Named by the user (`--project`): used as-is, never re-picked.
    Explicit,
    /// Picked by naming: a lost race re-picks through naming.
    Auto,
}

/// Picks a project name with `pick`, then attaches the checkout to it. An auto
/// name that a concurrent registration claimed for a different identity is
/// re-picked, up to `MAX_PICK_ATTEMPTS` picks in total, then `InvalidInput`.
pub(super) fn attach_picked<F>(
    storage: &dyn Storage,
    source: NameSource,
    pick: F,
    checkout: &Checkout,
) -> crate::Result<ResolvedRepo>
where
    F: Fn(&dyn Storage) -> crate::Result<String>,
{
    for _ in 0..MAX_PICK_ATTEMPTS {
        let name = pick(storage)?;
        if let Some(resolved) = attach_or_create(storage, &name, checkout, source)? {
            return Ok(resolved);
        }
    }
    Err(LievoError::InvalidInput(
        "could not pick a free project name: candidates were claimed concurrently; retry the registration"
            .to_string(),
    ))
}

/// Attach `checkout` to the project named `name`, creating it when the name is
/// free.
///
/// The reuse rule: an existing project is attached to only for an explicit
/// name, or for an auto name whose project already holds this identity. An
/// auto name now held by any other project returns `Ok(None)` so the caller
/// re-picks.
///
/// `created` comes from the atomic insert itself (`create_project_if_absent`),
/// not from a prior lookup. If this call created the project and the following
/// repo insert fails, the project is deleted best-effort only when it holds no
/// repo row for another checkout (see `holds_only_this_checkout`), and the
/// original error is returned. A project that pre-existed is never touched.
pub(super) fn attach_or_create(
    storage: &dyn Storage,
    name: &str,
    checkout: &Checkout,
    source: NameSource,
) -> crate::Result<Option<ResolvedRepo>> {
    let (project, created) = match storage.create_project_if_absent(name, None)? {
        Some(project) => (project, true),
        None => {
            let project = storage.get_project(name)?.ok_or_else(|| {
                LievoError::InvalidInput(format!(
                    "project '{}' vanished during registration",
                    sanitize(name)
                ))
            })?;
            if source == NameSource::Auto
                && !project_holds_identity(storage, &project.id, checkout.identity)?
            {
                return Ok(None);
            }
            (project, false)
        }
    };
    match add_repo_row(storage, &project.id, checkout) {
        Ok(repo) => Ok(Some(ResolvedRepo { project, repo })),
        Err(e) => {
            if created && holds_only_this_checkout(storage, &project.id, checkout.root) {
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
    checkout: &Checkout,
) -> crate::Result<Repository> {
    let repo = storage.add_repo(project_id, checkout.dir_name, checkout.root)?;
    if let Some(key) = checkout.identity {
        storage.set_repo_git_url(&repo.id, key)?;
    }
    storage.get_repo(&repo.id)?.ok_or_else(|| {
        LievoError::InvalidInput(format!("repo row missing after insert: {}", repo.id))
    })
}

/// True when the project has no repo row for any checkout other than
/// `repo_root_str`. A lookup failure answers false, so the project is kept.
fn holds_only_this_checkout(storage: &dyn Storage, project_id: &str, repo_root_str: &str) -> bool {
    matches!(
        storage.list_repos(project_id),
        Ok(repos) if repos.iter().all(|r| r.local_path == repo_root_str)
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::sqlite::SqliteStorage;

    #[test]
    fn create_project_if_absent_reports_creation_only_to_the_inserting_call() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let first = s.create_project_if_absent("racy", None).unwrap();
        assert!(first.is_some(), "the first insert creates the project");
        let second = s.create_project_if_absent("racy", None).unwrap();
        assert!(
            second.is_none(),
            "an existing name must not be reported as created"
        );
        assert_eq!(s.list_projects().unwrap().len(), 1);
    }

    #[test]
    fn holds_only_this_checkout_is_true_for_an_empty_project() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let p = s.create_project("empty", None).unwrap();
        assert!(holds_only_this_checkout(&s, &p.id, "/work/mine"));
    }

    #[test]
    fn holds_only_this_checkout_is_false_when_another_checkout_is_attached() {
        // A concurrent registration won the race and attached its own checkout
        // to the project; the failing call must not delete that project.
        let s = SqliteStorage::open_in_memory().unwrap();
        let p = s.create_project("raced", None).unwrap();
        s.add_repo(&p.id, "other", "/work/other").unwrap();
        assert!(!holds_only_this_checkout(&s, &p.id, "/work/mine"));
    }
}
