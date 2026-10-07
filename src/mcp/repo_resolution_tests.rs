//! Tests for `repo_resolution` (issue #863).
//!
//! Wired via `#[cfg(test)] #[path]` from `repo_resolution.rs` — the file-size
//! gate counts `repo_resolution.rs` against the 500-line source budget, and
//! the test body alone exceeds it.

use super::*;
use crate::storage::sqlite::SqliteStorage;
use tempfile::TempDir;

fn git_tempdir(name: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(name)).unwrap();
    git2::Repository::init(dir.path().join(name)).unwrap();
    dir
}

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

/// Take the crate-wide env lock (serialises LIEVO_PROJECT_DIR/CLAUDE_PROJECT_DIR
/// mutations against every other env-mutating test in the crate — issue #863).
fn with_env(lievo_project_dir: Option<&str>, claude_project_dir: Option<&str>, f: impl FnOnce()) {
    let _guard = crate::test_env_support::env_lock();
    unsafe {
        std::env::remove_var("LIEVO_PROJECT_DIR");
        std::env::remove_var("CLAUDE_PROJECT_DIR");
        if let Some(v) = lievo_project_dir {
            std::env::set_var("LIEVO_PROJECT_DIR", v);
        }
        if let Some(v) = claude_project_dir {
            std::env::set_var("CLAUDE_PROJECT_DIR", v);
        }
    }
    f();
    unsafe {
        std::env::remove_var("LIEVO_PROJECT_DIR");
        std::env::remove_var("CLAUDE_PROJECT_DIR");
    }
}

#[test]
fn lievo_project_dir_overrides_claude_project_dir_and_cwd() {
    let (a, b) = (git_tempdir("env-a"), git_tempdir("env-b"));
    let root_a = a.path().join("env-a");
    let root_b = b.path().join("env-b");
    with_env(
        Some(root_a.to_str().unwrap()),
        Some(root_b.to_str().unwrap()),
        || {
            let got = resolve_project_root().expect("must resolve");
            assert_eq!(
                got.root,
                root_a.canonicalize().unwrap(),
                "LIEVO_PROJECT_DIR must win"
            );
            assert_eq!(got.source, RepoRootSource::LievoProjectDir);
        },
    );
}

#[test]
fn claude_project_dir_overrides_cwd() {
    let b = git_tempdir("env-c");
    let root_b = b.path().join("env-c");
    with_env(None, Some(root_b.to_str().unwrap()), || {
        let got = resolve_project_root().expect("must resolve");
        assert_eq!(got.root, root_b.canonicalize().unwrap());
        assert_eq!(got.source, RepoRootSource::ClaudeProjectDir);
    });
}

#[test]
fn cwd_is_used_when_both_env_vars_are_unset() {
    let c = git_tempdir("env-d");
    let root_c = c.path().join("env-d");
    with_env(None, None, || {
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root_c).unwrap();
        let got = resolve_project_root().expect("must resolve");
        std::env::set_current_dir(old).unwrap();
        assert_eq!(got.root, root_c.canonicalize().unwrap());
        assert_eq!(got.source, RepoRootSource::CurrentDir);
    });
}

#[test]
fn subdirectory_resolves_to_git_root() {
    let d = git_tempdir("sub");
    let root = d.path().join("sub");
    let sub = root.join("src").join("deep");
    std::fs::create_dir_all(&sub).unwrap();
    with_env(Some(sub.to_str().unwrap()), None, || {
        let got = resolve_project_root().expect("must resolve");
        assert_eq!(
            got.root,
            root.canonicalize().unwrap(),
            "a subdirectory must resolve to the git work-tree root"
        );
    });
}

#[test]
fn non_git_directory_is_a_not_in_git_repo_result_naming_the_directory() {
    let plain = TempDir::new().unwrap();
    let non_git = plain.path().join("not-git");
    std::fs::create_dir_all(&non_git).unwrap();
    with_env(Some(non_git.to_str().unwrap()), None, || {
        let err = resolve_project_root().expect_err("must be NotInGitRepo");
        assert_eq!(
            err.checked,
            non_git.canonicalize().unwrap(),
            "the checked directory must be canonicalised"
        );
        assert_eq!(err.source, RepoRootSource::LievoProjectDir);
    });
}

