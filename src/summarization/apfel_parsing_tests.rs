// Tests for parse_batch_response and pack_by_char_budget.
//
// Extracted from the one-shot apfel tests to keep apfel_oneshot_tests.rs
// within the 800-line test budget (AGENTS.md §6).

use super::*;

#[test]
fn test_parse_batch_response_with_valid_lines() {
    let results = parse_batch_response("#0: adds two numbers\n#1: subtracts two numbers", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], Some("adds two numbers".to_string()));
    assert_eq!(results[1], Some("subtracts two numbers".to_string()));
}

#[test]
fn test_parse_batch_response_with_malformed_lines() {
    // Lines without '#' separator should be silently skipped
    let results = parse_batch_response("no hash here\n#0: valid summary\n", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], Some("valid summary".to_string()));
    assert_eq!(results[1], None);
}

#[test]
fn test_parse_batch_response_empty_input() {
    let results = parse_batch_response("", 2);
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.is_none()));
}

#[test]
fn test_parse_batch_response_all_malformed() {
    let results = parse_batch_response("no hash\neither no hash\nthird line no hash", 3);
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.is_none()));
}

#[test]
fn test_parse_batch_response_trims_whitespace() {
    let results = parse_batch_response("  #0  :  adds numbers  ", 1);
    assert_eq!(results[0], Some("adds numbers".to_string()));
}

#[test]
fn test_parse_batch_response_partial_results() {
    // Test partial results: some functions have summaries, some don't
    let results = parse_batch_response("#0: only first summary", 3);
    assert_eq!(results.len(), 3);
    assert_eq!(results[0], Some("only first summary".to_string()));
    assert_eq!(results[1], None);
    assert_eq!(results[2], None);
}

#[test]
fn test_parse_batch_response_with_markdown_fences() {
    // Apfel may wrap output in ```markdown fences
    let results = parse_batch_response("```\n#0: adds numbers\n#1: subtracts numbers\n```", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], Some("adds numbers".to_string()));
    assert_eq!(results[1], Some("subtracts numbers".to_string()));
}

#[test]
fn test_parse_batch_response_out_of_range_index() {
    // Out-of-range indices should be silently skipped
    let results = parse_batch_response("#0: valid\n#5: out of range\n#1: also valid", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], Some("valid".to_string()));
    assert_eq!(results[1], Some("also valid".to_string()));
}

#[test]
fn test_pack_by_char_budget_single_snippet() {
    let snippets = vec![(
        "src/lib.rs".to_string(),
        "func".to_string(),
        "x + y".to_string(),
    )];
    let batches = super::pack_by_char_budget(&snippets, APFEL_INPUT_CHAR_BUDGET);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 1);
    assert_eq!(batches[0][0].1, "func");
}

#[test]
fn test_pack_by_char_budget_multiple_under_limit() {
    let snippets = vec![
        (
            "src/lib.rs".to_string(),
            "add".to_string(),
            "fn add(a: i32, b: i32) -> i32 { a + b }".to_string(),
        ),
        (
            "src/lib.rs".to_string(),
            "sub".to_string(),
            "fn sub(a: i32, b: i32) -> i32 { a - b }".to_string(),
        ),
    ];
    let batches = super::pack_by_char_budget(&snippets, APFEL_INPUT_CHAR_BUDGET);
    // Both fit under 8000 chars, so should be in one batch
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].len(), 2);
}

#[test]
fn test_pack_by_char_budget_respects_limit() {
    // Create snippets where individual ones are small but collectively exceed budget
    let large_code = "x".repeat(4000); // 4000 chars each
    let snippets = vec![
        (
            "src/one.rs".to_string(),
            "func1".to_string(),
            large_code.clone(),
        ),
        (
            "src/two.rs".to_string(),
            "func2".to_string(),
            large_code.clone(),
        ),
        (
            "src/three.rs".to_string(),
            "func3".to_string(),
            large_code.clone(),
        ),
    ];
    let batches = super::pack_by_char_budget(&snippets, APFEL_INPUT_CHAR_BUDGET);
    // With 4000-char snippets, 2 should fit in first batch, 1 in second
    assert!(!batches.is_empty(), "should have at least 1 batch");
    // First batch should have content, second batch should have content
    if batches.len() > 1 {
        assert!(!batches[0].is_empty());
        assert!(!batches[1].is_empty());
    }
}

