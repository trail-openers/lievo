// Scope-listing storage-backed behaviour tests (issue #712, split out of
// tools_explore_scope_tests.rs to satisfy the 800-line test-file budget).
//
// Wired via `#[path]` from tools_explore_scope.rs. Covers:
//   - continuation on large scopes (offset-based paging)
//   - 24K output cap truncation with non-null stubs
//   - unknown/excluded-scope success-shaped guidance
//   - path traversal rejection
//   - storage errors propagate instead of being swallowed (round-2 review)
//   - out-of-range offset carries a distinguishing warning (round-2 review)

use std::sync::{Arc, Mutex};

use crate::model::{Entity, EntityTier};
use crate::retrieval::tools::ToolContext;
use crate::retrieval::tools_explore_scope::scope_listing;
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
// Continuation on large scopes
// ---------------------------------------------------------------------------

#[test]
fn scope_listing_continuation_pages_through_offset() {
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-continue");
    for i in 0..10 {
        let path = format!("src/f{i:02}.rs");
        storage
            .upsert_entity(&file_entity(
                &format!("f{i}"),
                &project_id,
                &repo_id,
                &format!("f{i:02}.rs"),
                &path,
            ))
            .unwrap();
    }

    let ctx = make_ctx(storage, project_id, None);

    let page1 = {
        let guard = ctx.storage.lock().unwrap();
        scope_listing(&*guard, &ctx, "src", 0, 4).unwrap()
    };
    let v1: serde_json::Value = serde_json::from_str(&page1).unwrap();
    assert_eq!(v1["returned"], json!(4));
    assert_eq!(v1["total"], json!(10));
    assert!(v1["continuation"].as_str().unwrap().contains("offset=4"));

    let page2 = {
        let guard = ctx.storage.lock().unwrap();
        scope_listing(&*guard, &ctx, "src", 4, 4).unwrap()
    };
    let v2: serde_json::Value = serde_json::from_str(&page2).unwrap();
    let page2_paths: Vec<&str> = v2["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        page2_paths,
        vec!["src/f04.rs", "src/f05.rs", "src/f06.rs", "src/f07.rs"]
    );

    let page3 = {
        let guard = ctx.storage.lock().unwrap();
        scope_listing(&*guard, &ctx, "src", 8, 4).unwrap()
    };
    let v3: serde_json::Value = serde_json::from_str(&page3).unwrap();
    assert_eq!(v3["returned"], json!(2));
    assert!(v3.get("continuation").is_none());
}

// ---------------------------------------------------------------------------
// 24K output cap: scope-listing entries are dropped whole (issue #836)
// ---------------------------------------------------------------------------

#[test]
fn scope_listing_over_24k_truncates_with_non_null_stubs() {
    // Scope-listing entries are objects carrying at minimum `path` +
    // `entity_id`. The value-level cap drops whole entries from the end of
    // the `files` array until the response fits, preserving the `files`
    // key and the structure of surviving entries (issue #836).
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-cap");
    // Long repo-relative paths push the serialized response past the 24K
    // char cap (issue #680/#711 MAX_EXPLORE_OUTPUT_CHARS).
    for i in 0..400 {
        let path = format!(
            "src/generated_module_{i:04}_with_a_long_descriptive_name_for_cap_testing/file.rs"
        );
        storage
            .upsert_entity(&file_entity(
                &format!("f{i}"),
                &project_id,
                &repo_id,
                &format!("file_{i:04}.rs"),
                &path,
            ))
            .unwrap();
    }

    let ctx = make_ctx(storage, project_id, None);
    let guard = ctx.storage.lock().unwrap();
    // max_files large enough that all 400 entries attempt to serialize
    // before the cap kicks in.
    let out = scope_listing(&*guard, &ctx, "src", 0, 400).unwrap();
    drop(guard);

    assert!(
        out.chars().count() <= crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS,
        "capped output must not exceed the 24K char cap, got {} chars",
        out.chars().count()
    );

    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["truncated"], json!(true));
    let files = v["files"]
        .as_array()
        .expect("capped response must carry a files array");
    assert!(!files.is_empty(), "files array must not be empty");
    for f in files {
        assert!(
            !f["entity_id"].is_null(),
            "entry entity_id must not be null, got: {f}"
        );
    }
}

