// Pipeline step cleanup utilities.

use crate::storage::Storage;

/// Applies Full-mode cleanup policies before extraction.
pub(super) fn apply_pre_extraction_cleanup(
    config: &crate::analysis::pipeline::PipelineConfig,
    repo: &crate::model::Repository,
    storage: &dyn Storage,
) -> crate::Result<()> {
    if matches!(config.reindex, crate::analysis::pipeline::ReindexMode::Full) {
        storage.clear_repo_summaries(&repo.id)?;
        eprintln!("  [{}] cleared all summaries (Full reindex)", repo.name);
    }
    Ok(())
}