#[test]
fn var_and_private_var_paths_canonicalise_equal() {
    // macOS /tmp -> /private/tmp: a stable path under /var/folders/... must
    // canonicalise equal to the same path spelled via /private/var/folders/....
    // (TempDir is avoided because its cleanup can race with this assertion.)
    let d = git_tempdir("var-can");
    let dir_name = d.path().file_name().unwrap().to_string_lossy().to_string();
    let parent = d.path().parent().unwrap();
    let parent_name = parent.to_string_lossy().to_string();
    let Some(link_parent) = parent_name
        .strip_prefix("/private/")
        .map(|s| format!("/{s}"))
    else {
        // No /private/ symlink on this host — canonical path is already
        // the short form; equality with itself trivially holds.
        let p = d.path().join("var-can");
        assert_eq!(p.canonicalize().unwrap(), p.canonicalize().unwrap());
        return;
    };
    // Both spellings are canonicalised inside the held TempDir scope, so the
    // assertion cannot race the TempDir's drop-time cleanup.
    let unsymlinked = d.path().join("var-can").canonicalize().unwrap();
    let via_link = Path::new(&link_parent)
        .join(parent.file_name().unwrap())
        .join(&dir_name);
    let via_link = via_link.canonicalize().expect("via-link must canonicalise");
    assert_eq!(
        unsymlinked, via_link,
        "symlinked and unsymlinked spellings must canonicalise equal"
    );
}

#[test]
fn find_repo_by_path_finds_registered_repo_across_projects() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.create_project("other", None).unwrap();
    let d = git_tempdir("by-path");
    let root = d.path().join("by-path");
    let p = storage.create_project("by-path", None).unwrap();
    storage
        .add_repo(&p.id, "by-path", root.to_str().unwrap())
        .unwrap();

    let found = find_repo_by_path(&storage, root.as_path())
        .unwrap()
        .expect("must find by canonical path");
    assert_eq!(found.project_id, p.id);
}

#[test]
fn find_repo_by_path_returns_none_when_no_repo_matches() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let p = storage.create_project("solo", None).unwrap();
    storage
        .add_repo(&p.id, "solo", "/definitely/not/here")
        .unwrap();
    let d = git_tempdir("unregistered");
    assert!(
        find_repo_by_path(&storage, d.path().join("unregistered").as_path())
            .unwrap()
            .is_none()
    );
}

// -- find_repos_by_git_url (issue #31): cross-project Vec lookup --
// (Empty/no-match cases are covered at the storage layer in
// src/storage/sqlite_tests.rs; the case unique to this file is the
// cross-project multi-checkout lookup below.)

#[test]
fn find_repos_by_git_url_finds_checkouts_across_projects() {
    // The same normalized remote can be registered at several local_paths
    // across projects — the lookup must return ALL matching rows.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let key = "github.com/example/shared";

    let p1 = storage.create_project("shared-a", None).unwrap();
    let r1 = storage
        .add_repo(&p1.id, "shared-a", "/checkouts/shared-a")
        .unwrap();
    storage.set_repo_git_url(&r1.id, key).unwrap();

    let p2 = storage.create_project("shared-b", None).unwrap();
    let r2 = storage
        .add_repo(&p2.id, "shared-b", "/checkouts/shared-b")
        .unwrap();
    storage.set_repo_git_url(&r2.id, key).unwrap();

    let found = storage.find_repos_by_git_url(key).unwrap();
    assert_eq!(found.len(), 2, "both checkouts must be returned");
    let ids: Vec<&str> = found.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&r1.id.as_str()));
    assert!(ids.contains(&r2.id.as_str()));
}

