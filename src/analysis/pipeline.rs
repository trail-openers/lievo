// AnalysisPipeline — orchestrates the full analysis pipeline for a single repository.
// Steps: index → read units → group → build relationships → compute metrics → persist.

use crate::analysis::incremental;
use crate::analysis::relationships::RelationshipBuilder;
use crate::error::{LievoError, Result};
use crate::model::{AnalysisRun, AnalysisStatus, CodeUnit, Entity, Repository};
use crate::storage::Storage;
use std::path::Path;
use std::time::Instant;

use super::pipeline_steps::run_pipeline_steps;

pub struct AnalysisPipeline;

/// Re-index mode for analysis pipeline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReindexMode {
    /// Default incremental mode: skip repos that haven't changed since last analysis.
    Incremental,
    /// Force re-analysis even if commit hash is unchanged (e.g. after upgrading lievo or changing config).
    /// Keeps existing summaries.
    Force,
    /// Full re-analysis: re-index all repos and clear all stale summaries.
    Full,
}

/// Configuration for a single analysis pipeline run.
pub struct PipelineConfig {
    pub respect_ignore: bool,
    pub reindex: ReindexMode,
    /// When true, skip summarization even if apfel is available.
    pub no_summarize: bool,
    /// When true, skip the semantic/vector index build (and any
    /// embedding-model download) — used by the serve-startup automatic
    /// index (issue #864) so a first `lievo mcp` start never downloads
    /// model files.
    pub skip_semantic_index: bool,
}

/// Internal result carrying all data produced during a single-repo analysis run.
struct RepoAnalysis {
    run: AnalysisRun,
    code_units: Vec<crate::model::CodeUnit>,
    subsystems: Vec<crate::model::Entity>,
    /// True when summarization was enabled but produced no summaries (issue #788).
    summarization_failed: bool,
}

/// Result of running analysis across all repos in a project.
///
/// `runs` contains one entry per successfully analyzed repo.
/// `repo_errors` contains errors from repos that failed — empty means full success.
pub struct ProjectAnalysisResult {
    pub runs: Vec<AnalysisRun>,
    pub repo_errors: Vec<(String, LievoError)>,
    /// True if any repo's summarization was enabled but produced no summaries (issue #788).
    /// The caller uses this to set the SUMMARIZATION_FAILED_FLAG so the resume path
    /// can skip re-entering the full pipeline.
    pub summarization_failed: bool,
}

impl AnalysisPipeline {
    /// Run analysis for a single repository. Returns run record or `None` if skipped.
    pub fn run_repo(
        storage: &dyn Storage,
        repo: &Repository,
        config: &PipelineConfig,
    ) -> Result<Option<AnalysisRun>> {
        Ok(Self::analyze_repo(storage, repo, config)?.map(|a| a.run))
    }

    /// Run analysis and return all produced data, or `None` on skip.
    /// Sets status to Failed and persists run record before returning errors.
    fn analyze_repo(
        storage: &dyn Storage,
        repo: &Repository,
        config: &PipelineConfig,
    ) -> Result<Option<RepoAnalysis>> {
        let repo_path = Path::new(&repo.local_path);
        let start = Instant::now();

        let head = incremental::head_commit(repo_path)?;

        // Skip if already analyzed at this commit (incremental), unless forced
        if config.reindex == ReindexMode::Incremental
            && let Some(last) = &repo.last_analyzed_commit
            && *last == head
        {
            eprintln!(
                "  [{}] already analyzed at {} — skipping",
                repo.name,
                &head[..8.min(head.len())]
            );
            return Ok(None);
        }

        eprintln!(
            "  [{}] analyzing at commit {}...",
            repo.name,
            &head[..8.min(head.len())]
        );

        // Create pending run record
        let mut run = storage.create_analysis_run(&repo.id, &head)?;

        // Pipeline steps may fail; mark run as Failed on error
        match run_pipeline_steps(storage, repo, repo_path, &head, &start, &mut run, config) {
            Ok((code_units, subsystems, summarization_failed)) => Ok(Some(RepoAnalysis {
                run,
                code_units,
                subsystems,
                summarization_failed,
            })),
            Err(e) => {
                run.status = AnalysisStatus::Failed;
                run.completed_at = Some(chrono::Utc::now().to_rfc3339());
                // Best-effort persist on error
                // Intentionally ignored: best-effort status update, failure doesn't affect analysis results
                let _ = storage.update_analysis_run(&run);
                Err(e)
            }
        }
    }

