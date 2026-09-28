// Tier-behavior and integration coverage for lievo_explore (issue #682,
// workstream task-d). Wired via `#[path]` from tools_explore.rs.
//
// Covers the acceptance criteria that the base suite in
// tools_explore_tests.rs does not:
//   - AC #4 (sharpened): measured tier-1 payload bytes < 1/10 of tier-2
//     payload bytes when every matched symbol is above the small-body
//     threshold; mixed queries carry no body key above the threshold.
//   - AC #2: verbatim line-numbered source for exactly the matched symbols
//     (no leakage of near-miss entities).
//   - AC #3 / Complexity Trap boundary: a body exactly at the threshold
//     deterministically lands in the summary tier (n-1 inline, n+1 summary).
//   - Tier-1 payload field shape: name, kind, qualified_path, signature,
//     call_paths, blast radius present; no source body for large symbols.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::model::{EdgeProvenance, Entity, EntityTier, RelType, Relationship};
use crate::retrieval::tool_trait::Tool;
use crate::retrieval::tools::ExploreTool;
use crate::retrieval::tools_explore::{MAX_MAX_FILES, SMALL_BODY_THRESHOLD_CHARS};
use crate::storage::Storage;
use crate::storage::sqlite::SqliteStorage;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn setup() -> (SqliteStorage, String, String) {
    let storage = SqliteStorage::open_in_memory().unwrap();
    // Distinct project name per call: create_project is INSERT OR IGNORE with
    // a UNIQUE name, so a shared name would silently reuse a row and leave the
    // in-memory connection's rowid sequence behind the entity rowids — which
    // makes list_entities' rowid-ascending order non-upsert-order.
    let unique = uuid::Uuid::new_v4().to_string();
    let project = storage
        .create_project(&format!("tier-{unique}"), None)
        .unwrap();
    let repo = storage
        .add_repo(&project.id, "repo1", "/tmp/repo1")
        .unwrap();
    (storage, project.id, repo.id)
}

fn upsert_file_entity(
    storage: &SqliteStorage,
    project_id: &str,
    repo_id: &str,
    name: &str,
    summary: Option<&str>,
) {
    let id = format!("p:repo1:file:{name}.rs");
    storage
        .upsert_entity(&file_entity(
            &id,
            project_id,
            repo_id,
            name,
            &format!("{name}.rs"),
            summary,
        ))
        .unwrap();
}

pub(super) fn file_entity(
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

pub(super) fn make_tool(
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

pub(super) fn write_file(root: &std::path::Path, rel: &str, contents: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut f = std::fs::File::create(full).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
}

/// Build a body of exactly `chars` chars (padding of `let x = N; ` lines).
fn body_of_chars(target: usize) -> String {
    let mut body = String::from("fn f() {\n");
    let mut i = 0u32;
    while body.chars().count() + "let x = 0; ".len() <= target {
        body.push_str("let x = ");
        body.push_str(&i.to_string());
        body.push_str("; ");
        i += 1;
    }
    body.push_str("}\n");
    body
}

// ---------------------------------------------------------------------------
// AC #4 (sharpened): payload-size ratio, large bodies only
// ---------------------------------------------------------------------------

#[test]
fn tier1_payload_is_an_order_of_magnitude_smaller_for_large_bodies() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();

    // Two files, both bodies above the small-body threshold.
    for name in ["alpha_one", "alpha_two"] {
        let body = body_of_chars(4000);
        write_file(tmp.path(), &format!("{name}.rs"), &body);
        let ent = file_entity(
            &format!("p:repo1:file:{name}.rs"),
            &project_id,
            &repo_id,
            name,
            &format!("{name}.rs"),
            Some(&format!("A one-line stored summary for {name}.")),
        );
        storage.upsert_entity(&ent).unwrap();
    }
    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());

    let tier1 = tool
        .call(json!({"query": "alpha"}))
        .expect("tier-1 call must succeed");
    let tier2 = tool
        .call(json!({"query": "alpha", "include_source": true}))
        .expect("tier-2 call must succeed");

    let t1: Value = serde_json::from_str(&tier1).unwrap();
    let t2: Value = serde_json::from_str(&tier2).unwrap();

    let t1_bytes = tier1.len();
    let t2_bytes = tier2.len();
    assert!(
        t1_bytes * 10 < t2_bytes,
        "tier-1 payload ({t1_bytes} bytes) must be < 1/10 of tier-2 payload ({t2_bytes} bytes)"
    );

    // No source bodies in tier 1, verbatim source in tier 2.
    for sym in t1["symbols"].as_array().unwrap() {
        assert!(
            sym.get("source").is_none(),
            "tier-1 must carry no source: {sym}"
        );
    }
    for sym in t2["symbols"].as_array().unwrap() {
        assert!(
            sym.get("source").is_some(),
            "tier-2 must carry source: {sym}"
        );
    }
}