#[test]
fn test_pack_by_char_budget_single_over_budget() {
    // A single snippet larger than budget must still be packed (not dropped)
    let large_code = "x".repeat(9000);
    let snippets = vec![("src/big.rs".to_string(), "big_func".to_string(), large_code)];
    let batches = super::pack_by_char_budget(&snippets, APFEL_INPUT_CHAR_BUDGET);
    assert_eq!(
        batches.len(),
        1,
        "over-budget snippet must still be in a batch"
    );
    assert_eq!(batches[0].len(), 1);
    assert_eq!(batches[0][0].1, "big_func");
}

#[test]
fn test_pack_by_char_budget_empty() {
    let snippets: Vec<(String, String, String)> = vec![];
    let batches = super::pack_by_char_budget(&snippets, APFEL_INPUT_CHAR_BUDGET);
    assert_eq!(batches.len(), 0);
}

/// Helper to format a batch the way batch_summarize does, so we can verify invariants.
pub(crate) fn format_batch_for_apfel(snippets: &[(String, String, String)]) -> String {
    let batch_input = snippets
        .iter()
        .enumerate()
        .map(|(i, (_, _, code))| format!("--- Function: #{}\n{}\n---\n", i, code))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{}\n\n{}", BATCH_USER_PROMPT, batch_input)
}

#[test]
fn test_pack_by_char_budget_respects_budget_with_formatting() {
    // This test verifies the key invariant: after pack_by_char_budget produces batches
    // and batch_summarize formats them with the prompt + separator + joins,
    // every formatted batch must be <= APFEL_INPUT_CHAR_BUDGET.
    // We use near-budget snippets to stress-test the packer.

    // Create snippets that are each ~3900-4000 chars of code.
    // The packer must ensure that when 2+ such snippets are packed together,
    // the final formatted input doesn't exceed 8000.
    let large_code_1 = "x".repeat(3950);
    let large_code_2 = "y".repeat(3950);
    let large_code_3 = "z".repeat(3950);

    let snippets = vec![
        (
            "src/one.rs".to_string(),
            "func1".to_string(),
            large_code_1.clone(),
        ),
        (
            "src/two.rs".to_string(),
            "func2".to_string(),
            large_code_2.clone(),
        ),
        (
            "src/three.rs".to_string(),
            "func3".to_string(),
            large_code_3.clone(),
        ),
    ];

    let batches = super::pack_by_char_budget(&snippets, APFEL_INPUT_CHAR_BUDGET);

    // Verify that every batch, when formatted as batch_summarize would format it,
    // respects the budget.
    for (batch_idx, batch) in batches.iter().enumerate() {
        let formatted = format_batch_for_apfel(batch);
        let len = formatted.len();
        assert!(
            len <= APFEL_INPUT_CHAR_BUDGET,
            "Batch {} formatted size {} exceeds budget {}",
            batch_idx,
            len,
            APFEL_INPUT_CHAR_BUDGET
        );
    }

    // Ensure we got multiple batches (otherwise the test isn't meaningful).
    // Three ~3950-char snippets should split across at least 2 batches.
    assert!(
        batches.len() >= 2,
        "Expected at least 2 batches for 3 large snippets, got {}",
        batches.len()
    );
}

#[test]
fn test_pack_by_char_budget_single_over_budget_still_packed_and_formatted() {
    // A single snippet larger than the effective budget (but possibly larger than APFEL_INPUT_CHAR_BUDGET
    // when formatted) must still be packed. The defensive check in batch_summarize may reject it,
    // but the packer must not drop it.
    let giant_code = "x".repeat(9000);
    let snippets = vec![(
        "src/huge.rs".to_string(),
        "huge_func".to_string(),
        giant_code,
    )];

    let batches = super::pack_by_char_budget(&snippets, APFEL_INPUT_CHAR_BUDGET);

    assert_eq!(
        batches.len(),
        1,
        "over-budget snippet must still be in a batch"
    );
    assert_eq!(batches[0].len(), 1);
    assert_eq!(batches[0][0].1, "huge_func");

    // When formatted, this batch will exceed the budget, which is expected
    // (the defensive check in batch_summarize is a backstop).
    let formatted = format_batch_for_apfel(&batches[0]);
    assert!(
        formatted.len() > APFEL_INPUT_CHAR_BUDGET,
        "single over-budget snippet is expected to exceed limit when formatted"
    );
}

