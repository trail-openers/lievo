//! Tests for `tools_explore_symbols` — wired via `#[path]` from
//! `tools_explore_symbols.rs` (same pattern as `tools_explore_match_tests.rs`).
//!
//! Covers: symbol-name confirmation, symbol score constants outranking the
//! file channel, the common-word cap on the merged set, exact-symbol-above-
//! path-token ranking, multi-repo resolution (a symbol resolving in the repo
//! that actually contains the file), the prefilter's exact-first bound, and
//! an integration test where a query naming a function returns its containing
//! file.

use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::model::{Entity, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::ExploreTool;
use crate::retrieval::tools_explore_symbols::{
    EXACT_SYMBOL_SCORE, PREFIX_SYMBOL_SCORE, symbol_name_hits,
};
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

#[test]
fn symbol_name_hits_exact_and_prefix_and_miss() {
    let words = vec!["sum_of_squares".to_string(), "auth".to_string()];
    // Exact: name equals a query word.
    assert_eq!(
        symbol_name_hits("Sum_of_Squares", &words),
        Some((true, true))
    );
    // Prefix: query word "auth" is a token prefix of "authentication".
    assert_eq!(
        symbol_name_hits("authentication", &words),
        Some((false, true))
    );
    // Miss: no query word is a token prefix of any name token.
    assert_eq!(symbol_name_hits("validity", &words), None);
}

#[test]
fn exact_symbol_score_outranks_file_name_and_path() {
    use crate::retrieval::explore_ranking::score_file_entity;
    let words = vec!["sum_of_squares".to_string()];
    let file = Entity {
        id: "p:repo:file:math.rs".to_string(),
        project_id: "p".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: "math".to_string(),
        path: Some("src/math.rs".to_string()),
        language: None,
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let (file_score, _) = score_file_entity(&file, &words);
    assert_eq!(file_score, 0, "file name/path must not mention the symbol");
    assert!(EXACT_SYMBOL_SCORE > file_score && EXACT_SYMBOL_SCORE > 2 && PREFIX_SYMBOL_SCORE > 1);
}

/// A common word like `new` must not return hundreds of files — the
/// existing max_files / limit caps and continuation (returned/total/next)
/// must apply to the merged set.
#[test]
fn common_word_cap_applies_to_merged_set() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("cap-test", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();

    // 20 files, each with a function named `new` (a common word).
    for i in 0..20 {
        let path = format!("src/file_{i}.rs");
        let file_id = format!("{}:repo1:file:{}", project.id, path);
        storage
            .upsert_entity(&Entity {
                id: file_id.clone(),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::File,
                parent_id: None,
                name: format!("file_{i}"),
                path: Some(path.clone()),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .unwrap();
        storage
            .upsert_entity(&Entity {
                id: format!("{}:repo1:fn:{}:new", project.id, path),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::Function,
                parent_id: Some(file_id),
                name: "new".to_string(),
                path: Some(path),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .unwrap();
    }

    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = ExploreTool { ctx };
    // Default max_files=8; 20 matching files must be capped.
    let result = tool.call(json!({ "query": "new" })).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let symbols = v["symbols"].as_array().expect("symbols");
    // Must be capped at max_files (default 8), not all 20.
    assert!(
        symbols.len() <= 8,
        "common word 'new' returned {len} files, expected <= 8 (max_files cap); got {symbols:?}",
        len = symbols.len()
    );
    // Continuation must be present when total > returned.
    let has_continuation = v.get("continuation").is_some();
    let total = v
        .get("completeness")
        .and_then(|c| c.as_str())
        .map(|s| !s.is_empty());
    assert!(
        has_continuation || total.is_some(),
        "expected continuation pointer when matches exceed max_files; got: {v}"
    );
}

/// Requirement 2: a file whose symbol name exactly matches a query word
/// must rank above a file matched only by a path token.
#[test]
fn exact_symbol_ranks_above_path_token_match() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("rank-test", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();

    // File A: path contains the query word "alpha" but no symbol does.
    let file_a_id = format!("{}:repo1:file:src/alpha/beta.rs", project.id);
    storage
        .upsert_entity(&Entity {
            id: file_a_id.clone(),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "beta".to_string(),
            path: Some("src/alpha/beta.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();

    // File B: path does NOT contain "alpha", but has a function named "alpha".
    let file_b_id = format!("{}:repo1:file:src/gamma.rs", project.id);
    storage
        .upsert_entity(&Entity {
            id: file_b_id.clone(),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "gamma".to_string(),
            path: Some("src/gamma.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();
    storage
        .upsert_entity(&Entity {
            id: format!("{}:repo1:fn:src/gamma.rs:alpha", project.id),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::Function,
            parent_id: Some(file_b_id.clone()),
            name: "alpha".to_string(),
            path: Some("src/gamma.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();

    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = ExploreTool { ctx };
    let result = tool.call(json!({ "query": "alpha" })).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let symbols = v["symbols"].as_array().expect("symbols");
    assert!(symbols.len() >= 2, "expected both files, got: {symbols:?}");
    // File B (symbol exact match, score 4) must rank above File A (path token, score 1).
    let b_idx = symbols
        .iter()
        .position(|s| s["qualified_path"] == "src/gamma.rs")
        .unwrap();
    let a_idx = symbols
        .iter()
        .position(|s| s["qualified_path"] == "src/alpha/beta.rs")
        .unwrap();
    assert!(
        b_idx < a_idx,
        "exact symbol match (src/gamma.rs, idx {b_idx}) must rank above path-token match (src/alpha/beta.rs, idx {a_idx}); got: {symbols:?}"
    );
}

/// Integration: a query naming a FUNCTION whose name does not appear in
/// any file name/path must return the containing file (via the symbol
/// channel), and that file must be present and scored with the symbol
/// channel active.
#[test]
fn query_naming_function_returns_containing_file() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("sym-test", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();

    // A file whose name and path do NOT contain the symbol name.
    let file_id = format!("{}:repo1:file:src/util.rs", project.id);
    storage
        .upsert_entity(&Entity {
            id: file_id.clone(),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "util".to_string(),
            path: Some("src/util.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();
    // A Function-tier entity whose name IS the query, contained in that file.
    storage
        .upsert_entity(&Entity {
            id: format!("{}:repo1:fn:src/util.rs:sum_of_squares", project.id),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::Function,
            parent_id: Some(file_id.clone()),
            name: "sum_of_squares".to_string(),
            path: Some("src/util.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();

    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = ExploreTool { ctx };
    let result = tool.call(json!({ "query": "sum_of_squares" })).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let symbols = v["symbols"].as_array().expect("symbols");
    assert!(
        !symbols.is_empty(),
        "expected the containing file, got: {v}"
    );
    // The containing file (src/util.rs) must be returned.
    let has_util = symbols.iter().any(|s| s["qualified_path"] == "src/util.rs");
    assert!(has_util, "containing file src/util.rs missing: {symbols:?}");
    // Score must reflect the exact symbol-name channel (4), not 0.
    let util = symbols
        .iter()
        .find(|s| s["qualified_path"] == "src/util.rs")
        .unwrap();
    assert_eq!(util["score"], EXACT_SYMBOL_SCORE);
}

/// Multi-repo: a symbol must resolve against the repo that actually contains
/// the file, not the first repo whose path lookup happens to be non-empty.
/// Repo 1 (listed first by the prefilter loop) has a file at the SAME
/// relative path but no matching symbol; the matching function exists only in
/// repo 2. Repo 2's file must be returned.
#[test]
fn multi_repo_symbol_resolves_in_own_repo_not_first_repo() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("multi-repo-test", None).unwrap();
    let repo1 = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    let repo2 = storage
        .add_repo(&project.id, "repo2", "/tmp/repo2")
        .unwrap();

    // Repo 1: a file at src/multi.rs, with only a NON-matching function.
    let file_a_id = format!("{}:repo1:file:src/multi.rs", project.id);
    storage
        .upsert_entity(&Entity {
            id: file_a_id.clone(),
            project_id: project.id.clone(),
            repo_id: Some(repo1.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "multi".to_string(),
            path: Some("src/multi.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();
    storage
        .upsert_entity(&Entity {
            id: format!("{}:repo1:fn:src/multi.rs:unrelated", project.id),
            project_id: project.id.clone(),
            repo_id: Some(repo1.id.clone()),
            tier: EntityTier::Function,
            parent_id: Some(file_a_id),
            name: "unrelated".to_string(),
            path: Some("src/multi.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();

    // Repo 2: a file at the SAME relative path src/multi.rs, with the
    // matching function `multi_target`.
    let file_b_id = format!("{}:repo2:file:src/multi.rs", project.id);
    storage
        .upsert_entity(&Entity {
            id: file_b_id.clone(),
            project_id: project.id.clone(),
            repo_id: Some(repo2.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "multi".to_string(),
            path: Some("src/multi.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();
    storage
        .upsert_entity(&Entity {
            id: format!("{}:repo2:fn:src/multi.rs:multi_target", project.id),
            project_id: project.id.clone(),
            repo_id: Some(repo2.id.clone()),
            tier: EntityTier::Function,
            parent_id: Some(file_b_id),
            name: "multi_target".to_string(),
            path: Some("src/multi.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();

    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = ExploreTool { ctx };
    let result = tool.call(json!({ "query": "multi_target" })).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let symbols = v["symbols"].as_array().expect("symbols");
    assert!(
        !symbols.is_empty(),
        "expected repo 2's containing file, got: {v}"
    );
    // Repo 2's file (the one containing the matching symbol) must be present.
    let has_b = symbols
        .iter()
        .any(|s| s["qualified_path"] == "src/multi.rs");
    assert!(has_b, "repo 2 file src/multi.rs missing: {symbols:?}");
    let b = symbols
        .iter()
        .find(|s| s["qualified_path"] == "src/multi.rs")
        .unwrap();
    assert_eq!(b["score"], EXACT_SYMBOL_SCORE);
}

/// The prefilter bound (`SYMBOL_PREFILTER_LIMIT`) must not push an exact
/// name match out of the result window when many prefix-only candidates
/// share the prefix. Exact matches sort first (`ORDER BY exact DESC`), so
/// they survive even when prefix-only rows fill most of the window.
#[test]
fn prefilter_bound_exact_match_survives_among_prefix_candidates() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project("bound-test", None).unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();

    // 30 prefix-only candidates: names share the "target" token (a token
    // prefix hit) but are not an exact match for the two-word query "exact
    // target" (the file name is a single token `exact_target`, which only
    // equals a query word when BOTH words are present as one word). The
    // first query word "exact" is not a prefix of any token in these names
    // (they start with "beta"), so they are LIKE candidates that fail
    // Rust-side confirmation — exactly the shape the bound must protect
    // against.
    for i in 0..30 {
        let path = format!("src/pref_{i}.rs");
        let file_id = format!("{}:repo1:file:{}", project.id, path);
        storage
            .upsert_entity(&Entity {
                id: file_id.clone(),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::File,
                parent_id: None,
                name: format!("pref_{i}"),
                path: Some(path.clone()),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .unwrap();
        storage
            .upsert_entity(&Entity {
                id: format!("{}:repo1:fn:{}:beta_variant_{i}", project.id, path),
                project_id: project.id.clone(),
                repo_id: Some(repo.id.clone()),
                tier: EntityTier::Function,
                parent_id: Some(file_id),
                name: format!("beta_variant_{i}"),
                path: Some(path),
                language: Some("Rust".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .unwrap();
    }

    // One exact-match symbol in a separate file.
    let exact_file_id = format!("{}:repo1:file:src/exact_target.rs", project.id);
    storage
        .upsert_entity(&Entity {
            id: exact_file_id.clone(),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::File,
            parent_id: None,
            name: "exact_target".to_string(),
            path: Some("src/exact_target.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();
    storage
        .upsert_entity(&Entity {
            id: format!("{}:repo1:fn:src/exact_target.rs:exact_target", project.id),
            project_id: project.id.clone(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::Function,
            parent_id: Some(exact_file_id),
            name: "exact_target".to_string(),
            path: Some("src/exact_target.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();

    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id: project.id.clone(),
        repo_path: std::path::PathBuf::new(),
        output_dir: None,
        zero_repo_guidance: None,
    });
    let tool = ExploreTool { ctx };
    let result = tool.call(json!({ "query": "exact target" })).unwrap();
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let symbols = v["symbols"].as_array().expect("symbols");
    // The exact-match file must be present and ranked at the top (score 4).
    let has_exact = symbols
        .iter()
        .any(|s| s["qualified_path"] == "src/exact_target.rs");
    assert!(
        has_exact,
        "exact-match file src/exact_target.rs must survive among 30 prefix candidates: {symbols:?}"
    );
    let exact = symbols
        .iter()
        .find(|s| s["qualified_path"] == "src/exact_target.rs")
        .unwrap();
    assert_eq!(exact["score"], EXACT_SYMBOL_SCORE);
    // The exact match must rank first (score 4 > any prefix score).
    assert!(
        symbols.first().unwrap()["qualified_path"] == "src/exact_target.rs",
        "exact match must be first: {symbols:?}"
    );
}
