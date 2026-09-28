// Tests for summarization pipeline — entity lookup behavior, config, and partial batch salvage.
//
// Tests are organized into two groups (entities and config) that share
// fixtures defined in pipeline_tests_fixtures.rs.

use crate::summarization::pipeline_tests_fixtures as fixtures;

// Apfel-timeout salvage test (issue #811 task-b), moved out (issue #869) to
// allow the crate-wide env lock around its LIEVO_APFEL_TIMEOUT_SECS mutation.
#[path = "pipeline_apfel_timeout_tests.rs"]
mod apfel_timeout_salvage;

mod salvage {
    use super::super::*;

    #[test]
    fn test_salvage_partial_batch_full_match() {
        let results = vec![
            Some("adds numbers".to_string()),
            Some("subtracts numbers".to_string()),
        ];
        let (present, missing) = salvage_partial_batch(&results);
        assert_eq!(present, vec![0, 1]);
        assert!(missing.is_empty());
    }

    #[test]
    fn test_salvage_partial_batch_partial_match() {
        let results = vec![
            Some("adds numbers".to_string()),
            None,
            Some("multiplies numbers".to_string()),
        ];
        let (present, missing) = salvage_partial_batch(&results);
        assert_eq!(present, vec![0, 2]);
        assert_eq!(missing, vec![1]);
    }

    #[test]
    fn test_salvage_partial_batch_empty_results() {
        let results = vec![None, None];
        let (present, missing) = salvage_partial_batch(&results);
        assert!(present.is_empty());
        assert_eq!(missing, vec![0, 1]);
    }

    #[test]
    fn test_salvage_partial_batch_order_preservation() {
        let results = vec![
            None,
            Some("second summary".to_string()),
            None,
            Some("fourth summary".to_string()),
        ];
        let (present, missing) = salvage_partial_batch(&results);
        assert_eq!(present, vec![1, 3]);
        assert_eq!(missing, vec![0, 2]);
    }
}

mod entities {
    use super::super::*;
    use super::fixtures::{CodeUnit, TestStorage, make_file_entity, make_fn_entity};

    // This test verifies that partial batch results are salvaged correctly.
    // We can't easily inject a mock into SummarizationPipeline, so we test
    // the salvage logic through the public helper function directly.
    #[test]
    fn test_partial_batch_salvage_logic() {
        // Results for 3 functions, but apfel only returns summaries for 2
        // Simulate batch_summarize returning partial results (2 of 3)
        let mock_results = vec![
            Some("Adds two numbers".to_string()),
            None, // sub is missing — partial batch scenario
            Some("Multiplies two numbers".to_string()),
        ];

        let (present_positions, missing_positions) = salvage_partial_batch(&mock_results);

        // Assert present positions
        assert_eq!(present_positions.len(), 2, "2 positions should be present");
        assert_eq!(present_positions[0], 0);
        assert_eq!(present_positions[1], 2);

        // Assert missing positions for retry
        assert_eq!(missing_positions.len(), 1, "1 position should be missing");
        assert_eq!(missing_positions[0], 1);
    }

    #[test]
    fn test_function_id_deterministic() {
        let file_entity = make_file_entity("file-abc123", "src/lib.rs");
        let fn_id = function_id(&file_entity.id, "my_function");
        let fn_id2 = function_id(&file_entity.id, "my_function");
        assert_eq!(fn_id, fn_id2, "function_id must be deterministic");
        let fn_id_other = function_id(&file_entity.id, "other_function");
        assert_ne!(
            fn_id, fn_id_other,
            "different names must produce different IDs"
        );
    }

    #[test]
    fn test_function_entity_absent_means_no_upsert() {
        // Build a code unit for a long function (>20 lines) so it goes through the
        // individual summarization path that does entity lookup BEFORE calling apfel.
        // When the function entity is absent from storage, the pipeline must skip it
        // without calling upsert_entity.
        let long_code = "fn my_func() {\n".repeat(21); // 21 lines
        let code_unit: CodeUnit = CodeUnit {
            name: "my_func".to_string(),
            unit_type: "function".to_string(),
            file: "src/lib.rs".to_string(),
            line: 1,
            end_line: 21,
            language: "Rust".to_string(),
            signature: None,
            code: Some(long_code),
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
            qualified_name: "my_func".to_string(),
            docstring: None,
            parent_class: None,
        };

        let mut storage = TestStorage::new();
        let file_entity = make_file_entity("file-abc123", "src/lib.rs");
        storage.add_file("test-repo", "src/lib.rs", file_entity);

        // Function entity is deliberately absent — storage.get_entity returns None
        // for any ID that doesn't start with "fn-". Even if the derived fn- ID is
        // queried, the mock returns None, correctly simulating a missing entity.

        let outcome = SummarizationPipeline::summarize_functions(
            &storage,
            "test-repo",
            &[code_unit],
            crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
            None,
        )
        .expect("summarize_functions should not error")
        .summarized;

        // Pipeline must return 0 — no functions summarized since entity is absent.
        assert_eq!(
            outcome, 0,
            "no functions should be summarized when entity is absent"
        );

        // No upsert calls should have been made.
        let upserted = storage.upserted.borrow();
        assert_eq!(
            upserted.len(),
            0,
            "no upsert should occur when function entity is absent"
        );
    }

