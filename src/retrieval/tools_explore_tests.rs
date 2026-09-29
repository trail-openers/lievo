// ExploreTool tests (issues #680/#682).
//
// Wired via `#[path]` from tools_explore.rs. Covers:
//   - tier-1 / tier-2 behavior
//   - Complexity Trap (small body inline, large body → summary)
//   - 24K output cap with truncation signal
//   - continuation pointer when matches exceed max_files
//   - UTF-8 boundary safety for line-numbered source
//   - zero-match and not-indexed success-shaped guidance
//   - input_schema: include_source + max_files only (two tiers, no third knob)

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::{ExploreTool, ToolContext};
use crate::retrieval::tools_explore::{
    SMALL_BODY_THRESHOLD_CHARS, continuation_pointer, is_small_body, line_numbered_source,
};
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn file_entity(
    id: &str,
    project_id: &str,
    repo_id: &str,
    name: &str,
    path: &str,
    summary: Option<&str>,
) -> Entity {
    Entity {
        id: id.to_string(),
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
    }
}

fn func_entity(
    id: &str,
    project_id: &str,
    repo_id: &str,
    name: &str,
    parent_id: Option<&str>,
) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: project_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        tier: EntityTier::Function,
        parent_id: parent_id.map(|s| s.to_string()),
        name: name.to_string(),
        path: Some("src/main.rs".to_string()),
        language: Some("Rust".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

/// Create the project + a repo row (repo_id foreign key) so entities
/// referencing the repo can be upserted.
fn setup_project_with_repo(name: &str) -> (SqliteStorage, String, String) {
    let (storage, project_id) = setup_project(name);
    let repo = storage
        .add_repo(&project_id, "repo1", "/tmp/repo1")
        .unwrap();
    (storage, project_id, repo.id)
}

fn make_ctx(
    storage: SqliteStorage,
    project_id: String,
    repo_path: PathBuf,
) -> Arc<ToolContext<SqliteStorage>> {
    Arc::new(ToolContext {
        storage: Arc::new(Mutex::new(storage)),
        project_id,
        repo_path,
        output_dir: None,
        zero_repo_guidance: None,
    })
}

fn setup_project(name: &str) -> (SqliteStorage, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let project = storage.create_project(name, None).unwrap();
    (storage, project.id)
}

fn write_file(tmp: &std::path::Path, rel: &str, contents: &str) {
    let full = tmp.join(rel);
    if let Some(parent) = full.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::File::create(full).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
}

// ---------------------------------------------------------------------------
// Free-function unit tests
// ---------------------------------------------------------------------------

#[test]
fn line_numbered_source_formats_lines() {
    let src = "fn a() {}\nfn b() {}\n";
    assert_eq!(line_numbered_source(src), "1\tfn a() {}\n2\tfn b() {}\n");
}

#[test]
fn line_numbered_source_handles_multibyte_chars() {
    let src = "fn main() { let x = \"héllo — wörld\"; }\n";
    let out = line_numbered_source(src);
    assert!(out.starts_with("1\t"));
    assert!(out.contains("héllo — wörld"));
}

#[test]
fn continuation_pointer_includes_counts_and_next() {
    let ptr = continuation_pointer(8, 42, "lievo_explore(query='x', max_files=15)");
    assert!(ptr.contains("returned: 8"));
    assert!(ptr.contains("total: 42"));
    assert!(ptr.contains("next: \"lievo_explore(query='x', max_files=15)\""));
}

#[test]
fn is_small_body_threshold_boundary() {
    let small = "x".repeat(SMALL_BODY_THRESHOLD_CHARS - 1);
    let exact = "x".repeat(SMALL_BODY_THRESHOLD_CHARS);
    let large = "x".repeat(SMALL_BODY_THRESHOLD_CHARS + 1);
    assert!(is_small_body(Some(&small)));
    assert!(
        !is_small_body(Some(&exact)),
        "boundary is exclusive: exactly-at-threshold is NOT small"
    );
    assert!(!is_small_body(Some(&large)));
    assert!(!is_small_body(None));
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

#[test]
fn explore_input_schema_exposes_only_two_tier_knobs() {
    let (storage, project_id) = setup_project("explore-schema");
    let ctx = make_ctx(storage, project_id, PathBuf::new());
    let tool = ExploreTool { ctx };

    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    // Exactly eight properties: query, include_source, max_files (two-tier
    // disclosure, issue #682 AC #5), include_depth (depth-volume knob,
    // issue #723) plus scope + offset (scope-membership listing mode, issue
    // #712 PM decision 2026-09-12), files (batched source fetch, issue #741)
    // and bundle (subsystem-bundle mode, issue #743) — no other disclosure knob
    // (tier/depth/detail) exists alongside them.
    assert_eq!(props.len(), 8, "expected 8 properties, got: {props:?}");
    assert!(props.contains_key("query"));
    assert!(props.contains_key("include_source"));
    assert!(props.contains_key("include_depth"));
    assert!(props.contains_key("max_files"));
    assert!(props.contains_key("scope"));
    assert!(props.contains_key("offset"));
    assert!(props.contains_key("files"));
    assert!(props.contains_key("bundle"));
    assert_eq!(props["files"]["type"], "array");
    assert_eq!(props["bundle"]["type"], "string");
    assert_eq!(props["include_source"]["type"], "boolean");
    assert_eq!(props["include_depth"]["type"], "boolean");
    assert_eq!(props["max_files"]["type"], "integer");
    assert_eq!(props["scope"]["type"], "string");
    assert_eq!(props["offset"]["type"], "integer");
    assert!(
        !props.contains_key("tier")
            && !props.contains_key("depth")
            && !props.contains_key("detail"),
        "no third disclosure knob may exist (two tiers only)"
    );
    let required = schema["required"].as_array().unwrap();
    assert_eq!(
        required,
        &Vec::<Value>::new(),
        "no field is required: query is optional when `files` is passed (issue #741)"
    );
    assert!(
        !required.contains(&"include_source".into()),
        "include_source must be optional (tier 1 is the default)"
    );
    assert!(
        !required.contains(&"scope".into()),
        "scope must be optional"
    );
    assert!(
        !required.contains(&"offset".into()),
        "offset must be optional"
    );
    assert!(
        !required.contains(&"include_depth".into()),
        "include_depth must be optional (default false, issue #731)"
    );
    assert!(
        !required.contains(&"files".into()),
        "files must be optional (issue #741)"
    );
    assert!(
        !required.contains(&"bundle".into()),
        "bundle must be optional (subsystem-bundle mode, issue #743)"
    );
}

// ---------------------------------------------------------------------------
// Tier 1: map only, no source body
// ---------------------------------------------------------------------------

#[test]
fn tier1_returns_symbol_map_without_source_bodies() {
    let (storage, project_id, repo_id) = setup_project_with_repo("explore-tier1");
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "auth.rs",
        "fn authenticate() { /* large body */ }\n",
    );
    let large_body = "fn authenticate() { ";
    let _ = large_body;

    // File 1: large body, with a stored summary.
    let f1 = file_entity(
        "proj1:repo1:file:auth.rs",
        &project_id,
        &repo_id,
        "auth",
        "auth.rs",
        Some("Implements token-based authentication for the API layer."),
    );
    storage.upsert_entity(&f1).unwrap();

    // Make the on-disk body LARGE (> 500 chars) so the Complexity Trap
    // classifies it as large → summary, not inline body.
    let big = format!(
        "fn authenticate() {{\n  {}\n}}\n",
        "let x = 1; ".repeat(200)
    );
    std::fs::write(tmp.path().join("auth.rs"), &big).unwrap();

    let ctx = make_ctx(storage, project_id, tmp.path().to_path_buf());
    let tool = ExploreTool { ctx };

    // Issue #731: explicit include_depth=true — the bare default is now lean.
    let result = tool
        .call(json!({"query": "auth", "include_depth": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 1);

    let sym = &symbols[0];
    // Tier 1 map fields present (entity_id intentionally absent from the
    // per-symbol payload — issue #834; derivable from qualified_path + tier).
    assert!(sym.get("entity_id").is_none());
    assert_eq!(sym["name"], "auth");
    assert_eq!(sym["kind"], "file");
    assert_eq!(sym["qualified_path"], "auth.rs");
    // Large body → summary, NOT inline body.
    assert!(
        sym.get("summary").is_some(),
        "large body should carry summary"
    );
    assert!(
        sym.get("source").is_none(),
        "tier-1 large-body symbol must NOT carry a source key, got: {sym}"
    );
    assert!(sym.get("small_body_inline").is_none());
    // call_paths + blast radius present (empty arrays ok for isolated file).
    assert!(sym.get("call_paths").is_some());
    assert!(sym.get("blast_radius").is_some());
}

// ---------------------------------------------------------------------------
// Complexity Trap: small body inline, no summary
// ---------------------------------------------------------------------------

#[test]
fn complexity_trap_small_body_inline_no_summary() {
    let (storage, project_id, repo_id) = setup_project_with_repo("explore-trap-small");
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "util.rs",
        "fn add(a: i32, b: i32) -> i32 { a + b }\n",
    );

    // Entity deliberately has NO stored summary (apfel unavailable scenario).
    let f1 = file_entity(
        "proj1:repo1:file:util.rs",
        &project_id,
        &repo_id,
        "util",
        "util.rs",
        None,
    );
    storage.upsert_entity(&f1).unwrap();

    let ctx = make_ctx(storage, project_id, tmp.path().to_path_buf());
    let tool = ExploreTool { ctx };

    let result = tool.call(json!({"query": "util"})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert!(
        sym.get("source").is_some(),
        "small body should carry inline source, got: {sym}"
    );
    assert_eq!(sym["small_body_inline"], true);
    assert!(
        sym.get("summary").is_none(),
        "small-body symbol must NOT carry a summary, got: {sym}"
    );
}

#[test]
fn complexity_trap_mixed_query_small_inline_large_summary() {
    let (storage, project_id, repo_id) = setup_project_with_repo("explore-trap-mixed");
    let tmp = tempfile::tempdir().unwrap();

    // Small body file.
    write_file(tmp.path(), "small.rs", "fn tiny() { 1 }\n");
    // Large body file (> 500 chars on disk) with a stored summary.
    let big = format!("fn large() {{\n  {}\n}}\n", "let y = 2; ".repeat(200));
    write_file(tmp.path(), "large.rs", &big);

    let small = file_entity(
        "p:repo1:file:small.rs",
        &project_id,
        &repo_id,
        "small",
        "small.rs",
        None,
    );
    let large = file_entity(
        "p:repo1:file:large.rs",
        &project_id,
        &repo_id,
        "large",
        "large.rs",
        Some("A very large body symbol with a stored summary."),
    );
    storage.upsert_entity(&small).unwrap();
    storage.upsert_entity(&large).unwrap();

    let ctx = make_ctx(storage, project_id, tmp.path().to_path_buf());
    let tool = ExploreTool { ctx };

    let result = tool.call(json!({"query": "rs"})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 2);

    let by_name: std::collections::HashMap<&str, &Value> = symbols
        .iter()
        .map(|s| (s["name"].as_str().unwrap(), s))
        .collect();

    let small_sym = by_name["small"];
    let large_sym = by_name["large"];

    // Small: inline body, no summary.
    assert!(small_sym.get("source").is_some());
    assert!(small_sym.get("summary").is_none());

    // Large: summary, no source.
    assert!(large_sym.get("summary").is_some());
    assert!(large_sym.get("source").is_none());
}

// ---------------------------------------------------------------------------
// Tier 2: include_source=true
// ---------------------------------------------------------------------------

#[test]
fn tier2_include_source_returns_verbatim_line_numbered() {
    let (storage, project_id, repo_id) = setup_project_with_repo("explore-tier2");
    let tmp = tempfile::tempdir().unwrap();
    write_file(
        tmp.path(),
        "src/hello.rs",
        "fn hello() { println!(\"hi\"); }\nfn world() { 1 }\n",
    );

    let f1 = file_entity(
        "p:repo1:file:src/hello.rs",
        &project_id,
        &repo_id,
        "hello",
        "src/hello.rs",
        Some("Hello-world demo functions."),
    );
    storage.upsert_entity(&f1).unwrap();

    let ctx = make_ctx(storage, project_id, tmp.path().to_path_buf());
    let tool = ExploreTool { ctx };

    let result = tool
        .call(json!({"query": "hello", "include_source": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    let source = sym["source"].as_str().unwrap();
    assert!(
        source.starts_with("1\tfn hello() { println!(\"hi\"); }\n2\tfn world() { 1 }\n"),
        "expected line-numbered source, got: {source}"
    );
    // Even small bodies get a source in tier 2 (no summary key needed).
    assert!(sym.get("summary").is_none());
}

// ---------------------------------------------------------------------------
// Continuation pointer when matches exceed max_files
// ---------------------------------------------------------------------------

#[test]
fn continuation_pointer_when_matches_exceed_max_files() {
    let (storage, project_id, repo_id) = setup_project_with_repo("explore-continue");
    let tmp = tempfile::tempdir().unwrap();

    // 10 file entities all matching "alpha".
    for i in 0..10 {
        let path = format!("files/alpha_{i}.rs");
        write_file(tmp.path(), &path, "fn alpha() {}\n");
        let ent = file_entity(
            &format!("p:repo1:file:{path}"),
            &project_id,
            &repo_id,
            &format!("alpha_{i}"),
            &path,
            None,
        );
        storage.upsert_entity(&ent).unwrap();
    }

    let ctx = make_ctx(storage, project_id, tmp.path().to_path_buf());
    let tool = ExploreTool { ctx };

    let result = tool
        .call(json!({"query": "alpha", "max_files": 3}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();

    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(
        symbols.len(),
        3,
        "should return exactly max_files=3 symbols"
    );

    let cont = parsed["continuation"].as_str().unwrap();
    assert!(cont.contains("returned: 3"), "got: {cont}");
    assert!(cont.contains("total: 10"), "got: {cont}");
    assert!(cont.contains("next:"), "got: {cont}");

    // Issue #711: the 3 returned files must be the 3 HIGHEST-scoring.
    // All 10 names hit "alpha" in both name and path, so all score 2 (one
    // distinct word, location max, no double count); the path-asc tiebreak
    // orders by full path: files/alpha_0 < files/alpha_1 < files/alpha_2 <
    // files/alpha_3 < ... < files/alpha_9 (string order of the numeric suffix).
    let names: Vec<&str> = parsed["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["alpha_0", "alpha_1", "alpha_2"]);

    // Omitted files are disclosed: 7 not shown, completeness line present.
    assert_eq!(parsed["not_shown"], 7);
    assert_eq!(
        parsed["completeness"].as_str(),
        Some("showing 3 of 10 matching files")
    );

    // Per-symbol score + reason are present and consistent with the order.
    let symbols = parsed["symbols"].as_array().unwrap();
    assert!(symbols[0].get("score").is_some());
    assert!(symbols[0].get("reason").is_some());
    assert_eq!(
        symbols[0]["score"], 2,
        "one distinct word 'alpha' hits name+path at the max"
    );
    assert_eq!(
        symbols[0]["reason"], "name: alpha",
        "name hit outranks path hit in the reason"
    );
}

// ---------------------------------------------------------------------------
// Call paths + blast radius presence
// ---------------------------------------------------------------------------

#[test]
fn call_paths_and_blast_radius_reflect_relationships() {
    let (storage, project_id, repo_id) = setup_project_with_repo("explore-calls");
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "caller.rs", "fn caller() { callee() }\n");
    write_file(tmp.path(), "callee.rs", "fn callee() {}\n");

    let caller = file_entity(
        "p:repo1:file:caller.rs",
        &project_id,
        &repo_id,
        "caller",
        "caller.rs",
        None,
    );
    let callee = file_entity(
        "p:repo1:file:callee.rs",
        &project_id,
        &repo_id,
        "callee",
        "callee.rs",
        None,
    );
    // Function entities for the call graph.
    let fn_caller = func_entity(
        "fn-caller",
        &project_id,
        &repo_id,
        "caller",
        Some("p:repo1:file:caller.rs"),
    );
    let fn_callee = func_entity(
        "fn-callee",
        &project_id,
        &repo_id,
        "callee",
        Some("p:repo1:file:callee.rs"),
    );
    storage.upsert_entity(&caller).unwrap();
    storage.upsert_entity(&callee).unwrap();
    storage.upsert_entity(&fn_caller).unwrap();
    storage.upsert_entity(&fn_callee).unwrap();

    let rel = Relationship {
        source_id: "fn-caller".to_string(),
        target_id: "fn-callee".to_string(),
        rel_type: RelType::Calls,
        weight: 1.0,
        evidence_json: None,
        provenance: EdgeProvenance::Heuristic,
    };
    storage.upsert_relationship(&rel).unwrap();

    let ctx = make_ctx(storage, project_id, tmp.path().to_path_buf());
    let tool = ExploreTool { ctx };

    // Issue #731: explicit include_depth=true — the bare default is now lean.
    let result = tool
        .call(json!({"query": "caller", "include_depth": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];
    assert_eq!(sym["name"], "caller");

    // caller's call_paths should include the call to callee.
    let call_paths = sym["call_paths"].as_array().unwrap();
    let has_out = call_paths
        .iter()
        .any(|cp| cp["name"] == "caller" && cp["calls"] == "callee" && cp["direction"] == "out");
    assert!(
        has_out,
        "expected an outgoing call path caller->callee, got: {call_paths:?}"
    );

    // Issue #759: blast_radius is INCOMING-only (dependents, not dependencies).
    // callee is an OUTGOING dependency of caller (caller calls callee), so it
    // must NOT appear in caller's blast_radius. The fixture has no incoming
    // edges to caller, so blast_radius is empty.
    let blast = sym["blast_radius"].as_array().unwrap();
    assert!(
        blast.is_empty(),
        "outgoing dependency must not appear in blast_radius (issue #759): {blast:?}"
    );
    let has_callee = blast.iter().any(|b| b["name"] == "callee");
    assert!(
        !has_callee,
        "callee is an outgoing dependency, not a dependent — must not be in blast_radius: {blast:?}"
    );
}

// ---------------------------------------------------------------------------
// Zero-match and not-indexed guidance
// ---------------------------------------------------------------------------

#[test]
fn zero_match_returns_guidance_with_empty_symbols() {
    let (storage, project_id, repo_id) = setup_project_with_repo("explore-zero-match");
    // One entity so the "no entities indexed" branch is NOT hit.
    let f1 = file_entity(
        "p:repo1:file:real.rs",
        &project_id,
        &repo_id,
        "real",
        "real.rs",
        None,
    );
    storage.upsert_entity(&f1).unwrap();

    let ctx = make_ctx(storage, project_id, PathBuf::new());
    let tool = ExploreTool { ctx };

    let result = tool.call(json!({"query": "nomatchxyz"})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["symbols"].as_array().unwrap().len(), 0);
    let warning = parsed["warning"].as_str().unwrap();
    assert!(
        warning.contains("No matching files or symbols"),
        "got: {warning}"
    );
}

#[test]
fn not_indexed_returns_refresh_guidance() {
    let (storage, project_id) = setup_project("explore-not-indexed");
    // No entities at all — the "run lievo refresh" branch.
    let ctx = make_ctx(storage, project_id, PathBuf::new());
    let tool = ExploreTool { ctx };

    let result = tool.call(json!({"query": "anything"})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["symbols"].as_array().unwrap().len(), 0);
    let warning = parsed["warning"].as_str().unwrap();
    assert!(warning.contains("No entities indexed"), "got: {warning}");
    // Issue #864: the not-indexed guidance no longer tells the agent to run
    // `lievo refresh` (the agent cannot run lievo commands while the MCP
    // server is running; indexing is automatic in the background).
    assert!(!warning.contains("lievo refresh"), "got: {warning}");
}

// ---------------------------------------------------------------------------
// UTF-8 boundary safety
// ---------------------------------------------------------------------------

#[test]
fn line_numbered_source_does_not_panic_on_multibyte_at_cap() {
    // Build a string of 3-byte chars (€) that would split a char if sliced
    // by byte index. line_numbered_source must preserve all chars.
    let line = "€".repeat(5000);
    let src = format!("{line}\n{line}\n");
    let out = line_numbered_source(&src);
    // 2 lines, each fully preserved.
    assert_eq!(out.lines().count(), 2);
    assert_eq!(out.lines().next().unwrap(), &format!("1\t{line}"));
}

#[test]
fn tier1_output_is_utf8_safe_with_multibyte_content() {
    let (storage, project_id, repo_id) = setup_project_with_repo("explore-utf8");
    let tmp = tempfile::tempdir().unwrap();
    let content = format!(
        "fn x() {{ let s = \"{}\"; }}\n",
        "héllo — wörld — 日本語".repeat(10)
    );
    write_file(tmp.path(), "utf8.rs", &content);

    let f1 = file_entity(
        "p:repo1:file:utf8.rs",
        &project_id,
        &repo_id,
        "utf8",
        "utf8.rs",
        None,
    );
    storage.upsert_entity(&f1).unwrap();

    let ctx = make_ctx(storage, project_id, tmp.path().to_path_buf());
    let tool = ExploreTool { ctx };

    let result = tool.call(json!({"query": "utf8"})).unwrap();
    // Must be valid JSON (not corrupted by a mid-char cut).
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let _ = parsed["symbols"].as_array().unwrap();
}

// ---------------------------------------------------------------------------
// max_files default
// ---------------------------------------------------------------------------

#[test]
fn default_max_files_is_eight() {
    let (storage, project_id, repo_id) = setup_project_with_repo("explore-default-max");
    let tmp = tempfile::tempdir().unwrap();

    // 10 matching files; default max_files=8 should return 8 and a continuation.
    for i in 0..10 {
        let path = format!("files/def_{i}.rs");
        write_file(tmp.path(), &path, "fn def() {}\n");
        let ent = file_entity(
            &format!("p:repo1:file:{path}"),
            &project_id,
            &repo_id,
            &format!("def_{i}"),
            &path,
            None,
        );
        storage.upsert_entity(&ent).unwrap();
    }

    let ctx = make_ctx(storage, project_id, tmp.path().to_path_buf());
    let tool = ExploreTool { ctx };

    let result = tool.call(json!({"query": "def"})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 8, "default max_files should be 8");
    let cont = parsed["continuation"].as_str().unwrap();
    assert!(cont.contains("returned: 8"), "got: {cont}");
    assert!(cont.contains("total: 10"), "got: {cont}");

    // Issue #711: pin the exact returned name sequence under the new
    // rank→take pipeline. All 10 names hit "def" in both name and path, so
    // all score 2 (one distinct word, location max, no double count); the
    // path-asc tiebreak then orders by full path: files/def_0 < files/def_1
    // < files/def_2 < files/def_3 < ... < files/def_9 (string order of the
    // numeric suffix).
    let names: Vec<&str> = symbols
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "def_0", "def_1", "def_2", "def_3", "def_4", "def_5", "def_6", "def_7"
        ]
    );
    // Tied scores, path-only reason (every word hits the path; none hit the
    // name stem uniquely beyond the shared "def").
    assert_eq!(symbols[0]["score"], 2);
    assert_eq!(
        symbols[0]["reason"], "name: def",
        "name hit outranks path hit in the reason"
    );

    // Breadth disclosure: 2 files cut → not-shown count + completeness line.
    assert_eq!(parsed["not_shown"], 2);
    assert_eq!(
        parsed["completeness"].as_str(),
        Some("showing 8 of 10 matching files")
    );
}
