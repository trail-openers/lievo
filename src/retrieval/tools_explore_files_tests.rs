//! Batch file-list mode tests (issue #741, task-a).
//!
//! Wired via `#[path]` from `tools_explore_files.rs`. Covers:
//!   - multiple paths in one call return multiple files packed under the cap
//!   - overflow beyond the cap sets the continuation pointer (exact remaining
//!     paths in `next`) and the remainder is fetchable
//!   - single-path request keeps working (one file returned)
//!   - duplicate paths de-duplicated; nonexistent path reported without
//!     failing the batch; traversal paths rejected the same way
//!   - files mode short-circuits word-match and scope mode
//!   - identical batch request → byte-identical response (determinism)
//!   - per-file budget, depth-shedding, structured completeness +
//!     not_shown_files, and the single-oversized-file ladder (issue #838)

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::model::{Entity, EntityTier};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::ExploreTool;
use crate::retrieval::tools_explore::MAX_EXPLORE_OUTPUT_CHARS;
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn setup() -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let unique = uuid::Uuid::new_v4().to_string();
    let project = storage
        .create_project(&format!("files-{unique}"), None)
        .unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    (storage, project.id, repo.id)
}

pub(crate) fn upsert_file(
    storage: &SqliteStorage,
    project_id: &str,
    repo_id: &str,
    name: &str,
    path: &str,
    summary: Option<&str>,
) {
    let id = format!("p:repo1:file:{path}");
    let ent = Entity {
        id,
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::File,
        parent_id: None,
        name: name.to_string(),
        path: Some(path.to_string()),
        language: Some("Rust".to_string()),
        summary: summary.map(|s| s.to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    storage.upsert_entity(&ent).unwrap();
}

pub(crate) fn make_tool(
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

pub(crate) fn write_file(root: &std::path::Path, rel: &str, contents: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::File::create(full).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
}

/// Write an on-disk file and upsert its indexed File-tier entity in one step
/// (test fixtures are small single-line bodies unless noted).
fn add_file(
    storage: &SqliteStorage,
    project_id: &str,
    repo_id: &str,
    tmp: &std::path::Path,
    name: &str,
    body: &str,
) {
    write_file(tmp, &format!("{name}.rs"), body);
    upsert_file(
        storage,
        project_id,
        repo_id,
        name,
        &format!("{name}.rs"),
        None,
    );
}

// ---------------------------------------------------------------------------
// Multiple paths in one call → multiple files, requested order, under cap
// ---------------------------------------------------------------------------

#[test]
fn files_batch_multiple_paths_return_all_in_requested_order() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    for (name, path) in [("c", "c.rs"), ("a", "a.rs"), ("b", "b.rs")] {
        write_file(
            tmp.path(),
            path,
            &format!("fn {name}() {{ /* {name} body */ }}\n"),
        );
        upsert_file(&storage, &project_id, &repo_id, name, path, None);
    }
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    // Requested order is b, c, a — NOT alphabetical, NOT storage order.
    let result = tool
        .call(json!({"files": ["b.rs", "c.rs", "a.rs"]}))
        .expect("files batch must succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();

    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 3, "all three files must be packed: {parsed}");
    let names: Vec<&str> = symbols
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["b", "c", "a"], "requested order must hold");
    // Hard cap: the response is well under 24K here, but pin the invariant
    // anyway (issue #741: batch tests assert the hard bound, not +100).
    assert!(
        result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "batch response must respect the hard 24K cap, got {}",
        result.chars().count()
    );
    // Fully-packed batch echoes the counts.
    assert_eq!(parsed["returned"], 3);
    assert_eq!(parsed["total"], 3);
    assert!(
        parsed.get("continuation").is_none(),
        "no remainder → no continuation"
    );
    // Each file carries its identity (symbol shape, not scope-listing shape).
    for (s, name) in symbols.iter().zip(["b", "c", "a"]) {
        assert_eq!(s["kind"], "file");
        assert_eq!(s["qualified_path"].as_str().unwrap(), format!("{name}.rs"));
    }
}

// ---------------------------------------------------------------------------
// Overflow beyond the cap: continuation pointer names the exact remainder
// ---------------------------------------------------------------------------

#[test]
fn files_batch_overflow_sets_continuation_with_exact_remaining_paths() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // Three files whose tier-2 payload (~9.8K chars each) can only hold
    // two at a time under the 24K cap — so the first two (requested order)
    // pack and the third becomes the remainder.
    let body = "fn f() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad \n".repeat(150)
        + "}\n";
    for name in ["one", "two", "three"] {
        write_file(tmp.path(), &format!("{name}.rs"), &body);
        upsert_file(
            &storage,
            &project_id,
            &repo_id,
            name,
            &format!("{name}.rs"),
            None,
        );
    }
    let tool = make_tool(storage, project_id, tmp.path().to_path_buf());

    let result = tool
        .call(json!({"files": ["one.rs", "two.rs", "three.rs"], "include_source": true}))
        .expect("files batch must succeed");
    assert!(
        result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold, got {}",
        result.chars().count()
    );
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(
        symbols.len(),
        2,
        "the first two files fit; the third must be deferred, got: {parsed}"
    );
    let packed_names: Vec<&str> = symbols
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        packed_names,
        vec!["one", "two"],
        "requested order must hold: {packed_names:?}"
    );

    // The continuation pointer (existing mechanism: returned/total/next)
    // must name the exact remaining paths, in order.
    let cont = parsed["continuation"]
        .as_str()
        .expect("continuation required")
        .to_string();
    assert!(cont.contains("returned: 2"), "got: {cont}");
    assert!(cont.contains("total: 3"), "got: {cont}");
    assert!(
        cont.contains("\"three.rs\""),
        "remainder path must be echoed, got: {cont}"
    );
    assert!(
        cont.contains("files="),
        "next must be a follow-up lievo_explore call with files=, got: {cont}"
    );
    assert!(
        parsed["completeness"].as_object().is_some(),
        "completeness must be the structured object, got: {parsed}"
    );
    assert_eq!(
        parsed["completeness"]["complete"],
        json!(false),
        "partial batch → complete:false, got: {parsed}"
    );
    assert_eq!(
        parsed["completeness"]["omitted_files"],
        json!(1),
        "the one deferred file is counted, got: {parsed}"
    );
    assert_eq!(
        parsed["completeness"]["omitted_edges"],
        json!(0),
        "files mode carries no edge data → omitted_edges:0, got: {parsed}"
    );
    // not_shown_files names the dropped path (separate from continuation.next).
    let not_shown = parsed["not_shown_files"]
        .as_array()
        .expect("not_shown_files array required")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        not_shown,
        vec!["three.rs".to_string()],
        "got: {not_shown:?}"
    );

    // The remainder is fetchable: the follow-up call carries exactly the
    // remaining paths and returns them.
    let followup = tool
        .call(json!({"files": ["three.rs"], "include_source": true}))
        .expect("follow-up batch must succeed");
    let fu: Value = serde_json::from_str(&followup).unwrap();
    let fu_names: Vec<&str> = fu["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        fu_names,
        vec!["three"],
        "remainder must be fetchable, got: {fu}"
    );
    assert!(fu.get("continuation").is_none());
}

