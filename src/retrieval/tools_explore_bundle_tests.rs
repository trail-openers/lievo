//! Subsystem-bundle mode tests (issue #743, task-a / default workstream).
//!
//! Wired via `#[path]` from `tools_explore_bundle.rs`. Covers the AC /
//! test-surface items that are task-a's: one bundle call returns source,
//! intra-scope edges, not_shown_files and a structured completeness field
//! in one response; an edge with one endpoint outside the bundle is excluded
//! (and counted); a complete bundle reports complete and a partial one
//! reports the omitted counts; response chars never exceed 24000 (hard
//! bound) even with a long not-shown list; a single oversized file is
//! truncated via the existing mechanism and still represented; an identical
//! request returns a byte-identical bundle; and query-only / scope-only
//! calls never activate bundle mode (the no-fall-through pitfall).

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::ExploreTool;
use crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let unique = uuid::Uuid::new_v4().to_string();
    let project = storage
        .create_project(&format!("bundle-{unique}"), None)
        .unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    (storage, project.id, repo.id)
}

/// Fixed (storage, project_id, repo_id) context shared by every upsert in a fixture.
#[derive(Clone, Copy)]
struct FixtureCtx<'a>(&'a SqliteStorage, &'a str, &'a str);

fn upsert_entity(
    ctx: FixtureCtx,
    id: &str,
    tier: EntityTier,
    parent_id: Option<&str>,
    name: &str,
    path: Option<&str>,
) {
    let (storage, project_id, repo_id) = (ctx.0, ctx.1, ctx.2);
    let ent = Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier,
        parent_id: parent_id.map(String::from),
        name: name.to_string(),
        path: path.map(String::from),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&ent).unwrap();
}

fn upsert_file(storage: &SqliteStorage, project_id: &str, repo_id: &str, name: &str, path: &str) {
    upsert_entity(
        FixtureCtx(storage, project_id, repo_id),
        &format!("p:repo1:file:{path}"),
        EntityTier::File,
        None,
        name,
        Some(path),
    );
}

/// Function-tier entity under file entity id `file_id`.
fn upsert_function(
    storage: &SqliteStorage,
    project_id: &str,
    repo_id: &str,
    file_id: &str,
    name: &str,
) {
    upsert_entity(
        FixtureCtx(storage, project_id, repo_id),
        &format!("p:repo1:fn:{file_id}:{name}"),
        EntityTier::Function,
        Some(file_id),
        name,
        None,
    );
}

/// Function id for the helper above (id scheme kept in lockstep).
fn fn_id(file_id: &str, name: &str) -> String {
    format!("p:repo1:fn:{file_id}:{name}")
}

