// End-to-end CLI tests for `lievo admin selfcheck` (issue #715).
//
// Each test gets its own isolated database file (unique path in
// `std::env::temp_dir()`, overridden via the `LIEVO_DB` env var), so tests
// are hermetic and order-independent. The pre-built `lievo` binary is
// driven directly (no `cargo run` — it would rebuild and swallow output).
//
// Metric-level behaviour (edge sampling, false-0-callers detection, gate
// predicates) is pinned by the unit tests in
// src/bin/lievo/commands/project/selfcheck_metrics_tests.rs,
// selfcheck_false_zero_tests.rs, and selfcheck_ops_tests.rs. This file
// covers the CLI contract end-to-end: help surface, exit codes, per-section
// threshold breaches, JSON shape, and the storage-backed probe/payload
// sections driven through the real binary + real index.

use lievo::extraction::code_extractor::CodeExtractor;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod common;

/// Run a single lievo binary invocation against an isolated database file.
fn run_once(bin: &PathBuf, db: &PathBuf, args: &[&str]) -> (String, String, i32) {
    let output = Command::new(bin)
        .args(args)
        .env("LIEVO_DB", db)
        .output()
        .expect("failed to run lievo");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(1),
    )
}

/// Run a sequence of lievo invocations against ONE isolated database file,
/// so state (projects, repos, index) carries across calls within a session.
fn run_session(args: &[Vec<String>]) -> Vec<(String, String, i32)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let db = std::env::temp_dir().join(format!("lievo-selfcheck-db-{pid}-{n}.db"));
    let _ = fs::remove_file(&db);
    let bin = common::lievo_bin();
    let results = args
        .iter()
        .map(|a| {
            let refs: Vec<&str> = a.iter().map(|s| s.as_str()).collect();
            run_once(&bin, &db, &refs)
        })
        .collect::<Vec<_>>();
    let _ = fs::remove_file(&db);
    results
}