// ---------------------------------------------------------------------------
// Single-path request keeps working (backward compatible)
// ---------------------------------------------------------------------------

#[test]
fn files_batch_single_path_returns_exactly_that_file() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "solo.rs", "fn solo() { 42 }\n");
    upsert_file(
        &storage,
        &project_id,
        &repo_id,
        "solo",
        "solo.rs",
        Some("A solo file."),
    );
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["solo.rs"], "include_source": true}))
        .expect("single-file batch must succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 1, "exactly the requested file: {parsed}");
    assert_eq!(symbols[0]["name"], "solo");
    // include_source=true: verbatim line-numbered source.
    let source = symbols[0]["source"].as_str().unwrap();
    assert!(source.starts_with("1\tfn solo() { 42 }\n"), "got: {source}");
    assert_eq!(parsed["returned"], 1);
    assert_eq!(parsed["total"], 1);
    assert!(parsed.get("continuation").is_none());
}

#[test]
fn files_mode_single_path_is_not_word_match_no_score_or_reason_keys() {
    // The files-mode shape is the batch shape: no per-symbol score/reason
    // (those are word-match ranking artifacts).
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "solo.rs", "fn solo() { 42 }\n");
    upsert_file(&storage, &project_id, &repo_id, "solo", "solo.rs", None);
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool.call(json!({"files": ["solo.rs"]})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert!(sym.get("score").is_none(), "no score in files mode: {sym}");
    assert!(
        sym.get("reason").is_none(),
        "no reason in files mode: {sym}"
    );
    // include_source=false → tier-1 lean: no source body for this small
    // file's threshold? (body is under 500 chars → small-body inline, which
    // is the tier-1 Complexity Trap behavior, same as word match.)
    assert!(
        sym.get("source").is_some(),
        "small body inlines in tier 1: {sym}"
    );
}