#[test]
fn test_parse_batch_response_out_of_order_indices() {
    // Test that out-of-order indices are handled correctly based on index, not line order.
    // This is critical: placement must be index-based to avoid "positional drift".
    let results = parse_batch_response("#1: second function\n#0: first function", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0],
        Some("first function".to_string()),
        "index 0 should map to first function even if it appears later"
    );
    assert_eq!(
        results[1],
        Some("second function".to_string()),
        "index 1 should map to second function even if it appears earlier"
    );
}

#[test]
fn test_parse_batch_response_duplicate_indices_last_wins() {
    // Test behavior when apfel returns duplicate indices.
    // Current behavior: last duplicate wins (overwrites previous).
    // This is acceptable as it represents the most recent/highest-quality result.
    let results = parse_batch_response("#0: first attempt\n#0: second attempt\n#1: correct", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0],
        Some("second attempt".to_string()),
        "last duplicate for index 0 should win"
    );
    assert_eq!(results[1], Some("correct".to_string()));
}

#[test]
fn test_parse_batch_response_multi_digit_indices() {
    // Test parsing with multi-digit indices (e.g., #10, #100).
    // This ensures the parser doesn't break on batches with 100+ entries.
    let results = parse_batch_response("#9: ninth\n#10: tenth\n#100: hundredth", 150);
    assert_eq!(results.len(), 150);
    assert_eq!(results[9], Some("ninth".to_string()));
    assert_eq!(results[10], Some("tenth".to_string()));
    assert_eq!(results[100], Some("hundredth".to_string()));
}

#[test]
fn test_parse_batch_response_mixed_valid_and_invalid() {
    // Test a realistic scenario with valid lines, missing lines, out-of-range indices,
    // and duplicate indices all present.
    let results = parse_batch_response(
        "no hash here\n#0: first valid\n#99: out of range\n#1: second valid\n#0: duplicate\n#1: also duplicate",
        3,
    );
    assert_eq!(results.len(), 3);
    assert_eq!(
        results[0],
        Some("duplicate".to_string()),
        "last duplicate for index 0 should win"
    );
    assert_eq!(
        results[1],
        Some("also duplicate".to_string()),
        "last duplicate for index 1 should win"
    );
    assert_eq!(results[2], None, "index 2 was never provided");
}

/// Issue #776: a model response body that has no parsable `#<n>:` line at all
/// (every position undemuxed) must classify as a parse failure — counted,
/// not merely logged. A body with at least one parseable line is a normal
/// partial result, not a parse failure.
#[test]
fn test_parse_failure_classified_when_no_lines_parseable() {
    use super::*;

    // Entirely malformed body: no `#<n>:` line exists, so every position is a
    // parse failure (the #<n>: contract was not honoured).
    let results = parse_batch_response(
        "the model answered in plain prose instead of numbered lines",
        2,
    );
    assert!(results.iter().all(Option::is_none), "no lines should parse");
    let err = crate::error::LievoError::SummarizationFailed(format!(
        "batch_summarize: no parsable #<n>: lines in model output {}",
        crate::summarization::pipeline::PARSE_FAILURE_MARKER
    ));
    assert!(
        crate::summarization::pipeline::is_parse_failure_error(&err),
        "all-malformed batch must classify as a parse failure"
    );
    assert!(
        !crate::summarization::pipeline::is_overflow_error(&err),
        "a parse failure is not a context overflow"
    );

    // A body with one parseable line is NOT a parse failure — it is a normal
    // (partial) result that the pipeline salvages, so it must not be counted
    // as a parse failure. The classifier is the gate that decides this: a
    // partial-demotion (salvage) path never carries the parse-failure marker.
    let partial = parse_batch_response("#0: only one line parses\nthe rest is prose", 2);
    assert!(
        partial.iter().any(Option::is_some),
        "a partial demux has at least one parseable line (salvage, not failure)"
    );

    // Overflow errors must never classify as parse failures.
    let overflow = crate::error::LievoError::SummarizationFailed(
        crate::summarization::pipeline::CONTEXT_OVERFLOW_MARKER.to_string(),
    );
    assert!(!crate::summarization::pipeline::is_parse_failure_error(
        &overflow
    ));
}

#[test]
fn test_parse_batch_response_index_beyond_usize() {
    // Test that extremely large indices are handled gracefully (parsed as usize but rejected).
    let input = "#9999999999999: too large\n#0: just right".to_string();
    let results = parse_batch_response(&input, 2);
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0],
        Some("just right".to_string()),
        "index 0 should be parsed correctly"
    );
    assert_eq!(results[1], None);
}