// ---------------------------------------------------------------------------
// Mixed query: small inline, large summary (no body key above threshold)
// ---------------------------------------------------------------------------

#[test]
fn mixed_query_small_inline_large_no_body_key_above_threshold() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();

    write_file(tmp.path(), "mix_small.rs", "fn tiny() { 1 }\n");
    let big = body_of_chars(SMALL_BODY_THRESHOLD_CHARS + 200);
    write_file(tmp.path(), "mix_large.rs", &big);
    for (name, summary) in [
        ("mix_small", None),
        (
            "mix_large",
            Some("A large symbol with a stored one-line summary."),
        ),
    ] {
        let ent = file_entity(
            &format!("p:repo1:file:{name}.rs"),
            &project_id,
            &repo_id,
            name,
            &format!("{name}.rs"),
            summary,
        );
        storage.upsert_entity(&ent).unwrap();
    }

    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());
    let result = tool.call(json!({"query": "mix"})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 2);

    let by_name: std::collections::HashMap<&str, &Value> = symbols
        .iter()
        .map(|s| (s["name"].as_str().unwrap(), s))
        .collect();

    // Above threshold: summary, NO body key.
    assert!(by_name["mix_large"].get("source").is_none());
    assert!(by_name["mix_large"].get("summary").is_some());
    // Below threshold: body inline, no summary.
    assert!(by_name["mix_small"].get("source").is_some());
    assert!(by_name["mix_small"].get("summary").is_none());
}

// ---------------------------------------------------------------------------
// Complexity Trap boundary: n-1 inline, n+1 summary (deterministic)
// ---------------------------------------------------------------------------

#[test]
fn small_body_threshold_boundary_is_deterministic() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();

    // n-1: strictly below threshold → inline body. Bodies are bare lines
    // (no wrapper) so the char count lands exactly on either side of the
    // threshold.
    let below = "x".repeat(SMALL_BODY_THRESHOLD_CHARS - 1);
    std::fs::write(tmp.path().join("bnd_below.rs"), &below).unwrap();
    // n+1: strictly above → summary (with stored summary).
    let above = "x".repeat(SMALL_BODY_THRESHOLD_CHARS + 1);
    std::fs::write(tmp.path().join("bnd_above.rs"), &above).unwrap();

    upsert_file_entity(
        &storage,
        &project_id,
        &repo_id,
        "bnd_below",
        Some("should NOT appear for small bodies"),
    );
    upsert_file_entity(
        &storage,
        &project_id,
        &repo_id,
        "bnd_above",
        Some("the stored summary for the above-threshold symbol"),
    );

    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());
    let result = tool.call(json!({"query": "bnd"})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    let by_name: std::collections::HashMap<&str, &Value> = symbols
        .iter()
        .map(|s| (s["name"].as_str().unwrap(), s))
        .collect();

    assert!(
        by_name["bnd_below"].get("source").is_some(),
        "below-threshold body must be inline"
    );
    assert!(
        by_name["bnd_below"].get("summary").is_none(),
        "below-threshold symbol must NOT carry a summary (Complexity Trap)"
    );
    assert!(
        by_name["bnd_above"].get("source").is_none(),
        "above-threshold body must NOT be inlined in tier 1"
    );
    assert_eq!(
        by_name["bnd_above"]["summary"].as_str().unwrap(),
        "the stored summary for the above-threshold symbol"
    );
}