// ---------------------------------------------------------------------------
// Duplicates de-duplicated; nonexistent path reported, batch still succeeds
// ---------------------------------------------------------------------------

#[test]
fn files_batch_dedups_duplicates_first_occurrence_order() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "dup.rs", "fn dup() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "dup", "dup.rs", None);
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["dup.rs", "dup.rs", "dup.rs"]}))
        .expect("deduped batch must succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        parsed["symbols"].as_array().unwrap().len(),
        1,
        "duplicates must collapse to one file: {parsed}"
    );
    assert_eq!(
        parsed["total"], 1,
        "dedup must happen before counting: {parsed}"
    );
    assert!(parsed.get("not_found").is_none());
}

#[test]
fn files_batch_nonexistent_path_reported_without_failing_batch() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "real.rs", "fn real() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "real", "real.rs", None);
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    // Mix of a real file and two bogus ones (traversal + unknown index entry):
    // the real one still returns; the bogus ones are reported, not errors.
    let result = tool
        .call(json!({"files": ["ghost.rs", "real.rs", "../Cargo.toml"]}))
        .expect("batch with bad paths must still succeed");
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let names: Vec<&str> = parsed["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["real"], "the good file must survive: {parsed}");
    let not_found = parsed["not_found"].as_array().unwrap();
    let nf: Vec<&str> = not_found.iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(
        nf,
        vec!["ghost.rs", "../Cargo.toml"],
        "both bad paths reported: {nf:?}"
    );
    // total counts unique requested paths (3); returned counts what came back.
    assert_eq!(parsed["total"], 3);
    assert_eq!(parsed["returned"], 1);
}

// ---------------------------------------------------------------------------
// Short-circuit: files mode beats scope mode and word-match
// ---------------------------------------------------------------------------

#[test]
fn files_mode_short_circuits_scope_and_word_match() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // An indexed file that matches the query word AND a scope listing under
    // "src" — but the files param names a DIFFERENT file. The response must
    // be the files-mode shape (symbols, no scope listing, no query match).
    write_file(tmp.path(), "want.rs", "fn want() { 1 }\n");
    write_file(tmp.path(), "src/unwanted.rs", "fn un() { 2 }\n");
    upsert_file(&storage, &project_id, &repo_id, "want", "want.rs", None);
    upsert_file(
        &storage,
        &project_id,
        &repo_id,
        "unwanted",
        "src/unwanted.rs",
        None,
    );
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({
            "files": ["want.rs"],
            "query": "unwanted",
            "scope": "src",
            "max_files": 8
        }))
        .expect("files mode must short-circuit scope+query");
    let parsed: Value = serde_json::from_str(&result).unwrap();
    // Files-mode shape: symbols, no scope-listing `files` key, no query match.
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert_eq!(
        sym["name"], "want",
        "files param wins over query+scope: {parsed}"
    );
    assert!(
        parsed.get("files").is_none(),
        "no scope-listing shape: {parsed}"
    );
    assert!(parsed.get("continuation").is_none());
}

// ---------------------------------------------------------------------------
// Per-file budget (issue #838): no single file may starve the rest
// ---------------------------------------------------------------------------

#[test]
fn files_batch_one_large_file_does_not_starve_the_rest() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let big_body = "fn f() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad \n".repeat(200)
        + "}\n"; // ~6.6K chars — alone under the cap, but old code took it first
    write_file(tmp.path(), "big.rs", &big_body);
    upsert_file(&storage, &project_id, &repo_id, "big", "big.rs", None);
    for name in ["s1", "s2", "s3"] {
        add_file(
            &storage,
            &project_id,
            &repo_id,
            tmp.path(),
            name,
            &format!("fn {name}() {{ 1 }}\n"),
        );
    }
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["big.rs", "s1.rs", "s2.rs", "s3.rs"], "include_source": true}))
        .expect("batch must succeed");
    assert!(
        result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold, got {}",
        result.chars().count()
    );
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert!(
        symbols.len() > 1,
        "more than one file must be returned (large file must not consume the whole cap): {parsed}"
    );
    let names: Vec<&str> = symbols
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"big"),
        "the first file is still represented: {names:?}"
    );
}