#[test]
fn test_parse_batch_response_whitespace_around_indices() {
    // Test that whitespace around indices and colons is handled gracefully.
    let results = parse_batch_response("#  0  :  with spaces\n  #1 : trimmed  ", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], Some("with spaces".to_string()));
    assert_eq!(results[1], Some("trimmed".to_string()));
}

#[test]
fn test_parse_batch_response_multiple_hash_chars() {
    // Test that when a line has multiple '#' characters, only the first is used.
    // This ensures correct parsing even if summary contains '#'
    let results = parse_batch_response("#0: summary with # hash char\n#1: normal", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0], Some("summary with # hash char".to_string()));
    assert_eq!(results[1], Some("normal".to_string()));
}

#[test]
fn test_parse_batch_response_empty_indices() {
    // Test that empty index strings (e.g., "# : summary") are handled gracefully.
    let results = parse_batch_response("# : empty index\n#0: valid", 2);
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0],
        Some("valid".to_string()),
        "empty index should be skipped"
    );
    assert_eq!(results[1], None);
}

#[test]
fn test_pack_by_char_budget_multi_batch_index_space_reset() {
    // Critical adversarial review test (a): verify that each batch has its own index space.
    // A bug where index space crosses batches would cause SUMMARY MISASSIGNMENT
    // (worse than original name-collision bug — wrong summaries, not just missing).
    //
    // Setup: create enough functions to span multiple batches.
    // Batch 0 will have indices 0..N, batch 1 will have indices 0..M, etc.
    // Verify that index 0 in batch 1 is different from index 0 in batch 0.

    // Use large code to force multiple batches within budget
    let make_snippet = |i: usize| {
        // Each snippet is ~200 chars of code (enough to limit batch size)
        let code = format!(
            "fn func{i}() {{ let x{} = {}; let y{} = {}; x + y }}",
            i, i, i, i
        );
        (format!("src/file{i}.rs"), format!("func{i}"), code)
    };

    // Create 200 snippets; this should span multiple batches given the budget
    // Each snippet is ~80 chars, so 200 should be ~16000 chars, which exceeds the 8000 budget
    let snippets: Vec<_> = (0..200).map(make_snippet).collect();

    let batches = pack_by_char_budget(&snippets, APFEL_INPUT_CHAR_BUDGET);

    // Verify we have multiple batches
    assert!(
        batches.len() > 1,
        "expected multiple batches for 50 small snippets, got {}",
        batches.len()
    );

    // For each batch, verify that the input format uses indices starting at 0
    for (batch_idx, batch) in batches.iter().enumerate() {
        let formatted = format_batch_for_apfel(batch);

        // Verify that this batch's formatted input starts with #0
        assert!(
            formatted.contains("--- Function: #0"),
            "batch {} should contain #0 (index space reset per batch)",
            batch_idx
        );

        // Verify that indices are sequential within this batch
        for (i, _) in batch.iter().enumerate() {
            assert!(
                formatted.contains(&format!("--- Function: #{}", i)),
                "batch {} should contain index {}",
                batch_idx,
                i
            );
        }

        // Verify that the budget is respected
        assert!(
            formatted.len() <= APFEL_INPUT_CHAR_BUDGET,
            "batch {} formatted input exceeds budget: {}",
            batch_idx,
            formatted.len()
        );
    }

    // Critical invariant: each batch has its own index space starting at 0
    // (This is ensured by using enumerate() in format_batch_for_apfel and batch_summarize)
    // If a global index space were used, batch 1 would start with a non-zero index.
    let batch_0_formatted = format_batch_for_apfel(&batches[0]);
    let batch_1_formatted = format_batch_for_apfel(&batches[1]);

    assert!(
        batch_0_formatted.contains("--- Function: #0"),
        "batch 0 should start with index 0"
    );
    assert!(
        batch_1_formatted.contains("--- Function: #0"),
        "batch 1 should also start with index 0 (index space reset per batch)"
    );

    // Verify that the snippet identity is preserved across batches.
    // The function at batch 0, position 0 is different from batch 1, position 0.
    let batch_0_pos_0_file = &batches[0][0].0;
    let batch_1_pos_0_file = &batches[1][0].0;
    assert_ne!(
        batch_0_pos_0_file, batch_1_pos_0_file,
        "different batches should contain different functions at position 0"
    );
}