#[test]
fn scope_listing_pathologically_long_path_stubs_stay_under_cap() {
    // ISSUE #1 (round-1 adversarial): a single entry with a ~10K-char path
    // must not defeat cap_response's 24K invariant on its own. Two matched
    // entities carry a pathologically long `path`/`entity_id`; the value-
    // level cap drops whole entries from the end (or shrinks a single
    // oversized entry field-by-field) so the final payload still fits.
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-cap-long-path");
    let long_segment = "a".repeat(10_000);
    for i in 0..2 {
        let path = format!("src/{long_segment}_{i}/file.rs");
        storage
            .upsert_entity(&file_entity(
                &format!("f{i}_{long_segment}"),
                &project_id,
                &repo_id,
                &format!("file_{i}.rs"),
                &path,
            ))
            .unwrap();
    }

    let ctx = make_ctx(storage, project_id, None);
    let guard = ctx.storage.lock().unwrap();
    let out = scope_listing(&*guard, &ctx, "src", 0, 8).unwrap();
    drop(guard);

    assert!(
        out.chars().count() <= crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS,
        "capped output must not exceed the 24K char cap even with pathologically \
         long single paths, got {} chars",
        out.chars().count()
    );

    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["truncated"],
        json!(true),
        "two 10K-char paths must trigger the truncation branch, got: {out}"
    );
    let files = v["files"].as_array().unwrap();
    assert!(!files.is_empty(), "files array must not be empty");
    for f in files {
        assert!(!f["entity_id"].is_null());
    }
}

// ---------------------------------------------------------------------------
// Unknown / excluded scope guidance (success-shaped, is_error false)
// ---------------------------------------------------------------------------

#[test]
fn scope_listing_unknown_scope_returns_success_shaped_guidance() {
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-unknown");
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
    let out = scope_listing(&*guard, &ctx, "no_such_dir", 0, 8).unwrap();
    drop(guard);

    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["files"], json!([]));
    assert_eq!(v["returned"], json!(0));
    assert_eq!(v["total"], json!(0));
    assert!(v["warning"].as_str().unwrap().contains("no_such_dir"));
}

#[test]
fn scope_listing_all_excluded_by_output_dir_returns_guidance() {
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-excluded");
    storage
        .upsert_entity(&file_entity(
            "f1",
            &project_id,
            &repo_id,
            "readme.md",
            "docs_out/readme.md",
        ))
        .unwrap();

    let ctx = make_ctx(storage, project_id, Some("docs_out".to_string()));
    let guard = ctx.storage.lock().unwrap();
    let out = scope_listing(&*guard, &ctx, "docs_out", 0, 8).unwrap();
    drop(guard);

    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["files"], json!([]));
    assert!(v["warning"].as_str().unwrap().contains("excluded"));
}

// ---------------------------------------------------------------------------
// Path traversal rejection
// ---------------------------------------------------------------------------

#[test]
fn scope_listing_rejects_path_traversal() {
    let (storage, project_id, _repo_id) = setup_project_with_repo("scope-traversal");
    let ctx = make_ctx(storage, project_id, None);
    let guard = ctx.storage.lock().unwrap();

    for bad_scope in ["../secret", "/etc/passwd", "src/../../etc"] {
        let out = scope_listing(&*guard, &ctx, bad_scope, 0, 8).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["files"], json!([]));
        assert!(v["warning"].as_str().unwrap().contains("traversal"));
    }
}

// ---------------------------------------------------------------------------
// Storage errors propagate instead of being swallowed (round-2 review)
// ---------------------------------------------------------------------------

/// Self-contained stub of the `Storage` trait: `list_entities` always fails
/// with a synthetic `RetrievalError`, mirroring `FailingRepoStorage` in
/// `bin/lievo/commands/project/query_ops.rs`. Every other method panics with
/// `unimplemented!` — `scope_listing` must never reach them once
/// `list_entities` errors.
struct FailingStorage;

