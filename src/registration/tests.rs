//! Registration tests for the path-match-wins rule with an explicit name, and
//! the bounded re-pick of an auto name claimed by a concurrent registration
//! (issue #29).

use super::RegistrationRequest;
use super::attach::{Checkout, NameSource, attach_picked};
use super::naming::pick_fresh_name;
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
fn sanitize_strips_control_and_bidi_format_characters() {
    assert_eq!(super::sanitize("a\u{1b}[2Jb"), "a[2Jb");
    assert_eq!(super::sanitize("x\u{202e}y\u{200b}z"), "xyz");
    assert_eq!(
        super::sanitize("a\u{2028}b\u{2029}c\u{feff}d\u{2060}e\u{2064}f"),
        "abcdef"
    );
}

#[test]
fn live_twin_does_not_reattach_to_vanished_only_holder_of_same_identity() {
    let s = SqliteStorage::open_in_memory().unwrap();
    let remote_checkout = |dir: &TempDir| {
        let root = dir.path().join("twin");
        let repo = git2::Repository::init(&root).unwrap();
        repo.config()
            .unwrap()
            .set_str("remote.origin.url", "git@github.com:acme/tool")
            .unwrap();
        root
    };
    let (d1, d2) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let live = remote_checkout(&d1);
    let newcomer = remote_checkout(&d2);
    // A live acme/tool checkout makes the next registration a live twin.
    let first = register(
        &s,
        &RegistrationRequest {
            repo_root: &live,
            project_name: None,
        },
    )
    .unwrap();
    assert_eq!(first.project.name, "twin");
    // `tool@twin` is held only by a vanished acme/tool checkout.
    let gone = TempDir::new().unwrap();
    claim(
        &s,
        "tool@twin",
        gone.path().join("gone").to_str().unwrap(),
        "github.com/acme/tool",
    );

    let second = register(
        &s,
        &RegistrationRequest {
            repo_root: &newcomer,
            project_name: None,
        },
    )
    .unwrap();
    assert_eq!(
        second.project.name, "tool@twin-2",
        "a vanished-only holder of the identity is skipped, not reattached"
    );
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
fn split_state_notice_names_both_paths_and_refuses_registration() {
    let split = vec![crate::refresh::relocate::Rename {
        from: std::path::PathBuf::from("/data/indices/aaa"),
        to: std::path::PathBuf::from("/data/indices/bbb"),
    }];
    let notice = super::split_state_notice("/old/checkout", "/new/checkout", &split);
    assert!(
        notice.contains("'/data/indices/bbb' (was '/data/indices/aaa')"),
        "{notice}"
    );
    assert!(notice.contains("'/old/checkout'"), "{notice}");
    assert!(notice.contains("Nothing was registered"), "{notice}");
    assert!(
        notice.contains("re-run lievo in '/new/checkout'"),
        "{notice}"
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
        pick_fresh_name(st, "racy", Some("github.com/acme/racy"))
    };
    let checkout = Checkout {
        root: "/work/racy",
        dir_name: "racy",
        identity: Some("github.com/acme/racy"),
    };

    let resolved = attach_picked(&s, NameSource::Auto, pick, &checkout).unwrap();

    assert_eq!(
        resolved.project.name, "acme/racy",
        "identity-bearing re-pick uses the owner name, never a numeric suffix"
    );
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
fn moved_checkout_reuses_vanished_holder_of_same_identity() {
    // A moved checkout re-keys the vanished row (issue #30), which touches the
    // lievo data dir, so HOME must be isolated.
    let home = TempDir::new().unwrap();
    let _env = crate::test_env_support::EnvGuard::new(&home.path().join("db.sqlite"));
    let s = SqliteStorage::open_in_memory().unwrap();
    // `acme/tool` is held only by a vanished checkout (its path is gone).
    let gone = TempDir::new().unwrap();
    let gone_root = gone.path().join("gone");
    claim(
        &s,
        "acme/tool",
        gone_root.to_str().unwrap(),
        "github.com/acme/tool",
    );
    // Occupy the bare dir name with a different identity so the fresh pick
    // would need `acme/tool`.
    claim(&s, "tool", "/elsewhere", "github.com/other/tool");

    let (_d, root) = checkout_dir("tool");
    let git = git2::Repository::init(&root).unwrap();
    git.config()
        .unwrap()
        .set_str("remote.origin.url", "git@github.com:acme/tool")
        .unwrap();
    let resolved = register(
        &s,
        &RegistrationRequest {
            repo_root: &root,
            project_name: None,
        },
    )
    .unwrap();

    assert_eq!(
        resolved.project.name, "acme/tool",
        "a moved checkout reuses the vanished project of the same identity"
    );
    assert_eq!(
        resolved.repo.local_path,
        root.canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(
        resolved.repo.git_url.as_deref(),
        Some("github.com/acme/tool")
    );
    let vanished = s
        .get_project("acme/tool")
        .unwrap()
        .expect("vanished project");
    let repos = s.list_repos(&vanished.id).unwrap();
    assert_eq!(repos.len(), 1, "the same row moved, no second row is added");
    assert_eq!(repos[0].id, resolved.repo.id);
    assert_ne!(repos[0].local_path, gone_root.to_str().unwrap());
    assert_eq!(s.list_projects().unwrap().len(), 2);
}

#[test]
fn fresh_identity_naming_walks_owner_suffixes_until_one_is_free() {
    let s = SqliteStorage::open_in_memory().unwrap();
    let identity = Some("github.com/acme/tool");
    claim(&s, "tool", "/a", "github.com/other/tool");
    assert_eq!(pick_fresh_name(&s, "tool", identity).unwrap(), "acme/tool");
    claim(&s, "acme/tool", "/b", "github.com/acme/tool");
    assert_eq!(
        pick_fresh_name(&s, "tool", identity).unwrap(),
        "acme/tool-2"
    );
    claim(&s, "acme/tool-2", "/c", "github.com/acme/tool");
    assert_eq!(
        pick_fresh_name(&s, "tool", identity).unwrap(),
        "acme/tool-3"
    );
}

#[test]
fn invalid_config_falls_back_to_origin_identity_and_registers() {
    let s = SqliteStorage::open_in_memory().unwrap();
    let (_d, root) = checkout_dir("cfg");
    let git = git2::Repository::init(&root).unwrap();
    git.config()
        .unwrap()
        .set_str("remote.origin.url", "git@github.com:acme/cfg")
        .unwrap();
    std::fs::create_dir_all(root.join(".lievo")).unwrap();
    std::fs::write(root.join(".lievo").join("config.yaml"), "identity: [\n").unwrap();
    let request = RegistrationRequest {
        repo_root: &root,
        project_name: None,
    };

    let first = register(&s, &request).expect("invalid config must not fail registration");
    assert_eq!(first.project.name, "cfg");
    assert_eq!(first.repo.git_url.as_deref(), Some("github.com/acme/cfg"));

    let again = register(&s, &request).expect("path match must also survive an invalid config");
    assert_eq!(again.project.id, first.project.id);
    assert_eq!(s.list_projects().unwrap().len(), 1);
}

#[test]
fn explicit_name_claimed_for_another_identity_after_check_is_rejected() {
    // Race: step 2 saw no project named `racy`; a concurrent registration then
    // created it for another identity. The attach step must re-run the
    // identity-conflict test instead of joining that project.
    let s = SqliteStorage::open_in_memory().unwrap();
    claim(&s, "racy", "/elsewhere", "github.com/other/racy");
    let checkout = Checkout {
        root: "/work/racy",
        dir_name: "racy",
        identity: Some("github.com/acme/racy"),
    };

    let err = attach_picked(
        &s,
        NameSource::Explicit,
        |_: &dyn Storage| Ok("racy".to_string()),
        &checkout,
    )
    .unwrap_err();

    assert!(
        matches!(err, LievoError::InvalidInput(ref m) if m.contains("identity conflict")),
        "explicit name held by another identity must fail with identity conflict; got {err:?}"
    );
    let claimed = s.get_project("racy").unwrap().expect("claimed project");
    assert_eq!(
        s.list_repos(&claimed.id).unwrap().len(),
        1,
        "the claimed project must not gain the checkout"
    );
}

#[test]
fn auto_name_repicks_through_repeated_races_until_one_attaches() {
    // Several concurrent registrations claim successive picked names for other
    // identities before this one attaches. An auto name must never fail merely
    // because names are taken.
    let s = SqliteStorage::open_in_memory().unwrap();
    let races = std::cell::Cell::new(0usize);
    let pick = |st: &dyn Storage| {
        let n = races.get();
        if n < 5 {
            races.set(n + 1);
            let name = format!("stuck-{n}");
            let path = format!("/elsewhere/{name}");
            claim(st, &name, &path, &format!("github.com/other/{name}"));
            return Ok(name);
        }
        pick_fresh_name(st, "stuck", Some("github.com/acme/stuck"))
    };
    let checkout = Checkout {
        root: "/work/stuck",
        dir_name: "stuck",
        identity: Some("github.com/acme/stuck"),
    };

    let resolved = attach_picked(&s, NameSource::Auto, pick, &checkout).unwrap();

    assert_eq!(races.get(), 5);
    assert_eq!(resolved.project.name, "stuck");
    assert_eq!(s.list_projects().unwrap().len(), 6);
}

#[test]
fn row_state_unknown_notice_lists_renamed_paths_and_recovery() {
    // The error names every file left at its new location, both paths, and
    // the two recovery branches; nothing else is implied to be registered.
    let renamed = vec![crate::refresh::relocate::Rename {
        from: std::path::PathBuf::from("/data/indices/old/ts"),
        to: std::path::PathBuf::from("/data/indices/new/ts"),
    }];

    let notice = super::row_state_unknown_notice("/old/co", "/new/co", &renamed);

    assert!(notice.contains("row state is unknown"), "{notice}");
    assert!(notice.contains("'/data/indices/new/ts' (was '/data/indices/old/ts')"));
    assert!(notice.contains("'/old/co'") && notice.contains("'/new/co'"));
    assert!(notice.contains("rename each listed file back"), "{notice}");
    assert!(notice.contains("re-run lievo in '/new/co'"), "{notice}");
    assert!(!notice.contains("could not be moved back"), "{notice}");
}

#[test]
fn row_state_unknown_notice_without_moved_files_says_so() {
    let notice = super::row_state_unknown_notice("/old/co", "/new/co", &[]);
    assert!(notice.contains("no index files were moved"), "{notice}");
}
