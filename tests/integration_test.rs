// End-to-end integration test for the full analyze + query cycle.
//
// Decision (#809): this test RUNS in the main CI job (blocking, every push/PR).
// It is in-memory (SqliteStorage::open_in_memory), network-free (no ONNX model
// download — embedding-model download only fires when the vector index is stale),
// and cheap. The repo path defaults to the current directory, which in CI is the
// lievo checkout itself (~30 s of analysis). Set LIEVO_TEST_REPO to a different
// git repo to analyze a different target.
use lievo::analysis::pipeline::{AnalysisPipeline, PipelineConfig, ReindexMode};
use lievo::query::{dependency, entity_queries};
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;

#[test]
fn test_full_analyze_and_query_cycle() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Setup: in-memory storage (never touches ~/.lievo/lievo.db)
    let storage = SqliteStorage::open_in_memory()?;

    // 2. Create project
    let project = storage.create_project("test-project", Some("Integration test"))?;

    // 3. Register the lievo repo itself as the target
    let repo_path = match std::env::var("LIEVO_TEST_REPO") {
        Ok(p) => std::path::PathBuf::from(p),
        Err(_) => std::env::current_dir()?,
    };

    let repo = storage.add_repo(
        &project.id,
        "lievo",
        repo_path.to_str().ok_or("repo path is not valid UTF-8")?,
    )?;
    let _ = repo; // used indirectly via list_repos below

    // 4. Run analysis pipeline
    let repos = storage.list_repos(&project.id)?;
    assert_eq!(repos.len(), 1, "exactly one repo registered");

    let config = PipelineConfig {
        respect_ignore: true,
        reindex: ReindexMode::Incremental,
        no_summarize: false,
        skip_semantic_index: false,
    };
    let result = AnalysisPipeline::run_repo(&storage, &repos[0], &config)?;
    assert!(result.is_some(), "analysis should produce a run record");
    let run = result.expect("analysis run should exist");
    assert!(
        run.entities_upserted >= 5,
        "analysis should extract at least 5 entities from lievo's codebase, got {}",
        run.entities_upserted
    );

    // 5. Query: subsystems — lievo has multiple top-level modules (storage, analysis, query, …)
    let subsystems = entity_queries::subsystems(&storage, &project.id)?;
    assert!(
        subsystems.len() >= 3,
        "lievo codebase should have at least 3 subsystems, got {}",
        subsystems.len()
    );

    // Test dependency queries — subsystems are valid entity IDs in storage
    let deps = dependency::dependencies_of(&storage, &subsystems[0].id)?;
    // deps may be empty for some subsystems, but the query must not error
    let _ = deps;

    // 6. Query: at least one subsystem should contain modules
    let has_modules = subsystems.iter().any(|s| {
        !entity_queries::modules_in(&storage, &s.id)
            .expect("modules_in should succeed")
            .is_empty()
    });
    assert!(has_modules, "at least one subsystem should contain modules");

    // Count total modules across all subsystems
    let total_modules: usize = subsystems
        .iter()
        .map(|s| {
            entity_queries::modules_in(&storage, &s.id)
                .expect("modules_in should succeed")
                .len()
        })
        .sum();
    assert!(
        total_modules >= 5,
        "lievo codebase should have at least 5 modules total, got {}",
        total_modules
    );

    // 7. Query: hotspots sorted by complexity descending
    let hotspots = entity_queries::hotspots(&storage, &project.id, 5)?;
    assert!(!hotspots.is_empty(), "should find at least one hotspot");

    // Verify hotspots are ordered by complexity descending.
    // Parse metrics_json inline; complexity_of is pub(crate) and not accessible here.
    let complexity_of_entity = |e: &lievo::model::Entity| -> f64 {
        e.metrics_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("complexity_max").and_then(|c| c.as_f64()))
            .unwrap_or(0.0)
    };
    let complexities: Vec<f64> = hotspots.iter().map(complexity_of_entity).collect();
    for window in complexities.windows(2) {
        assert!(
            window[0] >= window[1],
            "hotspots must be sorted by complexity descending, got {:?}",
            complexities
        );
    }

    // 8. Verify relationships were built
    assert!(
        run.relationships_upserted >= 10,
        "lievo codebase should produce at least 10 relationships, got {}",
        run.relationships_upserted
    );

    // Spot-check: at least one entity should have outgoing relationships
    let all_entities = storage.list_entities(&project.id, None)?;
    let has_relationships = all_entities.iter().any(|e| {
        !storage
            .relationships_from(&e.id)
            .expect("relationships_from should succeed")
            .is_empty()
    });
    assert!(
        has_relationships,
        "at least one entity should have outgoing relationships"
    );

    // 9. Verify repo commit tracking was updated
    let updated_repo = storage
        .get_repo(&repos[0].id)?
        .ok_or("repo should still exist after analysis")?;
    assert!(
        updated_repo.last_analyzed_commit.is_some(),
        "last_analyzed_commit should be set after analysis"
    );

    Ok(())
}
