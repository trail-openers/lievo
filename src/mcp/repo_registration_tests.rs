//! Registration coverage tests for `resolve_or_register` / `register`
//! (issue #29), split out of `repo_resolution_tests.rs` to keep that file
//! under the 800-line test budget. Wired via `#[cfg(test)] #[path]` from
//! `repo_resolution.rs`.

use super::*;
use crate::storage::sqlite::SqliteStorage;
use tempfile::TempDir;

/// A temp git repo with an origin remote, so the identity is derivable from
/// the remote URL (issue #29: identity-bearing tests need this; the plain
/// `git_tempdir` helper creates repos with NO remote, exercising the
/// no-identity path). The remote URL normalizes to `host/owner/repo`.
fn git_tempdir_identity(name: &str, remote_url: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join(name);
    std::fs::create_dir_all(&root).unwrap();
    let repo = git2::Repository::init(&root).unwrap();
    repo.config()
        .unwrap()
        .set_str("remote.origin.url", remote_url)
        .unwrap();
    dir
}

#[test]
fn same_directory_name_in_two_repos_gets_distinct_projects() {
    // Issue #29: two same-name dirs with DIFFERENT identities must register
    // as the bare name + an owner-prefixed name (owner/repo), NOT a -2
    // suffix.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d1 = git_tempdir_identity("twin", "git@github.com:alpha/twin");
    let d2 = git_tempdir_identity("twin", "git@github.com:beta/twin");
    let r1 = d1.path().join("twin");
    let r2 = d2.path().join("twin");

    let first = resolve_or_register(&storage, r1.as_path()).unwrap();
    let second = resolve_or_register(&storage, r2.as_path()).unwrap();

    assert_ne!(first.project.id, second.project.id);
    assert_eq!(
        first.project.name, "twin",
        "first registration is the bare dir name"
    );
    assert_eq!(
        second.project.name, "beta/twin",
        "second (different identity) is owner-prefixed, not a -2 suffix"
    );
    assert_eq!(storage.list_projects().unwrap().len(), 2);
}

#[test]
fn path_mismatch_registers_new_repo_instead_of_rewriting_stored_path() {
    // Issue #29: two folders with the SAME identity and both live paths —
    // the second registration becomes a distinct project named <repo>@<dir>
    // with its own index, never a move, never a -2 suffix.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d1 = git_tempdir_identity("moved", "git@github.com:acme/tool");
    let d2 = git_tempdir_identity("moved", "git@github.com:acme/tool");
    let old_root = d1.path().join("moved");
    let new_root = d2.path().join("moved");

    // Existing registration at the old path (live), project named after the
    // dir.
    let existing = storage.create_project("moved", None).unwrap();
    let existing_repo = storage
        .add_repo(&existing.id, "moved", old_root.to_str().unwrap())
        .unwrap();
    storage
        .set_repo_git_url(&existing_repo.id, "github.com/acme/tool")
        .unwrap();

    let resolved = resolve_or_register(&storage, new_root.as_path()).unwrap();
    // A new project, not attached to the existing one by name or identity.
    assert_ne!(resolved.project.id, existing.id);
    assert_eq!(
        resolved.project.name, "tool@moved",
        "live same-identity twin must be <repo>@<dir>, not a -2 suffix"
    );
    // The stored path was not rewritten.
    let old_repos = storage.list_repos(&existing.id).unwrap();
    assert_eq!(old_repos[0].local_path, old_root.to_str().unwrap());
}

/// Issue #29: a directory name is stored raw. Sanitizing is a print-boundary
/// concern only, so a 250-char name must round-trip unchanged into both the
/// repo row and the project name.
#[test]
fn long_dir_name_is_stored_raw_not_display_sanitized() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let long_name = "d".repeat(250);
    let d = git_tempdir_identity(&long_name, "git@github.com:acme/long");
    let root = d.path().join(&long_name);

    let resolved = resolve_or_register(&storage, root.as_path()).unwrap();

    assert_eq!(
        resolved.repo.name, long_name,
        "repo name must be stored raw"
    );
    assert_eq!(
        resolved.project.name, long_name,
        "project name must match the directory"
    );
}

/// Issue #29: two folders with the same identity, both live paths — the
/// second registration becomes a distinct project named <repo>@<dir> with
/// its own index, never a move (the "override" case).
#[test]
fn two_live_same_identity_folders_get_distinct_repo_at_dir_project() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d1 = git_tempdir_identity("live-a", "git@github.com:acme/tool");
    let d2 = git_tempdir_identity("live-b", "git@github.com:acme/tool");

    let first = resolve_or_register(&storage, d1.path().join("live-a").as_path()).unwrap();
    let second = resolve_or_register(&storage, d2.path().join("live-b").as_path()).unwrap();

    assert_ne!(first.project.id, second.project.id);
    assert_eq!(first.project.name, "live-a", "first is the bare dir name");
    assert_eq!(
        second.project.name, "tool@live-b",
        "second live same-identity folder is <repo>@<dir>"
    );
    // Both projects carry the same identity on their repo rows.
    let repos1 = storage.list_repos(&first.project.id).unwrap();
    let repos2 = storage.list_repos(&second.project.id).unwrap();
    assert_eq!(repos1[0].git_url.as_deref(), Some("github.com/acme/tool"));
    assert_eq!(repos2[0].git_url.as_deref(), Some("github.com/acme/tool"));
}