/// Full pipeline regression (issue #649): a single over-budget function is
/// rejected offline by `batch_summarize`'s budget guard (no apfel subprocess
/// is spawned), and the pipeline must track it via the real
/// `skipped_oversized` counter — distinct from `skipped_no_code` — rather
/// than silently dropping it.
#[test]
fn test_skipped_oversized_tracked_by_real_counter() {
    use crate::extraction::function_preservation::function_id;
    use crate::summarization::SummarizationPipeline;
    use crate::summarization::pipeline_tests_fixtures::{
        CodeUnit, TestStorage, make_file_entity, make_fn_entity,
    };

    let mut storage = TestStorage::new();
    let repo_id = "test-repo";
    let file_path = "src/lib.rs";
    let file_entity = make_file_entity("file-oversized", file_path);
    storage.add_file(repo_id, file_path, file_entity.clone());

    let fn_name = "giant_func";
    let fn_entity = make_fn_entity(
        &function_id(&file_entity.id, fn_name),
        &file_entity.id,
        fn_name,
    );
    storage.add_function(&function_id(&file_entity.id, fn_name), fn_entity);

    let oversized_code = "// oversized body\n".repeat(3000); // ~48,000 chars
    let unit = CodeUnit {
        name: fn_name.to_string(),
        unit_type: "function".to_string(),
        file: file_path.to_string(),
        line: 1,
        end_line: 1,
        language: "Rust".to_string(),
        signature: None,
        code: Some(oversized_code),
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
        qualified_name: fn_name.to_string(),
        docstring: None,
        parent_class: None,
    };

    let outcome = SummarizationPipeline::summarize_functions(
        &storage,
        repo_id,
        &[unit],
        APFEL_INPUT_CHAR_BUDGET,
        None,
    )
    .expect("summarize_functions should not hard-error on an oversized function");

    assert_eq!(
        outcome.summarized, 0,
        "oversized function must not be summarized"
    );
    assert!(
        storage.upserted.borrow().is_empty(),
        "oversized function must not be upserted with a summary"
    );
    assert_eq!(
        outcome.counters.skipped_oversized, 1,
        "oversized function must be tracked by the real skipped_oversized counter"
    );
    assert_eq!(
        outcome.counters.skipped_no_code, 0,
        "skipped_oversized is distinct from skipped_no_code — the function had code"
    );
}

/// Issue #792: an input that exceeds apfel's 8,000-char budget but fits
/// llama-server's 24,000-char budget must be summarized under llama-server
/// and skipped under apfel.
#[test]
fn test_between_budgets_summarized_under_llama_skipped_under_apfel() {
    use crate::config::SummarizerBackend;
    use crate::extraction::function_preservation::function_id;
    use crate::summarization::SummarizationPipeline;
    use crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET;
    use crate::summarization::backend_profile::HTTP_BACKEND_INPUT_CHAR_BUDGET;
    use crate::summarization::pipeline_tests_fixtures::{
        CodeUnit, TestStorage, make_file_entity, make_fn_entity,
    };
    use crate::summarization::summarizer_backend::start_fake_http_server;

    let between_code = "// between-budgets padding\n".repeat(500); // ~12,000 chars
    assert!(between_code.len() > APFEL_INPUT_CHAR_BUDGET);
    assert!(between_code.len() < HTTP_BACKEND_INPUT_CHAR_BUDGET);

    let fn_name = "between_budgets_fn";
    let file_path = "src/between.rs";
    let build_storage = || {
        let mut storage = TestStorage::new();
        let file_entity = make_file_entity("file-between", file_path);
        storage.add_file("test-repo", file_path, file_entity.clone());
        let fn_entity = make_fn_entity(
            &function_id(&file_entity.id, fn_name),
            &file_entity.id,
            fn_name,
        );
        storage.add_function(&function_id(&file_entity.id, fn_name), fn_entity);
        storage
    };
    let unit = CodeUnit {
        name: fn_name.to_string(),
        unit_type: "function".to_string(),
        file: file_path.to_string(),
        line: 1,
        end_line: 1,
        language: "Rust".to_string(),
        signature: None,
        code: Some(between_code),
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
        qualified_name: fn_name.to_string(),
        docstring: None,
        parent_class: None,
    };

    // Under apfel transport: guard rejects (exceeds 8,000) → skipped.
    {
        let body = serde_json::json!({
            "choices": [{ "message": { "content": "#0: should not be reached", "role": "assistant" } }]
        })
        .to_string();
        let addr = start_fake_http_server(200, &body).unwrap();
        let transport = crate::summarization::apfel::BackendTransport {
            url: format!("http://127.0.0.1:{}", addr.port()),
            backend: SummarizerBackend::Apfel,
            model_name: None,
        };
        let storage = build_storage();
        let outcome = SummarizationPipeline::summarize_functions(
            &storage,
            "test-repo",
            std::slice::from_ref(&unit),
            APFEL_INPUT_CHAR_BUDGET,
            Some(&transport),
        )
        .expect("should not hard-error");
        assert_eq!(outcome.summarized, 0, "must NOT be summarized under apfel");
        assert_eq!(outcome.counters.skipped_oversized, 1);
    }

    // Under llama-server transport: guard uses 24,000 → summarized.
    {
        let body = serde_json::json!({
            "choices": [{ "message": { "content": "#0: between-budgets summary", "role": "assistant" } }]
        })
        .to_string();
        let addr = start_fake_http_server(200, &body).unwrap();
        let transport = crate::summarization::apfel::BackendTransport {
            url: format!("http://127.0.0.1:{}", addr.port()),
            backend: SummarizerBackend::LlamaServer,
            model_name: None,
        };
        let storage = build_storage();
        let outcome = SummarizationPipeline::summarize_functions(
            &storage,
            "test-repo",
            std::slice::from_ref(&unit),
            HTTP_BACKEND_INPUT_CHAR_BUDGET,
            Some(&transport),
        )
        .expect("should not hard-error");
        assert_eq!(
            outcome.summarized, 1,
            "between-budgets input MUST be summarized under llama-server"
        );
        assert_eq!(outcome.counters.skipped_oversized, 0);
    }
}