#[test]
fn unregistered_repo_creates_a_fresh_single_repo_project() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d = git_tempdir("fresh");
    let root = d.path().join("fresh");

    let resolved = resolve_or_register(&storage, root.as_path()).unwrap();
    assert_eq!(resolved.project.name, "fresh");
    assert_eq!(resolved.repo.name, "fresh");
    let repos = storage.list_repos(&resolved.project.id).unwrap();
    assert_eq!(
        repos.len(),
        1,
        "auto-registered project must be single-repo"
    );

    // Second call reuses the same project and repo.
    let again = resolve_or_register(&storage, root.as_path()).unwrap();
    assert_eq!(again.project.id, resolved.project.id);
    assert_eq!(again.repo.id, resolved.repo.id);
    assert_eq!(storage.list_projects().unwrap().len(), 1);
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
fn collision_suffix_counts_up_deterministically() {
    // Issue #29 (resolved open question): no-identity registrations keep
    // today's -2/-3 suffixing (no derivable identity → no owner-prefixed
    // name to fall back to). Three identical no-remote dirs must still
    // suffix deterministically.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let (d1, d2, d3) = (git_tempdir("dup"), git_tempdir("dup"), git_tempdir("dup"));
    let r1 = d1.path().join("dup");
    let r2 = d2.path().join("dup");
    let r3 = d3.path().join("dup");

    let a = resolve_or_register(&storage, r1.as_path()).unwrap();
    let b = resolve_or_register(&storage, r2.as_path()).unwrap();
    let c = resolve_or_register(&storage, r3.as_path()).unwrap();

    assert_eq!(
        (
            a.project.name.as_str(),
            b.project.name.as_str(),
            c.project.name.as_str()
        ),
        ("dup", "dup-2", "dup-3"),
        "no-identity registrations keep today's -2/-3 suffixing"
    );
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

// ---------------------------------------------------------------------------
// Coverage-gap tests (issue #29)
// ---------------------------------------------------------------------------

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
    d.path().join("rekey").canonicalize().unwrap();
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
fn identity_bearing_twin_reuses_owner_prefixed_project() {
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
    // identity match exists (d1's repo is live) → step 3: distinct
    // <repo>@<dir> project ("twin@twin").
    let d2 = git_tempdir_identity("twin", "git@github.com:acme/twin");
    let r2 = d2.path().join("twin");
    let third = resolve_or_register(&storage, r2.as_path()).unwrap();
    assert_eq!(
        third.project.name, "twin@twin",
        "live same-identity twin is <repo>@<dir> (step 3), not a -2 suffix"
    );
    assert_ne!(third.project.id, second.project.id);
}

#[test]
fn registered_repo_under_multi_repo_project_is_found_by_path() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d = git_tempdir("multi");
    let root = d.path().join("multi");
    let p = storage.create_project("multi", None).unwrap();
    storage
        .add_repo(&p.id, "multi", root.to_str().unwrap())
        .unwrap();
    storage
        .add_repo(&p.id, "other-repo", "/somewhere/else")
        .unwrap();

    let resolved = resolve_or_register(&storage, root.as_path()).unwrap();
    assert_eq!(resolved.project.id, p.id);
    assert_eq!(storage.list_repos(&p.id).unwrap().len(), 2);
    assert_eq!(resolved.repo.name, "multi");
}

#[test]
fn many_projects_in_db_still_resolve_by_path() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    for i in 0..10 {
        let p = storage.create_project(&format!("proj-{i}"), None).unwrap();
        storage
            .add_repo(&p.id, &format!("r-{i}"), &format!("/noise/{i}"))
            .unwrap();
    }
    let d = git_tempdir("needle");
    let root = d.path().join("needle");
    let p = storage.create_project("needle", None).unwrap();
    storage
        .add_repo(&p.id, "needle", root.to_str().unwrap())
        .unwrap();

    let resolved = resolve_or_register(&storage, root.as_path()).unwrap();
    assert_eq!(resolved.project.id, p.id);
}

#[test]
fn canonical_stored_path_matches_non_canonical_query() {
    // Canonicalise both sides: stored canonical path matches symlinked query.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let d = git_tempdir("canon");
    let root = d.path().join("canon");
    let stored = root.canonicalize().unwrap();
    let p = storage.create_project("canon", None).unwrap();
    storage
        .add_repo(&p.id, "canon", stored.to_str().unwrap())
        .unwrap();
    assert!(
        find_repo_by_path(&storage, root.as_path())
            .unwrap()
            .is_some()
    );
}

// -- serve()-level guidance (issue #863 binding decisions 4 & 5) --

/// A zero-repo project reached via explicit PROJECT must produce
/// `zero_repo_guidance` text, not the generic "run lievo refresh" message
/// (binding decision 4). This is the tool-layer contract: `ToolContext`
/// carries the guidance, and `lievo_explore` returns it verbatim.
#[test]
fn zero_repo_project_yields_specific_guidance_not_generic_refresh() {
    use crate::retrieval::tool_trait::Tool;
    use crate::retrieval::tools::{ExploreTool, ToolContext};
    use std::sync::{Arc, Mutex};

    let storage = SqliteStorage::open_in_memory().unwrap();
    let p = storage.create_project("empty", None).unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: p.id,
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: Some(
            "No repository is registered for project 'empty'. \
             Run `lievo admin add-repo <path>` to register one."
                .to_string(),
        ),
    });
    let tool = ExploreTool { ctx };
    let out = tool
        .call(serde_json::json!({ "query": "anything" }))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    let warning = parsed["warning"].as_str().unwrap();
    assert!(
        warning.contains("No repository is registered"),
        "must name the no-repo condition, not the generic refresh message"
    );
    assert!(
        !warning.contains("run `lievo refresh`"),
        "must not be the generic refresh guidance"
    );
    assert!(parsed["symbols"].as_array().unwrap().is_empty());
}