    /// Run analysis for all repositories in a project.
    ///
    /// Per-repo errors are logged and collected; analysis continues for remaining repos.
    pub fn run_project(
        storage: &dyn Storage,
        project_id: &str,
        config: &PipelineConfig,
    ) -> Result<ProjectAnalysisResult> {
        let repos = storage.list_repos(project_id)?;
        if repos.is_empty() {
            return Err(LievoError::ProjectNotFound(project_id.to_string()));
        }

        eprintln!("Analyzing {} repositories...", repos.len());

        let mut runs = Vec::new();
        // Collect data from analyzed repos across project
        let mut all_subsystems: Vec<(String, Vec<Entity>)> = Vec::new();
        let mut all_code_units: Vec<(String, Vec<CodeUnit>)> = Vec::new();
        let mut repo_errors: Vec<(String, LievoError)> = Vec::new();
        let mut summarization_failed = false;

        for repo in &repos {
            match Self::analyze_repo(storage, repo, config) {
                Ok(Some(analysis)) => {
                    summarization_failed |= analysis.summarization_failed;
                    all_subsystems.push((repo.name.clone(), analysis.subsystems));
                    all_code_units.push((repo.name.clone(), analysis.code_units));
                    runs.push(analysis.run);
                }
                Ok(None) => {
                    // Repo was skipped (already up to date); no run record produced.
                }
                Err(e) => {
                    eprintln!("  [{}] error: {e}", repo.name);
                    repo_errors.push((repo.name.clone(), e));
                }
            }
        }

        Self::build_cross_repo_relationships(storage, &all_subsystems, &all_code_units);

        Ok(ProjectAnalysisResult {
            runs,
            repo_errors,
            summarization_failed,
        })
    }

    /// Build and persist cross-repo relationships, filtering out references to
    /// entities not present in the analyzed repos to avoid FOREIGN KEY violations.
    fn build_cross_repo_relationships(
        storage: &dyn Storage,
        all_subsystems: &[(String, Vec<Entity>)],
        all_code_units: &[(String, Vec<CodeUnit>)],
    ) {
        if all_subsystems.len() <= 1 {
            return;
        }

        match RelationshipBuilder::build_cross_repo(all_subsystems, all_code_units) {
            Ok(cross_rels) => {
                // Build set of all entity IDs across all repos in this analysis run.
                // Cross-repo relationships referencing entities outside the analyzed
                // set (e.g. external library imports) are silently dropped to avoid
                // FOREIGN KEY constraint violations.
                let all_entity_ids: std::collections::HashSet<&str> = all_subsystems
                    .iter()
                    .flat_map(|(_, entities)| entities.iter().map(|e| e.id.as_str()))
                    .collect();

                let mut skipped = 0u32;
                for rel in &cross_rels {
                    if !all_entity_ids.contains(rel.source_id.as_str())
                        || !all_entity_ids.contains(rel.target_id.as_str())
                    {
                        skipped += 1;
                        continue;
                    }
                    if let Err(e) = storage.upsert_relationship(rel) {
                        eprintln!("  [cross-repo] failed to persist relationship: {e}");
                    }
                }
                if skipped > 0 {
                    tracing::debug!(
                        skipped,
                        "skipped cross-repo dangling relationships referencing entities outside analyzed set"
                    );
                }
            }
            Err(e) => {
                eprintln!("  [cross-repo] build failed: {e}");
            }
        }
    }
}
