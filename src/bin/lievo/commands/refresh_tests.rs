// Tests for the refresh command and its per-tier coverage report (issue #793).
//
// This file is a `#[path]` sibling of `refresh.rs`: the module lives at the
// same crate path as `refresh.rs`'s own items, so `use super::*` resolves to
// the refresh command's scope (refresh, refresh_report_impl items).

use super::refresh_report_impl::compute_tier_coverage;
use super::*;
use lievo::model::EntityTier;
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;
use serde_json::json;

fn storage() -> SqliteStorage {
    SqliteStorage::open_in_memory().unwrap()
}

fn seed_repo_with_functions(
    s: &SqliteStorage,
    project_id: &str,
    summarized_names: &[&str],
    missing_names: &[&str],
) {
    let repo = s
        .add_repo(project_id, "repo1", "/nonexistent-but-allowed-in-seed")
        .expect("add_repo");
    let now = "2024-01-01T00:00:00Z";
    let pairs = summarized_names
        .iter()
        .map(|n| (n, true))
        .chain(missing_names.iter().map(|n| (n, false)))
        .collect::<Vec<_>>();
    for (id_counter, (name, is_summarized)) in (1u32..).zip(pairs) {
        let entity = lievo::model::Entity {
            id: format!("fn-{id_counter}"),
            project_id: project_id.to_string(),
            repo_id: Some(repo.id.clone()),
            tier: EntityTier::Function,
            parent_id: None,
            name: (*name).to_string(),
            path: Some("src/lib.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: is_summarized.then(|| "A function that does things.".to_string()),
            summary_commit: None,
            metrics_json: None,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        };
        s.upsert_entity(&entity).expect("upsert_entity");
    }
}

fn stats() -> lievo::refresh::RefreshStats {
    lievo::refresh::RefreshStats {
        was_stale: true,
        elapsed_secs: 1,
        repos_refreshed: 1,
    }
}

#[test]
fn test_refresh_unknown_project_returns_error() {
    let s = storage();
    let result = refresh(
        &s,
        Some("no-such-project"),
        false,
        false,
        false,
        false,
        OutputFormat::Human,
    );
    assert!(result.is_err());
}

#[test]
fn test_refresh_no_repos_prints_up_to_date() {
    let s = storage();
    s.create_project("myproj", None).unwrap();
    // Project has no repos → not stale → prints "up to date"
    let result = refresh(
        &s,
        Some("myproj"),
        false,
        false,
        false,
        false,
        OutputFormat::Human,
    );
    assert!(result.is_ok());
}

#[test]
fn test_refresh_no_summarize_flag_skips_coverage_and_succeeds() {
    // Finding #4: when `no_summarize=true`, `summarization_enabled` is
    // false regardless of apfel availability, so `compute_coverage` is
    // never called — a storage failure there cannot block the report.
    // This test pins that contract: the refresh succeeds end-to-end.
    let s = storage();
    let project = s.create_project("cov-disabled", None).unwrap();
    seed_repo_with_functions(&s, &project.id, &[], &["alpha"]);
    let result = refresh(
        &s,
        Some("cov-disabled"),
        false,
        true, // force=true: skip the "up to date" early return
        true, // no_summarize=true: summarization disabled regardless of apfel
        false,
        OutputFormat::Json,
    );
    // `refresh` prints internally and returns `Result<()>`. Success
    // confirms that the disabled path ran without calling
    // `compute_coverage` (no storage failure propagated).
    assert!(
        result.is_ok(),
        "disabled-summarization refresh must succeed"
    );
}

#[test]
fn test_coverage_partial_enabled_json() {
    // 2 summarized + 1 missing = 66.7% coverage; incomplete flag set.
    let coverage = TierCoverage {
        total: 3,
        missing: 1,
    };
    let tiers = (
        coverage,
        TierCoverage::default(),
        TierCoverage::default(),
        TierCoverage::default(),
    );
    let value: serde_json::Value =
        serde_json::from_str(&build_report(&stats(), tiers, true, OutputFormat::Json)).unwrap();
    assert_eq!(value["summary_coverage"]["total"], json!(3));
    assert_eq!(value["summary_coverage"]["summarized"], json!(2));
    assert_eq!(value["summary_coverage"]["missing"], json!(1));
    assert_eq!(value["summary_coverage"]["pct"], json!(66.7));
    assert_eq!(
        value["summaries_incomplete"],
        json!(true),
        "incomplete coverage must be flagged"
    );
}

#[test]
fn test_coverage_partial_enabled_human_line_and_warning() {
    let coverage = TierCoverage {
        total: 685,
        missing: 323,
    };
    let tiers = (
        coverage,
        TierCoverage::default(),
        TierCoverage::default(),
        TierCoverage::default(),
    );
    let report = build_report(&stats(), tiers, true, OutputFormat::Human);
    assert!(
        report.contains("Summarized 362/685 functions (52.8%); 323 missing"),
        "expected coverage line in: {report:?}"
    );
    assert!(
        report.contains("warning: summary coverage is incomplete"),
        "expected warning in: {report:?}"
    );
}

#[test]
fn test_coverage_full_no_incomplete_flag() {
    let coverage = TierCoverage {
        total: 2,
        missing: 0,
    };
    let tiers = (
        coverage,
        TierCoverage::default(),
        TierCoverage::default(),
        TierCoverage::default(),
    );
    let value: serde_json::Value =
        serde_json::from_str(&build_report(&stats(), tiers, true, OutputFormat::Json)).unwrap();
    assert_eq!(value["summary_coverage"]["total"], json!(2));
    assert_eq!(value["summary_coverage"]["missing"], json!(0));
    assert_eq!(value["summary_coverage"]["pct"], json!(100.0));
    assert!(
        value.get("summaries_incomplete").is_none(),
        "full coverage: no incomplete flag"
    );
}

#[test]
fn test_coverage_disabled_json() {
    // apfel absent or --no-summarize: no coverage field, no warning,
    // and nothing implying incompleteness.
    let coverage = TierCoverage {
        total: 3,
        missing: 3,
    };
    let tiers = (
        coverage,
        TierCoverage::default(),
        TierCoverage::default(),
        TierCoverage::default(),
    );
    let value: serde_json::Value =
        serde_json::from_str(&build_report(&stats(), tiers, false, OutputFormat::Json)).unwrap();
    assert_eq!(value["summarization_disabled"], json!(true));
    assert!(
        value.get("summary_coverage").is_none(),
        "disabled: no coverage field"
    );
    assert!(
        value.get("summaries_incomplete").is_none(),
        "disabled: must not imply incompleteness"
    );
}

#[test]
fn test_coverage_disabled_human_line() {
    let coverage = TierCoverage {
        total: 3,
        missing: 3,
    };
    let tiers = (
        coverage,
        TierCoverage::default(),
        TierCoverage::default(),
        TierCoverage::default(),
    );
    let report = build_report(&stats(), tiers, false, OutputFormat::Human);
    assert!(
        report.contains("Summarization disabled"),
        "expected disabled line in: {report:?}"
    );
    assert!(!report.contains("warning:"));
}

#[test]
fn test_compute_coverage_counts_known_split() {
    // TestStorage-style check: seed 3 functions (2 summarized + 1 NULL)
    // and assert coverage matches the known split.
    let s = storage();
    let project = s.create_project("cov", None).unwrap();
    seed_repo_with_functions(&s, &project.id, &["alpha", "beta"], &["gamma"]);

    let coverage = compute_tier_coverage(&s, &project.id).expect("compute_tier_coverage");
    assert_eq!(coverage.0.total, 3);
    assert_eq!(coverage.0.summarized(), 2);
    assert_eq!(coverage.0.missing, 1);
    assert_eq!(coverage.0.pct(), 66.7);
}

#[test]
fn test_compute_coverage_excludes_test_names_from_total() {
    // `count_missing_summaries` excludes `test_%`; the total must match
    // that population or pct would be skewed.
    let s = storage();
    let project = s.create_project("cov-test", None).unwrap();
    seed_repo_with_functions(
        &s,
        &project.id,
        &["alpha", "test_helper"],
        &["gamma", "test_extra"],
    );

    let coverage = compute_tier_coverage(&s, &project.id).expect("compute_tier_coverage");
    assert_eq!(
        coverage.0.total, 2,
        "test_ names must be excluded from total"
    );
    assert_eq!(
        coverage.0.total, 2,
        "test_ names must be excluded from total"
    );
    assert_eq!(
        coverage.0.missing, 1,
        "test_ names must be excluded from missing"
    );
}

#[test]
fn test_coverage_zero_total_pct_is_zero() {
    let coverage = TierCoverage::default();
    assert_eq!(coverage.pct(), 0.0);
    let tiers = (
        coverage,
        TierCoverage::default(),
        TierCoverage::default(),
        TierCoverage::default(),
    );
    let value: serde_json::Value =
        serde_json::from_str(&build_report(&stats(), tiers, true, OutputFormat::Json)).unwrap();
    assert_eq!(value["summary_coverage"]["total"], json!(0));
    assert!(value.get("summaries_incomplete").is_none());
}

// ---------------------------------------------------------------------------
// Issue #793: per-tier coverage
// ---------------------------------------------------------------------------

/// Seed a repo with entities at one tier, optionally giving some a summary.
fn seed_tier_entities(
    s: &SqliteStorage,
    project_id: &str,
    tier: EntityTier,
    summarized_names: &[&str],
    missing_names: &[&str],
) {
    let repos = s.list_repos(project_id).expect("list_repos");
    let repo = repos.first().cloned().unwrap_or_else(|| {
        s.add_repo(project_id, "repo1", "/nonexistent")
            .expect("add_repo")
    });
    let now = "2024-01-01T00:00:00Z";
    let pairs = summarized_names
        .iter()
        .map(|n| (n, true))
        .chain(missing_names.iter().map(|n| (n, false)))
        .collect::<Vec<_>>();
    for (id_counter, (name, is_summarized)) in (1000u32..).zip(pairs) {
        let entity = lievo::model::Entity {
            id: format!("{}-{id_counter}", tier),
            project_id: project_id.to_string(),
            repo_id: Some(repo.id.clone()),
            tier,
            parent_id: None,
            name: (*name).to_string(),
            path: Some(format!("src/{tier}_{name}.rs")),
            language: Some("Rust".to_string()),
            summary: is_summarized.then(|| "A summary.".to_string()),
            summary_commit: None,
            metrics_json: None,
            created_at: now.to_string(),
            updated_at: now.to_string(),
        };
        s.upsert_entity(&entity).expect("upsert_entity");
    }
}

/// Seed a repo with a file-tier entity at a specific path.
fn seed_file_entity(
    s: &SqliteStorage,
    project_id: &str,
    name: &str,
    path: &str,
    has_summary: bool,
) {
    let repos = s.list_repos(project_id).expect("list_repos");
    let repo = repos.first().cloned().unwrap_or_else(|| {
        s.add_repo(project_id, "repo1", "/nonexistent")
            .expect("add_repo")
    });
    let now = "2024-01-01T00:00:00Z";
    let entity = lievo::model::Entity {
        id: format!("file-{name}"),
        project_id: project_id.to_string(),
        repo_id: Some(repo.id.clone()),
        tier: EntityTier::File,
        parent_id: None,
        name: name.to_string(),
        path: Some(path.to_string()),
        language: Some("Rust".to_string()),
        summary: has_summary.then(|| "A summary.".to_string()),
        summary_commit: None,
        metrics_json: None,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    };
    s.upsert_entity(&entity).expect("upsert_entity");
}

/// The property the whole issue exists to guarantee (issue #793): a run in
/// which a WHOLE TIER fails to summarize cannot report high coverage.
///
/// The function tier is complete (100%) while the file tier is entirely
/// unsummarized — the report must NOT read as healthy: the `file` tier
/// shows 0% and `summaries_incomplete` is set, in both human and JSON.
#[test]
fn test_whole_tier_failure_cannot_report_high_coverage() {
    let s = storage();
    let project = s.create_project("tier-fail", None).unwrap();
    // Function tier: complete.
    seed_repo_with_functions(&s, &project.id, &["alpha", "beta"], &[]);
    // File tier: wholly failed — none of the three files has a summary.
    seed_file_entity(&s, &project.id, "main.rs", "src/main.rs", false);
    seed_file_entity(&s, &project.id, "lib.rs", "src/lib.rs", false);
    seed_file_entity(&s, &project.id, "util.rs", "src/util.rs", false);

    let tiers = compute_tier_coverage(&s, &project.id).expect("compute_tier_coverage");
    let (function, file, _module, _subsystem) = tiers;

    // The blind spot: function tier looks perfect, file tier is 0%.
    assert_eq!(function.pct(), 100.0, "function tier complete");
    assert_eq!(file.total, 3);
    assert_eq!(file.missing, 3);
    assert_eq!(file.pct(), 0.0, "whole file tier failed");

    // The report cannot read as healthy.
    let report_json: serde_json::Value =
        serde_json::from_str(&build_report(&stats(), tiers, true, OutputFormat::Json)).unwrap();
    assert_eq!(report_json["summary_coverage"]["pct"], json!(100.0));
    assert_eq!(report_json["tier_coverage"]["file"]["pct"], json!(0.0));
    assert_eq!(
        report_json["summaries_incomplete"],
        json!(true),
        "whole-tier failure must set the incomplete flag"
    );

    let report_human = build_report(&stats(), tiers, true, OutputFormat::Human);
    assert!(
        report_human.contains("file: 0/3 summarized (0.0%); 3 missing"),
        "file-tier line must show the failed tier in: {report_human:?}"
    );
    assert!(
        report_human.contains("warning: summary coverage is incomplete"),
        "warning must fire even though the function tier is complete: {report_human:?}"
    );
}

/// Per-tier coverage appears in both human and JSON output, with the
/// function tier complete and the file tier not — the report must not read
/// as healthy (issue #793 test surface).
#[test]
fn test_per_tier_coverage_in_human_and_json() {
    let s = storage();
    let project = s.create_project("per-tier", None).unwrap();
    seed_repo_with_functions(&s, &project.id, &["alpha"], &["beta"]);
    // 1 summarized file, 2 missing (one in a `tests/` dir — excluded from
    // the missing count by the test-file path policy, issue #793).
    seed_file_entity(&s, &project.id, "main.rs", "src/main.rs", true);
    seed_file_entity(&s, &project.id, "util.rs", "src/util.rs", false);
    seed_file_entity(
        &s,
        &project.id,
        "integration.rs",
        "tests/integration.rs",
        false,
    );
    // 1 summarized module, 1 missing.
    seed_tier_entities(&s, &project.id, EntityTier::Module, &["mod_a"], &["mod_b"]);

    let (function, file, module, subsystem) =
        compute_tier_coverage(&s, &project.id).expect("compute_tier_coverage");
    // Function: 1/2. File: 2 in the population (test path excluded), 1 missing.
    assert_eq!(function.total, 2);
    assert_eq!(function.missing, 1);
    assert_eq!(
        file.total, 3,
        "file total counts all files (test files remain entities)"
    );
    assert_eq!(
        file.missing, 1,
        "the tests/-dir file is excluded from the missing count"
    );
    assert_eq!(module.total, 2);
    assert_eq!(module.missing, 1);
    assert_eq!(subsystem.total, 0);

    // JSON: all four tiers present, each with its own numbers.
    let value: serde_json::Value = serde_json::from_str(&build_report(
        &stats(),
        (function, file, module, subsystem),
        true,
        OutputFormat::Json,
    ))
    .unwrap();
    assert_eq!(value["tier_coverage"]["function"]["total"], json!(2));
    assert_eq!(value["tier_coverage"]["file"]["total"], json!(3));
    assert_eq!(value["tier_coverage"]["file"]["missing"], json!(1));
    assert_eq!(value["tier_coverage"]["module"]["missing"], json!(1));
    assert_eq!(value["tier_coverage"]["subsystem"]["total"], json!(0));

    // Human: one line per rollup tier with entities, bounded output.
    let report = build_report(
        &stats(),
        (function, file, module, subsystem),
        true,
        OutputFormat::Human,
    );
    assert!(
        report.contains("file: 2/3 summarized (66.7%); 1 missing"),
        "file tier line in: {report:?}"
    );
    assert!(
        report.contains("module: 1/2 summarized (50.0%); 1 missing"),
        "module tier line in: {report:?}"
    );
    assert!(
        !report.contains("subsystem:"),
        "empty subsystem tier must not print a line"
    );
}

/// Output volume stays bounded for a run with hundreds of skips: the
/// report carries at most one line per tier, never per entity (issue #793).
#[test]
fn test_report_line_count_bounded_by_tiers_not_entities() {
    let s = storage();
    let project = s.create_project("bounded", None).unwrap();
    seed_repo_with_functions(&s, &project.id, &["alpha"], &[]);
    let missing_names: Vec<String> = (0..200).map(|i| format!("f{i}.rs")).collect();
    let refs: Vec<&str> = missing_names.iter().map(|s| s.as_str()).collect();
    seed_tier_entities(&s, &project.id, EntityTier::File, &[], &refs);

    let tiers = compute_tier_coverage(&s, &project.id).expect("compute_tier_coverage");
    let report = build_report(&stats(), tiers, true, OutputFormat::Human);
    let lines: Vec<&str> = report.lines().collect();
    // Lines: "Refresh complete" + function line + file line + warning = 4.
    // Even with 200 missing files, no per-entity lines.
    assert!(
        lines.len() <= 6,
        "report must stay bounded by tiers, got {} lines:\n{report}",
        lines.len()
    );
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("f")).count(),
        0,
        "no per-entity lines"
    );
}

