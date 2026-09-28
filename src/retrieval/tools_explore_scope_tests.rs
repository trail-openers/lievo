// Scope-membership listing tests (issue #712).
//
// Wired via `#[path]` from tools_explore_scope.rs. Covers:
//   - is_within_scope predicate (slash-anchored containment)
//   - scope listing correctness + sorted determinism vs a fixture tree
//   - "."/"./" normalize to the empty/invalid-scope path (round-2 review)
//   - maybe_scope_listing dispatch gate
//
// Storage-backed listing behaviour (continuation/paging, 24K truncation,
// unknown/excluded-scope guidance, path traversal, storage-error
// propagation, offset-past-end warning) lives in the sibling
// `tools_explore_scope_listing_tests.rs`, split out to keep both files
// under the 800-line test-file budget.

use std::sync::{Arc, Mutex};

use crate::model::{Entity, EntityTier};
use crate::retrieval::tools::ToolContext;
use crate::retrieval::tools_explore_scope::{is_within_scope, maybe_scope_listing, scope_listing};
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use serde_json::json;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn file_entity(id: &str, project_id: &str, repo_id: &str, name: &str, path: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: name.to_string(),
        path: Some(path.to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn setup_project_with_repo(name: &str) -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project(name, None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    (storage, project.id, repo.id)
}

fn make_ctx(
    storage: SqliteStorage,
    project_id: String,
    output_dir: Option<String>,
) -> Arc<ToolContext<SqliteStorage>> {
    Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id,
        repo_path: std::path::PathBuf::new(),
        output_dir,
        zero_repo_guidance: None,
    })
}

// ---------------------------------------------------------------------------
// is_within_scope
// ---------------------------------------------------------------------------

#[test]
fn is_within_scope_matches_exact_and_nested() {
    assert!(is_within_scope("src", "src"));
    assert!(is_within_scope("src/main.rs", "src"));
    assert!(is_within_scope("src/retrieval/tools.rs", "src/retrieval"));
}

#[test]
fn is_within_scope_rejects_sibling_prefix_collision() {
    // "src" must not match "src2/" or "srcx/" — slash-anchored, not a naive
    // starts_with.
    assert!(!is_within_scope("src2/main.rs", "src"));
    assert!(!is_within_scope("srcx/main.rs", "src"));
    assert!(!is_within_scope("src_extra/main.rs", "src"));
}

// ---------------------------------------------------------------------------
// scope_listing: correctness + determinism
// ---------------------------------------------------------------------------

#[test]
fn scope_listing_returns_sorted_files_under_prefix() {
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-basic");
    // Insert in scrambled order to prove the sort is on path, not insertion.
    storage
        .upsert_entity(&file_entity(
            "f3",
            &project_id,
            &repo_id,
            "c.rs",
            "src/c.rs",
        ))
        .unwrap();
    storage
        .upsert_entity(&file_entity(
            "f1",
            &project_id,
            &repo_id,
            "a.rs",
            "src/a.rs",
        ))
        .unwrap();
    storage
        .upsert_entity(&file_entity(
            "f2",
            &project_id,
            &repo_id,
            "b.rs",
            "src/b.rs",
        ))
        .unwrap();
    // Sibling directory that must NOT be included under scope "src".
    storage
        .upsert_entity(&file_entity(
            "f4",
            &project_id,
            &repo_id,
            "other.rs",
            "src_extra/other.rs",
        ))
        .unwrap();

    let ctx = make_ctx(storage, project_id, None);
    let guard = ctx.storage.lock().unwrap();
    let out = scope_listing(&*guard, &ctx, "src", 0, 8).unwrap();
    drop(guard);

    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["src/a.rs", "src/b.rs", "src/c.rs"]);
    assert_eq!(v["returned"], json!(3));
    assert_eq!(v["total"], json!(3));
    assert!(v.get("continuation").is_none());
}

#[test]
fn scope_listing_entries_carry_entity_id_and_path_only() {
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-shape");
    storage
        .upsert_entity(&file_entity(
            "f1",
            &project_id,
            &repo_id,
            "a.rs",
            "src/a.rs",
        ))
        .unwrap();

    let ctx = make_ctx(storage, project_id, None);
    let guard = ctx.storage.lock().unwrap();
    let out = scope_listing(&*guard, &ctx, "src", 0, 8).unwrap();
    drop(guard);

    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let entry = &v["files"][0];
    assert_eq!(entry["entity_id"], json!("f1"));
    assert_eq!(entry["path"], json!("src/a.rs"));
    // No symbol-building fields must leak into scope-listing entries.
    assert!(entry.get("signature").is_none());
    assert!(entry.get("call_paths").is_none());
    assert!(entry.get("blast_radius").is_none());
    assert!(entry.get("source").is_none());
}

// ---------------------------------------------------------------------------
// Trailing slash / leading "./" normalization
// ---------------------------------------------------------------------------