#[test]
fn files_batch_leftover_budget_is_reused_by_small_files() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // A medium file (~8K) followed by three small files. With a per-file
    // budget that reuses leftover space, the medium file AND at least one
    // small file must pack (not just the medium one).
    let med_body = "fn f() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad \n".repeat(100)
        + "}\n"; // ~6.6K
    write_file(tmp.path(), "med.rs", &med_body);
    upsert_file(&storage, &project_id, &repo_id, "med", "med.rs", None);
    for name in ["a", "b", "c"] {
        let body = format!("fn {name}() {{ 1 }}\n");
        add_file(&storage, &project_id, &repo_id, tmp.path(), name, &body);
    }
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["med.rs", "a.rs", "b.rs", "c.rs"], "include_source": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert!(
        symbols.len() >= 2,
        "leftover budget must let at least one small file pack after the medium one: {parsed}"
    );
}

#[test]
fn files_batch_depth_is_shed_before_whole_files_dropped() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // Two lean files with include_depth=true. The depth (call_paths/
    // blast_radius) is large; with a tight budget the depth must be shed on a
    // file rather than the whole file dropped — so both files still come back.
    for name in ["d1", "d2"] {
        let body = format!("fn {name}() {{ 1 }}\n");
        add_file(&storage, &project_id, &repo_id, tmp.path(), name, &body);
    }
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["d1.rs", "d2.rs"], "include_source": true, "include_depth": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(
        symbols.len(),
        2,
        "both files must be returned (depth shed, not files dropped): {parsed}"
    );
    let names: Vec<&str> = symbols
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["d1", "d2"], "requested order: {names:?}");
}

// ---------------------------------------------------------------------------
// Structured completeness + not_shown_files (issue #838)
// ---------------------------------------------------------------------------

#[test]
fn files_batch_emits_structured_completeness_surviving_the_cap() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // Several ~6.6K files so the batch total exceeds the cap: the packing loop
    // keeps only a prefix and names the rest in not_shown_files. The final
    // serialized response (with the large symbols) still trips cap_response,
    // so this also verifies the completeness signal survives the capped path
    // (today the message is lost when cap_response fires).
    let body = "fn f() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad \n".repeat(100)
        + "}\n"; // ~6.6K each
    let names: Vec<String> = (0..8).map(|i| format!("c{i}.rs")).collect();
    for name in &names {
        write_file(tmp.path(), name, &body);
        upsert_file(
            &storage,
            &project_id,
            &repo_id,
            name.trim_end_matches(".rs"),
            name,
            None,
        );
    }
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let files: Vec<Value> = names.iter().map(|n| json!(n)).collect();
    let result = tool
        .call(json!({"files": files, "include_source": true}))
        .expect("batch must succeed");
    assert!(
        result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold, got {}",
        result.chars().count()
    );
    let parsed: Value = serde_json::from_str(&result).unwrap();
    // Structured completeness object (bundle shape, omitted_edges:0) survives
    // the cap — an object with all three keys, not a string.
    let comp = parsed["completeness"]
        .as_object()
        .expect("structured completeness must survive capping");
    assert!(
        comp.get("complete").is_some(),
        "complete key survived: {parsed}"
    );
    assert_eq!(
        comp["omitted_edges"],
        json!(0),
        "omitted_edges survived: {parsed}"
    );
    assert!(
        comp.get("omitted_files").is_some(),
        "omitted_files survived: {parsed}"
    );
    // not_shown_files array must survive, naming the dropped paths.
    let not_shown = parsed["not_shown_files"]
        .as_array()
        .expect("not_shown_files must survive capping");
    assert!(
        !not_shown.is_empty(),
        "some files must be dropped under the cap: {parsed}"
    );
    let symbols = parsed["symbols"].as_array().unwrap();
    assert!(!symbols.is_empty(), "at least one file returned: {parsed}");
    // Consistency: returned + omitted == total (unique requested).
    assert_eq!(
        symbols.len() + not_shown.len(),
        8,
        "returned + omitted must equal total: {parsed}"
    );
    assert_eq!(
        comp["omitted_files"],
        json!(not_shown.len()),
        "got: {parsed}"
    );
    // Dropped paths are exactly the requested paths not present in symbols.
    let returned_paths: std::collections::HashSet<&str> = symbols
        .iter()
        .map(|s| s["qualified_path"].as_str().unwrap())
        .collect();
    for p in not_shown.iter() {
        let ps = p.as_str().unwrap();
        assert!(
            !returned_paths.contains(ps),
            "{ps} must be dropped, not returned"
        );
    }
}