fn fixture_repo() -> PathBuf {
    common::prepare_fixture_repo()
}

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Reproduce the full pure-section pipeline of `run_pure_sections`
/// (extract → group → RelationshipBuilder → sample/verify edges) exactly as
/// src/bin/lievo/commands/project/selfcheck_ops.rs does, and return the
/// resolved import edges as (source_file, target_file) path pairs.
///
/// The orchestration itself (`run_pure_sections`) only returns pass/fail
/// section reports — no per-edge detail — so an e2e guard that must assert
/// on WHICH edges got resolved (or that none did) needs this direct view of
/// the resolved_by_path data it operates on (issue #724 task-c).
fn resolved_import_edges_by_path(repo: &Path) -> Vec<(String, String)> {
    let mut extractor =
        lievo::extraction::tree_sitter_extractor::TreeSitterExtractor::new(repo, true)
            .expect("TreeSitterExtractor::new on fixture copy");
    extractor
        .index(false)
        .expect("extractor.index on fixture copy");
    let code_units = extractor.read_all_units().expect("read_all_units");
    let all_files = extractor.extracted_files().to_vec();

    let grouping_config = lievo::extraction::grouping::GroupingConfig {
        code_units: &code_units,
        scanned_file_paths: &all_files,
        project_id: "selfcheck",
        repo_name: "selfcheck",
        repo_id: "selfcheck",
        repo_path: repo,
        config: None,
        exclude_paths: &[],
    };
    let grouping =
        lievo::extraction::grouping::group_code_units(&grouping_config).expect("group_code_units");

    let (relationships, _unresolved) = lievo::analysis::relationships::RelationshipBuilder::build(
        &code_units,
        &grouping,
        "selfcheck",
        "selfcheck",
        repo,
    )
    .expect("RelationshipBuilder::build");

    let id_to_path: std::collections::HashMap<String, String> = grouping
        .files
        .iter()
        .filter_map(|f| f.path.as_ref().map(|p| (f.id.clone(), p.clone())))
        .collect();
    relationships
        .iter()
        .filter(|r| r.rel_type == lievo::model::RelType::Imports)
        .filter_map(|r| {
            Some((
                id_to_path.get(&r.source_id)?.clone(),
                id_to_path.get(&r.target_id)?.clone(),
            ))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Help surface
// ---------------------------------------------------------------------------

#[test]
fn test_admin_help_lists_selfcheck() {
    let (stdout, _stderr, status) = run_once(
        &common::lievo_bin(),
        &PathBuf::from("does-not-exist.db"),
        &["admin", "--help"],
    );
    assert_eq!(status, 0, "lievo admin --help should succeed");
    assert!(
        stdout.contains("selfcheck"),
        "admin --help must list the selfcheck subcommand, got: {stdout}"
    );
}

#[test]
fn test_admin_selfcheck_help_documents_flags() {
    let (stdout, _stderr, status) = run_once(
        &common::lievo_bin(),
        &PathBuf::from("does-not-exist.db"),
        &["admin", "selfcheck", "--help"],
    );
    assert_eq!(status, 0, "lievo admin selfcheck --help should succeed");
    for flag in [
        "--repo",
        "--project",
        "--probes",
        "--gate",
        "--max-wrong-edge-rate",
        "--max-false-zero-callers",
        "--min-recall-k",
        "--min-recall-floor",
        "--min-worst-probe",
        "--min-payload-bytes",
    ] {
        assert!(
            stdout.contains(flag),
            "selfcheck --help must document {flag}, got: {stdout}"
        );
    }
}

// ---------------------------------------------------------------------------
// Pure sections against the expanded fixture (no --project/--probes needed)
// ---------------------------------------------------------------------------

#[test]
fn test_selfcheck_pure_sections_pass_on_clean_fixture() {
    let repo_path = fixture_repo();
    let repo_arg = repo_path.to_str().expect("fixture path is utf-8");
    let db = std::env::temp_dir().join(format!("lievo-selfcheck-pure-{}.db", std::process::id()));
    let _ = fs::remove_file(&db);
    let (stdout, stderr, status) = run_once(
        &common::lievo_bin(),
        &db,
        &["admin", "selfcheck", "--repo", repo_arg, "--gate"],
    );
    let _ = fs::remove_file(&db);
    assert_eq!(
        0, status,
        "selfcheck --gate must exit 0 on the clean fixture with default thresholds; stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stdout.contains("edge_correctness"),
        "human output must show edge_correctness section: {stdout}"
    );
    assert!(
        stdout.contains("false_zero_callers"),
        "human output must show false_zero_callers section: {stdout}"
    );
}

/// E2E guard for issue #724: the nested-module fixture
/// (src/main.rs `mod rustmod;` → src/rustmod.rs `pub mod deep;` →
/// src/rustmod/deep/mod.rs) must produce the `src/main.rs →
/// src/rustmod/deep/mod.rs` import edge through the REAL extraction pipeline
/// (RelationshipBuilder's module-key resolver), and `admin selfcheck --gate`
/// must exit 0 on it with the default 0.0 wrong-edge threshold — i.e. the
/// symbol-level `use crate::rustmod::deep::deep_label` specifier must not be
/// flagged as a wrong edge (the 732/1033 false-flag regression this issue
/// fixes). The e2e --gate assertion only proves the healthy direction
/// end-to-end; the negative direction (a genuinely wrong edge still failing
/// the gate on a nested layout) is pinned by the unit tests in
/// selfcheck_metrics_tests.rs / selfcheck_ops_tests.rs, because a wrong
/// edge in the fixture would trip the CI step that runs this same gate on
/// the in-tree fixture.
#[test]
fn test_selfcheck_nested_module_edge_is_not_wrong() {
    let repo_path = fixture_repo();
    let repo_arg = repo_path.to_str().expect("fixture path is utf-8");
    let db = std::env::temp_dir().join(format!("lievo-selfcheck-nested-{}.db", std::process::id()));
    let _ = fs::remove_file(&db);
    let (stdout, stderr, status) = run_once(
        &common::lievo_bin(),
        &db,
        &["admin", "selfcheck", "--repo", repo_arg, "--gate"],
    );
    let _ = fs::remove_file(&db);
    assert_eq!(
        0, status,
        "selfcheck --gate must exit 0 on the clean fixture with default thresholds (incl. the nested rustmod edge); stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stdout.contains("edge_correctness"),
        "human output must show edge_correctness section: {stdout}"
    );

    // The edge must actually exist via the real extraction pipeline — an
    // empty resolved_by_path would also make the gate pass (0.0 rate), so
    // the --gate exit-0 alone is not sufficient proof the fixture is
    // exercising the nested path.
    let edges = resolved_import_edges_by_path(&repo_path);
    assert!(
        edges
            .iter()
            .any(|(src, tgt)| src == "src/main.rs" && tgt == "src/rustmod/deep/mod.rs"),
        "real extraction must resolve the `use crate::rustmod::deep::deep_label` specifier to the nested module file; got edges: {edges:?}"
    );
}

/// E2E guard for issue #732: the #[path]-declared module in the fixture
/// (src/steps.rs `#[path = "steps/cleanup.rs"] mod cleanup;` →
/// src/steps/cleanup.rs) must produce the `src/main.rs → src/steps/cleanup.rs`
/// import edge through the REAL extraction pipeline, and `admin selfcheck
/// --gate` must exit 0 on it with the default 0.0 wrong-edge threshold — i.e.
/// the `use crate::steps::cleanup::cleanup_label` specifier must not be
/// flagged as a wrong edge. The #[path] attribute is exercised (parsed by
/// the alias-map scanner) without a physical/logical divergence (which the
/// production extractor does not yet handle — see issue #732).
#[test]
fn test_selfcheck_path_module_edge_is_not_wrong() {
    let repo_path = fixture_repo();
    let repo_arg = repo_path.to_str().expect("fixture path is utf-8");
    let db = std::env::temp_dir().join(format!("lievo-selfcheck-path-{}.db", std::process::id()));
    let _ = fs::remove_file(&db);
    let (stdout, stderr, status) = run_once(
        &common::lievo_bin(),
        &db,
        &["admin", "selfcheck", "--repo", repo_arg, "--gate"],
    );
    let _ = fs::remove_file(&db);
    assert_eq!(
        0, status,
        "selfcheck --gate must exit 0 on the clean fixture with default thresholds (incl. the #[path] module edge); stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stdout.contains("edge_correctness"),
        "human output must show edge_correctness section: {stdout}"
    );

    // The edge must actually exist via the real extraction pipeline — an
    // empty resolved_by_path would also make the gate pass (0.0 rate), so
    // the --gate exit-0 alone is not sufficient proof the fixture is
    // exercising the #[path] path.
    let edges = resolved_import_edges_by_path(&repo_path);
    assert!(
        edges
            .iter()
            .any(|(src, tgt)| src == "src/main.rs" && tgt == "src/steps/cleanup.rs"),
        "real extraction must resolve the `use crate::steps::cleanup::cleanup_label` specifier to the #[path]-declared module file; got edges: {edges:?}"
    );
}

#[test]
fn test_selfcheck_gate_fails_on_edge_correctness_breach() {
    let repo_path = fixture_repo();
    let repo_arg = repo_path.to_str().expect("fixture path is utf-8");
    let db = std::env::temp_dir().join(format!("lievo-selfcheck-edge-{}.db", std::process::id()));
    let _ = fs::remove_file(&db);
    // A negative threshold is impossible to satisfy (wrong_edge_rate is
    // always >= 0.0), forcing a deterministic edge_correctness breach
    // without depending on the fixture actually containing a wrong edge.
    let (stdout, stderr, status) = run_once(
        &common::lievo_bin(),
        &db,
        &[
            "admin",
            "selfcheck",
            "--repo",
            repo_arg,
            "--gate",
            "--max-wrong-edge-rate=-1.0",
        ],
    );
    let _ = fs::remove_file(&db);
    assert_eq!(
        1, status,
        "selfcheck --gate must exit 1 on an edge-correctness breach; stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stderr.contains("edge_correctness") || stdout.contains("edge_correctness"),
        "failure must name the edge_correctness section; stdout: {stdout}, stderr: {stderr}"
    );

    // Without --gate the same breach only reports and must exit 0 — the
    // gate must not break reporting mode.
    let (out2, err2, status2) = run_once(
        &common::lievo_bin(),
        &PathBuf::from("does-not-exist-2.db"),
        &[
            "admin",
            "selfcheck",
            "--repo",
            repo_arg,
            "--max-wrong-edge-rate=-1.0",
        ],
    );
    assert_eq!(
        0, status2,
        "selfcheck without --gate must always exit 0 on successful runs; stdout: {out2}, stderr: {err2}"
    );
}

#[test]
fn test_selfcheck_gate_fails_on_false_zero_callers_breach() {
    let repo_path = fixture_repo();
    let repo_arg = repo_path.to_str().expect("fixture path is utf-8");
    let db = std::env::temp_dir().join(format!("lievo-selfcheck-fzc-{}.db", std::process::id()));
    let _ = fs::remove_file(&db);
    // false_zero_callers is a usize count (can't go negative), so a
    // threshold breach can't be forced the way edge_correctness's f64
    // threshold can. This test instead pins the boundary: the clean
    // fixture's count is 0, which must pass a threshold of exactly 0 —
    // confirming the gate isn't trivially always-failing on this section.
    // (The section's own breach behaviour — count > threshold => fail — is
    // covered by the pure unit tests in selfcheck_false_zero_tests.rs.)
    let (stdout, stderr, status) = run_once(
        &common::lievo_bin(),
        &db,
        &[
            "admin",
            "selfcheck",
            "--repo",
            repo_arg,
            "--gate",
            "--max-false-zero-callers",
            "0",
        ],
    );
    let _ = fs::remove_file(&db);
    assert_eq!(
        0, status,
        "false_zero_callers count==0 must pass a threshold of 0; stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stdout.contains("false_zero_callers"),
        "output must show the false_zero_callers section: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// JSON output contract
// ---------------------------------------------------------------------------

#[test]
fn test_selfcheck_json_output_contract() {
    let repo_path = fixture_repo();
    let repo_arg = repo_path.to_str().expect("fixture path is utf-8");
    let db = std::env::temp_dir().join(format!("lievo-selfcheck-json-{}.db", std::process::id()));
    let _ = fs::remove_file(&db);
    let (stdout, stderr, status) = run_once(
        &common::lievo_bin(),
        &db,
        &["admin", "selfcheck", "--repo", repo_arg, "--format", "json"],
    );
    let _ = fs::remove_file(&db);
    assert_eq!(
        0, status,
        "selfcheck --format json must succeed; stderr: {stderr}"
    );

    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("selfcheck JSON output must parse");
    assert!(
        parsed.get("repo").and_then(|v| v.as_str()).is_some(),
        "top-level 'repo' must be a string: {stdout}"
    );
    assert!(
        parsed
            .get("gate_failed")
            .and_then(|v| v.as_bool())
            .is_some(),
        "top-level 'gate_failed' must be a boolean: {stdout}"
    );
    let sections = parsed
        .get("sections")
        .and_then(|v| v.as_array())
        .expect("sections must be a JSON array");
    assert_eq!(
        sections.len(),
        5,
        "selfcheck must report exactly 5 sections: {stdout}"
    );
    let expected_names = [
        "edge_correctness",
        "false_zero_callers",
        "super_structural_probe",
        "retrieval_probes",
        "payload_bytes",
    ];
    for name in expected_names {
        assert!(
            sections
                .iter()
                .any(|s| s.get("name").and_then(|n| n.as_str()) == Some(name)),
            "sections must include {name}: {stdout}"
        );
    }
    for row in sections {
        assert!(row.get("name").is_some(), "row must have name: {row}");
        assert!(row.get("passed").is_some(), "row must have passed: {row}");
        assert!(row.get("skipped").is_some(), "row must have skipped: {row}");
        assert!(
            row.get("skip_reason").is_some(),
            "row must have skip_reason (possibly null): {row}"
        );
        assert!(row.get("detail").is_some(), "row must have detail: {row}");
    }

    // Census shape (issue #768) + the split population (issue #777): the
    // edge_correctness detail's `sample_size=` now reports the IMPORT-RESOLVED
    // population (use-statement-resolved edges — the denominator population of
    // wrong_edge_rate); previously it reported the whole dedup'd edge
    // population (import-resolved + call-based). Call-based edges are
    // reported as a separate `call_based=` census and never scored. The
    // values themselves must not be pinned here (they move with the tree),
    // but the fields must carry non-negative integers — a census reading of 0
    // is vacuous and the unit tests pin the exact counts for known fixtures.
    let edge_correctness = sections
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("edge_correctness"))
        .expect("edge_correctness section present");
    let edge_detail = edge_correctness
        .get("detail")
        .and_then(|d| d.as_str())
        .expect("edge_correctness detail must be a string: {edge_correctness}");
    let sample_size_str = edge_detail
        .rsplit("sample_size=")
        .next()
        .and_then(|s| s.split(' ').next())
        .expect("detail must carry a sample_size= field: {edge_detail}");
    let sample_size: u64 = sample_size_str
        .parse()
        .expect("sample_size must be an integer: {edge_detail}");
    let _ = sample_size; // presence + integer shape is the contract here
    // The call-based census field must be present and integral (issue #777):
    // call-based edges are reported, never silently dropped from the report.
    let call_based_str = edge_detail
        .rsplit("call_based=")
        .next()
        .and_then(|s| s.split(' ').next())
        .expect("detail must carry a call_based= field: {edge_detail}");
    let call_based: u64 = call_based_str
        .parse()
        .expect("call_base must be an integer: {edge_detail}");
    let _ = call_based;
    assert!(
        !edge_correctness
            .get("skipped")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        "edge_correctness must NOT be skipped in the default pure-section run: {edge_correctness}"
    );

    // Without --probes/--project, the storage-backed sections must be
    // SKIPPED (not failed) per the #715 PM decision.
    let retrieval = sections
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("retrieval_probes"))
        .expect("retrieval_probes section present");
    assert_eq!(
        retrieval.get("skipped").and_then(|v| v.as_bool()),
        Some(true),
        "retrieval_probes must be skipped without --probes: {stdout}"
    );
    let payload = sections
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("payload_bytes"))
        .expect("payload_bytes section present");
    assert_eq!(
        payload.get("skipped").and_then(|v| v.as_bool()),
        Some(true),
        "payload_bytes must be skipped without --probes: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Storage-backed sections: retrieval-probes + payload-bytes (indexed project)
// ---------------------------------------------------------------------------

/// Set up an isolated project + indexed repo (create-project, add-repo,
/// refresh) in one session, returning the session's results plus the probes
/// file path used for the final selfcheck call.
fn indexed_selfcheck_session(
    project: &str,
    probes_content: &str,
    selfcheck_extra: &[&str],
) -> (Vec<(String, String, i32)>, PathBuf) {
    indexed_selfcheck_session_for_repo(project, fixture_repo(), probes_content, selfcheck_extra)
}

/// Same as [`indexed_selfcheck_session`] but indexes an arbitrary repo path
/// instead of the sample_repo fixture copy (issue #733: the probe section is
/// also exercised against lievo's own tree in CI).
fn indexed_selfcheck_session_for_repo(
    project: &str,
    repo_path: PathBuf,
    probes_content: &str,
    selfcheck_extra: &[&str],
) -> (Vec<(String, String, i32)>, PathBuf) {
    let repo_arg = repo_path.to_str().expect("repo path is utf-8").to_string();

    let probes_path = std::env::temp_dir().join(format!(
        "lievo-selfcheck-probes-{}-{}.tsv",
        std::process::id(),
        project
    ));
    fs::write(&probes_path, probes_content).expect("write probes file");
    let probes_arg = probes_path
        .to_str()
        .expect("probes path is utf-8")
        .to_string();

    let mut sessions: Vec<Vec<String>> = Vec::new();
    sessions.push(args(&["admin", "create-project", project]));
    sessions.push(args(&["admin", "add-repo", &repo_arg, project]));
    sessions.push(args(&["refresh", project, "--no-summarize"]));

    let mut selfcheck_args = args(&[
        "admin",
        "selfcheck",
        "--repo",
        &repo_arg,
        "--project",
        project,
        "--probes",
        &probes_arg,
        "--format",
        "json",
    ]);
    selfcheck_args.extend(selfcheck_extra.iter().map(|s| s.to_string()));
    sessions.push(selfcheck_args);

    (run_session(&sessions), probes_path)
}

#[test]
fn test_selfcheck_probe_and_payload_sections_run_when_indexed() {
    let probes = "button\ttreeA/core/button/index.js\n";
    let (results, probes_path) = indexed_selfcheck_session("selfcheck-probe-proj", probes, &[]);
    let _ = fs::remove_file(&probes_path);

    assert_eq!(
        0, results[0].2,
        "create-project must succeed; stderr: {}",
        results[0].1
    );
    assert_eq!(
        0, results[1].2,
        "add-repo must succeed; stderr: {}",
        results[1].1
    );
    assert_eq!(
        0, results[2].2,
        "refresh must succeed; stderr: {}",
        results[2].1
    );
    let (stdout, stderr, status) = &results[3];
    assert_eq!(
        0, *status,
        "selfcheck with an indexed project + probes must exit 0; stdout: {stdout}, stderr: {stderr}"
    );

    let parsed: serde_json::Value =
        serde_json::from_str(stdout).expect("selfcheck JSON output must parse");
    let sections = parsed
        .get("sections")
        .and_then(|v| v.as_array())
        .expect("sections array");

    let retrieval = sections
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("retrieval_probes"))
        .expect("retrieval_probes section present");
    assert_eq!(
        retrieval.get("skipped").and_then(|v| v.as_bool()),
        Some(false),
        "retrieval_probes must NOT be skipped when --project and --probes are both given: {stdout}"
    );
    assert_eq!(
        retrieval.get("passed").and_then(|v| v.as_bool()),
        Some(true),
        "retrieval_probes must pass with default (0.0) floors: {stdout}"
    );

    let payload = sections
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("payload_bytes"))
        .expect("payload_bytes section present");
    assert_eq!(
        payload.get("skipped").and_then(|v| v.as_bool()),
        Some(false),
        "payload_bytes must NOT be skipped when --project and --probes are both given: {stdout}"
    );
    assert_eq!(
        payload.get("passed").and_then(|v| v.as_bool()),
        Some(true),
        "payload_bytes must pass with the default 200-byte floor on real explore output: {stdout}"
    );
}

#[test]
fn test_selfcheck_gate_fails_on_payload_bytes_breach() {
    let probes = "button\ttreeA/core/button/index.js\n";
    let (results, probes_path) = indexed_selfcheck_session(
        "selfcheck-payload-breach-proj",
        probes,
        &["--gate", "--min-payload-bytes", "100000"],
    );
    let _ = fs::remove_file(&probes_path);

    assert_eq!(0, results[0].2, "create-project must succeed");
    assert_eq!(0, results[1].2, "add-repo must succeed");
    assert_eq!(0, results[2].2, "refresh must succeed");
    let (stdout, stderr, status) = &results[3];
    assert_eq!(
        1, *status,
        "an unreachable payload-bytes floor must fail the gate; stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stderr.contains("payload_bytes"),
        "failure must name the payload_bytes section; stderr: {stderr}"
    );
}

#[test]
fn test_selfcheck_gate_fails_on_worst_probe_breach() {
    let probes = "button\ttreeA/core/button/index.js\n";
    let (results, probes_path) = indexed_selfcheck_session(
        "selfcheck-recall-breach-proj",
        probes,
        &["--gate", "--min-worst-probe", "1.5"],
    );
    let _ = fs::remove_file(&probes_path);

    assert_eq!(0, results[0].2, "create-project must succeed");
    assert_eq!(0, results[1].2, "add-repo must succeed");
    assert_eq!(0, results[2].2, "refresh must succeed");
    let (stdout, stderr, status) = &results[3];
    assert_eq!(
        1, *status,
        "an unreachable worst-probe floor (>1.0) must fail the gate; stdout: {stdout}, stderr: {stderr}"
    );
    assert!(
        stderr.contains("retrieval_probes"),
        "failure must name the retrieval_probes section; stderr: {stderr}"
    );
}

/// Issue #733: the checked-in `tests/fixtures/selfcheck_probes.tsv` (used by
/// the CI real-corpus selfcheck step) must parse cleanly and, when indexed
/// against lievo's own tree, must drive the storage-backed retrieval_probes
/// section to a passing recall ≥ 0.5 — proving the probe file's queries
/// actually locate their expected files in the real corpus rather than
/// silently scoring 0.0 (which the 0.0 default floor would mask).
#[test]
fn test_selfcheck_probes_file_hits_lievos_own_tree() {
    let probes_path = PathBuf::from("tests/fixtures/selfcheck_probes.tsv");
    let probes_content =
        fs::read_to_string(&probes_path).expect("checked-in selfcheck probes file readable");

    let repo_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Pure-section thresholds (edge_correctness, false_zero_callers) are
    // relaxed so the run exits 0 — the assertions below target the
    // retrieval_probes section specifically, which is the contract the
    // checked-in probes file pins. (The gate predicate's own behaviour on a
    // real-density corpus is exercised by the CI step, which runs the
    // strict 0.0 defaults.)
    let (results, probes_tmp) = indexed_selfcheck_session_for_repo(
        "selfcheck-selftest-proj",
        repo_path.clone(),
        &probes_content,
        &[
            "--gate",
            "--min-worst-probe",
            "0.5",
            "--max-wrong-edge-rate",
            "1.0",
            "--max-false-zero-callers",
            "10000",
        ],
    );
    let _ = fs::remove_file(&probes_tmp);

    assert_eq!(0, results[0].2, "create-project must succeed");
    assert_eq!(0, results[1].2, "add-repo must succeed");
    assert_eq!(0, results[2].2, "refresh must succeed");
    let (stdout, stderr, status) = &results[3];
    assert_eq!(
        0, *status,
        "selfcheck --gate --min-worst-probe 0.5 (relaxed pure-section thresholds) must pass on lievo's own tree with the checked-in probes; stdout: {stdout}, stderr: {stderr}"
    );

    let parsed: serde_json::Value =
        serde_json::from_str(stdout).expect("selfcheck JSON output must parse");
    let sections = parsed
        .get("sections")
        .and_then(|v| v.as_array())
        .expect("sections array");
    let retrieval = sections
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("retrieval_probes"))
        .expect("retrieval_probes section present");
    assert_eq!(
        retrieval.get("skipped").and_then(|v| v.as_bool()),
        Some(false),
        "retrieval_probes must not be skipped: {stdout}"
    );
    assert_eq!(
        retrieval.get("passed").and_then(|v| v.as_bool()),
        Some(true),
        "retrieval_probes must pass with worst_recall >= 0.5 on lievo's own tree: {stdout}"
    );
    // The gate must not fail because of the storage-backed sections themselves
    // (the pure-section relaxations above are a test-local convenience, not a
    // contract); gate_failed=false confirms no storage-backed section breached.
    assert_eq!(
        parsed.get("gate_failed").and_then(|v| v.as_bool()),
        Some(false),
        "gate must not fail on a clean tree with the checked-in probes: {stdout}"
    );
}

#[test]
fn test_selfcheck_probe_sections_skipped_without_project_even_with_probes_file() {
    let repo_path = fixture_repo();
    let repo_arg = repo_path.to_str().expect("fixture path is utf-8");
    let probes_path = std::env::temp_dir().join(format!(
        "lievo-selfcheck-noproj-probes-{}.tsv",
        std::process::id()
    ));
    fs::write(&probes_path, "button\ttreeA/core/button/index.js\n").expect("write probes file");
    let probes_arg = probes_path.to_str().expect("probes path is utf-8");

    let db = std::env::temp_dir().join(format!("lievo-selfcheck-noproj-{}.db", std::process::id()));
    let _ = fs::remove_file(&db);
    let (stdout, stderr, status) = run_once(
        &common::lievo_bin(),
        &db,
        &[
            "admin",
            "selfcheck",
            "--repo",
            repo_arg,
            "--probes",
            probes_arg,
            "--format",
            "json",
        ],
    );
    let _ = fs::remove_file(&db);
    let _ = fs::remove_file(&probes_path);

    assert_eq!(
        0, status,
        "missing --project must not fail the run (sections skip instead); stdout: {stdout}, stderr: {stderr}"
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("selfcheck JSON output must parse");
    let sections = parsed
        .get("sections")
        .and_then(|v| v.as_array())
        .expect("sections array");
    let retrieval = sections
        .iter()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("retrieval_probes"))
        .expect("retrieval_probes section present");
    assert_eq!(
        retrieval.get("skipped").and_then(|v| v.as_bool()),
        Some(true),
        "retrieval_probes must be skipped when --probes is given but --project is not: {stdout}"
    );
}