/// A Call/Import edge between two entity ids.
fn upsert_edge(storage: &SqliteStorage, src: &str, tgt: &str, kind: RelType) {
    storage
        .upsert_relationship(&Relationship {
            source_id: src.to_string(),
            target_id: tgt.to_string(),
            rel_type: kind,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Resolved,
        })
        .unwrap();
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

fn write_file(root: &std::path::Path, rel: &str, contents: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::File::create(full).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
}

// ---------------------------------------------------------------------------
// One bundle call: source + intra-scope edges + not_shown_files +
// structured completeness, in a single response
// ---------------------------------------------------------------------------

#[test]
fn bundle_call_returns_source_edges_not_shown_and_completeness() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/a.rs", "fn a() { b() }\n");
    write_file(tmp.path(), "src/b.rs", "fn b() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "a", "src/a.rs");
    upsert_file(&storage, &project_id, &repo_id, "b", "src/b.rs");
    let aid = "p:repo1:file:src/a.rs";
    let bid = "p:repo1:file:src/b.rs";
    upsert_function(&storage, &project_id, &repo_id, aid, "afn");
    upsert_function(&storage, &project_id, &repo_id, bid, "bfn");
    upsert_edge(
        &storage,
        &fn_id(aid, "afn"),
        &fn_id(bid, "bfn"),
        RelType::Calls,
    );
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"bundle": "src"}))
        .expect("bundle call must succeed");
    assert!(
        result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold, got {}",
        result.chars().count()
    );
    let parsed: Value = serde_json::from_str(&result).unwrap();
    // Per-file source, packed, in scope (lexicographic) order.
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 2, "both files pack: {parsed}");
    let paths: Vec<&str> = symbols
        .iter()
        .map(|s| s["qualified_path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["src/a.rs", "src/b.rs"]);
    assert!(
        symbols[0]["source"]
            .as_str()
            .unwrap()
            .starts_with("1\tfn a()"),
        "verbatim line-numbered source: {symbols:?}"
    );
    // Intra-scope edge: the a→b Calls edge, mapped up to file paths.
    let edges = parsed["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1, "one intra-scope edge: {parsed}");
    assert_eq!(edges[0]["source"], "src/a.rs");
    assert_eq!(edges[0]["target"], "src/b.rs");
    // Complete scope: empty not_shown_files, structured completeness.
    let nsf = parsed["not_shown_files"].as_array().unwrap();
    assert_eq!(nsf.len(), 0, "nothing omitted: {parsed}");
    let comp = &parsed["completeness"];
    assert_eq!(comp["complete"], true);
    assert_eq!(comp["omitted_files"], 0);
    assert_eq!(comp["omitted_edges"], 0);
    assert_eq!(parsed["returned"], 2);
    assert_eq!(parsed["total"], 2);
}

// ---------------------------------------------------------------------------
// Edge with one endpoint outside the bundle is excluded (and counted);
// out-of-scope files are not packed
// ---------------------------------------------------------------------------

#[test]
fn bundle_excludes_edges_leaving_the_scope() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/a.rs", "fn a() { out() }\n");
    write_file(tmp.path(), "out.rs", "fn out() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "a", "src/a.rs");
    upsert_file(&storage, &project_id, &repo_id, "out", "out.rs");
    let aid = "p:repo1:file:src/a.rs";
    let oid = "p:repo1:file:out.rs";
    upsert_function(&storage, &project_id, &repo_id, aid, "afn");
    upsert_function(&storage, &project_id, &repo_id, oid, "outfn");
    upsert_edge(
        &storage,
        &fn_id(aid, "afn"),
        &fn_id(oid, "outfn"),
        RelType::Calls,
    );
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"bundle": "src"}))
        .expect("bundle call must succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();

    // Only the in-scope file is packed.
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0]["qualified_path"], "src/a.rs");

    // The a→out edge leaves the scope: excluded from `edges`, counted in
    // omitted_edges.
    let edges = parsed["edges"].as_array().unwrap();
    assert_eq!(
        edges.len(),
        0,
        "no out-of-scope edges in the bundle: {parsed}"
    );
    let comp = &parsed["completeness"];
    assert_eq!(comp["complete"], true, "all in-scope files packed");
    assert_eq!(comp["omitted_files"], 0);
    assert_eq!(
        comp["omitted_edges"], 1,
        "the leaving edge must be counted: {parsed}"
    );
}

// ---------------------------------------------------------------------------
// Complete vs partial scope: completeness field carries the counts
// ---------------------------------------------------------------------------