// ---------------------------------------------------------------------------
// AC #2: verbatim source for exactly the matched symbols (no leakage)
// ---------------------------------------------------------------------------

#[test]
fn tier2_returns_source_for_exactly_matched_symbols_no_leakage() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();

    // One matched file and one near-miss file that must NOT leak.
    for (path, name) in [("needle.rs", "needle"), ("unrelated.rs", "unrelated")] {
        let fn_body = if name == "needle" {
            "fn needle() {}"
        } else {
            "fn unrelated() {}"
        };
        write_file(tmp.path(), path, &format!("{}\n", fn_body));
        let ent = file_entity(
            &format!("p:repo1:file:{path}"),
            &project_id,
            &repo_id,
            name,
            path,
            None,
        );
        storage.upsert_entity(&ent).unwrap();
    }

    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());
    let result = tool
        .call(json!({"query": "needle", "include_source": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 1, "only the matched file may be returned");
    let sym = &symbols[0];
    assert_eq!(sym["name"], "needle");
    let source = sym["source"].as_str().unwrap();
    assert!(source.starts_with("1\tfn needle() {}\n"));
    assert!(!source.contains("unrelated"));
}

// ---------------------------------------------------------------------------
// Tier-1 field shape pin (deterministic order is not guaranteed — keys are a
// set; we pin presence + the absence of any body for large symbols).
// ---------------------------------------------------------------------------

#[test]
fn tier1_fields_present_per_symbol_and_no_body_for_large() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    let big = body_of_chars(SMALL_BODY_THRESHOLD_CHARS + 100);
    write_file(tmp.path(), "shape.rs", &big);
    let ent = file_entity(
        "p:repo1:file:shape.rs",
        &project_id,
        &repo_id,
        "shape",
        "shape.rs",
        Some("A one-line stored summary."),
    );
    storage.upsert_entity(&ent).unwrap();

    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());
    // Issue #731: explicit include_depth=true — the bare default is now lean.
    let result = tool
        .call(json!({"query": "shape", "include_depth": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    let sym = &parsed["symbols"].as_array().unwrap()[0];

    for key in [
        "name",
        "kind",
        "qualified_path",
        "call_paths",
        "blast_radius",
        "summary",
        "score",
        "reason",
    ] {
        assert!(sym.get(key).is_some(), "tier-1 symbol must carry '{key}'");
    }
    // entity_id intentionally absent from the per-symbol payload (issue #834).
    // Issue #711: score is a stable numeric value, reason names the hit.
    assert!(sym.get("entity_id").is_none());
    assert!(
        sym["score"].as_i64().is_some(),
        "score must be numeric: {sym}"
    );
    assert!(sym["reason"].as_str().unwrap().contains("name: shape"));
    // signature: first line of the body is the crude hint (no fn entity).
    assert!(sym.get("signature").is_some());
    assert!(sym.get("source").is_none());

    // No-files-cut case: total == returned (one file, default cap 8) → the
    // breadth disclosure is absent; exhausted breadth looks like a warning-
    // free full result.
    assert!(parsed.get("not_shown").is_none());
    assert!(parsed.get("completeness").is_none());
    assert!(parsed.get("continuation").is_none());
}

// ---------------------------------------------------------------------------
// 24K cap + truncation signal
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 24K cap + truncation signal
// ---------------------------------------------------------------------------

#[test]
fn output_capped_at_24k_with_truncation_signal() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();

    // Create enough large files to push the JSON past 24K chars.
    for i in 0..12 {
        let path = format!("files/alpha_{i:02}.rs");
        let body = format!(
            "fn alpha_{i:02}() {{\n  {}\n}}\n",
            "let x = 0; // padding padding padding padding padding padding padding padding padding padding \n    ".repeat(300)
        );
        write_file(tmp.path(), &path, &body);
        let ent = file_entity(
            &format!("p:repo1:file:{path}"),
            &project_id,
            &repo_id,
            &format!("alpha_{i:02}"),
            &path,
            Some(&format!(
                "A long summary for alpha_{i:02} designed to push the response past the 24K output cap for the explore tool truncation test."
            )),
        );
        storage.upsert_entity(&ent).unwrap();
    }

    let tool = make_tool(storage, project_id, tmp.path().to_path_buf());
    let result = tool
        .call(json!({"query": "alpha", "max_files": 12, "include_source": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();

    // The serialized output must be under the cap (JSON stays parseable —
    // the cap truncates on UTF-8 char boundaries and re-serializes).
    let chars = result.chars().count();
    assert!(
        chars <= 24_000 + 100,
        "output should be near or below the 24K cap, got {chars} chars"
    );

    // Truncation signal must be present.
    assert!(
        parsed.get("truncated").is_some() || result.contains("..."),
        "expected a truncation signal in the response, got: {result}"
    );

    // Width-cap/serialisation-cap interplay (issue #711): all 12 files are
    // shown (returned == total) so no breadth truncation occurred, and the
    // 24K serialisation cap is the second line that fires.
    assert!(
        parsed["truncated"] == true || result.contains("truncated"),
        "expected the 24K serialisation cap to fire, got: {result}"
    );
    // The breadth disclosure (not_shown / completeness) must NOT appear
    // anywhere — it is reserved for per-call width-cap cuts.
    assert!(
        !result.contains("\"not_shown\""),
        "no breadth cut → no not_shown key anywhere in the capped shape: {result}"
    );
    assert!(
        !result.contains("\"completeness\""),
        "no breadth cut → no completeness key anywhere in the capped shape: {result}"
    );
}

// ---------------------------------------------------------------------------
// Issue #711: per-symbol score + reason, name-hits outrank path-only hits
// ---------------------------------------------------------------------------

#[test]
fn per_symbol_score_and_reason_name_hits_outrank_path_only_hits() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();

    // Name hits: "alpha" in 'alpha.rs' (name, score 2) and "beta" in
    // 'beta/two.rs' (name, score 2). Path-only hit: "alpha" only in the
    // directory (score 1).
    for (path, name) in [
        ("alpha.rs", "alpha"),
        ("alpha/zzz.rs", "zzz"),
        ("beta/two.rs", "two"),
    ] {
        write_file(tmp.path(), path, "fn x() {}\n");
        let ent = file_entity(
            &format!("p:repo1:file:{path}"),
            &project_id,
            &repo_id,
            name,
            path,
            None,
        );
        storage.upsert_entity(&ent).unwrap();
    }

    let tool = make_tool(storage, project_id, tmp.path().to_path_buf());
    let result = tool.call(json!({"query": "alpha beta"})).unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    // All three files match: 'alpha' (name), 'zzz' (path), 'two' (path).
    let symbols = parsed["symbols"].as_array().unwrap();
    let by_name: std::collections::HashMap<&str, &Value> = symbols
        .iter()
        .map(|s| (s["name"].as_str().unwrap(), s))
        .collect();
    // Name hits score 2, path-only hits score 1 (location max, distinct words);
    // a name hit strictly outranks a path-only hit.
    assert_eq!(by_name["alpha"]["score"], 2, "name hit scores 2");
    assert_eq!(by_name["alpha"]["reason"], "name: alpha");
    assert_eq!(by_name["two"]["score"], 1, "path hit on 'beta' scores 1");
    assert_eq!(by_name["two"]["reason"], "path: beta");
    assert_eq!(by_name["zzz"]["score"], 1, "path hit on 'alpha' scores 1");
    assert_eq!(by_name["zzz"]["reason"], "path: alpha");
    let (s_alpha, s_two, s_zzz) = (
        by_name["alpha"]["score"].as_i64().unwrap(),
        by_name["two"]["score"].as_i64().unwrap(),
        by_name["zzz"]["score"].as_i64().unwrap(),
    );
    assert!(s_alpha > s_two && s_alpha > s_zzz);
    // Order: 'alpha' (score 2) first; the two score-1 files tie and sort by
    // path asc: "alpha/zzz.rs" < "beta/two.rs" ("a" < "b").
    let ordered: Vec<&str> = symbols
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(ordered, vec!["alpha", "zzz", "two"]);
}

// ---------------------------------------------------------------------------
// Issue #711: width cap binds BEFORE symbol-building — expensive files cut
// by the cap contribute no symbol payload anywhere
// ---------------------------------------------------------------------------

#[test]
fn width_cap_binds_before_symbol_building_no_payload_for_cut_files() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();

    // Six files matching "gamma"; each gets 4 Function children with Calls
    // edges (expensive build_symbol / call_paths_and_blast_radius work).
    // Every name scores 2 (name + path, location max); the path-asc tiebreak
    // decides: gamma_01 < gamma_4 < gamma_5 < gamma_6 < gamma_8 < gamma_9.
    // max_files=1 keeps gamma_01 and cuts the other five — proving the cap
    // binds BEFORE symbol-building.
    // Phase 1: upsert the six file entities + their 24 Function children.
    let names: [&str; 6] = [
        "gamma_8", "gamma_01", "gamma_9", "gamma_4", "gamma_5", "gamma_6",
    ];
    for name in names.iter() {
        let path = format!("gamma/{name}.rs");
        write_file(tmp.path(), &path, "fn g() {}\n");
        let ent = file_entity(
            &format!("p:repo1:file:{path}"),
            &project_id,
            &repo_id,
            name,
            &path,
            None,
        );
        let id = ent.id.clone();
        storage.upsert_entity(&ent).unwrap();
        for k in 0..4 {
            let fid = format!("fn-{name}-{k}");
            storage
                .upsert_entity(&Entity {
                    id: fid.clone(),
                    project_id: project_id.clone(),
                    repo_id: Some(repo_id.clone()),
                    tier: EntityTier::Function,
                    parent_id: Some(id.clone()),
                    name: format!("{name}_fn{k}"),
                    path: Some(path.clone()),
                    language: Some("Rust".to_string()),
                    summary: None,
                    summary_commit: None,
                    metrics_json: None,
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                })
                .unwrap();
        }
    }

    // Phase 2: each Function entity gets one outgoing Calls edge to a
    // same-file sibling (target is a real, already-upserted entity).
    for name in names.iter() {
        for k in 0..4 {
            let fid = format!("fn-{name}-{k}");
            let other_idx = (k + 1) % 4;
            let rel = Relationship {
                source_id: fid,
                target_id: format!("fn-{name}-{other_idx}"),
                rel_type: RelType::Calls,
                weight: 1.0,
                evidence_json: None,
                provenance: EdgeProvenance::Heuristic,
            };
            storage.upsert_relationship(&rel).unwrap();
        }
    }

    let tool = make_tool(storage, project_id, tmp.path().to_path_buf());
    // Issue #731: explicit include_depth=true — the bare default is now lean.
    let result = tool
        .call(json!({"query": "gamma", "max_files": 1, "include_depth": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();

    // Only the top-1 file is returned (path-asc tiebreak on equal scores),
    // with call-graph payload from its own children.
    let symbols = parsed["symbols"].as_array().unwrap();
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0]["name"], "gamma_01");
    assert_eq!(
        symbols[0]["score"], 2,
        "'gamma' hits name+path at the location max"
    );
    let call_paths = symbols[0]["call_paths"].as_array().unwrap();
    assert!(
        !call_paths.is_empty(),
        "kept file must carry its call paths"
    );
    assert!(
        call_paths
            .iter()
            .all(|cp| cp["name"].as_str().unwrap().starts_with("gamma_01")),
        "call paths must come only from the kept file: {call_paths:?}"
    );

    // No symbol payload for any cut file appears ANYWHERE in the response
    // (the kept file's own name carries the only "gamma_0" substring).
    assert!(
        !result.contains("gamma_8"),
        "cut file leaked into the payload"
    );
    assert!(!result.contains("gamma_4"));
    assert!(!result.contains("gamma_5"));
    assert!(!result.contains("gamma_6"));

    // Breadth disclosure reflects the cap cut.
    assert_eq!(parsed["not_shown"], 5);
    assert_eq!(parsed["completeness"], "showing 1 of 6 matching files");
    assert!(parsed.get("continuation").is_some());
}

// ---------------------------------------------------------------------------
// Issue #711: determinism — same query + same index → byte-identical JSON
// ---------------------------------------------------------------------------

#[test]
fn same_query_same_index_produces_byte_identical_json() {
    let build = |_seed: u8| -> String {
        let (storage, project_id, repo_id) = setup();
        let tmp = tempfile::tempdir().unwrap();
        // Deliberately upserted in a non-path order; storage order must not
        // leak into the result.
        for (path, name) in [
            ("f2x/f3.rs", "f3"),
            ("f1x/f1.rs", "f1"),
            ("f4x/f4.rs", "f4"),
            ("f2x/f22.rs", "f22"),
            ("f4x/f44.rs", "f44"),
            ("f1x/f21.rs", "f21"),
        ] {
            write_file(tmp.path(), path, "fn z() {}\n");
            let ent = file_entity(
                &format!("p:repo1:file:{path}"),
                &project_id,
                &repo_id,
                name,
                path,
                None,
            );
            storage.upsert_entity(&ent).unwrap();
        }
        let tool = make_tool(storage, project_id, tmp.path().to_path_buf());
        tool.call(json!({"query": "f2 f4 f1", "max_files": MAX_MAX_FILES}))
            .expect("call must succeed")
    };

    // Two identically-seeded in-memory indexes → byte-identical serialized
    // responses (scores, ordering, and disclosures all stable).
    let out_a = build(0);
    let out_b = build(1);
    assert_eq!(
        out_a, out_b,
        "two calls on identical indexes must be byte-identical"
    );

    // Sanity: all 6 matched, none cut → no breadth disclosure.
    let parsed: Value = serde_json::from_str(&out_a).unwrap();
    assert_eq!(parsed["symbols"].as_array().unwrap().len(), 6);
    assert!(parsed.get("not_shown").is_none());
    assert!(parsed.get("completeness").is_none());
}

// ---------------------------------------------------------------------------
// Both caps fire together (issue #711 PM decision): not_shown/completeness
// from the width cut must survive the 24K serialisation cap.
// ---------------------------------------------------------------------------

#[test]
fn breadth_disclosure_survives_24k_cap_when_both_caps_fire() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    // 12 large-bodied files match "alpha"; max_files=5 cuts 7 (not_shown>0)
    // while the 5 kept large bodies independently blow the 24K cap.
    for i in 0..12 {
        let name = format!("alpha_{i:02}");
        let path = format!("{name}.rs");
        let body = format!(
            "fn {name}() {{\n  {}\n}}\n",
            "let x = 0; // padding padding padding padding padding padding padding padding padding padding \n    ".repeat(300)
        );
        write_file(tmp.path(), &path, &body);
        let ent = file_entity(
            &format!("p:repo1:file:{path}"),
            &project_id,
            &repo_id,
            &name,
            &path,
            Some(&format!(
                "A long summary for {name} to push past the 24K cap."
            )),
        );
        storage.upsert_entity(&ent).unwrap();
    }
    let tool = make_tool(storage, project_id, tmp.path().to_path_buf());
    let result = tool
        .call(json!({"query": "alpha", "max_files": 5, "include_source": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();

    assert!(result.chars().count() <= 24_000 + 100, "got: {result}");
    assert!(parsed["truncated"] == true, "got: {result}");
    // PM invariant: disclosures from the width cut must survive the 24K cut.
    assert_eq!(parsed["not_shown"], 7, "got: {result}");
    assert_eq!(
        parsed["completeness"], "showing 5 of 12 matching files",
        "got: {result}"
    );
}

#[test]
fn tier2_zero_match_returns_empty_map_guidance() {
    let (storage, project_id, repo_id) = setup();
    let tmp = tempfile::tempdir().unwrap();
    write_file(tmp.path(), "present.rs", "fn present() {}\n");
    let ent = file_entity(
        "p:repo1:file:present.rs",
        &project_id,
        &repo_id,
        "present",
        "present.rs",
        None,
    );
    storage.upsert_entity(&ent).unwrap();

    let tool = make_tool(storage, project_id, tmp.path().canonicalize().unwrap());
    let result = tool
        .call(json!({"query": "zzzznomatch", "include_source": true}))
        .unwrap();
    let parsed: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["symbols"].as_array().unwrap().len(), 0);
    assert!(parsed["warning"].as_str().is_some());
}
