//! Registration tests for the path-match-wins rule with an explicit name, and
//! the bounded re-pick of an auto name claimed by a concurrent registration
//! (issue #29).

use super::RegistrationRequest;
use super::attach::{Checkout, NameSource, attach_picked};
use super::naming::pick_name;
use super::register;
use crate::LievoError;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use tempfile::TempDir;

/// A plain existing directory named `name` inside a fresh temp dir.
fn checkout_dir(name: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join(name);
    std::fs::create_dir_all(&root).unwrap();
    (dir, root)
}

/// Claim project `name` for `identity`, as a concurrent registration would.
fn claim(storage: &dyn Storage, name: &str, path: &str, identity: &str) {
    let project = storage
        .create_project_if_absent(name, None)
        .unwrap()
        .expect("name must be free to claim");
    let repo = storage.add_repo(&project.id, name, path).unwrap();
    storage.set_repo_git_url(&repo.id, identity).unwrap();
}

#[test]
fn explicit_name_does_not_move_a_path_already_registered_elsewhere() {
    let s = SqliteStorage::open_in_memory().unwrap();
    let (_d, root) = checkout_dir("stay");
    let a = s.create_project("A", None).unwrap();
    s.add_repo(&a.id, "stay", root.to_str().unwrap()).unwrap();

    let request = RegistrationRequest {
        repo_root: &root,
        project_name: Some("B"),
    };
    let resolved = register(&s, &request).unwrap();

    assert_eq!(
        resolved.project.id, a.id,
        "path match wins over --project B"
    );
    assert_eq!(resolved.project.name, "A");
    assert_eq!(
        s.list_projects().unwrap().len(),
        1,
        "no project named B may be created"
    );
    assert_eq!(s.list_repos(&a.id).unwrap().len(), 1);
}

#[test]
fn third_same_identity_checkout_with_same_dir_name_reuses_repo_at_dir_project() {
    let s = SqliteStorage::open_in_memory().unwrap();
    let identity = "github.com/acme/twin";
    let roots: Vec<_> = (0..3)
        .map(|_| {
            let dir = TempDir::new().unwrap();
            let root = dir.path().join("twin");
            std::fs::create_dir_all(&root).unwrap();
            let repo = git2::Repository::init(&root).unwrap();
            repo.config()
                .unwrap()
                .set_str("remote.origin.url", "git@github.com:acme/twin")
                .unwrap();
            (dir, root)
        })
        .collect();
    let resolve = |root: &std::path::Path| {
        register(
            &s,
            &RegistrationRequest {
                repo_root: root,
                project_name: None,
            },
        )
        .unwrap()
    };
    let first = resolve(&roots[0].1);
    let second = resolve(&roots[1].1);
    let third = resolve(&roots[2].1);

    assert_eq!(first.project.name, "twin");
    assert_eq!(second.project.name, "twin@twin");
    assert_eq!(third.project.id, second.project.id, "reuses twin@twin");
    assert_eq!(s.list_repos(&second.project.id).unwrap().len(), 2);
    assert!(
        s.list_repos(&second.project.id)
            .unwrap()
            .iter()
            .all(|r| r.git_url.as_deref() == Some(identity)),
    );
}

#[test]
fn auto_name_claimed_by_another_identity_before_attach_is_repicked() {
    let s = SqliteStorage::open_in_memory().unwrap();
    let raced = std::cell::Cell::new(false);
    // The first pick returns "racy" and, between that pick and the attach, a
    // concurrent registration claims "racy" for another identity. Later picks
    // go through naming, which skips the claimed name.
    let pick = |st: &dyn Storage| {
        if !raced.replace(true) {
            claim(st, "racy", "/elsewhere", "github.com/other/racy");
            return Ok("racy".to_string());
        }
        pick_name(st, "racy", Some("github.com/acme/racy"))
    };
    let checkout = Checkout {
        root: "/work/racy",
        dir_name: "racy",
        identity: Some("github.com/acme/racy"),
    };

    let resolved = attach_picked(&s, NameSource::Auto, pick, &checkout).unwrap();

    assert_eq!(resolved.project.name, "racy-2");
    let claimed = s.get_project("racy").unwrap().expect("claimed project");
    let claimed_repos = s.list_repos(&claimed.id).unwrap();
    assert_eq!(
        claimed_repos.len(),
        1,
        "the other identity's project is untouched"
    );
    assert_eq!(claimed_repos[0].local_path, "/elsewhere");
}

#[test]
fn auto_name_repicks_are_bounded_then_fail_without_attaching() {
    let s = SqliteStorage::open_in_memory().unwrap();
    claim(&s, "stuck", "/elsewhere", "github.com/other/stuck");
    let checkout = Checkout {
        root: "/work/stuck",
        dir_name: "stuck",
        identity: Some("github.com/acme/stuck"),
    };

    let err = attach_picked(
        &s,
        NameSource::Auto,
        |_: &dyn Storage| Ok("stuck".to_string()),
        &checkout,
    )
    .unwrap_err();

    assert!(
        matches!(err, LievoError::InvalidInput(_)),
        "exhausted re-picks must be InvalidInput, got {err:?}"
    );
    assert_eq!(s.list_projects().unwrap().len(), 1, "no project created");
    let stuck = s.get_project("stuck").unwrap().expect("claimed project");
    assert_eq!(s.list_repos(&stuck.id).unwrap().len(), 1);
}
