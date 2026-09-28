// Resume incomplete summarization for structurally-current repos.
//
// Split from `refresh.rs` to keep that file under its 500-line budget
// (AGENTS.md §6). Operates on repos whose structural analysis is current but
// whose summarization is incomplete (missing summaries), filling them in
// without re-running the full analysis pipeline.

use crate::Result;
use crate::storage::Storage;

/// Resume summarization for structurally-current repos with missing summaries.
/// Called after main analysis pipeline to fill in incomplete summaries.
///
/// Note: the extractor is built without `set_known_hashes`, so `index(false)`
/// re-parses every file (an empty hash map makes `should_parse` true for all of
/// them). This does NOT persist a new graph — it only re-reads code units so
/// summarization can fill in the entities that still lack a summary.
pub(crate) fn resume_incomplete_summarization(
    storage: &dyn Storage,
    project_id: &str,
    no_summarize: bool,
    summarization_failed: bool,
) -> Result<()> {
    let repos = storage.list_repos(project_id)?;
    for repo in repos {
        // Load this repo's config first — the shared enablement rule (issue
        // #786) is evaluated per repo below, so an apfel-only backend with
        // the CLI still needs its apfel probe, while a configured non-apfel
        // backend (or explicit `summarize: true`) resumes without it.
        let repo_path = std::path::Path::new(&repo.local_path);
        // A config that fails to parse or validate warns here (issue #788) and
        // falls back to defaults; a missing config is silent.
        let repo_config = crate::config::RepoConfig::load_or_default(repo_path);
        let apfel_available = crate::summarization::apfel::is_apfel_available();
        if !crate::summarization::pipeline::summarization_enabled(
            no_summarize,
            &repo_config,
            apfel_available,
        ) {
            continue;
        }

        // Check for missing summaries. Skip if summarization already failed in
        // this invocation (issue #788): the resume path would re-enter the full
        // pipeline for no benefit.
        if summarization_failed {
            continue;
        }

        let missing = match storage.count_missing_summaries(&repo.id) {
            Ok(count) => count,
            Err(e) => {
                eprintln!(
                    "[auto-refresh] warning: failed to count missing summaries for '{}': {}",
                    repo.name, e
                );
                continue;
            }
        };

        if missing == 0 {
            continue; // No missing summaries, skip
        }

        eprintln!(
            "  [{}] resuming summarization ({} functions with missing summaries)...",
            repo.name, missing
        );

        // Extract code units just for summarization (don't persist)
        let extractor: Box<dyn crate::extraction::code_extractor::CodeExtractor> = Box::new(
            match crate::extraction::tree_sitter_extractor::TreeSitterExtractor::new(
                repo_path, true,
            ) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!(
                        "[auto-refresh] warning: failed to create extractor for '{}': {}",
                        repo.name, e
                    );
                    continue;
                }
            },
        );

        // Index without force (incremental is fine for code extraction)
        let mut extractor = extractor;
        if let Err(e) = extractor.index(false) {
            eprintln!(
                "[auto-refresh] warning: indexing failed for '{}': {}",
                repo.name, e
            );
            continue;
        }

        let code_units = match extractor.read_all_units() {
            Ok(units) => units,
            Err(e) => {
                eprintln!(
                    "[auto-refresh] warning: failed to read code units for '{}': {}",
                    repo.name, e
                );
                continue;
            }
        };

        // Run summarization with the shared enablement rule (issue #786) so
        // this resume matches what the analyze pipeline and refresh coverage
        // actually ran. `no_summarize` is the CLI flag in force for this
        // refresh; the per-repo rule consults `summarize` and the backend.
        let sum_config = crate::summarization::pipeline::SummarizationConfig::new(
            no_summarize,
            &repo_config,
            apfel_available,
        );

        match crate::summarization::pipeline::SummarizationPipeline::run(
            storage,
            &repo.id,
            project_id,
            &code_units,
            &sum_config,
            &repo_config,
        ) {
            Ok(_) => {}
            Err(e) => eprintln!(
                "[auto-refresh] warning: summarization resume failed for '{}': {}",
                repo.name, e
            ),
        }
    }

    Ok(())
}
