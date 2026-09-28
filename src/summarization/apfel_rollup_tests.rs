// Rollup batch tests for issue #771.
//
// This file is included via `#[path]` from apfel.rs's tests module. It tests
// the rollup-specific batch functions (rollup_batch_summarize, ROLLUP_MAX_ENTITIES_PER_BATCH)
// and counts subprocess invocations through each test's own fake-apfel log.

use crate::summarization::apfel::{
    APFEL_INPUT_CHAR_BUDGET, BATCH_SYSTEM_PROMPT, BATCH_USER_PROMPT, BUDGET_EXCEEDED_MARKER,
    ROLLUP_BATCH_SYSTEM_PROMPT, ROLLUP_BATCH_USER_PROMPT, ROLLUP_MAX_ENTITIES_PER_BATCH,
    pack_by_char_budget, parse_batch_response, rollup_batch_summarize,
};
use crate::summarization::pipeline::is_overflow_error;
use crate::summarization::pipeline_tests_fixtures::write_fake_apfel;

fn format_rollup_batch(entries: &[(String, String, String)]) -> String {
    let input = entries
        .iter()
        .enumerate()
        .map(|(i, (_, _, p))| format!("--- Entry: #{}\n{}\n---\n", i, p))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n\n{}", ROLLUP_BATCH_USER_PROMPT, input)
}

/// rollup_batch_summarize with empty input returns an empty vec.
#[test]
fn test_rollup_batch_summarize_empty() {
    assert!(rollup_batch_summarize(&[], None).unwrap().is_empty());
}

/// An over-budget single entry is rejected with BUDGET_EXCEEDED_MARKER
/// and classified as overflow by is_overflow_error.
#[test]
fn test_rollup_batch_over_budget_rejected() {
    let entries = vec![("f".to_string(), "M".to_string(), "x".repeat(9000))];
    let err = rollup_batch_summarize(&entries, None).expect_err("over-budget must be rejected");
    assert!(format!("{err}").contains(BUDGET_EXCEEDED_MARKER));
    assert!(
        is_overflow_error(&err),
        "over-budget rollup error must classify as overflow"
    );
}

/// pack_by_char_budget produces stable, identical batches for the same input.
#[test]
fn test_rollup_pack_stable() {
    let mk = |i: usize| {
        (
            format!("f{i}"),
            format!("F{i}"),
            format!("desc {i} ").repeat(30),
        )
    };
    let entries: Vec<_> = (0..20).map(mk).collect();
    let b1 = pack_by_char_budget(&entries, APFEL_INPUT_CHAR_BUDGET);
    let b2 = pack_by_char_budget(&entries, APFEL_INPUT_CHAR_BUDGET);
    assert_eq!(b1.len(), b2.len(), "batch count must be stable");
    for (x, y) in b1.iter().zip(b2.iter()) {
        assert_eq!(x.len(), y.len(), "batch size must be stable");
        for (a, b) in x.iter().zip(y.iter()) {
            assert_eq!(a.2, b.2, "entry prompt must be stable");
        }
    }
}

/// Every rollup batch, when formatted the way rollup_batch_summarize would,
/// respects APFEL_INPUT_CHAR_BUDGET.
#[test]
fn test_rollup_pack_respects_budget() {
    let entries: Vec<_> = (0..10)
        .map(|i| (format!("f{i}"), format!("F{i}"), "line ".repeat(150)))
        .collect();
    for batch in &pack_by_char_budget(&entries, APFEL_INPUT_CHAR_BUDGET) {
        assert!(
            format_rollup_batch(batch).len() <= APFEL_INPUT_CHAR_BUDGET,
            "rollup batch formatted size {} exceeds budget {}",
            format_rollup_batch(batch).len(),
            APFEL_INPUT_CHAR_BUDGET
        );
    }
}

/// parse_batch_response correctly handles partial rollup results (fewer
/// summaries than entries) and out-of-order indices with rollup-shaped content.
#[test]
fn test_parse_rollup_partial_and_out_of_order() {
    let r1 = parse_batch_response("#0: auth logic\n#2: db connections", 4);
    assert_eq!(r1.len(), 4);
    assert_eq!(r1[0], Some("auth logic".into()));
    assert_eq!(r1[1], None);
    assert_eq!(r1[2], Some("db connections".into()));
    assert_eq!(r1[3], None);

    let r2 = parse_batch_response("#2: third entry\n#0: first entry\n#1: second entry", 3);
    assert_eq!(r2[0], Some("first entry".into()));
    assert_eq!(r2[1], Some("second entry".into()));
    assert_eq!(r2[2], Some("third entry".into()));
}

/// ROLLUP_MAX_ENTITIES_PER_BATCH is 40, and the rollup prompts are distinct
/// from the function-tier prompts. The fake apfel subprocess appends to the
/// per-test invocation log on every call (issue #869).
#[test]
fn test_rollup_constants_and_counter() {
    assert_eq!(ROLLUP_MAX_ENTITIES_PER_BATCH, 40);
    assert_ne!(ROLLUP_BATCH_USER_PROMPT, BATCH_USER_PROMPT);
    assert_ne!(ROLLUP_BATCH_SYSTEM_PROMPT, BATCH_SYSTEM_PROMPT);

    let tmp = tempfile::tempdir().unwrap();
    let bin_path = tmp.path().join("apfel");
    let log = crate::summarization::pipeline_tests_fixtures::fake_apfel_invocation_log(tmp.path());
    write_fake_apfel(
        &bin_path,
        &crate::summarization::pipeline_tests_fixtures::logging_fake_apfel_script(
            &log,
            "printf '%s' '{\"content\": \"#0: s1\\n#1: s2\"}'\nexit 0\n",
        ),
    );
    let s = bin_path.to_string_lossy().to_string();
    // Hold the guard: serializes the fake apfel subprocess against other
    // apfel-interacting tests (issue #869).
    let _guard = crate::summarization::summarizer_fullpath_tests::env_guard();
    let _ =
        crate::summarization::apfel::run_apfel(&s, 10, &["-o", "json", "-s", "t"], "x").unwrap();
    let invocations =
        crate::summarization::pipeline_tests_fixtures::read_fake_apfel_invocations(&log);
    assert_eq!(
        invocations, 1,
        "expected exactly 1 fake apfel invocation; got {invocations}"
    );
}