/// Issue #29: a path match against a row with NULL git_url backfills
/// git_url from the derived identity.
#[test]
fn path_match_with_null_git_url_backfills_git_url() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d = git_tempdir_identity("backfill", "git@github.com:acme/tool");
    let root = d.path().join("backfill");
    // Pre-register the path with NO git_url set (NULL identity).
    let p = storage.create_project("backfill", None).unwrap();
    let r = storage
        .add_repo(&p.id, "backfill", root.to_str().unwrap())
        .unwrap();
    assert_eq!(r.git_url, None, "precondition: NULL git_url");

    let resolved = resolve_or_register(&storage, root.as_path()).unwrap();
    // Path match resolved to the existing project (not a new one).
    assert_eq!(resolved.project.id, p.id);
    // The NULL git_url was backfilled from the derived identity.
    let updated = storage.get_repo(&r.id).unwrap().unwrap();
    assert_eq!(
        updated.git_url.as_deref(),
        Some("github.com/acme/tool"),
        "NULL git_url must be backfilled from the derived identity"
    );
}

/// Issue #29: when backfilling git_url would collide with the derived key of
/// another repo, path match wins — no merge, no move, both rows remain.
#[test]
fn backfill_collision_path_match_wins_no_merge() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    // Two different directories, same identity (acme/tool).
    let d1 = git_tempdir_identity("owner-a", "git@github.com:acme/tool");
    let d2 = git_tempdir_identity("owner-b", "git@github.com:acme/tool");
    let root_a = d1.path().join("owner-a");
    let root_b = d2.path().join("owner-b");

    // Pre-register root_a with a NULL git_url, and root_b with the acme/tool
    // identity (so the derived key already belongs to repo_b).
    let p_a = storage.create_project("owner-a", None).unwrap();
    let r_a = storage
        .add_repo(&p_a.id, "owner-a", root_a.to_str().unwrap())
        .unwrap();
    let p_b = storage.create_project("owner-b", None).unwrap();
    let r_b = storage
        .add_repo(&p_b.id, "owner-b", root_b.to_str().unwrap())
        .unwrap();
    storage
        .set_repo_git_url(&r_b.id, "github.com/acme/tool")
        .unwrap();

    // resolve_or_register on root_a: path match wins. The derived identity
    // (github.com/acme/tool) already belongs to r_b, so the backfill is
    // skipped (overlap logged) and the git_url stays NULL.
    let resolved = resolve_or_register(&storage, root_a.as_path()).unwrap();
    assert_eq!(
        resolved.project.id, p_a.id,
        "path match resolves to the existing project"
    );
    let updated_a = storage.get_repo(&r_a.id).unwrap().unwrap();
    assert_eq!(
        updated_a.git_url, None,
        "backfill must be skipped when the derived key is already owned by another repo"
    );
    // Both rows remain; no merge, no move.
    assert_eq!(storage.list_projects().unwrap().len(), 2);
    assert_eq!(storage.list_repos(&p_a.id).unwrap().len(), 1);
    assert_eq!(storage.list_repos(&p_b.id).unwrap().len(), 1);
}

/// Issue #29: two live same-identity checkouts at the SAME directory name —
/// the second registers a DISTINCT project named `<repo>@<dir>` (not attached
/// to the first's project by name or identity, never a move). Distinct from
/// the same-dir-name re-registration case (where an already-taken
/// `<repo>@<dir>` name owned by a same-identity project is reused as a second
/// repo row).
#[test]
fn two_live_same_identity_folders_same_dir_name_get_distinct_repo_at_dir_project() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d1 = git_tempdir_identity("twin", "git@github.com:acme/twin");
    let d2 = git_tempdir_identity("twin", "git@github.com:acme/twin");
    let r1 = d1.path().join("twin");
    let r2 = d2.path().join("twin");

    let first = resolve_or_register(&storage, r1.as_path()).unwrap();
    let second = resolve_or_register(&storage, r2.as_path()).unwrap();

    assert_ne!(first.project.id, second.project.id);
    assert_eq!(first.project.name, "twin", "first is the bare dir name");
    assert_eq!(
        second.project.name, "twin@twin",
        "second live same-identity checkout at the same dir name is <repo>@<dir>"
    );
    // Never a move: the stored path of the first registration is untouched.
    let repos1 = storage.list_repos(&first.project.id).unwrap();
    assert_eq!(repos1.len(), 1);
    assert_eq!(repos1[0].local_path, r1.to_str().unwrap());
}