/// The 8,000-character budget skips (#792) are visible in the refresh report
/// too (issue #793): they surface as the function tier's `missing` count in
/// both human and JSON output, alongside the per-tier coverage. A function
/// dropped by the input budget is a function with a NULL summary in storage,
/// so the coverage view is authoritative for it.
#[test]
fn test_budget_skips_visible_in_refresh_report() {
    let s = storage();
    let project = s.create_project("budget-skip", None).unwrap();
    // Two functions dropped by the budget (no summary) + one summarized.
    seed_repo_with_functions(&s, &project.id, &["alpha"], &["beta", "gamma"]);

    let tiers = compute_tier_coverage(&s, &project.id).expect("compute_tier_coverage");
    let (function, _file, _module, _subsystem) = tiers;
    assert_eq!(function.total, 3);
    assert_eq!(
        function.missing, 2,
        "budget-dropped functions surface as missing"
    );

    let value: serde_json::Value =
        serde_json::from_str(&build_report(&stats(), tiers, true, OutputFormat::Json)).unwrap();
    assert_eq!(value["summary_coverage"]["missing"], json!(2));
    assert_eq!(value["tier_coverage"]["function"]["missing"], json!(2));
    assert_eq!(value["summaries_incomplete"], json!(true));

    let report = build_report(&stats(), tiers, true, OutputFormat::Human);
    assert!(
        report.contains("Summarized 1/3 functions (33.3%); 2 missing"),
        "budget skips visible in the human line: {report:?}"
    );
}