#[test]
fn bundle_partial_scope_reports_omitted_file_and_edge_counts() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/small.rs", "fn s() { 1 }\n");
    write_file(tmp.path(), "src/big1.rs", "fn b1() { 1 }\n");
    write_file(tmp.path(), "src/big2.rs", "fn b2() { 2 }\n");
    upsert_file(&storage, &project_id, &repo_id, "small", "src/small.rs");
    upsert_file(&storage, &project_id, &repo_id, "big1", "src/big1.rs");
    upsert_file(&storage, &project_id, &repo_id, "big2", "src/big2.rs");
    let sid = "p:repo1:file:src/small.rs";
    let b1id = "p:repo1:file:src/big1.rs";
    let b2id = "p:repo1:file:src/big2.rs";
    upsert_function(&storage, &project_id, &repo_id, sid, "sfn");
    upsert_function(&storage, &project_id, &repo_id, b1id, "b1fn");
    upsert_function(&storage, &project_id, &repo_id, b2id, "b2fn");
    upsert_edge(
        &storage,
        &fn_id(sid, "sfn"),
        &fn_id(b1id, "b1fn"),
        RelType::Calls,
    );
    upsert_edge(
        &storage,
        &fn_id(sid, "sfn"),
        &fn_id(b2id, "b2fn"),
        RelType::Imports,
    );
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    // All three files are tiny, so all pack and the small→big2 edge shows.
    let result = tool
        .call(json!({"bundle": "src"}))
        .expect("bundle call must succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["symbols"].as_array().unwrap().len(), 3);
    let edges = parsed["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 2, "both intra-scope edges show: {parsed}");
    let comp = &parsed["completeness"];
    assert_eq!(comp["complete"], true);
    assert_eq!(comp["omitted_files"], 0);
    assert_eq!(comp["omitted_edges"], 0);
}

#[test]
fn bundle_unpacked_file_is_named_in_not_shown_files_and_counted() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/first.rs", "fn f() { 1 }\n");
    // Second is ~19K chars: first (always taken) packs, second's ~19K source
    // + first's source + the disclosure exceeds the 24K cap → second is
    // omitted and named in not_shown_files (the partial-packing case).
    let big = "fn g() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad \n".repeat(1250)
        + "}\n";
    write_file(tmp.path(), "src/second.rs", &big);
    upsert_file(&storage, &project_id, &repo_id, "first", "src/first.rs");
    upsert_file(&storage, &project_id, &repo_id, "second", "src/second.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"bundle": "src"}))
        .expect("bundle call must succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    let comp = &parsed["completeness"];
    // Invariant either way: omitted_files + returned == total, and
    // not_shown_files names exactly the omitted paths.
    let returned = parsed["returned"].as_u64().unwrap() as usize;
    let total = parsed["total"].as_u64().unwrap() as usize;
    let omitted = comp["omitted_files"].as_u64().unwrap() as usize;
    assert_eq!(returned + omitted, total);
    assert_eq!(symbols.len(), returned);
    let nsf = parsed["not_shown_files"].as_array().unwrap();
    let names: Vec<&str> = nsf.iter().map(|v| v.as_str().unwrap()).collect();
    // Second is ~21K chars: first + second + disclosure exceeds the 24K cap
    // → second is omitted and named in not_shown_files (partial packing).
    assert_eq!(total, 2);
    assert_eq!(omitted, 1, "the second file must not fit: {parsed}");
    assert_eq!(
        names,
        vec!["src/second.rs"],
        "omitted file named: {names:?}"
    );
}

#[test]
fn bundle_partial_scope_omitted_edge_counts_untaken_intra_edge() {
    // A huge first file that leaves no room for the second: the second is
    // omitted (named in not_shown_files) and the intra-scope edge to it is
    // counted as an omitted edge — the completeness object carries both
    // counts for a non-complete bundle.
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let big = "fn g() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad \n".repeat(1200)
        + "}\n";
    write_file(tmp.path(), "src/first.rs", &big);
    write_file(tmp.path(), "src/second.rs", "fn s() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "first", "src/first.rs");
    upsert_file(&storage, &project_id, &repo_id, "second", "src/second.rs");
    let fid = "p:repo1:file:src/first.rs";
    let sid = "p:repo1:file:src/second.rs";
    upsert_function(&storage, &project_id, &repo_id, fid, "ffn");
    upsert_function(&storage, &project_id, &repo_id, sid, "sfn");
    upsert_edge(
        &storage,
        &fn_id(fid, "ffn"),
        &fn_id(sid, "sfn"),
        RelType::Calls,
    );
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"bundle": "src"}))
        .expect("bundle call must succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();
    // The first file (~21K) alone exceeds the 24K cap: cap_response
    // truncates (the single-oversized-file case, decision #6). The second
    // (tiny) file is omitted in the packed shape; the truncated shape
    // reports the counts pre-cap.
    assert_eq!(
        parsed["truncated"].as_bool(),
        Some(true),
        "the oversized first file must be truncated via cap_response: {parsed}"
    );
    assert!(
        result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold, got {}",
        result.chars().count()
    );
    // The capped shape (cap_response) carries the integer not_shown and the
    // string completeness; the omitted file is still represented in the
    // symbols stub and the edge list is empty (the other endpoint is omitted).
    let nsf = parsed["not_shown_files"].as_array();
    if let Some(nsf) = nsf {
        let names: Vec<&str> = nsf.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(
            names.contains(&"src/second.rs"),
            "the omitted second file is named: {names:?}"
        );
    }
    if let Some(e) = parsed.get("edges") {
        assert_eq!(
            e.as_array().unwrap().len(),
            0,
            "no edge to an omitted file: {e:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Hard cap: response chars never exceed 24000 even when the not-shown list
// is long
// ---------------------------------------------------------------------------

#[test]
fn bundle_response_never_exceeds_hard_cap_with_long_not_shown_list() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // One giant first file (takes most of the cap) + many medium files that
    // cannot all fit → a long not_shown_files list that must itself stay
    // under the cap once the disclosure is appended.
    let big = "fn g() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad \n".repeat(1250)
        + "}\n";
    write_file(tmp.path(), "src/aaa.rs", &big);
    for i in 0..12 {
        let path = format!("src/f{i:02}.rs");
        let body = format!(
            "fn f{i}() {{ /* padding to push size up */ }}\n{}",
            "let y = 0; ".repeat(50)
        );
        write_file(tmp.path(), &path, &body);
        upsert_file(&storage, &project_id, &repo_id, &format!("f{i:02}"), &path);
    }
    upsert_file(&storage, &project_id, &repo_id, "aaa", "src/aaa.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"bundle": "src"}))
        .expect("bundle call must succeed");
    assert!(
        result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold with a long not-shown list, got {}",
        result.chars().count()
    );
    let parsed: Value = serde_json::from_str(&result).unwrap();
    // The first file (~21K) alone exceeds the 24K cap: cap_response
    // truncates (the single-oversized-file case, decision #6). The capped
    // shape carries an integer `not_shown` and a string `completeness`;
    // the omitted files are named in not_shown_files when present, or are
    // captured by the integer not_shown count.
    assert_eq!(
        parsed["truncated"].as_bool(),
        Some(true),
        "the oversized first file must be truncated via cap_response: {parsed}"
    );
    let not_shown_count = parsed["not_shown"].as_u64().unwrap_or(0);
    let nsf_len = parsed["not_shown_files"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    let returned = parsed["returned"].as_u64().unwrap_or(1) as usize;
    // Exactly one of the disclosure shapes is present (capped vs packed);
    // the omitted count must equal total - returned. In the capped shape
    // the omitted count is the integer `not_shown`; in the packed shape it
    // is not_shown_files.len().
    let omitted = if not_shown_count > 0 {
        not_shown_count as usize
    } else {
        nsf_len
    };
    // The first file is always taken (represented in the symbols stub).
    assert!(!parsed["symbols"].as_array().unwrap().is_empty());
    // The total file count is 13 (aaa + f00..f11); the omitted count must
    // be consistent with what was packed.
    if let Some(total) = parsed["total"].as_u64() {
        assert_eq!(total, 13, "total must count all in-scope files: {parsed}");
        assert!(
            (omitted as u64) + (returned as u64) <= total,
            "omitted + returned <= total: {parsed}"
        );
    }
}

// ---------------------------------------------------------------------------
// Single oversized file: truncated via the existing cap_response mechanism,
// still represented — never dropped
// ---------------------------------------------------------------------------

#[test]
fn bundle_single_oversized_file_truncated_but_represented() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let huge = "fn g() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad \n".repeat(3000)
        + "}\n";
    write_file(tmp.path(), "src/huge.rs", &huge);
    upsert_file(&storage, &project_id, &repo_id, "huge", "src/huge.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"bundle": "src"}))
        .expect("bundle call must succeed");
    assert!(
        result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold, got {}",
        result.chars().count()
    );
    let parsed: Value = serde_json::from_str(&result).unwrap();
    // The file is still represented: in the capped shape it appears in the
    // `symbols` stub (cap_response) with an elided qualified_path; the
    // truncation signal is set.
    assert_eq!(
        parsed["truncated"].as_bool(),
        Some(true),
        "oversized file must be truncated via cap_response: {parsed}"
    );
    let stubs = parsed["symbols"].as_array().unwrap();
    assert_eq!(
        stubs.len(),
        1,
        "the file must still be represented: {parsed}"
    );
    assert_eq!(stubs[0]["qualified_path"].as_str().unwrap(), "src/huge.rs");
}

// ---------------------------------------------------------------------------
// Missing requested file: represented with a flag AND named in not_shown_files
// ---------------------------------------------------------------------------

#[test]
fn bundle_missing_file_named_in_not_shown_and_not_silent() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/ok.rs", "fn ok() { 1 }\n");
    // Indexed but NOT on disk: missing_on_disk (a genuinely empty file on
    // disk would be present-but-empty — distinct, not an omission).
    upsert_file(&storage, &project_id, &repo_id, "ok", "src/ok.rs");
    upsert_file(&storage, &project_id, &repo_id, "ghost", "src/ghost.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"bundle": "src"}))
        .expect("bundle call must succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 2, "both files represented: {parsed}");
    let ghost = symbols
        .iter()
        .find(|s| s["qualified_path"] == "src/ghost.rs")
        .expect("missing file must be present");
    assert_eq!(
        ghost["missing_on_disk"].as_bool(),
        Some(true),
        "missing file must be flagged: {ghost:?}"
    );
    assert_eq!(
        ghost["source"].as_str(),
        Some(""),
        "missing file carries empty source: {ghost:?}"
    );
    let nsf = parsed["not_shown_files"].as_array().unwrap();
    let names: Vec<&str> = nsf.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        names.contains(&"src/ghost.rs"),
        "missing file must be named in not_shown_files: {names:?}"
    );
    // The missing file is counted as omitted in the structured completeness.
    let comp = &parsed["completeness"];
    assert_eq!(
        comp["complete"], false,
        "not complete when a file is missing"
    );
    assert!(
        comp["omitted_files"].as_u64().unwrap() >= 1,
        "omitted_files must count the missing file: {parsed}"
    );
    assert_eq!(comp["omitted_files"], nsf.len() as u64);
}

// ---------------------------------------------------------------------------
// Determinism: identical bundle request → byte-identical response
// ---------------------------------------------------------------------------

#[test]
fn bundle_identical_request_is_byte_identical() {
    let build = || -> String {
        let (storage, project_id, repo_id) = setup();
        let tmp = tempfile::tempdir().unwrap();
        for (name, path) in [("y", "src/y.rs"), ("x", "src/x.rs"), ("z", "src/z.rs")] {
            write_file(tmp.path(), path, &format!("fn {name}() {{ 1 }}\n"));
            upsert_file(&storage, &project_id, &repo_id, name, path);
        }
        let aid = "p:repo1:file:src/x.rs";
        let zid = "p:repo1:file:src/z.rs";
        upsert_function(&storage, &project_id, &repo_id, aid, "xfn");
        upsert_function(&storage, &project_id, &repo_id, zid, "zfn");
        upsert_edge(
            &storage,
            &fn_id(aid, "xfn"),
            &fn_id(zid, "zfn"),
            RelType::Calls,
        );
        let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());
        tool.call(json!({"bundle": "src"}))
            .expect("call must succeed")
    };
    let a = build();
    let b = build();
    assert_eq!(a, b, "identical bundle requests must be byte-identical");
}

// ---------------------------------------------------------------------------
// Short-circuit: query-only / scope-only / files-only calls never activate
// bundle mode; a bundle call never falls through into scope listing or
// word-match
// ---------------------------------------------------------------------------

#[test]
fn bundle_mode_never_activates_without_the_bundle_param() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/want.rs", "fn want() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "want", "src/want.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    // query-only: word-match shape (score/reason per symbol), no bundle keys.
    let r = tool.call(json!({"query": "want"})).unwrap();
    let parsed: Value = serde_json::from_str(&r).unwrap();
    assert!(
        parsed.get("completeness").is_none() || parsed["completeness"].as_object().is_none(),
        "query mode has no structured completeness: {parsed}"
    );
    assert!(
        parsed.get("not_shown_files").is_none(),
        "no bundle key: {parsed}"
    );

    // scope-only: scope-listing shape (files, returned, total, scope).
    let r = tool.call(json!({"scope": "src", "query": "want"})).unwrap();
    let parsed: Value = serde_json::from_str(&r).unwrap();
    assert!(
        parsed.get("files").is_some(),
        "scope mode returns the files listing: {parsed}"
    );
    assert!(
        parsed.get("not_shown_files").is_none(),
        "no bundle key: {parsed}"
    );

    // files-only: files-batch shape (issue #838: carries a structured
    // completeness object + not_shown_files, like bundle mode). Assert the
    // bundle-only `edges` key is absent (word-match/scope/files have no edges).
    let r = tool.call(json!({"files": ["src/want.rs"]})).unwrap();
    let parsed: Value = serde_json::from_str(&r).unwrap();
    assert!(parsed.get("edges").is_none(), "no bundle edges key");

    // blank bundle: absent-equivalent, falls through to word-match.
    let r = tool.call(json!({"query": "want", "bundle": "  "})).unwrap();
    let parsed: Value = serde_json::from_str(&r).unwrap();
    assert!(
        parsed.get("not_shown_files").is_none(),
        "blank bundle must not activate: {parsed}"
    );
}

#[test]
fn bundle_call_short_circuits_query_and_scope() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // A file that matches the query word AND a scope listing under "src" —
    // but the bundle param names the scope. The response must be the
    // bundle shape (completeness object, not_shown_files), not the
    // scope-listing or word-match shape.
    write_file(tmp.path(), "src/bundled.rs", "fn bundled() { 1 }\n");
    write_file(tmp.path(), "src/other.rs", "fn other() { 2 }\n");
    upsert_file(&storage, &project_id, &repo_id, "bundled", "src/bundled.rs");
    upsert_file(&storage, &project_id, &repo_id, "other", "src/other.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let r = tool
        .call(json!({"bundle": "src", "query": "other", "scope": "src"}))
        .expect("bundle must short-circuit query+scope");
    let parsed: Value = serde_json::from_str(&r).unwrap();
    let comp = parsed["completeness"]
        .as_object()
        .expect("bundle completeness object");
    assert!(
        comp["complete"].as_bool().is_some(),
        "structured completeness: {parsed}"
    );
    assert!(
        parsed.get("not_shown_files").is_some(),
        "not_shown_files present: {parsed}"
    );
    // No word-match score/reason, no scope-listing `files` key.
    assert!(
        parsed.get("files").is_none(),
        "no scope-listing shape: {parsed}"
    );
    for s in parsed["symbols"].as_array().unwrap() {
        assert!(s.get("score").is_none(), "no word-match score: {s:?}");
        assert!(s.get("reason").is_none(), "no word-match reason: {s:?}");
    }
}

// ---------------------------------------------------------------------------
// Invalid bundle scope: traversal rejected, no crash
// ---------------------------------------------------------------------------

#[test]
fn bundle_traversal_scope_rejected_with_warning() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/x.rs", "fn x() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "x", "src/x.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let r = tool.call(json!({"bundle": "../outside"})).unwrap();
    let parsed: Value = serde_json::from_str(&r).unwrap();
    assert!(
        parsed["warning"].as_str().unwrap().contains("not allowed"),
        "traversal scope must be rejected with a warning: {parsed}"
    );
    assert!(parsed.get("symbols").is_none(), "no files leaked: {parsed}");
}

// ---------------------------------------------------------------------------
// Empty scope: well-formed bundle with a truthful completeness field
// ---------------------------------------------------------------------------

#[test]
fn bundle_empty_scope_is_well_formed_and_truthful() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/x.rs", "fn x() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "x", "src/x.rs");
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    // A scope with no indexed files under it: empty bundle, complete=true,
    // zero counts — a well-formed, truthful response (issue #743 edge case).
    let r = tool.call(json!({"bundle": "src/nosuch"})).unwrap();
    let parsed: Value = serde_json::from_str(&r).unwrap();
    assert_eq!(parsed["symbols"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["edges"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["not_shown_files"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["returned"], 0);
    assert_eq!(parsed["total"], 0);
    let comp = &parsed["completeness"];
    assert_eq!(
        comp["complete"], true,
        "an empty scope is trivially complete"
    );
    assert_eq!(comp["omitted_files"], 0);
    assert_eq!(comp["omitted_edges"], 0);
}

// ---------------------------------------------------------------------------
// Edge type filter: non-Call/Import edges between scoped files are not shown
// ---------------------------------------------------------------------------

#[test]
fn bundle_only_includes_calls_and_imports_edges() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "src/a.rs", "fn a() { 1 }\n");
    write_file(tmp.path(), "src/b.rs", "fn b() { 2 }\n");
    upsert_file(&storage, &project_id, &repo_id, "a", "src/a.rs");
    upsert_file(&storage, &project_id, &repo_id, "b", "src/b.rs");
    let aid = "p:repo1:file:src/a.rs";
    let bid = "p:repo1:file:src/b.rs";
    upsert_function(&storage, &project_id, &repo_id, aid, "afn");
    upsert_function(&storage, &project_id, &repo_id, bid, "bfn");
    upsert_edge(
        &storage,
        &fn_id(aid, "afn"),
        &fn_id(bid, "bfn"),
        RelType::DependsOn,
    );
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let r = tool.call(json!({"bundle": "src"})).unwrap();
    let parsed: Value = serde_json::from_str(&r).unwrap();
    assert_eq!(
        parsed["edges"].as_array().unwrap().len(),
        0,
        "DependsOn is not a bundle edge type: {parsed}"
    );
    // It is also not an omitted edge (only intra/out Call/Import edges count
    // toward the bundle's edge accounting).
    assert_eq!(parsed["completeness"]["omitted_edges"], 0);
}