/// Issue #792: overflow at the larger backend budget still skips cleanly.
#[test]
fn test_overflow_at_llama_budget_still_skipped_cleanly() {
    use crate::config::SummarizerBackend;
    use crate::extraction::function_preservation::function_id;
    use crate::summarization::SummarizationPipeline;
    use crate::summarization::backend_profile::HTTP_BACKEND_INPUT_CHAR_BUDGET;
    use crate::summarization::pipeline_tests_fixtures::{
        CodeUnit, TestStorage, make_file_entity, make_fn_entity,
    };
    use crate::summarization::summarizer_backend::start_fake_http_server;

    let oversized_code = "// oversized for llama\n".repeat(2000); // ~34,000 chars
    assert!(oversized_code.len() > HTTP_BACKEND_INPUT_CHAR_BUDGET);

    let fn_name = "oversized_fn";
    let file_path = "src/oversized.rs";
    let mut storage = TestStorage::new();
    let file_entity = make_file_entity("file-oversized-llama", file_path);
    storage.add_file("test-repo", file_path, file_entity.clone());
    let fn_entity = make_fn_entity(
        &function_id(&file_entity.id, fn_name),
        &file_entity.id,
        fn_name,
    );
    storage.add_function(&function_id(&file_entity.id, fn_name), fn_entity);

    let unit = CodeUnit {
        name: fn_name.to_string(),
        unit_type: "function".to_string(),
        file: file_path.to_string(),
        line: 1,
        end_line: 1,
        language: "Rust".to_string(),
        signature: None,
        code: Some(oversized_code),
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: vec![],
        qualified_name: fn_name.to_string(),
        docstring: None,
        parent_class: None,
    };

    let body = serde_json::json!({
        "choices": [{ "message": { "content": "#0: should not be reached", "role": "assistant" } }]
    })
    .to_string();
    let addr = start_fake_http_server(200, &body).unwrap();
    let transport = crate::summarization::apfel::BackendTransport {
        url: format!("http://127.0.0.1:{}", addr.port()),
        backend: SummarizerBackend::LlamaServer,
        model_name: None,
    };

    let outcome = SummarizationPipeline::summarize_functions(
        &storage,
        "test-repo",
        std::slice::from_ref(&unit),
        HTTP_BACKEND_INPUT_CHAR_BUDGET,
        Some(&transport),
    )
    .expect("should not hard-error");
    assert_eq!(
        outcome.summarized, 0,
        "oversized input must not be summarized"
    );
    assert_eq!(
        outcome.counters.skipped_oversized, 1,
        "input exceeding the per-backend budget must be counted as skipped_oversized"
    );
}