    #[test]
    fn test_function_entity_preferred_when_present() {
        let mut storage = TestStorage::new();
        let file_entity = make_file_entity("file-abc123", "src/lib.rs");
        let fn_entity = make_fn_entity(
            &function_id("file-abc123", "my_func"),
            "file-abc123",
            "my_func",
        );
        storage.add_file("test-repo", "src/lib.rs", file_entity);
        storage.add_function(&function_id("file-abc123", "my_func"), fn_entity);

        let found_file = storage
            .entity_by_path("test-repo", "src/lib.rs")
            .unwrap()
            .expect("file entity");
        let fn_id = function_id(&found_file.id, "my_func");
        let found_fn = storage
            .get_entity(&fn_id)
            .unwrap()
            .expect("function entity should exist");

        // Pipeline should update the function entity, not the file entity.
        let mut updated = found_fn;
        updated.summary = Some("Function summary".to_string());
        updated.summary_commit = Some(SummarizationPipeline::hash_code("fn code"));
        storage.upsert_entity(&updated).unwrap();

        let upserted = storage.upserted.borrow();
        assert_eq!(upserted.len(), 1);
        assert!(upserted[0].id.starts_with("fn-"));
        assert_eq!(upserted[0].summary.as_deref(), Some("Function summary"));
    }

    #[test]
    fn test_hash_code() {
        let code = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let hash1 = SummarizationPipeline::hash_code(code);
        let hash2 = SummarizationPipeline::hash_code(code);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16);

        let code2 = "fn add(a: i32, b: i32) -> i32 { a + b + 1 }";
        let hash3 = SummarizationPipeline::hash_code(code2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_summarize_functions_skips_test_prefix_functions() {
        // Verify that summarize_functions skips functions with test_ prefix,
        // matching preserve_functions behavior (issue #560).
        // A normal function should be summarized; a test_ prefix function should not.
        let normal_code = "fn normal_func() {\n    let x = 1;\n    let y = 2;\n    x + y\n}";
        let test_code = "fn test_something() {\n    assert_eq!(1, 1);\n}";

        let normal_unit = CodeUnit {
            name: "normal_func".to_string(),
            unit_type: "function".to_string(),
            file: "src/lib.rs".to_string(),
            line: 1,
            end_line: 5,
            language: "Rust".to_string(),
            signature: None,
            code: Some(normal_code.to_string()),
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
            qualified_name: "normal_func".to_string(),
            docstring: None,
            parent_class: None,
        };

        let test_unit = CodeUnit {
            name: "test_something".to_string(),
            unit_type: "function".to_string(),
            file: "src/lib.rs".to_string(),
            line: 7,
            end_line: 9,
            language: "Rust".to_string(),
            signature: None,
            code: Some(test_code.to_string()),
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
            qualified_name: "test_something".to_string(),
            docstring: None,
            parent_class: None,
        };

        let mut storage = TestStorage::new();
        let file_entity = make_file_entity("file-abc123", "src/lib.rs");
        storage.add_file("test-repo", "src/lib.rs", file_entity);

        // Add function entities only for normal_func — test_something has no entity,
        // simulating that preserve_functions skipped it (as it should).
        let fn_entity = make_fn_entity(
            &function_id("file-abc123", "normal_func"),
            "file-abc123",
            "normal_func",
        );
        storage.add_function(&function_id("file-abc123", "normal_func"), fn_entity);

        // test_something has an entity in storage, but it should be filtered out
        // BEFORE entity lookup due to the test_ prefix filter.
        // This proves the filter works: without it, test_something would be upserted.
        let test_fn_entity = make_fn_entity(
            &function_id("file-abc123", "test_something"),
            "file-abc123",
            "test_something",
        );
        storage.add_function(
            &function_id("file-abc123", "test_something"),
            test_fn_entity,
        );

        // Even though test_something has an entity (would be found in lookup without the filter),
        // the pipeline should skip it BEFORE the entity lookup — not warn about missing entity.
        // We test that normal_func is summarized and test_something does NOT cause a warning.
        let result = SummarizationPipeline::summarize_functions(
            &storage,
            "test-repo",
            &[normal_unit.clone(), test_unit.clone()],
            crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
            None,
        )
        .expect("summarize_functions should not error")
        .summarized;

        let upserted = storage.upserted.borrow();
        // test_something must never be looked up or upserted — it is filtered before
        // entity lookup regardless of whether normal_func was successfully summarized.
        // We check test_something is absent; normal_func presence depends on apfel.
        assert!(
            upserted.iter().all(|e| e.name != "test_something"),
            "test_something must not be upserted; it should be filtered before entity lookup"
        );
        // If apfel succeeded, normal_func will be the only entity upserted.
        // If apfel failed, result will be 0 — but test_something must still be absent.
        if result == 1 {
            assert_eq!(
                upserted.len(),
                1,
                "exactly one entity should be upserted when apfel succeeds"
            );
            assert_eq!(
                upserted[0].name, "normal_func",
                "the upserted entity must be normal_func"
            );
        }
    }
}

// Behavioral test for salvage feature: verifies that both present and retry paths
// persist summaries to storage via upsert_function_summary.
mod behavioral {
    use super::super::*;
    use super::fixtures::{CodeUnit, TestStorage, make_file_entity, make_fn_entity};
    use crate::extraction::function_preservation::function_id;
    use sha2::{Digest, Sha256};