#[test]
fn scope_listing_normalizes_trailing_slash_and_dot_prefix() {
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-normalize");
    storage
        .upsert_entity(&file_entity(
            "f1",
            &project_id,
            &repo_id,
            "a.rs",
            "src/a.rs",
        ))
        .unwrap();

    let ctx = make_ctx(storage, project_id, None);
    let guard = ctx.storage.lock().unwrap();

    let with_slash = scope_listing(&*guard, &ctx, "src/", 0, 8).unwrap();
    let with_dot = scope_listing(&*guard, &ctx, "./src", 0, 8).unwrap();
    drop(guard);

    let v1: serde_json::Value = serde_json::from_str(&with_slash).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&with_dot).unwrap();
    assert_eq!(v1["returned"], json!(1));
    assert_eq!(v2["returned"], json!(1));
}

// ---------------------------------------------------------------------------
// maybe_scope_listing dispatch gate
// ---------------------------------------------------------------------------

#[test]
fn maybe_scope_listing_none_when_scope_absent_or_blank() {
    let (storage, project_id, _repo_id) = setup_project_with_repo("scope-dispatch-absent");
    let ctx = make_ctx(storage, project_id, None);

    assert!(maybe_scope_listing(&ctx, &json!({"query": "auth"}), 8).is_none());
    assert!(maybe_scope_listing(&ctx, &json!({"query": "auth", "scope": ""}), 8).is_none());
    assert!(maybe_scope_listing(&ctx, &json!({"query": "auth", "scope": "   "}), 8).is_none());
}

#[test]
fn maybe_scope_listing_some_when_scope_present() {
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-dispatch-present");
    storage
        .upsert_entity(&file_entity(
            "f1",
            &project_id,
            &repo_id,
            "a.rs",
            "src/a.rs",
        ))
        .unwrap();
    let ctx = make_ctx(storage, project_id, None);

    let result = maybe_scope_listing(&ctx, &json!({"query": "", "scope": "src"}), 8);
    assert!(result.is_some());
    let out = result.unwrap().unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["returned"], json!(1));
}

// ---------------------------------------------------------------------------
// "." / "./" normalize to the empty/invalid-scope path (round-2 review)
// ---------------------------------------------------------------------------

#[test]
fn scope_listing_dot_and_dot_slash_take_the_empty_scope_path() {
    // Round-2 review finding: `scope="."` and `scope="./"` previously
    // normalized to a non-empty ".", skipping the empty-scope guard and
    // falling through to a confusing "No indexed files under '.'" warning.
    // Both must now take the same invalid/absent-scope guidance path as "".
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-dot");
    storage
        .upsert_entity(&file_entity(
            "f1",
            &project_id,
            &repo_id,
            "a.rs",
            "src/a.rs",
        ))
        .unwrap();
    let ctx = make_ctx(storage, project_id, None);
    let guard = ctx.storage.lock().unwrap();

    for scope in [".", "./"] {
        let out = scope_listing(&*guard, &ctx, scope, 0, 8).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["files"],
            json!([]),
            "scope {scope:?} must return no files"
        );
        assert!(
            v["warning"]
                .as_str()
                .unwrap()
                .contains("empty scope after normalization"),
            "scope {scope:?} must take the empty-scope guidance path, got: {out}"
        );
    }
}

#[test]
fn scope_listing_empty_and_blank_scope_take_the_empty_scope_path() {
    let (storage, project_id, _repo_id) = setup_project_with_repo("scope-blank");
    let ctx = make_ctx(storage, project_id, None);
    let guard = ctx.storage.lock().unwrap();

    for scope in ["", "   "] {
        let out = scope_listing(&*guard, &ctx, scope, 0, 8).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["files"], json!([]));
        assert!(
            v["warning"]
                .as_str()
                .unwrap()
                .contains("empty scope after normalization"),
            "scope {scope:?} must take the empty-scope guidance path, got: {out}"
        );
    }
}

#[test]
fn scope_listing_dotted_mid_path_segment_is_not_treated_as_empty() {
    // "src/./x" is a literal (if odd) scope value carrying a mid-path "."
    // segment — unlike a bare "."/"./", it must NOT be treated as empty; it
    // simply matches no indexed file (a normal empty-result guidance, not
    // the "empty scope after normalization" guard).
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-dotted-midpath");
    storage
        .upsert_entity(&file_entity(
            "f1",
            &project_id,
            &repo_id,
            "a.rs",
            "src/a.rs",
        ))
        .unwrap();
    let ctx = make_ctx(storage, project_id, None);
    let guard = ctx.storage.lock().unwrap();

    let out = scope_listing(&*guard, &ctx, "src/./x", 0, 8).unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["files"], json!([]));
    assert!(
        !v["warning"]
            .as_str()
            .unwrap()
            .contains("empty scope after normalization"),
        "'src/./x' must not be treated as an empty scope, got: {out}"
    );
    assert!(v["warning"].as_str().unwrap().contains("src/./x"));
}