/// The file-tier test exclusion has ONE source of truth (issue #793 review,
/// finding 2): the Rust `is_test_file_path` predicate in the trait default,
/// not a SQL approximation. This test pins a filename that the two
/// implementations (Rust name-patterns vs SQL `path NOT LIKE 'test%'`)
/// would disagree on: `src/util_test.rs` is a test file by name-pattern
/// (contains `_test.`) but does NOT start with `test`, so a SQL
/// approximation would have counted it missing. The report must exclude it
/// — exactly one missing file (src/main.rs).
#[test]
fn test_file_tier_test_exclusion_single_source_of_truth() {
    let s = storage();
    let project = s.create_project("single-source", None).unwrap();
    seed_file_entity(&s, &project.id, "main.rs", "src/main.rs", false);
    seed_file_entity(&s, &project.id, "util_test.rs", "src/util_test.rs", false);

    let tiers = compute_tier_coverage(&s, &project.id).expect("compute_tier_coverage");
    let (_function, file, _module, _subsystem) = tiers;
    assert_eq!(
        file.total, 2,
        "both files are entities; the test file stays in the population"
    );
    assert_eq!(
        file.missing, 1,
        "src/util_test.rs is excluded by the Rust is_test_file_path predicate, not a SQL approximation"
    );
}