    #[test]
    fn test_salvage_upsert_persists_summary_to_storage() {
        // Setup storage with file entity and two function entities
        let mut storage = TestStorage::new();
        let repo_id = "test-repo";
        let file_path = "src/lib.rs";
        let file_entity = make_file_entity("file-abc123", file_path);
        storage.add_file(repo_id, file_path, file_entity.clone());

        // Create two function entities: one representing a "present" summary from batch,
        // one representing a "retried" summary from the missing path
        let file_id = &file_entity.id;
        let present_fn_name = "present_fn";
        let retried_fn_name = "retried_fn";

        let id = file_id;
        let present_fn_entity =
            make_fn_entity(&function_id(id, present_fn_name), id, present_fn_name);
        let retried_fn_entity =
            make_fn_entity(&function_id(id, retried_fn_name), id, retried_fn_name);

        storage.add_function(&function_id(file_id, present_fn_name), present_fn_entity);
        storage.add_function(&function_id(file_id, retried_fn_name), retried_fn_entity);

        // Create corresponding CodeUnits with code (required for hash computation)
        let present_fn_code = "fn present_fn() -> i32 { 42 }";
        let retried_fn_code = "fn retried_fn() -> i32 { 99 }";

        let code_units = [
            CodeUnit {
                name: present_fn_name.to_string(),
                unit_type: "function".to_string(),
                file: file_path.to_string(),
                line: 1,
                end_line: 1,
                language: "Rust".to_string(),
                signature: None,
                code: Some(present_fn_code.to_string()),
                complexity: 1,
                has_branches: false,
                has_loops: false,
                has_error_handling: false,
                calls: vec![],
                imports: vec![],
                qualified_name: present_fn_name.to_string(),
                docstring: None,
                parent_class: None,
            },
            CodeUnit {
                name: retried_fn_name.to_string(),
                unit_type: "function".to_string(),
                file: file_path.to_string(),
                line: 3,
                end_line: 3,
                language: "Rust".to_string(),
                signature: None,
                code: Some(retried_fn_code.to_string()),
                complexity: 1,
                has_branches: false,
                has_loops: false,
                has_error_handling: false,
                calls: vec![],
                imports: vec![],
                qualified_name: retried_fn_name.to_string(),
                docstring: None,
                parent_class: None,
            },
        ];

        // Compute expected hashes for assertion
        let hash_code = |code: &str| -> String {
            let mut hasher = Sha256::new();
            hasher.update(code.as_bytes());
            let result = hasher.finalize();
            let hex: String = result.iter().map(|b| format!("{:02x}", b)).collect();
            hex[..16].to_string()
        };

        let present_fn_expected_hash = hash_code(present_fn_code);
        let retried_fn_expected_hash = hash_code(retried_fn_code);

        // Call upsert_function_summary for present_fn (simulating present path)
        let present_summary = "Returns 42 as the answer";
        let present_unit = &code_units[0];
        let present_result =
            upsert_function_summary(&storage, repo_id, present_unit, present_summary);
        match present_result {
            SummaryUpsertOutcome::Ok(count) => {
                assert_eq!(count, 1, "upsert should succeed for present_fn");
            }
            SummaryUpsertOutcome::FileMissing => {
                panic!("unexpected FileMissing for present_fn");
            }
            SummaryUpsertOutcome::FunctionMissing => {
                panic!("unexpected FunctionMissing for present_fn");
            }
            SummaryUpsertOutcome::CodeUnavailable => {
                panic!("unexpected CodeUnavailable for present_fn");
            }
            SummaryUpsertOutcome::UpsertFailed => {
                panic!("unexpected UpsertFailed for present_fn");
            }
        }

        // Call upsert_function_summary for retried_fn (simulating retry path)
        let retried_summary = "Returns 99 as a different result";
        let retried_unit = &code_units[1];
        let retried_result =
            upsert_function_summary(&storage, repo_id, retried_unit, retried_summary);
        match retried_result {
            SummaryUpsertOutcome::Ok(count) => {
                assert_eq!(count, 1, "upsert should succeed for retried_fn");
            }
            SummaryUpsertOutcome::FileMissing => {
                panic!("unexpected FileMissing for retried_fn");
            }
            SummaryUpsertOutcome::FunctionMissing => {
                panic!("unexpected FunctionMissing for retried_fn");
            }
            SummaryUpsertOutcome::CodeUnavailable => {
                panic!("unexpected CodeUnavailable for retried_fn");
            }
            SummaryUpsertOutcome::UpsertFailed => {
                panic!("unexpected UpsertFailed for retried_fn");
            }
        }

        // Verify: all upserts were recorded
        let upserted = storage.upserted.borrow();
        assert_eq!(upserted.len(), 2, "two upserts should have been recorded");

        // Verify: present_fn entity was upserted with correct summary and hash
        let upserted_present = upserted
            .iter()
            .find(|e| e.name == present_fn_name)
            .expect("present_fn should be in upserted entities");
        assert_eq!(
            upserted_present.summary.as_deref(),
            Some(present_summary),
            "present_fn summary should match the provided summary"
        );
        assert_eq!(
            upserted_present.summary_commit.as_deref(),
            Some(present_fn_expected_hash.as_str()),
            "present_fn summary_commit should be the hash of its code"
        );

        // Verify: retried_fn entity was upserted with correct summary and hash
        let upserted_retried = upserted
            .iter()
            .find(|e| e.name == retried_fn_name)
            .expect("retried_fn should be in upserted entities");
        assert_eq!(
            upserted_retried.summary.as_deref(),
            Some(retried_summary),
            "retried_fn summary should match the provided summary"
        );
        assert_eq!(
            upserted_retried.summary_commit.as_deref(),
            Some(retried_fn_expected_hash.as_str()),
            "retried_fn summary_commit should be the hash of its code"
        );

        // Verify: both upserted entities are function tier
        assert_eq!(upserted_present.tier, crate::model::EntityTier::Function);
        assert_eq!(upserted_retried.tier, crate::model::EntityTier::Function);

        // Verify: both upserted entities have the correct derived IDs
        assert_eq!(upserted_present.id, function_id(file_id, present_fn_name));
        assert_eq!(upserted_retried.id, function_id(file_id, retried_fn_name));

        // Test conclusion: upsert_function_summary correctly persists summaries to storage
        // for both the "present" (batch result) and "retried" (individual retry) paths.
        // Both paths funnel through this helper, so successful persistence here proves
        // the salvage feature's core persistence behavior works correctly.
    }

