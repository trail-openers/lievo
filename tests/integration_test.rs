// End-to-end integration test for the full analyze + query cycle.
//
// Hermetic by construction (#20, after the #809 decision): the test always
// analyzes the small in-tree fixture (tests/fixtures/sample_repo), copied into
// a git-initialized temp dir and dropped on cleanup. Summarization and the
// semantic index are both off (no_summarize: true, skip_semantic_index: true),
// so the test never runs a summarizer and never downloads the embedding
// model, regardless of what is installed on the host (e.g. apfel on PATH).
// It uses in-memory storage. A before/after entry-set probe verifies the
// only side effect on the developer's real ~/.lievo/indices is the empty
// ts-index dir skeleton (cleaned up by the test itself).
//
// Decision (#809): this test RUNS in the main CI job (blocking, every push/PR)
// and is cheap (seconds, not minutes).
use std::collections::BTreeSet;
use std::path::PathBuf;

use lievo::analysis::pipeline::{AnalysisPipeline, PipelineConfig, ReindexMode};
use lievo::query::{dependency, entity_queries};
use lievo::storage::Storage;
use lievo::storage::sqlite::SqliteStorage;

/// Resolve the developer's real ~/.lievo/indices directory, if a home dir exists.
fn lievo_indices_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".lievo").join("indices"))
}

/// Entry set under the indices directory (empty if absent — the test never
/// *creates* the directory itself, only observes entries in it).
fn indices_entries(indices_dir: &std::path::Path) -> BTreeSet<String> {
    std::fs::read_dir(indices_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

/// Copy tests/fixtures/sample_repo into a fresh temp dir and git-init it (the
/// in-tree copy has no .git; analysis needs a real committed working tree for
/// head_commit tracking). Returns a TempDir so cleanup happens on drop.
fn prepare_fixture_repo() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let src = PathBuf::from("tests/fixtures/sample_repo");
    let tmp = tempfile::TempDir::new()?;
    let dst = tmp.path().to_path_buf();

    // Recursive copy of the fixture.
    fn copy_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let target = dst.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                std::fs::create_dir_all(&target)?;
                copy_recursive(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), &target)?;
            }
        }
        Ok(())
    }
    copy_recursive(&src, &dst).map_err(|e| format!("failed to copy fixture: {e}"))?;

    // git2: init + single commit on refs/heads/main (matches the CI step).
    let git = git2::Repository::init(&dst).map_err(|e| format!("git init on fixture copy: {e}"))?;
    // `git2::Repository::init` may default HEAD to `refs/heads/master`
    // regardless of init.defaultBranch; pin it to main before the commit
    // (see tests/admin_selfcheck_test.rs for the same pin, issue #715).
    git.set_head("refs/heads/main")
        .map_err(|e| format!("point HEAD at refs/heads/main: {e}"))?;
    let mut index = git.index().map_err(|e| format!("open git index: {e}"))?;
    index
        .add_all(["**"], git2::IndexAddOption::DEFAULT, None)
        .map_err(|e| format!("stage fixture files: {e}"))?;
    index.write().map_err(|e| format!("write git index: {e}"))?;
    let tree_oid = index.write_tree().map_err(|e| format!("write tree: {e}"))?;
    let tree = git
        .find_tree(tree_oid)
        .map_err(|e| format!("find tree: {e}"))?;
    let sig =
        git2::Signature::now("lievo-test", "lievo@test").map_err(|e| format!("signature: {e}"))?;
    git.commit(Some("refs/heads/main"), &sig, &sig, "fixture", &tree, &[])
        .map_err(|e| format!("commit fixture: {e}"))?;

    Ok(tmp)
}

#[test]
fn test_full_analyze_and_query_cycle() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Setup: in-memory storage (never touches ~/.lievo/lievo.db)
    let storage = SqliteStorage::open_in_memory()?;

    // 2. Create project
    let project = storage.create_project("test-project", Some("Integration test"))?;

    // 3. Prepare the pinned fixture repo (temp dir + git init, dropped on cleanup).
    let fixture_dir = prepare_fixture_repo()?;
    let repo_path = fixture_dir.path().to_path_buf();

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

    // Probe (before): entry set of the developer's real ~/.lievo/indices. The
    // test must not grow this set — any new entry after the analyze call is a
    // regression in hermeticity (e.g. ts_index_dir_for_repo writing to home).
    // Note: `skip_semantic_index: true` does NOT stop the ts-index dir —
    // TreeSitterExtractor::index unconditionally runs
    // fs::create_dir_all(ts_index_dir_for_repo(...)). We assert the new hash
    // directory is empty after the test (no files written), and then clean
    // it up so the probe stays hermetic for re-runs.
    let indices_dir = lievo_indices_dir();
    let entries_before: BTreeSet<String> = indices_dir
        .as_deref()
        .map(indices_entries)
        .unwrap_or_default();

    let result = AnalysisPipeline::run_repo(&storage, &repos[0], &config)?;
    assert!(result.is_some(), "analysis should produce a run record");
    let run = result.expect("analysis run should exist");

    // Probe (after): if a new entry appeared under ~/.lievo/indices, it must
    // be only the ts-index dir skeleton (an empty `ts-index` subdirectory with
    // no files inside — skip_semantic_index: true means no semantic index is
    // built, so no files are written). Any file written inside is a
    // regression; assert and clean up the skeleton so re-runs stay hermetic.
    let entries_after = indices_dir
        .as_deref()
        .map(indices_entries)
        .unwrap_or_default();
    let new_entries: Vec<String> = entries_after.difference(&entries_before).cloned().collect();
    if let Some(indices_dir) = indices_dir.as_ref()
        && !new_entries.is_empty()
    {
        for entry in &new_entries {
            let subdir = indices_dir.join(entry);
            // Expected shape: subdir/ts-index (an empty directory skeleton).
            // No files at any level, no unexpected sibling directories.
            let expected_skeleton = subdir.join("ts-index");
            let skeleton_exists = expected_skeleton.is_dir();
            let skeleton_is_empty = skeleton_exists
                && std::fs::read_dir(&expected_skeleton)
                    .map(|entries| entries.count() == 0)
                    .unwrap_or(false);
            assert!(
                skeleton_exists && skeleton_is_empty,
                "new entry {:?} under ~/.lievo/indices must be only the empty ts-index skeleton, \
                 got: exists={}, empty={}",
                subdir,
                skeleton_exists,
                skeleton_is_empty
            );
            // Clean up the skeleton so the developer's ~/.lievo stays pristine.
            let _ = std::fs::remove_dir_all(&subdir);
        }
    }

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
