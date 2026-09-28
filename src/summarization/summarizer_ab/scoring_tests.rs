// Unit tests for the parse-success scorer and license gate (issue #773).
//
// Test surface (from the issue):
//   - parse-success computation correctly scores a known-good batch response
//     and a known-malformed one
//   - a candidate with a disqualifying licence is excluded before any
//     measurement runs

use super::scoring::{
    LicenseStatus, ParseSuccessReport, build_parse_success_report, check_license,
    compute_parse_success,
};

// ── compute_parse_success ──────────────────────────────────────────────────

#[test]
fn test_compute_parse_success_all_some_returns_100() {
    // Known-good batch: all Some → 100.0
    let good = vec![
        vec![Some("summary a".into()), Some("summary b".into())],
        vec![Some("summary c".into())],
    ];
    assert_eq!(compute_parse_success(&good), 100.0);
}

#[test]
fn test_compute_parse_success_all_none_returns_0() {
    // Known-malformed batch: all None (markdown bullets) → 0.0
    let bad = vec![vec![None, None], vec![None]];
    assert_eq!(compute_parse_success(&bad), 0.0);
}

#[test]
fn test_compute_parse_success_mixed_returns_correct_percentage() {
    // Mixed batch: 2 of 3 Some → 66.67
    let mixed = vec![vec![Some("a".into()), None, Some("c".into())]];
    let result = compute_parse_success(&mixed);
    assert!(
        (result - 66.67).abs() < 0.01,
        "expected ~66.67, got {}",
        result
    );
}

#[test]
fn test_compute_parse_success_empty_returns_0() {
    // No entities at all → 0.0
    assert_eq!(compute_parse_success(&[]), 0.0);
    // All empty batches → 0.0
    let all_empty: Vec<Vec<Option<String>>> = vec![vec![], vec![]];
    assert_eq!(compute_parse_success(&all_empty), 0.0);
}

#[test]
fn test_compute_parse_success_multiple_batches() {
    // 3 batches: batch 0 has 2/2, batch 1 has 1/2, batch 2 has 0/1
    // Total: 3/5 = 60.0
    let results = vec![
        vec![Some("a".into()), Some("b".into())],
        vec![Some("c".into()), None],
        vec![None],
    ];
    let result = compute_parse_success(&results);
    assert!(
        (result - 60.0).abs() < 0.01,
        "expected 60.0, got {}",
        result
    );
}

#[test]
fn test_compute_parse_success_markdown_bullet_fixture() {
    // The known-malformed fixture should be a markdown bullet-list response
    // matching the code-daemon-summary-v1 card's documented output shape:
    // "- **<EntityName>**: <one-sentence description>"
    // This exercises the same code path (all None) that would make such a
    // model unusable.
    let markdown_bullet_response = vec![vec![None, None, None]];
    assert_eq!(compute_parse_success(&markdown_bullet_response), 0.0);
}

// ── build_parse_success_report ─────────────────────────────────────────────

#[test]
fn test_build_parse_success_report_multiple_batch_sizes() {
    // Batch size 2: 2/2 → 100.0
    let batch_2: Vec<Vec<Option<String>>> = vec![vec![Some("a".into()), Some("b".into())]];
    // Batch size 4: 2/4 → 50.0
    let batch_4: Vec<Vec<Option<String>>> =
        vec![vec![Some("a".into()), None, Some("c".into()), None]];

    let report: ParseSuccessReport =
        build_parse_success_report("test-model", vec![(2, &batch_2), (4, &batch_4)]);

    assert_eq!(report.model, "test-model");
    assert_eq!(report.entries.len(), 2);

    // Batch 2 entry
    assert_eq!(report.entries[0].batch_size, 2);
    assert!((report.entries[0].percentage - 100.0).abs() < 0.01);
    assert_eq!(report.entries[0].successes, 2);
    assert_eq!(report.entries[0].total, 2);

    // Batch 4 entry
    assert_eq!(report.entries[1].batch_size, 4);
    assert!((report.entries[1].percentage - 50.0).abs() < 0.01);
    assert_eq!(report.entries[1].successes, 2);
    assert_eq!(report.entries[1].total, 4);

    // Overall: 4/6 ≈ 66.67
    assert!((report.overall - 66.67).abs() < 0.01);
}

// ── check_license ──────────────────────────────────────────────────────────

#[test]
fn test_check_license_apache_2_0_admissible() {
    let result = check_license("Apache-2.0");
    assert_eq!(
        result,
        LicenseStatus::Admissible {
            license: "Apache-2.0".to_string(),
        }
    );
}

#[test]
fn test_check_license_mit_admissible() {
    let result = check_license("MIT");
    match result {
        LicenseStatus::Admissible { license } => assert_eq!(license, "MIT"),
        other => panic!("expected Admissible, got {:?}", other),
    }
}

#[test]
fn test_check_license_qwen_research_excluded() {
    let result = check_license("qwen-research");
    match result {
        LicenseStatus::Excluded { license, reason } => {
            assert_eq!(license, "qwen-research");
            assert!(
                reason.contains("non-commercial"),
                "reason should mention non-commercial: {}",
                reason
            );
        }
        other => panic!("expected Excluded, got {:?}", other),
    };
}

#[test]
fn test_check_license_unknown_excluded() {
    let result = check_license("SomeUnknownLicense");
    match result {
        LicenseStatus::Excluded { reason, .. } => {
            assert!(
                reason.contains("unrecognised"),
                "reason should mention unrecognised: {}",
                reason
            );
        }
        other => panic!("expected Excluded, got {:?}", other),
    };
}

#[test]
fn test_check_license_case_insensitive() {
    // "apache-2.0" (lowercase) should be admissible
    let result = check_license("apache-2.0");
    assert!(matches!(result, LicenseStatus::Admissible { .. }));

    // "APACHE-2.0" (uppercase) should also be admissible
    let result = check_license("APACHE-2.0");
    assert!(matches!(result, LicenseStatus::Admissible { .. }));
}

#[test]
fn test_check_license_gpl_excluded() {
    let result = check_license("GPL-3.0");
    assert!(matches!(result, LicenseStatus::Excluded { .. }));
}