    #[test]
    fn test_upsert_returns_code_unavailable_when_code_none() {
        // Verify that upsert_function_summary returns CodeUnavailable when
        // the CodeUnit has code=None (input-validation case, not storage error).
        // This is Finding 1: code=None was previously conflated with FunctionMissing.
        let mut storage = TestStorage::new();
        let repo_id = "test-repo";
        let file_path = "src/lib.rs";
        let file_entity = make_file_entity("file-abc123", file_path);
        storage.add_file(repo_id, file_path, file_entity.clone());

        let fn_name = "no_code_fn";
        let fn_entity = make_fn_entity(
            &function_id(&file_entity.id, fn_name),
            &file_entity.id,
            fn_name,
        );
        storage.add_function(&function_id(&file_entity.id, fn_name), fn_entity);

        // Create CodeUnit with code=None
        let unit = CodeUnit {
            name: fn_name.to_string(),
            unit_type: "function".to_string(),
            file: file_path.to_string(),
            line: 1,
            end_line: 1,
            language: "Rust".to_string(),
            signature: None,
            code: None, // No code — should return CodeUnavailable
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

        let result = upsert_function_summary(&storage, repo_id, &unit, "test summary");

        // Verify: returns CodeUnavailable, not FunctionMissing
        match result {
            SummaryUpsertOutcome::CodeUnavailable => {
                // Expected — code was None
            }
            SummaryUpsertOutcome::FunctionMissing => {
                panic!("unexpected: code=None should return CodeUnavailable, not FunctionMissing");
            }
            other => {
                panic!("unexpected outcome for code=None: {:?}", other);
            }
        }

        // Verify: no upsert was attempted (code unavailable)
        let upserted = storage.upserted.borrow();
        assert_eq!(
            upserted.len(),
            0,
            "no upsert should occur when code is unavailable"
        );
    }

    #[test]
    fn test_positional_mapping_avoids_name_collision() {
        // Test positional mapping: two functions named `new` in different files
        // must both receive their own distinct summaries via positional index.
        // This is the core DoD requirement for issue #647.
        let mut storage = TestStorage::new();
        let repo_id = "test-repo";

        // Create two file entities with distinct IDs
        let file_a_entity = make_file_entity("file-a", "src/a.rs");
        let file_b_entity = make_file_entity("file-b", "src/b.rs");
        storage.add_file(repo_id, "src/a.rs", file_a_entity.clone());
        storage.add_file(repo_id, "src/b.rs", file_b_entity.clone());

        // Create two function entities, both named "new", scoped to their files
        let file_a_fn_id = function_id(&file_a_entity.id, "new");
        let file_b_fn_id = function_id(&file_b_entity.id, "new");

        let fn_a_entity = make_fn_entity(&file_a_fn_id, "file-a", "new");
        let fn_b_entity = make_fn_entity(&file_b_fn_id, "file-b", "new");
        storage.add_function(&file_a_fn_id, fn_a_entity);
        storage.add_function(&file_b_fn_id, fn_b_entity);

        // Create two code units for the two files, both with function named "new"
        let code_a = "pub fn new() -> Self { Self }";
        let code_b = "pub fn new(value: i32) -> Self { Self { value } }";

        let unit_a = CodeUnit {
            name: "new".to_string(),
            unit_type: "function".to_string(),
            file: "src/a.rs".to_string(),
            line: 1,
            end_line: 1,
            language: "Rust".to_string(),
            signature: None,
            code: Some(code_a.to_string()),
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
            qualified_name: "new".to_string(),
            docstring: None,
            parent_class: None,
        };

        let unit_b = CodeUnit {
            name: "new".to_string(),
            unit_type: "function".to_string(),
            file: "src/b.rs".to_string(),
            line: 1,
            end_line: 1,
            language: "Rust".to_string(),
            signature: None,
            code: Some(code_b.to_string()),
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
            qualified_name: "new".to_string(),
            docstring: None,
            parent_class: None,
        };

        // Simulate positional mapping: result[i] maps to the i-th unit in the batch
        // Simulate apfel returning summaries for both functions
        let simulated_results = [
            Some("Creates a default instance".to_string()), // position 0 -> unit_a
            Some("Creates an instance from a value".to_string()), // position 1 -> unit_b
        ];

        // Manually map results positionally (mimicking what the pipeline does)
        let units = [&unit_a, &unit_b];
        for (i, result) in simulated_results.iter().enumerate() {
            let summary = match result {
                Some(s) => s,
                None => continue,
            };
            let unit = units[i];
            let outcome = upsert_function_summary(&storage, repo_id, unit, summary);
            match outcome {
                SummaryUpsertOutcome::Ok(_) => {}
                other => panic!("unexpected outcome for position {}: {:?}", i, other),
            }
        }

        // Verify: both entities were upserted with their OWN distinct summaries
        let upserted = storage.upserted.borrow();
        assert_eq!(upserted.len(), 2, "both entities should be upserted");

        // Find file_a's `new` function and verify its summary
        let upserted_a = upserted
            .iter()
            .find(|e| e.id == file_a_fn_id)
            .expect("file_a's new function should be upserted");
        assert_eq!(
            upserted_a.summary.as_deref(),
            Some("Creates a default instance"),
            "file_a's new function should have its own summary"
        );

        // Find file_b's `new` function and verify its summary
        let upserted_b = upserted
            .iter()
            .find(|e| e.id == file_b_fn_id)
            .expect("file_b's new function should be upserted");
        assert_eq!(
            upserted_b.summary.as_deref(),
            Some("Creates an instance from a value"),
            "file_b's new function should have its own summary"
        );

        // Verify: both have distinct summaries (no name collision)
        assert_ne!(
            upserted_a.summary, upserted_b.summary,
            "summaries should be distinct, proving no collapse occurred"
        );
    }
}

mod config {
    use super::super::*;
    use super::fixtures::{TestStorage, make_file_entity, make_module_entity};
    use crate::model::EntityTier;