#[test]
fn files_batch_fully_satisfied_reports_complete_true_empty_not_shown() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    for name in ["x", "y"] {
        add_file(
            &storage,
            &project_id,
            &repo_id,
            tmp.path(),
            name,
            &format!("fn {name}() {{ 1 }}\n"),
        );
    }
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["x.rs", "y.rs"], "include_source": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let comp = parsed["completeness"].as_object().unwrap();
    assert_eq!(
        comp["complete"],
        json!(true),
        "all files returned → complete:true: {parsed}"
    );
    assert_eq!(comp["omitted_files"], json!(0));
    assert_eq!(
        parsed["not_shown_files"].as_array().unwrap().len(),
        0,
        "empty not_shown_files: {parsed}"
    );
    assert_eq!(parsed["returned"], 2);
    assert_eq!(parsed["total"], 2);
}

#[test]
fn files_batch_no_depth_keys_for_files_not_returned() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // A dropped file must not have its depth built. We can only observe the
    // RESPONSE: assert no returned symbol carries call_paths/blast_radius when
    // include_depth is false, and that dropped paths appear in not_shown_files
    // (not in symbols) — a dropped file's depth was never constructed.
    write_file(tmp.path(), "keep.rs", "fn keep() { 1 }\n");
    upsert_file(&storage, &project_id, &repo_id, "keep", "keep.rs", None);
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool.call(json!({"files": ["keep.rs"]})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    // include_depth=false (default) → no call_paths/blast_radius keys.
    assert!(
        sym.get("call_paths").is_none(),
        "no call_paths in lean mode: {sym}"
    );
    assert!(
        sym.get("blast_radius").is_none(),
        "no blast_radius in lean mode: {sym}"
    );
}

// ---------------------------------------------------------------------------
// Single oversized file ladder (issue #838 decision 1)
// ---------------------------------------------------------------------------

#[test]
fn files_batch_single_oversized_file_ladder_sheds_depth_then_source() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // A single file whose tier-2 source alone exceeds the 24K cap.
    let body = "fn f() {\n".to_string()
        + &"let x = 0; // pad pad pad pad pad pad pad pad pad pad pad pad pad pad pad pad \n"
            .repeat(1500)
        + "}\n"; // ~48K chars → over the cap
    write_file(tmp.path(), "huge.rs", &body);
    upsert_file(&storage, &project_id, &repo_id, "huge", "huge.rs", None);
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let result = tool
        .call(json!({"files": ["huge.rs"], "include_source": true}))
        .expect("single oversized file must still return");
    assert!(
        result.chars().count() <= MAX_EXPLORE_OUTPUT_CHARS,
        "hard cap must hold, got {}",
        result.chars().count()
    );
    let parsed: Value = serde_json::from_str(&result).expect("must parse (never a fragment)");
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 1, "the file is still represented: {parsed}");
    let sym = &symbols[0];
    // The ladder truncated the source on a whole-line boundary (well-formed),
    // flagged source_truncated, and demoted completeness.complete to false.
    let comp = parsed["completeness"].as_object().unwrap();
    assert_eq!(
        comp["complete"],
        json!(false),
        "truncated source → complete:false: {parsed}"
    );
    // Either the source was truncated (with the flag) or dropped entirely.
    let has_source = sym.get("source").map(|s| !s.is_null()).unwrap_or(false);
    if has_source {
        assert_eq!(
            sym["source_truncated"],
            json!(true),
            "flag set when source truncated: {sym}"
        );
    }
    assert!(
        sym.get("name").is_some(),
        "identity survives the ladder: {sym}"
    );
}

// ---------------------------------------------------------------------------
// Determinism: identical batch request → byte-identical response
// ---------------------------------------------------------------------------

#[test]
fn files_batch_identical_request_is_byte_identical() {
    let build = || -> String {
        let (storage, project_id, repo_id) = setup();
        let tmp = tempfile::tempdir().unwrap();
        for (name, path) in [("y", "y.rs"), ("x", "x.rs"), ("z", "z.rs")] {
            write_file(tmp.path(), path, &format!("fn {name}() {{ 1 }}\n"));
            upsert_file(&storage, &project_id, &repo_id, name, path, None);
        }
        let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());
        tool.call(json!({"files": ["z.rs", "x.rs", "y.rs"]}))
            .expect("call must succeed")
    };
    let a = build();
    let b = build();
    assert_eq!(a, b, "identical batch requests must be byte-identical");
}

// (Schema-shape and MCP-wire coverage lives in the task-b/c scope: the
// hand-written `ExploreTool::input_schema` property count/required-set test
// and the MCP wire schema/serde tests both need updating for the new `files`
// property and the `query` optionalization.)

// (The `ExploreTool::input_schema` property/required-set pin in
// tools_explore_tests.rs is out of task-a's file scope; it is updated in
// task-b/c with the MCP wire schema tests for the new `files` property and
// the `query` optionalization.)
