// Pipeline step execution logic for AnalysisPipeline.

mod metrics;

#[path = "pipeline_cleanup.rs"]
mod pipeline_cleanup;
use pipeline_cleanup::apply_pre_extraction_cleanup;

use crate::analysis::file_hashing;
use crate::analysis::flow_tracer::FlowTracer;
use crate::extraction::code_extractor::CodeExtractor;
use crate::extraction::grouping::{extract_function_entities, group_code_units};
use crate::model::CodeUnit;
use crate::storage::Storage;
use std::path::Path;
use std::time::Instant;

/// Internal struct carrying pipeline step data for the finalization phase.
pub(crate) struct PipelineStepData {
    code_units: Vec<CodeUnit>,
    grouping: crate::extraction::grouping::GroupingResult,
    /// True if summarization should be skipped (CLI flag OR config file)
    no_summarize: bool,
    /// The loaded repo config, used to pass summarize setting to SummarizationConfig
    repo_config: crate::config::RepoConfig,
}

/// Execute pipeline steps, mutating `run` with final statistics.
/// Returns `(code_units, subsystem_entities, summarization_failed)` on success.
/// The `summarization_failed` flag is true when summarization was enabled but
/// the pipeline produced no summaries (issue #788: the resume path uses this
/// to skip re-entering the full pipeline).
pub(crate) fn run_pipeline_steps(
    storage: &dyn Storage,
    repo: &crate::model::Repository,
    repo_path: &Path,
    head: &str,
    start: &Instant,
    run: &mut crate::model::AnalysisRun,
    config: &crate::analysis::pipeline::PipelineConfig,
) -> crate::Result<(Vec<CodeUnit>, Vec<crate::model::Entity>, bool)> {
    let mut extractor: Box<dyn CodeExtractor> = Box::new(
        crate::extraction::tree_sitter_extractor::TreeSitterExtractor::new(
            repo_path,
            config.respect_ignore,
        )?,
    );
    let force = match config.reindex {
        crate::analysis::pipeline::ReindexMode::Force => true,
        crate::analysis::pipeline::ReindexMode::Full => true,
        crate::analysis::pipeline::ReindexMode::Incremental => false,
    };

    // Load file hashes for incremental reindexing
    let all_hashes = storage.get_all_file_hashes(&repo.id).unwrap_or_default();
    let known_hash_keys: std::collections::HashSet<_> = all_hashes.keys().cloned().collect();
    let known_hashes = if force {
        std::collections::HashMap::new()
    } else {
        all_hashes
    };

    // Pass known hashes to extractor for incremental reindexing
    if let Some(ts_extractor) = extractor
        .as_mut()
        .as_any_mut()
        .downcast_mut::<crate::extraction::tree_sitter_extractor::TreeSitterExtractor>(
    ) {
        ts_extractor.set_known_hashes(known_hashes);
    }

    extractor.index(force).map_err(|e| {
        eprintln!("  [{}] indexing failed: {e}", repo.name);
        e
    })?;

    // Get incremental reindexing stats
    let files_parsed_count;
    let file_set_changed;
    if let Some(ts_extractor) = extractor
        .as_any()
        .downcast_ref::<crate::extraction::tree_sitter_extractor::TreeSitterExtractor>(
    ) {
        files_parsed_count = ts_extractor.files_parsed().len();
        file_set_changed = ts_extractor.file_set_changed();
    } else {
        files_parsed_count = 0;
        file_set_changed = true;
    }

    let code_units = extractor.read_all_units()?;
    eprintln!(
        "  [{}] indexed {} files ({} extracted), {} code units",
        repo.name,
        files_parsed_count,
        code_units.len(),
        code_units.len()
    );

    // Compute vector search index path for later use (after persistence)
    let vector_index_path = extractor
        .index_dir()
        .map(|d| d.join("vectors.usearch"))
        .ok_or_else(|| crate::LievoError::RetrievalError("No index dir for vector index".into()))?;

    // Store new file hashes and clean up deleted files
    if let Some(ts_extractor) = extractor
        .as_mut()
        .as_any_mut()
        .downcast_mut::<crate::extraction::tree_sitter_extractor::TreeSitterExtractor>(
    ) {
        let new_hashes = ts_extractor.take_new_hashes();
        let new_hash_paths: std::collections::HashSet<_> = new_hashes.keys().cloned().collect();

        // Upsert new/changed file hashes
        for (file_path, content_hash) in new_hashes {
            storage.upsert_file_hash(&repo.id, &file_path, &content_hash)?;
        }

        // Delete hashes for files that no longer exist
        for deleted_path in known_hash_keys.difference(&new_hash_paths) {
            eprintln!(
                "  [{}] cleaning up hash for deleted file: {}",
                repo.name, deleted_path
            );
            // Intentionally ignored: best-effort cleanup, .ok() already converts to Option
            let _ = storage.delete_file_hash(&repo.id, deleted_path).ok();
        }
    }

    let repo_config = crate::config::RepoConfig::load(repo_path)?;

    let from_commit = repo.last_analyzed_commit.as_deref();
    let changed = crate::analysis::incremental::changed_files(repo_path, from_commit, head)?;
    run.files_changed = changed.len() as i64;

    // Read output directories to exclude from entity creation
    let exclude_paths = storage
        .get_output_dirs(&repo.project_id)
        .unwrap_or_default();

    // Delete old entities from excluded paths (full or force analysis only)
    if matches!(
        config.reindex,
        crate::analysis::pipeline::ReindexMode::Full
            | crate::analysis::pipeline::ReindexMode::Force
    ) {
        let deleted = storage.delete_entities_by_paths(&repo.id, &exclude_paths)?;
        if deleted > 0 {
            eprintln!(
                "  [{}] cleaned up {} old entities from excluded paths",
                repo.name, deleted
            );
        }
    }

    // Apply Full-mode cleanup policies
    apply_pre_extraction_cleanup(config, repo, storage)?;
    eprintln!("  [{}] applied Full-mode cleanup", repo.name);

    // Every file the tree-sitter extractor scanned (including zero-unit
    // files) so grouping can seed file entities for them (issue #701).
    let scanned_file_paths: &[String] = extractor
        .as_ref()
        .as_any()
        .downcast_ref::<crate::extraction::tree_sitter_extractor::TreeSitterExtractor>()
        .map(|ts| ts.extracted_files())
        .unwrap_or_default();

    let grouping_config = crate::extraction::grouping::GroupingConfig {
        code_units: &code_units,
        scanned_file_paths,
        project_id: &repo.project_id,
        repo_name: &repo.name,
        repo_id: &repo.id,
        repo_path,
        config: repo_config.as_ref(),
        exclude_paths: &exclude_paths,
    };
    let grouping = group_code_units(&grouping_config)?;

    // Compute and store file content hashes for incremental detection
    file_hashing::store_file_hashes(storage, repo, repo_path, &grouping)?;

    // Merge CLI --no-summarize flag with repo config summarize: false setting
    let repo_disables_summarize = repo_config
        .as_ref()
        .and_then(|c| c.summarize)
        .map(|v| !v)
        .unwrap_or(false);
    let step_data = PipelineStepData {
        code_units,
        grouping,
        no_summarize: config.no_summarize || repo_disables_summarize,
        repo_config: repo_config.unwrap_or_default(),
    };

    let (code_units, subsystems, summarization_failed) =
        finish_pipeline_steps(storage, repo, head, start, run, step_data)?;

    // Build vector search index AFTER persistence (best-effort, failure doesn't abort).
    // Rebuild when files changed, --force was passed, or the on-disk vector index
    // is stale (built with a different embedding model / missing model-id marker).
    // The staleness check is cheap — it only reads the `.usearch.meta.json` marker
    // and never touches the model or the index — so it runs every refresh.
    let vector_index_stale =
        !crate::retrieval::model_cache::index_staleness_marker_matches(&vector_index_path);
    // The automatic index (issue #864) builds the core index only — the
    // semantic/vector index (and the embedding-model download it triggers)
    // is left to a manual `lievo refresh`. The flag propagates here so no
    // model download runs while the MCP serve session holds the lock.
    let rebuild_vector_index =
        !config.skip_semantic_index && (file_set_changed || force || vector_index_stale);
    if rebuild_vector_index {
        match extractor.build_semantic_index(&code_units, &vector_index_path) {
            Ok(()) => {
                storage
                    .update_repo_index_path(&repo.id, &vector_index_path.display().to_string())?;
                eprintln!(
                    "  [{}] built vector index at {}",
                    repo.name,
                    vector_index_path.display()
                );
            }
            Err(e) => eprintln!(
                "  [{}] warning: semantic index unavailable (structural analysis saved; semantic search disabled until next successful refresh): {e}",
                repo.name
            ),
        }
    } else {
        eprintln!(
            "  [{}] skipped vector index rebuild (no file changes, index is fresh)",
            repo.name
        );
    }

    Ok((code_units, subsystems, summarization_failed))
}