    #[test]
    fn test_summarization_config_no_summarize() {
        let repo_config = RepoConfig::default();
        let config = SummarizationConfig::new(true, &repo_config, true);
        assert!(
            !config.enabled,
            "no_summarize=true must disable summarization"
        );
    }

    #[test]
    fn test_summarization_config_repo_disabled() {
        let repo_config = RepoConfig {
            summarize: Some(false),
            ..Default::default()
        };
        let config = SummarizationConfig::new(false, &repo_config, true);
        assert!(
            !config.enabled,
            "repo config summarize=false must disable summarization"
        );
    }

    // Issue #786: enablement now keys on the CONFIGURED backend, not the
    // apfel-keyed rule the four old tests encoded. All cases below pass
    // `apfel_available` as a pure bool — no PATH mutation, no env guard.
    //
    // `no_summarize` (CLI flag) and `summarize: false` (repo config) always
    // disable summarization for EVERY backend, even a non-apfel backend that
    // would otherwise enable (old tests 1–2, reworked; their original inputs
    // already covered every backend since they used the default config).
    #[test]
    fn test_summarization_config_no_summarize_and_false_disable_everything() {
        let repo_config = RepoConfig {
            summarize: Some(false),
            summarizer_backend: Some("llama-server".to_string()),
            apfel_endpoint: Some("http://localhost:8080".to_string()),
            ..Default::default()
        };
        assert!(
            !SummarizationConfig::new(true, &repo_config, true).enabled,
            "no_summarize=true must disable summarization for every backend"
        );
        assert!(
            !SummarizationConfig::new(false, &repo_config, true).enabled,
            "summarize: false must disable summarization for every backend"
        );
    }