/// Issue #29: a changed origin on a live repo does not re-key — only the
/// stored git_url governs identity, so resolve_or_register on a path-matched
/// live repo whose on-disk remote changed must still resolve by path and
/// leave the stored identity untouched.
#[test]
fn changed_origin_on_live_repo_does_not_rekey() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d = git_tempdir_identity("rekey", "git@github.com:oldorg/tool");
    let root = d.path().join("rekey");
    // Pre-register with the old identity.
    let p = storage.create_project("rekey", None).unwrap();
    let r = storage
        .add_repo(&p.id, "rekey", root.to_str().unwrap())
        .unwrap();
    storage
        .set_repo_git_url(&r.id, "github.com/oldorg/tool")
        .unwrap();

    // Change the on-disk origin remote to a different identity.
    let repo = git2::Repository::open(root.as_path()).unwrap();
    repo.config()
        .unwrap()
        .set_str("remote.origin.url", "git@github.com:neworg/tool")
        .unwrap();

    // resolve_or_register on the same path: path match wins, stored identity
    // is unchanged.
    let resolved = resolve_or_register(&storage, root.as_path()).unwrap();
    assert_eq!(
        resolved.project.id, p.id,
        "path match resolves to the same project"
    );
    let updated = storage.get_repo(&r.id).unwrap().unwrap();
    assert_eq!(
        updated.git_url.as_deref(),
        Some("github.com/oldorg/tool"),
        "stored git_url must be unchanged when the on-disk origin changes"
    );
}

/// Issue #29 naming rules: bare repo name used when free; owner-prefixed
/// (owner/repo) when the short name is already taken by a DIFFERENT identity.
#[test]
fn naming_bare_when_free_owner_prefixed_when_taken_by_different_identity() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d1 = git_tempdir_identity("myapp", "git@github.com:alice/myapp");
    let d2 = git_tempdir_identity("myapp", "git@github.com:bob/myapp");

    // First: bare name is free → "myapp".
    let first = resolve_or_register(&storage, d1.path().join("myapp").as_path()).unwrap();
    assert_eq!(first.project.name, "myapp", "bare name is used when free");

    // Second: same dir name, different identity → owner-prefixed "bob/myapp".
    let second = resolve_or_register(&storage, d2.path().join("myapp").as_path()).unwrap();
    assert_eq!(
        second.project.name, "bob/myapp",
        "owner-prefixed name when the short name is taken by a different identity"
    );
    assert_ne!(
        second.project.name, "myapp-2",
        "no -2 suffix for identity-bearing registrations"
    );
}

/// Issue #29: an identity-bearing repo whose bare dir name is taken by a
/// DIFFERENT identity gets the owner-prefixed name (step 4); a subsequent
/// same-identity re-registration hits step 3 and gets the <repo>@<dir>
/// name.
#[test]
fn identity_bearing_twin_never_reuses_a_different_identity_project() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    // A different-identity repo occupies the "twin" dir name.
    let d0 = git_tempdir_identity("twin", "git@github.com:other/twin");
    let r0 = d0.path().join("twin");
    let first = resolve_or_register(&storage, r0.as_path()).unwrap();
    assert_eq!(first.project.name, "twin", "first is the bare dir name");

    // An acme/twin repo at a new "twin" dir: no acme/twin identity match
    // yet (d0 is other/twin), so step 4 fresh registration. The bare name
    // "twin" is taken by a different identity → owner-prefixed "acme/twin".
    let d1 = git_tempdir_identity("twin", "git@github.com:acme/twin");
    let r1 = d1.path().join("twin");
    let second = resolve_or_register(&storage, r1.as_path()).unwrap();
    assert_eq!(
        second.project.name, "acme/twin",
        "owner-prefixed when bare name is taken by a different identity (step 4)"
    );
    assert_ne!(second.project.id, first.project.id);

    // A third acme/twin repo at yet another "twin" dir: now an acme/twin
    // identity match exists (d1's repo is live) → step 3. The base <repo>@<dir>
    // name "twin@twin" is already taken by a DIFFERENT-identity project (d0,
    // the bare "twin" project), so a distinct project is created instead — a
    // same-identity reuse only applies to a project that carries the same
    // identity. Never a move, never a -2 suffix.
    let d2 = git_tempdir_identity("twin", "git@github.com:acme/twin");
    let r2 = d2.path().join("twin");
    let third = resolve_or_register(&storage, r2.as_path()).unwrap();
    assert_eq!(
        third.project.name, "twin@twin",
        "live same-identity twin takes <repo>@<dir> (step 3)"
    );
    assert_ne!(
        third.project.id, second.project.id,
        "a distinct project is created; the different-identity owner is never reused"
    );
    assert_ne!(
        third.project.id, first.project.id,
        "the different-identity bare-name project is never reused"
    );
    let second_repos = storage.list_repos(&second.project.id).unwrap();
    assert_eq!(
        second_repos.len(),
        1,
        "the different-identity project keeps only its original repo row"
    );
    assert!(
        second_repos
            .iter()
            .all(|r| r.local_path != r2.to_str().unwrap()),
        "the new path must not be attached to the different-identity project"
    );
    let third_repos = storage.list_repos(&third.project.id).unwrap();
    assert!(
        third_repos
            .iter()
            .any(|r| r.local_path == r2.to_str().unwrap()),
        "the new path must be registered with its own row in the new project"
    );
}
