//! Scope-membership listing tests for the `lievo_explore` MCP tool (issue #712).
//!
//! Wired via `#[cfg(test)] #[path]` from `tools.rs` (sibling of the
//! `explore_tests` module there, hence the `super::explore_tests::` imports).
//! Split out of `explore_tests.rs` under the 800-line test-file budget
//! (issue #702 policy, scripts/check_file_size.rs).

use std::path::PathBuf;

use super::explore_tests::{assert_success_shaped, file_entity};
use crate::mcp::LievoMcpServer;
use crate::mcp::params::ExploreParams;
use crate::retrieval::tools::ToolContext;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;
use rmcp::handler::server::wrapper::Parameters;
use std::sync::{Arc, Mutex};
#[tokio::test]
async fn lievo_explore_scope_mode_lists_indexed_files_under_prefix() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    for (id, name, path) in [
        ("p:repo1:file:src/b.rs", "b", "src/b.rs"),
        ("p:repo1:file:src/a.rs", "a", "src/a.rs"),
        ("p:repo1:file:other/c.rs", "c", "other/c.rs"),
    ] {
        storage
            .upsert_entity(&file_entity(id, &project.id, name, path, None))
            .unwrap();
    }
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);

    let result = server
        .lievo_explore(Parameters(ExploreParams {
            query: "unused".into(),
            include_depth: true,
            scope: Some("src".into()),
            ..Default::default()
        }))
        .await
        .expect("scope listing must be success-shaped");
    let body = assert_success_shaped(&result);
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();

    let files = parsed["files"].as_array().unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        vec!["src/a.rs", "src/b.rs"],
        "scope listing must be sorted, repo-relative, and exclude the sibling `other/` dir"
    );
    assert_eq!(parsed["returned"], 2);
    assert_eq!(parsed["total"], 2);
    // No symbol-building fields anywhere in scope-listing entries.
    assert!(files[0].get("signature").is_none());
    assert!(files[0].get("call_paths").is_none());
    assert!(files[0].get("source").is_none());
}

#[tokio::test]
async fn lievo_explore_scope_mode_unknown_scope_returns_success_shaped_guidance() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:src/a.rs",
            &project.id,
            "a",
            "src/a.rs",
            None,
        ))
        .unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);

    let result = server
        .lievo_explore(Parameters(ExploreParams {
            query: "unused".into(),
            include_depth: true,
            scope: Some("no_such_dir".into()),
            ..Default::default()
        }))
        .await
        .expect("unknown scope must be success-shaped, not an MCP error");
    let body = assert_success_shaped(&result);
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(parsed["files"], serde_json::json!([]));
    assert_eq!(parsed["returned"], 0);
    assert_eq!(parsed["total"], 0);
    assert!(
        parsed["warning"].as_str().unwrap().contains("no_such_dir"),
        "got: {}",
        parsed["warning"]
    );
}

#[tokio::test]
async fn lievo_explore_scope_mode_paginates_via_offset() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    for i in 0..6 {
        let path = format!("src/f{i:02}.rs");
        storage
            .upsert_entity(&file_entity(
                &format!("p:repo1:file:{path}"),
                &project.id,
                &format!("f{i:02}"),
                &path,
                None,
            ))
            .unwrap();
    }
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);

    let page1 = server
        .lievo_explore(Parameters(ExploreParams {
            query: "unused".into(),
            max_files: 4,
            include_depth: true,
            scope: Some("src".into()),
            ..Default::default()
        }))
        .await
        .unwrap();
    let v1: serde_json::Value = serde_json::from_str(assert_success_shaped(&page1)).unwrap();
    assert_eq!(v1["returned"], 4);
    assert_eq!(v1["total"], 6);
    assert!(v1["continuation"].as_str().unwrap().contains("offset=4"));

    let page2 = server
        .lievo_explore(Parameters(ExploreParams {
            query: "unused".into(),
            max_files: 4,
            include_depth: true,
            scope: Some("src".into()),
            offset: Some(4),
            ..Default::default()
        }))
        .await
        .unwrap();
    let v2: serde_json::Value = serde_json::from_str(assert_success_shaped(&page2)).unwrap();
    let paths: Vec<&str> = v2["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["src/f04.rs", "src/f05.rs"]);
    assert!(v2.get("continuation").is_none());
}

#[tokio::test]
async fn lievo_explore_word_match_unaffected_when_scope_absent() {
    // Non-regression (issue #712 AC): with `scope` unset, a path-like query
    // ("src") must still go through the word-match path unchanged, even
    // though "src" is also a valid scope prefix.
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("p", None).unwrap();
    storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    storage
        .upsert_entity(&file_entity(
            "p:repo1:file:src/main.rs",
            &project.id,
            "main",
            "src/main.rs",
            None,
        ))
        .unwrap();
    let ctx = Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id,
        repo_path: PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let server = LievoMcpServer::new(ctx);

    let result = server
        .lievo_explore(Parameters(ExploreParams {
            query: "src".into(),
            include_depth: true,
            ..Default::default()
        }))
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(assert_success_shaped(&result)).unwrap();
    assert!(
        parsed.get("symbols").is_some(),
        "expected the word-match `symbols` shape, got: {parsed}"
    );
    assert!(
        parsed.get("files").is_none(),
        "scope-listing `files` key must not appear when scope is absent, got: {parsed}"
    );
}