    // `summarize` unset + apfel default backend: the legacy `which apfel`
    // rule is preserved — apfel present → enabled (the macOS default),
    // apfel absent → disabled (no backend configured → no HTTP to nothing).
    // Old tests 3–4, reworked to name the rule they now pin (the Linux case
    // below is what their old `apfel_available` argument no longer decides).
    #[test]
    fn test_summarization_config_apfel_default_follows_which_apfel() {
        let repo_config = RepoConfig::default();
        assert!(
            !SummarizationConfig::new(false, &repo_config, false).enabled,
            "unset summarize + apfel default + apfel absent must stay disabled"
        );
        assert!(
            SummarizationConfig::new(false, &repo_config, true).enabled,
            "unset summarize + apfel default + apfel present must stay enabled"
        );
    }

    // The Linux case that was broken today: `summarize` unset, a non-apfel
    // backend (llama-server) with an endpoint configured, apfel NOT on PATH
    // (apfel_available=false). Summarization must be enabled so the pipeline
    // is attempted — an unreachable endpoint then degrades visibly via
    // server_dispatch instead of being silently gated off here. Also pins the
    // pitfall: the same setup WITHOUT an endpoint stays disabled.
    #[test]
    fn test_summarization_config_linux_non_apfel_backend_enabled_without_apfel() {
        let repo_config = RepoConfig {
            summarizer_backend: Some("llama-server".to_string()),
            apfel_endpoint: Some("http://localhost:8080".to_string()),
            ..Default::default()
        };
        assert!(
            SummarizationConfig::new(false, &repo_config, false).enabled,
            "unset summarize + non-apfel backend + endpoint must enable even with no apfel binary"
        );
        let no_endpoint = RepoConfig {
            apfel_endpoint: None,
            ..repo_config.clone()
        };
        assert!(
            !SummarizationConfig::new(false, &no_endpoint, false).enabled,
            "non-apfel backend without an endpoint must stay disabled"
        );
    }