/// Finalize metrics, relationships, and persistence.
///
/// Takes ownership of `code_units` and `grouping`; returns them so callers
/// can forward them to cross-repo relationship building.
///
/// Function entity extraction (when enabled) must be handled by the caller
/// since it requires the config flag.
pub(super) fn finish_pipeline_steps(
    storage: &dyn Storage,
    repo: &crate::model::Repository,
    head: &str,
    start: &Instant,
    run: &mut crate::model::AnalysisRun,
    data: PipelineStepData,
) -> crate::Result<(Vec<CodeUnit>, Vec<crate::model::Entity>, bool)> {
    let mut all_entities: Vec<crate::model::Entity> = data
        .grouping
        .subsystems
        .iter()
        .chain(data.grouping.modules.iter())
        .chain(data.grouping.files.iter())
        .cloned()
        .collect();

    // Extract function entities if enabled
    let repo_root = Path::new(&repo.local_path);
    let (func_entities_opt, func_contains_rels) = if data.grouping.preserve_function_entities {
        let (func_entities, func_rels) =
            extract_function_entities(&data.code_units, &data.grouping.files, repo_root);
        all_entities.extend(func_entities.clone());
        (Some(func_entities), func_rels)
    } else {
        (None, Vec::new())
    };

    // Build relationships (including function-level edges if available)
    let (mut relationships, unresolved_repo) =
        crate::analysis::relationships::RelationshipBuilder::build_with_functions(
            &data.code_units,
            &data.grouping,
            &repo.project_id,
            &repo.name,
            repo_root,
            func_entities_opt.as_deref(),
        )?;

    // Record the repo-wide unresolved-import counter (split internal/external
    // per the #690 amendment) unconditionally on the repository row (#856):
    // every successful index writes both counts, zeros included, so a fully
    // resolved fresh index reads as Some((0, 0)) — never confusable with
    // NULL (never recorded, pre-fix indexes). Query-time consumers read it
    // via Storage::get_unresolved_counts without replaying resolution.
    storage.record_unresolved_counts(
        &repo.id,
        unresolved_repo.internal as u64,
        unresolved_repo.external as u64,
    )?;

    // Add the function contains relationships
    relationships.extend(func_contains_rels);

    // Trace execution flows from entry points
    let flows = FlowTracer::trace_flows(&all_entities, &relationships, None);

    // Attach file metrics and coupling
    metrics::attach_file_metrics(&mut all_entities, &data.code_units, &relationships);

    // Attach execution flows
    metrics::attach_execution_flows(&mut all_entities, &flows);

    // Aggregate metrics across hierarchy
    metrics::aggregate_module_metrics(&mut all_entities);
    metrics::aggregate_subsystem_metrics(&mut all_entities);

    // Populate run statistics then persist everything atomically
    run.files_analyzed = all_entities
        .iter()
        .filter(|e| e.tier == crate::model::EntityTier::File)
        .count() as i64;
    run.entities_upserted = all_entities.len() as i64;
    run.relationships_upserted = relationships.len() as i64;
    run.status = crate::model::AnalysisStatus::Completed;
    run.duration_ms = Some(start.elapsed().as_millis() as i64);
    run.completed_at = Some(chrono::Utc::now().to_rfc3339());

    let entity_refs: Vec<&crate::model::Entity> = all_entities.iter().collect();
    let (entities_upserted, relationships_upserted) =
        storage.persist_analysis_batch(&entity_refs, &relationships, run, &repo.id, head)?;

    eprintln!(
        "  [{}] done: {} entities, {} relationships in {}ms",
        repo.name,
        entities_upserted,
        relationships_upserted,
        run.duration_ms.unwrap_or(0)
    );

    // Run on-device summarization if apfel is available and not disabled
    let apfel_available = crate::summarization::apfel::is_apfel_available();
    let sum_config = crate::summarization::pipeline::SummarizationConfig::new(
        data.no_summarize,
        &data.repo_config,
        apfel_available,
    );
    let summarization_failed = if sum_config.enabled {
        match crate::summarization::pipeline::SummarizationPipeline::run(
            storage,
            &repo.id,
            &repo.project_id,
            &data.code_units,
            &sum_config,
            &data.repo_config,
        ) {
            Ok(outcome) if outcome.summarized > 0 => {
                eprintln!(
                    "  [{}] summarized {} entities",
                    repo.name, outcome.summarized
                );
                false
            }
            Ok(_) => {
                // Summarization was enabled but produced zero summaries
                // (degradation path). Mark as failed so the resume path can
                // skip re-entry (issue #788).
                true
            }
            Err(e) => {
                eprintln!(
                    "  [{}] warning: summarization failed (partial results saved — re-run analyze to retry): {e}",
                    repo.name
                );
                true
            }
        }
    } else {
        false
    };

    // Detect and persist project conventions
    let detector = crate::analysis::convention_detector::ConventionDetector::new(storage);
    detector.detect(&repo.project_id)?;

    Ok((
        data.code_units,
        data.grouping.subsystems,
        summarization_failed,
    ))
}
