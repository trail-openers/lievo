//! Create-or-attach a project and add the repository row (issue #29). The one
//! place that creates projects for a registration, so the orphan-cleanup
//! logic and the same-identity reuse rule exist exactly once.

use super::lookup::project_holds_live_identity;
use super::policy::ensure_no_identity_conflict;
use super::{ResolvedRepo, sanitize};
use crate::LievoError;
use crate::model::{Project, Repository};
use crate::storage::Storage;

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
/// re-picked until one attaches. Each lost race means another registration
/// claimed a name, and the project table is finite, so this terminates. An
/// explicit name never yields `Ok(None)`, so it is attempted once.
pub(super) fn attach_picked<F>(
    storage: &dyn Storage,
    source: NameSource,
    pick: F,
    checkout: &Checkout,
) -> crate::Result<ResolvedRepo>
where
    F: Fn(&dyn Storage) -> crate::Result<String>,
{
    loop {
        let name = pick(storage)?;
        if let Some(resolved) = attach_or_create(storage, &name, checkout, source)? {
            return Ok(resolved);
        }
    }
}

/// Attach `checkout` to the project named `name`, creating it when the name is
/// free.
///
/// The reuse rule: an existing project is attached to only for an explicit
/// name, or for an auto name whose project already holds this identity at a
/// live checkout. A vanished-only holder is never reattached. An
/// auto name now held by any other project returns `Ok(None)` so the caller
/// re-picks.
///
/// `created` comes from the atomic insert itself (`create_project_if_absent`),
/// not from a prior lookup. If this call created the project and the following
/// repo insert fails, the project is deleted best-effort only when it holds no
/// repo row for another checkout (see `holds_only_this_checkout`), and the
/// original error is returned. A project that pre-existed is never touched.
/// The outcome of claiming a project name: either this call inserted the
/// project, or it pre-existed and the registration may attach to it.
enum Claim {
    Created(Project),
    Existing(Project),
}

/// Claim `name` for `checkout`. `Ok(None)` means an auto name must be re-picked.
/// Creation is atomic, so `Created` is reported only to the inserting call.
/// A lost race is resolved against the current holder: an auto name is reused
/// only for a live checkout of the same identity, and an explicit name gets the
/// identity-conflict test re-run against the holder's repos.
fn claim_project(
    storage: &dyn Storage,
    name: &str,
    checkout: &Checkout,
    source: NameSource,
) -> crate::Result<Option<Claim>> {
    if let Some(project) = storage.create_project_if_absent(name, None)? {
        return Ok(Some(Claim::Created(project)));
    }
    let Some(project) = storage.get_project(name)? else {
        // The project was deleted between the failed insert and this lookup.
        // An auto name re-picks; an explicit name cannot.
        return match source {
            NameSource::Auto => Ok(None),
            NameSource::Explicit => Err(LievoError::InvalidInput(format!(
                "project '{}' vanished during registration",
                sanitize(name)
            ))),
        };
    };
    match source {
        NameSource::Auto => {
            if !project_holds_live_identity(storage, &project.id, checkout.identity)? {
                return Ok(None);
            }
        }
        // A concurrent registration may have created this project for another
        // identity after the step-2 check, so the conflict test runs again.
        NameSource::Explicit => {
            let repos = storage.list_repos(&project.id)?;
            ensure_no_identity_conflict(&repos, name, checkout.identity)?;
        }
    }
    Ok(Some(Claim::Existing(project)))
}

pub(super) fn attach_or_create(
    storage: &dyn Storage,
    name: &str,
    checkout: &Checkout,
    source: NameSource,
) -> crate::Result<Option<ResolvedRepo>> {
    let (project, created) = match claim_project(storage, name, checkout, source)? {
        None => return Ok(None),
        Some(Claim::Created(project)) => (project, true),
        Some(Claim::Existing(project)) => (project, false),
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