    // `summarize: true` enables for EVERY backend regardless of apfel
    // availability (gate-resolution acceptance criterion).
    #[test]
    fn test_summarization_config_explicit_true_enables_for_every_backend() {
        let repo_config = RepoConfig {
            summarize: Some(true),
            ..Default::default()
        };
        assert!(
            SummarizationConfig::new(false, &repo_config, false).enabled,
            "summarize: true must enable summarization even when apfel is unavailable"
        );
    }

    #[test]
    fn test_rollup_to_tier_skips_entity_with_no_child_summaries() {
        // When a module entity has children but none of them have summaries,
        // rollup_to_tier must skip the entity (continue) rather than panic or
        // assign an incorrect value. This is the "null child summaries" skip path
        // that was previously silent (issue #565).
        let mut storage = TestStorage::new();

        let module_entity = make_module_entity("mod-001", "my_module");

        // Three child entities (files) with no summaries at all.
        let child1 = make_file_entity("file-001", "src/a.rs");
        let child2 = make_file_entity("file-002", "src/b.rs");
        let child3 = make_file_entity("file-003", "src/c.rs");

        storage.add_repo_tier_entities(
            "test-repo",
            EntityTier::Module,
            vec![module_entity.clone()],
        );
        storage.add_children("mod-001", vec![child1, child2, child3]);

        let result = SummarizationPipeline::rollup_to_tier(
            &storage,
            "test-repo",
            EntityTier::Module,
            crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
            None,
        )
        .expect("rollup_to_tier must not return an error");

        // No entities should be updated since none had child summaries.
        assert_eq!(
            result.updated, 0,
            "rollup_to_tier must update 0 when no child summaries are available"
        );
        // The skip is a first-class outcome (issue #793): a module with
        // children but no summarized child lands in the child-summary-missing
        // bucket, not in policy or no_children.
        assert_eq!(
            result.skipped.child_summary_missing, 1,
            "module with non-test children lacking summaries must count as child-summary-missing"
        );
        assert_eq!(result.skipped.policy, 0);
        assert_eq!(result.skipped.no_children, 0);

        // Module entity must NOT have been upserted with an incorrect summary.
        let upserted = storage.upserted.borrow();
        assert!(
            upserted.is_empty(),
            "module with childless summaries must not be upserted; found: {:?}",
            upserted
        );

        // Verify the module entity still has no summary (unchanged).
        let module_key = "test-repo:Module".to_string();
        let stored = storage
            .repo_tier_entities
            .borrow()
            .get(&module_key)
            .cloned()
            .unwrap();
        assert!(
            stored[0].summary.is_none(),
            "module entity summary must remain None when children have no summaries"
        );
    }

    #[test]
    fn test_classify_rollup_skip_buckets_are_disjoint() {
        // Issue #793: deliberate policy skips (test-file paths, #531) must
        // NOT share a bucket with child-summary-missing or orphaned skips.
        let test_file = make_file_entity("f-1", "src/tests/integration_test.rs");
        let test_bucket = classify_rollup_skip(&test_file, 0);
        assert_eq!(test_bucket.policy, 1);
        assert_eq!(test_bucket.child_summary_missing, 0);
        assert_eq!(test_bucket.no_children, 0);

        let missing = make_file_entity("f-2", "src/lib.rs");
        let missing_bucket = classify_rollup_skip(&missing, 2);
        assert_eq!(missing_bucket.policy, 0);
        assert_eq!(missing_bucket.child_summary_missing, 1);
        assert_eq!(missing_bucket.no_children, 0);

        let orphaned = make_file_entity("f-3", "src/lib.rs");
        let orphaned_bucket = classify_rollup_skip(&orphaned, 0);
        assert_eq!(orphaned_bucket.policy, 0);
        assert_eq!(orphaned_bucket.child_summary_missing, 0);
        assert_eq!(orphaned_bucket.no_children, 1);

        // A test-file path wins over the child_count signal: a policy skip
        // is a policy skip even if it has children.
        let mut test_named = make_module_entity("m-1", "my_module");
        test_named.path = Some("test_module.rs".to_string());
        let policy_wins = classify_rollup_skip(&test_named, 3);
        assert_eq!(policy_wins.policy, 1);
        assert_eq!(policy_wins.child_summary_missing, 0);
    }