impl Storage for FailingStorage {
    fn create_project(&self, _: &str, _: Option<&str>) -> crate::Result<crate::model::Project> {
        unimplemented!()
    }
    fn get_project(&self, _: &str) -> crate::Result<Option<crate::model::Project>> {
        unimplemented!()
    }
    fn get_project_by_id(&self, _: &str) -> crate::Result<Option<crate::model::Project>> {
        unimplemented!()
    }
    fn list_projects(&self) -> crate::Result<Vec<crate::model::Project>> {
        unimplemented!()
    }
    fn delete_project(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn add_output_dir(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_output_dirs(&self, _: &str) -> crate::Result<Vec<String>> {
        unimplemented!()
    }
    fn add_repo(&self, _: &str, _: &str, _: &str) -> crate::Result<crate::model::Repository> {
        unimplemented!()
    }
    fn get_repo(&self, _: &str) -> crate::Result<Option<crate::model::Repository>> {
        unimplemented!()
    }
    fn list_repos(&self, _: &str) -> crate::Result<Vec<crate::model::Repository>> {
        unimplemented!()
    }
    fn update_repo_index_path(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_last_commit(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn update_repo_project(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn delete_repo(&self, _: &str) -> crate::Result<crate::storage::DeleteStats> {
        unimplemented!()
    }
    fn upsert_entity(&self, _: &Entity) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_entity_summary(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_entity(&self, _: &str) -> crate::Result<Option<Entity>> {
        unimplemented!()
    }
    fn list_entities(&self, _: &str, _: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
        Err(crate::LievoError::RetrievalError(
            "synthetic storage fault (index poisoned)".into(),
        ))
    }
    fn search_entities_by_name(
        &self,
        _: &str,
        _: &[&str],
        _: usize,
        _: Option<&str>,
    ) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entities_by_repo(&self, _: &str, _: Option<EntityTier>) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entities_by_parent(&self, _: &str) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn entity_by_path(&self, _: &str, _: &str) -> crate::Result<Option<Entity>> {
        unimplemented!()
    }
    fn entity_ids_for_paths(
        &self,
        _: &str,
        _: &[&str],
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        unimplemented!()
    }
    fn entity_by_path_projectwide(&self, _: &str, _: &str) -> crate::Result<Vec<Entity>> {
        unimplemented!()
    }
    fn delete_entities_by_repo(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn delete_entities_by_paths(&self, _: &str, _: &[String]) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_relationship(&self, _: &crate::model::Relationship) -> crate::Result<()> {
        unimplemented!()
    }
    fn relationships_from(
        &self,
        _: &str,
    ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
        unimplemented!()
    }
    fn relationships_to(
        &self,
        _: &str,
    ) -> crate::Result<Vec<(crate::model::Relationship, Entity)>> {
        unimplemented!()
    }
    fn delete_relationships_by_source(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
    fn upsert_insight(&self, _: &crate::model::Insight) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_insights(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: usize,
    ) -> crate::Result<Vec<crate::model::Insight>> {
        unimplemented!()
    }
    fn invalidate_insights(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn upsert_convention(&self, _: &crate::model::Convention) -> crate::Result<()> {
        unimplemented!()
    }
    fn list_conventions(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> crate::Result<Vec<crate::model::Convention>> {
        unimplemented!()
    }
    fn create_analysis_run(&self, _: &str, _: &str) -> crate::Result<crate::model::AnalysisRun> {
        unimplemented!()
    }
    fn update_analysis_run(&self, _: &crate::model::AnalysisRun) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_file_hash(&self, _: &str, _: &str) -> crate::Result<Option<String>> {
        unimplemented!()
    }
    fn upsert_file_hash(&self, _: &str, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn get_all_file_hashes(
        &self,
        _: &str,
    ) -> crate::Result<std::collections::HashMap<String, String>> {
        unimplemented!()
    }
    fn delete_file_hash(&self, _: &str, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn persist_analysis_batch(
        &self,
        _: &[&Entity],
        _: &[crate::model::Relationship],
        _: &crate::model::AnalysisRun,
        _: &str,
        _: &str,
    ) -> crate::Result<(i64, i64)> {
        unimplemented!()
    }
    fn reconcile_entities(
        &self,
        _: &str,
        _: &str,
        _: &std::path::Path,
    ) -> crate::Result<crate::storage::reconcile::ReconcileStats> {
        unimplemented!()
    }
    fn clear_all_summaries(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn clear_repo_summaries(&self, _: &str) -> crate::Result<()> {
        unimplemented!()
    }
    fn count_missing_summaries(&self, _: &str) -> crate::Result<u64> {
        unimplemented!()
    }
}

#[test]
fn scope_listing_propagates_storage_error_instead_of_swallowing_it() {
    // Round-2 review finding: `list_entities(...).unwrap_or_default()`
    // previously swallowed a real storage fault (DB failure, poisoned
    // index) and reported it as "no files indexed — run lievo refresh",
    // misdirecting the agent. A storage fault is not a recoverable
    // condition (#680's success-shaped contract does not cover it) and
    // must propagate as `Err`.
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(FailingStorage)),
        project_id: "p1".to_string(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });

    let result = scope_listing(&FailingStorage, &ctx, "src", 0, 8);
    assert!(
        result.is_err(),
        "a real storage fault must propagate as Err, not success-shaped guidance"
    );
}

// ---------------------------------------------------------------------------
// Out-of-range offset carries a distinguishing warning (round-2 review)
// ---------------------------------------------------------------------------

#[test]
fn scope_listing_offset_past_end_carries_distinguishing_warning() {
    // Round-2 review finding: an out-of-range offset (offset >= total,
    // total > 0) previously returned a bare empty page indistinguishable
    // from a genuinely-empty scope. It must now carry a warning naming the
    // paged-past-the-end condition.
    let (storage, project_id, repo_id) = setup_project_with_repo("scope-offset-overrun");
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

    let out = scope_listing(&*guard, &ctx, "src", 5, 8).unwrap();
    drop(guard);

    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["files"], json!([]));
    assert_eq!(v["returned"], json!(0));
    assert_eq!(v["total"], json!(1));
    assert!(
        v["warning"].as_str().unwrap().contains("past the end"),
        "out-of-range offset must carry a distinguishing warning, got: {out}"
    );
}
