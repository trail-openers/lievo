// End-to-end between-budgets behaviour test (issue #792).
//
// Split out of pipeline_tests.rs to respect the file-size gate (AGENTS.md §6):
// a dedicated test file for the per-backend input-budget behavioural test,
// driven through the real production entry point `SummarizationPipeline::run`.

/// End-to-end between-budgets behaviour through the real production entry
/// point (issue #792): a function whose code exceeds apfel's 8,000-char
/// budget but fits llama-server's 24,000-char budget is summarized when
/// `run()` dispatches against a `llama-server` endpoint, and skipped
/// cleanly (`skipped_oversized`) when it dispatches against an `apfel`
/// endpoint.
///
/// Both legs go through the production `run()` path — `acquire_for_run`
/// → health probe → explicit transport value → `resolve_input_char_budget`
/// → packer + guards — with no ambient-state shortcut (issue #783), so the
/// test locks the budget to the backend the run actually resolved, not to an
/// assumption about which transport is in force.
#[test]
fn test_run_between_budgets_per_backend_end_to_end() {
    use crate::config::RepoConfig;
    use crate::extraction::function_preservation::function_id;
    use crate::model::EntityTier;
    use crate::summarization::backend_profile::{
        APFEL_INPUT_CHAR_BUDGET, HTTP_BACKEND_INPUT_CHAR_BUDGET,
    };
    use crate::summarization::pipeline::{SummarizationConfig, SummarizationPipeline};
    use crate::summarization::pipeline_tests_fixtures as fixtures;
    use crate::summarization::summarizer_backend::start_fake_http_server;
    use crate::summarization::summarizer_fullpath_tests::env_guard;
    use fixtures::{CodeUnit, TestStorage, make_file_entity, make_fn_entity};

    // The apfel-backend leg is spawned and health-checked via PATH lookup;
    // serialize against PATH-mutating tests (issue #786 guard).
    let _env_guard = env_guard();

    // Between the two per-backend budgets: > apfel (8,000), <
    // llama-server/generic (24,000). The guards compare the FORMATTED
    // input (user prompt + "\n\n" + code), so the code alone must clear
    // apfel's budget after that formatting overhead is added.
    let between_code = "// between-budgets padding\n".repeat(500); // ~12,000 chars
    assert!(
        between_code.len() > APFEL_INPUT_CHAR_BUDGET,
        "fixture must exceed apfel's budget"
    );
    assert!(
        between_code.len() < HTTP_BACKEND_INPUT_CHAR_BUDGET,
        "fixture must fit the larger backend's budget"
    );

    let fn_name = "between_budgets_fn";
    let file_path = "src/between.rs";
    let build_storage = || {
        let mut storage = TestStorage::new();
        let file_entity = make_file_entity("file-between-e2e", file_path);
        storage.add_file("test-repo", file_path, file_entity.clone());
        let fn_entity = make_fn_entity(
            &function_id(&file_entity.id, fn_name),
            &file_entity.id,
            fn_name,
        );
        storage.add_function(&function_id(&file_entity.id, fn_name), fn_entity);
        // No parent-child rollup edges: the post-function rollup tiers
        // see an empty tier listing and exit without touching a budget.
        storage.add_children(&file_entity.id, vec![]);
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

    // Leg 1: llama-server configured → run() resolves the 24,000 budget →
    // the between-budgets function is summarized, not skipped.
    {
        // 200 with a valid body for both probe shapes: llama-server
        // health (`/health` → `{"status":"ok"}`) and chat completions.
        let health_and_chat = serde_json::json!({
            "status": "ok",
            "choices": [{ "message": { "content": "#0: between-budgets summary", "role": "assistant" } }]
        })
        .to_string();
        let addr = start_fake_http_server(200, &health_and_chat).expect("bind llama-server fake");
        let repo_config = RepoConfig {
            apfel_endpoint: Some(format!("http://127.0.0.1:{}", addr.port())),
            summarizer_backend: Some("llama-server".to_string()),
            ..Default::default()
        };
        let storage = build_storage();
        let outcome = SummarizationPipeline::run(
            &storage,
            "test-repo",
            "test-project",
            std::slice::from_ref(&unit),
            &SummarizationConfig::new(false, &repo_config, true),
            &repo_config,
        )
        .expect("run() must not hard-error under llama-server");
        assert_eq!(
            outcome.skipped_oversized, 0,
            "between-budgets input must NOT be skipped under llama-server"
        );
        let upserted = storage.upserted.borrow();
        let fn_upserts: Vec<_> = upserted
            .iter()
            .filter(|e| e.tier == EntityTier::Function)
            .collect();
        assert_eq!(
            fn_upserts.len(),
            1,
            "the between-budgets function must be upserted with a summary under llama-server"
        );
        assert!(
            fn_upserts[0].summary.is_some(),
            "the upserted function entity must carry a summary"
        );
    }

    // Leg 2: apfel configured → run() resolves the 8,000 budget → the
    // same input is rejected by the offline guard and tracked as
    // skipped_oversized — summarized under llama-server, skipped under
    // apfel, the exact behaviour the issue exists for.
    {
        // Valid apfel `/health` shape so the run dispatches to the HTTP
        // transport (the guard then rejects offline); chat body is never
        // reached under apfel because the batch is over the 8,000 budget.
        let apfel_health = r#"{"prewarmed":true,"active_requests":0,"context_window":8192,"model_available":true}"#;
        let addr = start_fake_http_server(200, apfel_health).expect("bind apfel fake");
        let repo_config = RepoConfig {
            apfel_endpoint: Some(format!("http://127.0.0.1:{}", addr.port())),
            summarizer_backend: Some("apfel".to_string()),
            ..Default::default()
        };
        let storage = build_storage();
        let outcome = SummarizationPipeline::run(
            &storage,
            "test-repo",
            "test-project",
            std::slice::from_ref(&unit),
            &SummarizationConfig::new(false, &repo_config, true),
            &repo_config,
        )
        .expect("run() must not hard-error under apfel");
        assert_eq!(
            outcome.skipped_oversized, 1,
            "between-budgets input must be counted as skipped_oversized under apfel"
        );
        assert!(
            storage.upserted.borrow().is_empty(),
            "the between-budgets function must NOT be upserted under apfel"
        );
    }
}