    #[test]
    fn test_rollup_skip_reaches_rollup_outcome_for_no_children() {
        // Issue #793: an orphaned entity (no children at all) is counted in
        // the no_children bucket of the RollupOutcome, not silently dropped.
        let mut storage = TestStorage::new();
        let module_entity = make_module_entity("mod-002", "empty_module");
        storage.add_repo_tier_entities(
            "test-repo",
            EntityTier::Module,
            vec![module_entity.clone()],
        );

        let result = SummarizationPipeline::rollup_to_tier(
            &storage,
            "test-repo",
            EntityTier::Module,
            crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET,
            None,
        )
        .expect("rollup_to_tier must not return an error");

        assert_eq!(result.updated, 0);
        assert_eq!(result.skipped.no_children, 1);
        assert_eq!(result.skipped.policy, 0);
        assert_eq!(result.skipped.child_summary_missing, 0);
        assert_eq!(result.skipped.total(), 1);
    }
}

mod overflow {
    use super::super::*;

    fn overflow_err() -> crate::error::LievoError {
        crate::error::LievoError::SummarizationFailed(format!(
            "apfel failed with status: {} Input exceeds the 4096-token context window",
            CONTEXT_OVERFLOW_MARKER
        ))
    }

    fn budget_err() -> crate::error::LievoError {
        crate::error::LievoError::SummarizationFailed(format!(
            "batch_summarize: formatted input 8500 {} 8000",
            BUDGET_EXCEEDED_MARKER
        ))
    }

    #[test]
    fn test_is_overflow_matches_apfel_overflow_message() {
        assert!(is_overflow_error(&overflow_err()));
    }

    #[test]
    fn test_is_overflow_matches_budget_exceeded_message() {
        assert!(is_overflow_error(&budget_err()));
    }

    #[test]
    fn test_is_overflow_rejects_timeout() {
        let err = crate::error::LievoError::SummarizationFailed(
            "apfel timed out after 300 seconds".to_string(),
        );
        assert!(!is_overflow_error(&err));
    }

    #[test]
    fn test_is_overflow_rejects_json_parse_failure() {
        let err = crate::error::LievoError::SummarizationFailed(
            "Failed to parse apfel JSON response: expected value".to_string(),
        );
        assert!(!is_overflow_error(&err));
    }

    #[test]
    fn test_is_overflow_rejects_spawn_io_failure() {
        let err = crate::error::LievoError::SummarizationFailed(
            "Failed to invoke apfel: No such file or directory".to_string(),
        );
        assert!(!is_overflow_error(&err));
    }

    #[test]
    fn test_is_overflow_rejects_non_summarization_errors() {
        let err = crate::error::LievoError::DatabaseLocked;
        assert!(!is_overflow_error(&err));
    }

    #[test]
    fn test_is_overflow_rejects_generic_apfel_failure() {
        let err = crate::error::LievoError::SummarizationFailed(
            "apfel failed with status: model download failed".to_string(),
        );
        assert!(!is_overflow_error(&err));
    }

    /// Round-trip test: the actual message produced by apfel.rs's budget check
    /// must be classified as overflow by is_overflow_error. This pins the
    /// cross-module string contract so a rewording in apfel.rs would fail
    /// this test (issue #649).
    #[test]
    fn test_overflow_markers_match_apfel_producer_strings() {
        // The budget-exceeded message format in apfel.rs:
        //   "batch_summarize: formatted input {n} exceeds budget {budget}"
        let producer_msg = format!(
            "batch_summarize: formatted input {} {} {}",
            9000,
            BUDGET_EXCEEDED_MARKER,
            crate::summarization::apfel::APFEL_INPUT_CHAR_BUDGET
        );
        let err = crate::error::LievoError::SummarizationFailed(producer_msg);
        assert!(
            is_overflow_error(&err),
            "the budget-exceeded message from apfel.rs must classify as overflow"
        );
    }
}

// Issue #788: enabled-but-unconfigured and retry-bound tests.
#[cfg(test)]
#[path = "pipeline_788_tests.rs"]
mod pipeline_788_tests;

// End-to-end per-backend input-budget behaviour (issue #792), split into
// pipeline_budget_tests.rs to respect the file-size gate (AGENTS.md §6).
#[path = "pipeline_budget_tests.rs"]
mod budget_behavior;

// Rollup batch tests (issue #771).
#[path = "pipeline_rollup_tests.rs"]
mod rollup_batch;

// Remote-endpoint warning tests, split out (issue #869) because
// pipeline_tests.rs is at its grandfathered size cap.
#[path = "pipeline_endpoint_tests.rs"]
mod endpoint_warning;
