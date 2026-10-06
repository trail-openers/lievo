// End-to-end integration test for the full analyze + query cycle.
//
// Hermetic by construction (#20, after the #809 decision): the test always
// analyzes the small in-tree fixture (tests/fixtures/sample_repo), copied into
// a git-initialized temp dir and dropped on cleanup. Summarization and the
// semantic index are both off (no_summarize: true, skip_semantic_index: true),
// so the test never runs a summarizer and never downloads the embedding
// model, regardless of what is installed on the host (e.g. apfel on PATH).
// It uses in-memory storage. A scoped probe verifies the only side effect on
// the developer's real ~/.lievo/indices is the empty ts-index dir skeleton
// for the fixture's own hash (cleaned up by the test itself); no other entry
// under ~/.lievo/indices is read, asserted on, or touched.
//
// Decision (#809): this test RUNS in the main CI job (blocking, every push/PR)
// and is cheap (seconds, not minutes).
use lievo::analysis::pipeline::{AnalysisPipeline, PipelineConfig, ReindexMode};
use lievo::extraction::ts_index_dir_for_repo;
use lievo::query::{dependency, entity_queries};
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;

pub mod common;

#[test]
fn test_full_analyze_and_query_cycle() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Setup: in-memory storage (never touches ~/.lievo/lievo.db)
    let storage = SqliteStorage::open_in_memory()?;

    // 2. Create project
    let project = storage.create_project("test-project", Some("Integration test"))?;

    // 3. Prepare the pinned fixture repo (temp dir + git init, dropped on cleanup).
    let fixture = common::prepare_fixture_repo()?;
    let repo_path = fixture.path().to_path_buf();

    let repo = storage.add_repo(
        &project.id,
        "fixture",
        repo_path.to_str().ok_or("repo path is not valid UTF-8")?,
    )?;
    let _ = repo; // used indirectly via list_repos below

    // 4. Run analysis pipeline
    let repos = storage.list_repos(&project.id)?;
    assert_eq!(repos.len(), 1, "exactly one repo registered");

    let config = PipelineConfig {
        respect_ignore: true,
        reindex: ReindexMode::Incremental,
        no_summarize: true,
        skip_semantic_index: true,
    };

    // Scoped probe: the pipeline unconditionally creates
    // ~/.lievo/indices/<fixture-hash>/ts-index (TreeSitterExtractor::index
    // runs fs::create_dir_all on ts_index_dir_for_repo), so we verify that
    // hash dir is the only side effect and clean it up. The temp path is new,
    // so the hash dir must not already exist; if it does, something is wrong
    // and we fail rather than touching it.
    let ts_index_dir =
        ts_index_dir_for_repo(&repo_path).map_err(|e| format!("ts_index_dir_for_repo: {e}"))?;
    let hash_dir = ts_index_dir
        .parent()
        .ok_or("ts-index dir has no parent (unexpected)")?;

    if hash_dir.exists() {
        return Err(format!(
            "hash dir {hash_dir:?} already exists before the analyze call; \
             the temp fixture path should be new — refusing to touch it"
        )
        .into());
    }

    let result = AnalysisPipeline::run_repo(&storage, &repos[0], &config)?;
    assert!(result.is_some(), "analysis should produce a run record");
    let run = result.expect("analysis run should exist");

    // Post-run probe (#20): run_repo always creates the hash dir
    // (TreeSitterExtractor::index runs fs::create_dir_all on
    // ts_index_dir_for_repo), so the "must not pre-exist" guard above is
    // paired here with an explicit existence assert. With
    // skip_semantic_index: true the dir must contain only an empty
    // `ts-index` directory; delete exactly that dir.
    assert!(
        hash_dir.is_dir(),
        "hash dir {hash_dir:?} must exist after run_repo (issue #20)"
    );
    let mut entries = std::fs::read_dir(hash_dir)
        .expect("read fixture hash dir under ~/.lievo/indices")
        .map(|e| e.expect("hash dir entry").file_name())
        .collect::<Vec<_>>();
    entries.sort();
    assert_eq!(
        entries,
        vec!["ts-index"],
        "hash dir {hash_dir:?} must contain only the ts-index directory, got: {entries:?}"
    );
    assert!(
        ts_index_dir.is_dir(),
        "expected ts-index at {ts_index_dir:?} to be a directory"
    );
    let ts_index_entries = std::fs::read_dir(&ts_index_dir)
        .expect("read ts-index dir")
        .count();
    assert_eq!(
        ts_index_entries, 0,
        "ts-index dir {ts_index_dir:?} must be empty (skip_semantic_index: true writes no files)"
    );
    std::fs::remove_dir_all(hash_dir)
        .map_err(|e| format!("remove fixture hash dir {hash_dir:?} under ~/.lievo/indices: {e}"))?;

    assert!(
        run.entities_upserted >= 5,
        "analysis should extract at least 5 entities from the fixture, got {}",
        run.entities_upserted
    );

    // 5. Query: subsystems — the fixture spans src/, treeA/, treeB/ top-level dirs
    let subsystems = entity_queries::subsystems(&storage, &project.id)?;
    assert!(
        subsystems.len() >= 3,
        "fixture should have at least 3 subsystems, got {}",
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
        "fixture should have at least 5 modules total, got {}",
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
        "fixture should produce at least 10 relationships, got {}",
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
