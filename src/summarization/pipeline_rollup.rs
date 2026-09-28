// Rollup tier summarization (issue #771).
//
// This is an include!d file — `super` refers to the pipeline module scope.

use crate::summarization::pipeline::{
    classify_rollup_skip, is_overflow_error, is_test_file_path, RollupOutcome, SkippedRollup,
};
use crate::summarization::SummarizationPipeline;
use crate::model::EntityTier;
use crate::storage::Storage;
use crate::error::Result;
use crate::summarization::apfel::{
    BackendTransport, ROLLUP_MAX_ENTITIES_PER_BATCH, USER_PROMPT, pack_by_char_budget,
    rollup_batch_summarize, summarize_code,
};

impl SummarizationPipeline {
    /// Roll up summaries to a tier using batch summarization (issue #771).
    ///
    /// Entities with no summarized child are skipped and counted per bucket
    /// (issue #793) instead of only logged: the skip is a first-class
    /// `RollupOutcome`, not a `tracing::warn!` line.
    ///
    /// `transport` is the explicit HTTP transport for the run (issue #783),
    /// or `None` for the one-shot CLI fallback.
    pub(super) fn rollup_to_tier(
        storage: &dyn Storage,
        repo_id: &str,
        tier: EntityTier,
        input_char_budget: usize,
        transport: Option<&BackendTransport>,
    ) -> Result<RollupOutcome> {
        let mut skipped = SkippedRollup::default();
        let entities = storage.entities_by_repo(repo_id, Some(tier))?;
        let tier_label = match tier {
            EntityTier::File => "Functions in", EntityTier::Module => "Module",
            EntityTier::Subsystem => "Subsystem", EntityTier::Function => "Function",
        };

        let mut packable: Vec<(String, String, String)> = Vec::new();
        let mut packable_entities: Vec<&crate::model::Entity> = Vec::new();
        let mut fallbacks: Vec<crate::model::Entity> = Vec::new();

        for entity in &entities {
            let children = storage.entities_by_parent(&entity.id)?;
            if !children.iter().any(|c| c.summary.is_some()) {
                // File-tier childless entities may be summarized from their
                // source text (issue #827); policy skips take precedence and
                // every other bucket is unchanged.
                if tier == EntityTier::File
                    && children.is_empty()
                    && entity
                        .path
                        .as_deref()
                        .map(|p| !is_test_file_path(p))
                        .unwrap_or(false)
                {
                    // A missing/unreadable source is folded into the
                    // no_children bucket (issue #827: the seam degrades on
                    // failure, it never errors).
                    let source = match read_source(storage, entity, input_char_budget) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("warning: read_source failed for {}: {e}", entity.name);
                            None
                        }
                    };
                    match source {
                        Some(_) => fallbacks.push(entity.clone()),
                        None => {
                            skipped = skipped + classify_rollup_skip(entity, children.len());
                        }
                    }
                } else {
                    skipped = skipped + classify_rollup_skip(entity, children.len());
                }
                continue;
            }
            let child_entries: Vec<String> = children.iter().take(3)
                .map(|c| format!("- {} ({}): {}", c.name, c.path.clone().unwrap_or_default(), c.summary.clone().unwrap_or_default()))
                .collect();
            let children_str = child_entries.join("\n");
            let prompt = match tier {
                EntityTier::File | EntityTier::Subsystem => {
                    let kind = if tier == EntityTier::File { "file" } else { "subsystem" };
                    format!(
                        "Based on the following components of the {} '{}', write a concise 1-2 sentence \
                         architectural description of what this {} is responsible for.\n\
                         Focus on PURPOSE and ROLE. Do NOT describe individual functions.\n\
                         Respond with only the description, no preamble.\n\nComponents:\n{}",
                        kind, entity.name, kind, children_str
                    )
                }
                _ => format!(
                    "Describe the role of {} '{}' in the codebase.\nComponents:\n{}",
                    tier_label, entity.name, children_str
                ),
            };
            packable_entities.push(entity);
            packable.push((entity.id.clone(), entity.name.clone(), prompt));
        }

        // File-tier source-text fallbacks (issue #827): each gets the
        // summarize_code USER_PROMPT (not the "Components:" rollup prompt) and
        // a single summarizer call. Source is already truncated at the read
        // seam, so the budget guard inside summarize_code cannot fire.
        let mut updated = 0usize;
        for entity in &fallbacks {
            // A None source was already counted in no_children during the
            // collection pass; an IO error is logged and left there as well.
            let Ok(Some(source)) = read_source(storage, entity, input_char_budget) else {
                continue;
            };
            let prompt = format!("{}\n\n{}", USER_PROMPT, source);
            match summarize_code(&source, transport) {
                Ok(result) => {
                    let mut e = entity.clone();
                    e.summary = Some(result.content);
                    e.summary_commit = Some(Self::hash_code(&prompt));
                    if storage.upsert_entity(&e).is_ok() {
                        updated += 1;
                    }
                }
                Err(e) => {
                    eprintln!("warning: source fallback failed for {}: {e}", entity.name);
                }
            }
        }

        if packable.is_empty() {
            return Ok(RollupOutcome { updated, skipped });
        }

        let initial_batches = pack_by_char_budget(&packable, input_char_budget);
        let batches: Vec<Vec<(String, String, String)>> = initial_batches
            .into_iter()
            .flat_map(|b| b.chunks(ROLLUP_MAX_ENTITIES_PER_BATCH).map(|c| c.to_vec()).collect::<Vec<_>>())
            .collect();

        let mut skipped_oversized = 0u64;

        // Each batch carries its own start offset, computed from the batch
        // index: `offsets[batch_idx]` is the position of the batch's first
        // entity in `packable_entities`, so no arm of the loop below can
        // double-advance an offset (issue #790 — the previous manual offset
        // was advanced in the generic `Err` arm AND at the tail of the loop,
        // writing later batches' summaries onto the wrong entities).
        let offsets: Vec<usize> = batches
            .iter()
            .scan(0usize, |acc, b| {
                let start = *acc;
                *acc += b.len();
                Some(start)
            })
            .collect();

        for (batch_idx, batch) in batches.iter().enumerate() {
            let results = match rollup_batch_summarize(batch, transport) {
                Ok(r) => r,
                Err(e) if is_overflow_error(&e) => {
                    for (_, name, _) in batch.iter() {
                        eprintln!("warning: rollup entity {} exceeds the summarization budget ({} chars), skipped", name, input_char_budget);
                    }
                    skipped_oversized += batch.len() as u64;
                    continue;
                }
                Err(e) => {
                    eprintln!("warning: rollup batch failed for {} entities: {e}", batch.len());
                    vec![None; batch.len()]
                }
            };

            let present = results.iter().filter(|r| r.is_some()).count();
            if present != batch.len() {
                eprintln!("warning: rollup batch returned {present} results for {} entities", batch.len());
            }

            let offset = offsets[batch_idx];
            for (pos, result) in results.iter().enumerate() {
                let Some(summary) = result else { continue };
                let idx = offset + pos;
                let mut e = packable_entities[idx].clone();
                e.summary = Some(summary.clone());
                e.summary_commit = Some(Self::hash_code(&packable[idx].2));
                if storage.upsert_entity(&e).is_ok() { updated += 1; }
            }
        }

        if skipped_oversized > 0 {
            eprintln!("warning: {} rollup entities too large to summarize (context window exceeded).", skipped_oversized);
        }

        Ok(RollupOutcome { updated, skipped })
    }
}

/// Read the source text for a File-tier entity via the repo's local path
/// (issue #827), truncated to `input_char_budget` at the char boundary so the
/// caller's prompt can never exceed the budget.
///
/// The seam is a standalone `fn` (not a `Storage` trait method) so `TestStorage`
/// can mock it by implementing `get_repo` with a `local_path` pointing at a
/// temp dir — no `unimplemented!()` panic path. All failure shapes (missing
/// repo, missing/empty file, IO error) collapse into `Ok(None)` so the caller
/// falls through to the `no_children` bucket uniformly.
pub(crate) fn read_source(
    storage: &dyn Storage,
    entity: &crate::model::Entity,
    input_char_budget: usize,
) -> Result<Option<String>> {
    let path = match entity.path.as_deref() {
        Some(p) => p,
        None => return Ok(None),
    };
    let repo = match storage.get_repo(&entity.repo_id.clone().unwrap_or_default())? {
        Some(r) => r,
        None => return Ok(None),
    };
    let full = std::path::Path::new(&repo.local_path).join(path);
    let source = std::fs::read_to_string(&full)?;
    if source.is_empty() {
        return Ok(None);
    }
    let truncated: String = source.chars().take(input_char_budget).collect();
    Ok(Some(truncated))
}
