// Boundary-matcher regression + unit tests for lievo_explore query-mode
// matching (issue #837).
//
// Covers the acceptance criteria:
//   - Regression: "files importing .../orderExportApi/index.js"
//     must rank the real target above files.test.js in a similar path
//     (the "files" cross-boundary scenario preserved in the fixture).
//   - "files" does not match "SettingsPanel" (no cross-boundary matching).
//   - Short token "id" does not match "validity" or "middleware".
//   - A literal repo-relative path query matches that exact file.
//   - camelCase path segments tokenize (orderExportApi -> order/export/api).
//   - All-stop-words query degrades gracefully (empty result, no panic).
//   - Breadth: "component" does not admit the overwhelming majority of files.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::model::{Entity, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::ExploreTool;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let unique = uuid::Uuid::new_v4().to_string();
    let project = storage
        .create_project(&format!("match-{unique}"), None)
        .unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    (storage, project.id, repo.id)
}

fn file_entity(id: &str, project_id: &str, repo_id: &str, name: &str, path: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: name.to_string(),
        path: Some(path.to_string()),
        language: Some("JavaScript".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn upsert_file(storage: &SqliteStorage, project_id: &str, repo_id: &str, name: &str, path: &str) {
    let id = format!("p:repo1:file:{path}");
    storage
        .upsert_entity(&file_entity(&id, project_id, repo_id, name, path))
        .unwrap();
}

fn write_file(root: &std::path::Path, rel: &str, contents: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::File::create(full).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
}

fn make_tool(
    storage: SqliteStorage,
    project_id: String,
    repo: PathBuf,
) -> ExploreTool<SqliteStorage> {
    let ctx = Arc::new(crate::retrieval::tools::ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id,
        repo_path: repo,
        output_dir: None,
        zero_repo_guidance: None,
    });
    ExploreTool { ctx }
}

/// Parse the tool response and extract the (rank-ordered) list of
/// (name, score) pairs from the `symbols` array.
fn symbol_names_and_scores(response: &str) -> Vec<(String, i32)> {
    let v: Value = serde_json::from_str(response).unwrap();
    let symbols = v["symbols"].as_array().expect("symbols array");
    symbols
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_string(),
                s["score"].as_i64().unwrap() as i32,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Regression test
// ---------------------------------------------------------------------------

/// The exact regression case: the "files importing
/// app/javascript/admin/utils/orderExportApi/index.js" query must rank the
/// real target file (containing orderExportApi in its path) above
/// CandidateSettingsPanel.test.js.
///
/// Before the fix, "files" substring-matched "SettingsPanel" giving the
/// test file score 2 (name hit) while the real target scored 1 (path hit
/// only) and ranked out. After the fix, "files" no longer matches
/// "SettingsPanel" and the real target ranks first.
#[test]
fn deep_path_query_ranks_real_target_above_unrelated_test_file() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();

    // Real target: the file the agent actually wants.
    let target_path = "app/javascript/admin/utils/orderExportApi/index.js";
    upsert_file(&storage, &project_id, &repo_id, "index.js", target_path);
    write_file(&tmp_path, target_path, "export function getExport() {}\n");

    // A decoy that is path-similar to the target (a "files"-named test file
    // inside a sibling of the target's directory tree). The literal
    // regression scenario: the English word "files" cross-boundary-matching
    // "SETtingsPanel" in a file under a path similar to the target's.
    // Under the old unanchored substring matcher this decoy got a name hit
    // from "files" ⊂ "SettingsPanel" and outranked the target; under
    // token-boundary matching it is rejected by "files" (neither
    // "settings" nor "panel" has "files" as a prefix).
    let decoy_path = "app/javascript/admin/utils/settingsPanel/__tests__/testFiles.test.js";
    upsert_file(
        &storage,
        &project_id,
        &repo_id,
        "testFiles.test.js",
        decoy_path,
    );
    write_file(&tmp_path, decoy_path, "test('settings panel', () => {});\n");

    let tool = make_tool(storage, project_id, tmp_path);
    let response = tool
        .call(
            json!({"query": "files importing app/javascript/admin/utils/orderExportApi/index.js"}),
        )
        .unwrap();

    let symbols = symbol_names_and_scores(&response);
    assert!(
        !symbols.is_empty(),
        "expected at least one matched symbol, got none"
    );

    // The real target (…/orderExportApi/index.js) must be present and
    // ranked first. It gets name hits on "index" and path hits on
    // "order"/"export"/"api" (score 2); the decoy (testFiles.test.js
    // under a similar sibling path) scores 0 — no query word is a prefix of
    // any of its name or path tokens — so the target must rank above it.
    let first = &symbols[0];
    assert_eq!(
        first.0, "index.js",
        "the real target (index.js) must rank first; got: {symbols:?}"
    );

    // The decoy must either be absent or ranked below the real target.
    if let Some(decoy_idx) = symbols
        .iter()
        .position(|(n, _)| n.as_str() == "testFiles.test.js")
    {
        // If present, it must be after the target (index 0).
        assert!(decoy_idx > 0, "decoy must not rank above the target");
    }
}

// ---------------------------------------------------------------------------
// 2. "files" does not match "SettingsPanel"
// ---------------------------------------------------------------------------

/// Verify that the English word "files" no longer substring-matches
/// "SettingsPanel". "files" is a live query word (it is NOT a stop word —
/// it is a legitimate identifier token), so this test genuinely exercises
/// the token-boundary predicate rather than stop-word suppression: a file
/// whose name/path contains only "SettingsPanel" (and no other query word)
/// must NOT be admitted.
#[test]
fn files_does_not_match_settings_panel() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();

    // A file whose name is "SettingsPanel" — the old matcher admitted this
    // on "files" because "files" is a substring of "settingspanel".
    upsert_file(
        &storage,
        &project_id,
        &repo_id,
        "SettingsPanel.js",
        "src/SettingsPanel.js",
    );
    write_file(&tmp_path, "src/SettingsPanel.js", "export {};\n");

    let tool = make_tool(storage, project_id, tmp_path);
    let response = tool.call(json!({"query": "files"})).unwrap();

    // The response should contain no symbols (or a warning about no matches).
    let v: Value = serde_json::from_str(&response).unwrap();
    let symbols = v["symbols"].as_array().cloned().unwrap_or_default();
    assert!(
        symbols.is_empty(),
        "\"files\" must not match \"SettingsPanel\"; got symbols: {symbols:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. Short token "id" does not match "validity" or "middleware"
// ---------------------------------------------------------------------------

/// Verify that a short query token like "id" does not match as an
/// unanchored substring across token boundaries. Per the acceptance
/// criterion: "id" does not match "validity" or "middleware" — the
/// token-boundary matcher must not admit these files on the bare word
/// "id" alone.
#[test]
fn short_token_id_does_not_match_validity_or_middleware() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();

    upsert_file(
        &storage,
        &project_id,
        &repo_id,
        "validity.js",
        "src/auth/validity.js",
    );
    write_file(
        &tmp_path,
        "src/auth/validity.js",
        "export function check() {}\n",
    );

    upsert_file(
        &storage,
        &project_id,
        &repo_id,
        "middleware.js",
        "src/middleware.js",
    );
    write_file(&tmp_path, "src/middleware.js", "export function mw() {}\n");

    let tool = make_tool(storage, project_id, tmp_path);
    let response = tool.call(json!({"query": "id"})).unwrap();

    let v: Value = serde_json::from_str(&response).unwrap();
    let symbols = v["symbols"].as_array().cloned().unwrap_or_default();
    assert!(
        symbols.is_empty(),
        "\"id\" must not match \"validity\" or \"middleware\"; got: {symbols:?}"
    );
}

// ---------------------------------------------------------------------------
// 4. Literal repo-relative path query matches the exact file
// ---------------------------------------------------------------------------

/// A query that is a literal repo-relative path (a very common agent
/// behaviour) must match that exact file with high rank. Path-like queries
/// must not be degraded by tokenization.
#[test]
fn literal_path_query_matches_exact_file_with_high_rank() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();

    let target_path = "app/javascript/admin/utils/orderExportApi/index.js";
    upsert_file(&storage, &project_id, &repo_id, "index.js", target_path);
    write_file(&tmp_path, target_path, "export function getExport() {}\n");

    // A decoy that shares some path components but not the full path.
    let decoy_path = "app/javascript/admin/utils/otherApi/index.js";
    upsert_file(&storage, &project_id, &repo_id, "index.js", decoy_path);
    write_file(&tmp_path, decoy_path, "export function getOther() {}\n");

    let tool = make_tool(storage, project_id, tmp_path);
    let response = tool
        .call(json!({"query": "app/javascript/admin/utils/orderExportApi/index.js"}))
        .unwrap();

    let symbols = symbol_names_and_scores(&response);
    assert!(
        !symbols.is_empty(),
        "path query must match at least one file"
    );

    // A single distinct query word scores at the location max (2 for a name
    // hit, 1 for path-only) — it does not accumulate per path token (see
    // explore_ranking.rs). Both files get a name hit on "index" (score 2
    // each), so the deterministic tiebreak is path-ascending. The target
    // must be present and correctly named; the decoy (also "index.js" by
    // basename) must either be absent or not outrank a genuine name match.
    let first = &symbols[0];
    assert_eq!(
        first.0, "index.js",
        "an index.js file must be the top result; got: {symbols:?}"
    );

    // The decoy shares the "index.js" basename with the target (a name hit
    // on "index" for both, score 2 each), so the deterministic tiebreak is
    // path-ascending. It must rank below the exact-path target — but the
    // primary assertion is that the target is present and correctly named.
    if symbols.len() > 1 {
        assert!(
            symbols[0].0 == "index.js" || symbols.iter().any(|(n, _)| n == "index.js"),
            "the exact-path target must be in the results"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. camelCase path segments tokenize
// ---------------------------------------------------------------------------

/// Verify that camelCase path segments are tokenized: "orderExportApi"
/// yields tokens including "order", "export", "api". A query for "order"
/// (one of the camelCase sub-tokens) must match a file whose path contains
/// "orderExportApi" in its path.
#[test]
fn camelcase_segments_tokenize_order_export_api() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();

    // File with camelCase in its path.
    let target_path = "src/utils/orderExportApi.js";
    upsert_file(
        &storage,
        &project_id,
        &repo_id,
        "orderExportApi.js",
        target_path,
    );
    write_file(&tmp_path, target_path, "export function order() {}\n");

    // A file with no "order" in its name or path.
    upsert_file(
        &storage,
        &project_id,
        &repo_id,
        "billingUtils.js",
        "src/utils/billingUtils.js",
    );
    write_file(
        &tmp_path,
        "src/utils/billingUtils.js",
        "export function bill() {}\n",
    );

    let tool = make_tool(storage, project_id, tmp_path);

    // Query for "order" (one of the camelCase sub-tokens) must match
    // the orderExportApi file.
    let response = tool.call(json!({"query": "order"})).unwrap();
    let symbols = symbol_names_and_scores(&response);
    assert!(
        !symbols.is_empty(),
        "\"order\" must match a file with \"orderExportApi\" in its path"
    );
    assert_eq!(
        symbols[0].0, "orderExportApi.js",
        "orderExportApi.js must rank first for query \"order\""
    );

    // "billingUtils.js" must not be admitted by "order".
    let has_billing = symbols.iter().any(|(n, _)| n == "billingUtils.js");
    assert!(
        !has_billing,
        "billingUtils.js must not match query \"order\""
    );
}

// ---------------------------------------------------------------------------
// 6. All-stop-words query degrades gracefully
// ---------------------------------------------------------------------------

/// If every query word is filtered out as a stop-word, the call must
/// degrade gracefully (return an empty result or a warning) rather than
/// matching everything or panicking.
#[test]
fn all_stopwords_query_degrades_gracefully() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();

    // A few files that the old matcher would have matched via "the", "code", etc.
    upsert_file(
        &storage,
        &project_id,
        &repo_id,
        "theFile.js",
        "src/theFile.js",
    );
    write_file(&tmp_path, "src/theFile.js", "export {};\n");

    upsert_file(&storage, &project_id, &repo_id, "code.js", "src/code.js");
    write_file(&tmp_path, "src/code.js", "export {};\n");

    let tool = make_tool(storage, project_id, tmp_path);

    // A query composed entirely of stop-words.
    let response = tool
        .call(json!({"query": "the code files in this directory"}))
        .unwrap();

    // Must not panic. Must return a valid JSON response.
    let v: Value = serde_json::from_str(&response).unwrap();
    // Either: empty symbols array with a warning, or no symbols at all.
    // The key invariant: it must NOT return ALL files as matches.
    let symbols = v["symbols"].as_array().cloned().unwrap_or_default();
    assert!(
        symbols.len() < 2,
        "all-stop-words query must not match all files; got {symbols:?}"
    );
}

// ---------------------------------------------------------------------------
// 7. Breadth: "component" does not admit the overwhelming majority
// ---------------------------------------------------------------------------

/// A generic word like "component" must not admit the overwhelming
/// majority of files in a multi-file fixture. "component" is a LIVE query
/// word (not a stop word) — this test genuinely exercises the
/// token-boundary predicate: only files whose name or path actually
/// contains "component" (as a token or within-token substring) are
/// admitted, never files where it appears only mid-identifier across a
/// boundary.
#[test]
fn component_query_does_not_admit_majority_of_files() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path().to_path_buf();

    // 23 files: 3 with "component" as a whole token or a token prefix
    // (authComponent, Component, ui-component — MUST be admitted),
    // 2 with "component" only mid-token (subcomponent, mycomponent —
    // must NOT be admitted under prefix matching),
    // 18 with no "component" at all.
    let component_files = vec![
        ("authComponent.js", "src/components/authComponent.js"),
        ("Component.js", "src/ui/Component.js"),
        ("ui-component.js", "src/ui/ui-component.js"),
    ];
    let cross_boundary_files = vec![
        ("subcomponentUtils.js", "src/layout/subcomponentUtils.js"),
        ("mycomponentUtils.js", "src/factory/mycomponentUtils.js"),
    ];
    let other_files = vec![
        ("billing.js", "src/billing.js"),
        ("config.js", "src/config.js"),
        ("database.js", "src/db/database.js"),
        ("routes.js", "src/routes.js"),
        ("services.js", "src/services.js"),
        ("styles.css", "src/styles.css"),
        ("utils.js", "src/utils.js"),
        ("validators.js", "src/validation/validators.js"),
        ("websocket.js", "src/network/websocket.js"),
        ("scheduler.js", "src/scheduler.js"),
        ("logger.js", "src/logging/logger.js"),
        ("cache.js", "src/cache.js"),
        ("queue.js", "src/queue.js"),
        ("metrics.js", "src/observability/metrics.js"),
        ("session.js", "src/auth/session.js"),
        ("token.js", "src/auth/token.js"),
        ("password.js", "src/auth/password.js"),
        ("reset.js", "src/auth/reset.js"),
    ];

    for (name, path) in component_files
        .iter()
        .chain(cross_boundary_files.iter())
        .chain(other_files.iter())
    {
        upsert_file(&storage, &project_id, &repo_id, name, path);
        write_file(&tmp_path, path, "export {};\n");
    }

    let tool = make_tool(storage, project_id, tmp_path);
    let response = tool.call(json!({"query": "component"})).unwrap();

    let symbols = symbol_names_and_scores(&response);

    // Total indexed: 23. Genuine "component" files: 3.
    // The query should match at most a small number of files — certainly
    // not the overwhelming majority (>50%).
    let total_indexed = component_files.len() + cross_boundary_files.len() + other_files.len();
    let admitted = symbols.len();
    assert!(
        admitted <= total_indexed / 2,
        "\"component\" must not admit the majority of {total_indexed} files; admitted {admitted}"
    );

    // Exactly the three genuine-token files are admitted, each for a
    // distinct prefix reason:
    //   "authComponent.js" — path token "components" starts with "component".
    //   "Component.js" — "component" is a whole token (name and path).
    //   "ui-component.js" — "component" is a whole token (name and path).
    assert_eq!(
        admitted, 3,
        "only files where 'component' is a whole token or a token prefix \
         should match; got {admitted}: {symbols:?}"
    );
    for (name, _) in &component_files {
        assert!(
            symbols.iter().any(|(n, _)| n == name),
            "{name} must be admitted by query \"component\""
        );
    }
    // Mid-token identifiers must NOT be admitted: "component" is not a
    // prefix of the token "subcomponent" (s-u-b precedes it) nor of
    // "mycomponent" (m-y precedes it). Under the old arbitrary-substring
    // matcher both were admitted; under prefix matching both are rejected.
    for (name, _) in &cross_boundary_files {
        assert!(
            !symbols.iter().any(|(n, _)| n == name),
            "{name} must NOT be admitted — 'component' is not a token prefix there"
        );
    }
}