/// Outside a git repo, the server still starts and `lievo_explore` returns
/// success-shaped guidance naming the directory checked (binding decision 4).
#[test]
fn non_git_directory_yields_guidance_naming_the_directory() {
    use crate::retrieval::tool_trait::Tool;
    use crate::retrieval::tools::{ExploreTool, ToolContext};
    use std::sync::{Arc, Mutex};

    let plain = TempDir::new().unwrap();
    let non_git = plain.path().join("not-git");
    std::fs::create_dir_all(&non_git).unwrap();
    let checked = non_git.canonicalize().unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: String::new(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: Some(format!(
            "The directory {dir} (checked for a git repository) is not inside a git repository. \
             lievo works inside a git repository.",
            dir = checked.display()
        )),
    });
    let tool = ExploreTool { ctx };
    let out = tool.call(serde_json::json!({ "query": "x" })).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    let warning = parsed["warning"].as_str().unwrap();
    assert!(
        warning.contains(checked.to_str().unwrap()),
        "guidance must name the directory checked: {warning}"
    );
    assert!(warning.contains("not inside a git repository"));
}

// -- resolve_project_root_from (issue #865 refactor of resolve_project_root) --

#[test]
fn resolve_project_root_from_uses_start_dir_when_no_env_vars_set() {
    let d = git_tempdir("from-start");
    let root = d.path().join("from-start");
    let outside = TempDir::new().unwrap();
    let other = outside.path().join("elsewhere");
    std::fs::create_dir_all(&other).unwrap();
    with_env(None, None, || {
        let got = resolve_project_root_from(&root).expect("must resolve");
        assert_eq!(got.root, root.canonicalize().unwrap());
        assert_eq!(got.source, RepoRootSource::CurrentDir);
    });
}

#[test]
fn resolve_project_root_from_env_precedence_over_start_dir() {
    let (a, b) = (git_tempdir("from-env-a"), git_tempdir("from-env-b"));
    let root_a = a.path().join("from-env-a");
    let root_b = b.path().join("from-env-b");
    // LIEVO_PROJECT_DIR beats CLAUDE_PROJECT_DIR beats start_dir.
    with_env(
        Some(root_a.to_str().unwrap()),
        Some(root_b.to_str().unwrap()),
        || {
            let got = resolve_project_root_from(&root_b).expect("must resolve");
            assert_eq!(got.root, root_a.canonicalize().unwrap());
            assert_eq!(got.source, RepoRootSource::LievoProjectDir);
        },
    );
    // CLAUDE_PROJECT_DIR beats start_dir when LIEVO_PROJECT_DIR is unset.
    with_env(None, Some(root_b.to_str().unwrap()), || {
        let got = resolve_project_root_from(&root_a).expect("must resolve");
        assert_eq!(got.root, root_b.canonicalize().unwrap());
        assert_eq!(got.source, RepoRootSource::ClaudeProjectDir);
    });
}

#[test]
fn resolve_project_root_from_subdirectory_walks_to_git_root() {
    let d = git_tempdir("from-sub");
    let root = d.path().join("from-sub");
    let sub = root.join("src").join("deep");
    std::fs::create_dir_all(&sub).unwrap();
    with_env(None, None, || {
        let got = resolve_project_root_from(&sub).expect("must resolve");
        assert_eq!(got.root, root.canonicalize().unwrap());
        assert_eq!(got.source, RepoRootSource::CurrentDir);
    });
}

#[test]
fn resolve_project_root_from_non_git_dir_is_not_in_git_repo() {
    let plain = TempDir::new().unwrap();
    let non_git = plain.path().join("from-not-git");
    std::fs::create_dir_all(&non_git).unwrap();
    with_env(None, None, || {
        let err = resolve_project_root_from(&non_git).expect_err("must be NotInGitRepo");
        assert_eq!(err.checked, non_git.canonicalize().unwrap());
        assert_eq!(err.source, RepoRootSource::CurrentDir);
    });
}

#[test]
fn resolve_project_root_matches_resolve_project_root_from_cwd() {
    let d = git_tempdir("from-cwd");
    let root = d.path().join("from-cwd");
    with_env(None, None, || {
        let old = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();
        let from_cwd = resolve_project_root().expect("must resolve");
        let explicit = resolve_project_root_from(&root).expect("must resolve");
        std::env::set_current_dir(old).unwrap();
        assert_eq!(from_cwd, explicit, "both entry points must agree on cwd");
    });
}
