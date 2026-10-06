// End-to-end CLI tests for `lievo admin coverage` (issue #679).
//
// Each test gets its own isolated database file (unique path in
// `std::env::temp_dir()`, overridden via the `LIEVO_DB` env var), so tests
// are hermetic and order-independent. The pre-built `lievo` binary is
// driven directly (no `cargo run` — it would rebuild and swallow output).
//
// Metric-level behaviour (coverage formula, gate boundaries) is pinned by
// the unit tests in src/analysis/coverage.rs and the handler tests in
// src/bin/lievo/commands/project/project_tests.rs.

use std::fs;
use std::path::PathBuf;
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
/// so state (projects, repos) carries across calls within a session.
fn run_session(args: &[&[&str]]) -> Vec<(String, String, i32)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let db = std::env::temp_dir().join(format!("lievo-cov-db-{pid}-{n}.db"));
    let _ = fs::remove_file(&db);
    let results = args
        .iter()
        .map(|a| run_once(&common::lievo_bin(), &db, a))
        .collect::<Vec<_>>();
    let _ = fs::remove_file(&db);
    results
}

// ---------------------------------------------------------------------------
// Help surface
// ---------------------------------------------------------------------------

#[test]
fn test_admin_help_lists_coverage() {
    let (stdout, _stderr, status) = run_once(
        &common::lievo_bin(),
        &PathBuf::from("does-not-exist.db"),
        &["admin", "--help"],
    );
    assert_eq!(status, 0, "lievo admin --help should succeed");
    assert!(
        stdout.contains("coverage"),
        "admin --help must list the coverage subcommand, got: {stdout}"
    );
}

#[test]
fn test_admin_coverage_help_shows_project_option() {
    let (stdout, _stderr, status) = run_once(
        &common::lievo_bin(),
        &PathBuf::from("does-not-exist.db"),
        &["admin", "coverage", "--help"],
    );
    assert_eq!(status, 0, "lievo admin coverage --help should succeed");
    assert!(
        stdout.contains("PROJECT") || stdout.contains("project"),
        "coverage --help must document the PROJECT option, got: {stdout}"
    );
    assert!(
        stdout.contains("--gate"),
        "coverage --help must document the --gate option, got: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Gate exit code + JSON contract
// ---------------------------------------------------------------------------

/// Register the sample_repo fixture (as a standalone temp git repo) in an
/// isolated project, then run `admin coverage` in the same session.
/// Returns the three results: create-project, add-repo, coverage.
fn coverage_session(project: &str, coverage_args: &[&str]) -> Vec<(String, String, i32)> {
    let repo_path = common::prepare_fixture_repo();
    let repo_arg = repo_path.to_str().expect("fixture path is utf-8");
    let mut args: Vec<Vec<String>> = Vec::new();
    args.push(vec![
        "admin".to_string(),
        "create-project".to_string(),
        project.to_string(),
    ]);
    args.push(vec![
        "admin".to_string(),
        "add-repo".to_string(),
        repo_arg.to_string(),
        project.to_string(),
    ]);
    let mut cov = vec![
        "admin".to_string(),
        "coverage".to_string(),
        "--project".to_string(),
        project.to_string(),
    ];
    cov.extend(coverage_args.iter().map(|s| s.to_string()));
    args.push(cov);

    // Leak each Vec<String> into a Vec<&'static str>.
    let leaked: Vec<Vec<&'static str>> = args
        .into_iter()
        .map(|v| {
            v.into_iter()
                .map(|s| &*Box::leak(s.into_boxed_str()))
                .collect()
        })
        .collect();

    let sessions: Vec<&[&str]> = leaked.iter().map(|v| v.as_slice()).collect();
    run_session(&sessions)
}

#[test]
fn test_admin_coverage_gate_fails_on_fan_out_violation() {
    // The gate must exit non-zero when the fan-out threshold is violated,
    // so the CI step `lievo admin coverage --project X --gate` can fail the
    // build (issue #679 acceptance criterion 2). A threshold of 0 forces a
    // violation: every repo has ≥1 function, so max fan-out ≥ 1 > 0 —
    // pinned without depending on the fixture's actual fan-out value.
    let results = coverage_session("cov-gate-proj", &["--gate", "--fan-out-threshold", "0"]);
    let (create_out, create_err, create_status) = &results[0];
    assert_eq!(
        0, *create_status,
        "create-project must succeed; stderr: {create_err}"
    );
    let _ = create_out;
    let (_out, add_err, add_status) = &results[1];
    assert_eq!(0, *add_status, "add-repo must succeed; stderr: {add_err}");
    let (_out, err, status) = &results[2];
    assert_eq!(
        1, *status,
        "coverage --gate must exit 1 on a fan-out violation; stderr: {err}"
    );
    assert!(
        err.contains("gate") || _out.contains("Gate: FAIL"),
        "failure must name the gate; stdout: {_out}, stderr: {err}"
    );

    // Without --gate the same repo only reports and must exit 0 even with a
    // failing threshold — the gate must not break reporting mode (JS/TS 0%
    // coverage tolerance, issue #679 pitfall).
    let results = coverage_session("cov-nogate-proj", &["--fan-out-threshold", "0"]);
    let (_out, _err, status) = &results[2];
    assert_eq!(
        0, *status,
        "coverage without --gate must always exit 0 on successful runs"
    );
}

#[test]
fn test_admin_coverage_json_output_contract() {
    let results = coverage_session("cov-json-proj", &["--format", "json"]);
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
    let (stdout, err, status) = &results[2];
    assert_eq!(
        0, *status,
        "coverage --format json must succeed; stderr: {err}"
    );

    let parsed: serde_json::Value =
        serde_json::from_str(stdout).expect("coverage JSON output must parse");
    assert_eq!(parsed["project"], "cov-json-proj");
    let languages = parsed
        .get("languages")
        .and_then(|v| v.as_array())
        .expect("languages must be a JSON array");
    assert!(
        !languages.is_empty(),
        "sample_repo must produce at least one language row: {stdout}"
    );
    for row in languages {
        assert!(
            row.get("language").is_some(),
            "row must have language: {row}"
        );
        assert!(
            row.get("coverage_pct").is_some(),
            "row must have coverage_pct: {row}"
        );
        assert!(
            row.get("files_with_dependents").is_some(),
            "row must have files_with_dependents: {row}"
        );
        assert!(
            row.get("symbol_files").is_some(),
            "row must have symbol_files: {row}"
        );
        assert!(
            row.get("entities").is_some(),
            "row must have entities: {row}"
        );
        assert!(
            row.get("max_fan_out").is_some(),
            "row must have max_fan_out: {row}"
        );
        assert!(
            row.get("single_char_callees").is_some(),
            "row must have single_char_callees: {row}"
        );
    }
    assert!(
        parsed.get("gates").and_then(|v| v.as_array()).is_some(),
        "gates must be a JSON array (possibly empty): {stdout}"
    );
    assert!(
        parsed
            .get("gate_failed")
            .and_then(|v| v.as_bool())
            .is_some(),
        "gate_failed must be a boolean: {stdout}"
    );
}
