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
/// `created` comes from the atomic insert itself (`create_project_if_absent`),
/// not from a prior lookup: a concurrent process may create `name` between a
/// lookup and the insert, and then this call must not claim the project. If
/// this call created the project and the following repo insert fails, the
/// project is deleted best-effort only when it holds no repo row for another
/// checkout (see `holds_only_this_checkout`), and the original error is
/// returned. A project that pre-existed is never touched.
pub(super) fn attach_or_create(
    storage: &dyn Storage,
    name: &str,
    repo_root_str: &str,
    dir_name: &str,
    identity: Option<&str>,
) -> crate::Result<ResolvedRepo> {
    let (project, created) = match storage.create_project_if_absent(name, None)? {
        Some(project) => (project, true),
        None => {
            let project = storage.get_project(name)?.ok_or_else(|| {
                LievoError::InvalidInput(format!(
                    "project '{}' vanished during registration",
                    sanitize(name)
                ))
            })?;
            (project, false)
        }
    };
    match add_repo_row(storage, &project.id, dir_name, repo_root_str, identity) {
        Ok(repo) => Ok(ResolvedRepo { project, repo }),
        Err(e) => {
            if created && holds_only_this_checkout(storage, &project.id, repo_root_str) {
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
