/// Helper functions for entity, relationship, insight, convention, and analysis-run operations on
/// [`SqliteStorage`]. The [`Storage`] trait impl in `sqlite.rs` delegates to these free functions,
/// keeping individual files under 500 lines.
use rusqlite::Connection;

use crate::Result;
use crate::model::*;
use crate::storage::queries as q;

pub(crate) use super::parse_err;

#[path = "sqlite_rel_ops.rs"]
mod sqlite_rel_ops;

#[path = "sqlite_insights_ops.rs"]
mod sqlite_insights_ops;

#[path = "sqlite_ops_entities.rs"]
mod sqlite_ops_entities;

#[path = "sqlite_ops_entities_batch_tests.rs"]
#[cfg(test)]
mod sqlite_ops_entities_batch_tests;

pub(crate) use sqlite_rel_ops::delete_relationships_by_source;
pub(crate) use sqlite_rel_ops::list_all_relationships;
pub(crate) use sqlite_rel_ops::relationships_from;
pub(crate) use sqlite_rel_ops::relationships_to;
pub(crate) use sqlite_rel_ops::upsert_relationship;

pub(crate) use sqlite_insights_ops::invalidate_insights;
pub(crate) use sqlite_insights_ops::list_conventions;
pub(crate) use sqlite_insights_ops::list_insights;
pub(crate) use sqlite_insights_ops::upsert_convention;
pub(crate) use sqlite_insights_ops::upsert_insight;

pub(crate) use sqlite_ops_entities::clear_entity_summary;
pub(crate) use sqlite_ops_entities::delete_entities_by_paths;
pub(crate) use sqlite_ops_entities::delete_entities_by_repo;
pub(crate) use sqlite_ops_entities::entities_by_parent;
pub(crate) use sqlite_ops_entities::entities_by_repo;
pub(crate) use sqlite_ops_entities::entity_by_path;
pub(crate) use sqlite_ops_entities::entity_by_path_projectwide;
pub(crate) use sqlite_ops_entities::entity_ids_for_paths;
pub(crate) use sqlite_ops_entities::get_entity;
pub(crate) use sqlite_ops_entities::get_unresolved_counts;
pub(crate) use sqlite_ops_entities::list_entities;
pub(crate) use sqlite_ops_entities::search_entities_by_name;
pub(crate) use sqlite_ops_entities::upsert_entity;

/// Collect mapped rows from a `MappedRows` iterator into a `Vec`, propagating any error.
pub(crate) fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

pub(crate) fn create_analysis_run(
    conn: &Connection,
    repo_id: &str,
    commit_hash: &str,
) -> Result<AnalysisRun> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        q::CREATE_ANALYSIS_RUN,
        (&id, repo_id, commit_hash, "pending"),
    )?;
    Ok(AnalysisRun {
        id,
        repo_id: repo_id.to_string(),
        commit_hash: commit_hash.to_string(),
        files_analyzed: 0,
        files_changed: 0,
        entities_upserted: 0,
        relationships_upserted: 0,
        duration_ms: None,
        status: AnalysisStatus::Pending,
        completed_at: None,
    })
}

pub(crate) fn update_analysis_run(conn: &Connection, run: &AnalysisRun) -> Result<()> {
    let status = run.status.to_string();
    conn.execute(
        q::UPDATE_ANALYSIS_RUN,
        (
            run.files_analyzed,
            run.files_changed,
            run.entities_upserted,
            run.relationships_upserted,
            run.duration_ms,
            &status,
            &run.completed_at,
            &run.id,
        ),
    )?;
    Ok(())
}

pub(crate) fn get_file_hash(
    conn: &Connection,
    repo_id: &str,
    file_path: &str,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached(q::GET_FILE_HASH)?;
    let mut rows = stmt.query((repo_id, file_path))?;
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

pub(crate) fn upsert_file_hash(
    conn: &Connection,
    repo_id: &str,
    file_path: &str,
    content_hash: &str,
    now: &str,
) -> Result<()> {
    conn.execute(q::UPSERT_FILE_HASH, (repo_id, file_path, content_hash, now))?;
    Ok(())
}

/// Get all file hashes for a repository.
pub(crate) fn get_all_file_hashes(
    conn: &Connection,
    repo_id: &str,
) -> Result<std::collections::HashMap<String, String>> {
    let mut stmt = conn.prepare_cached(q::GET_ALL_FILE_HASHES)?;
    let mut rows = stmt.query((repo_id,))?;
    let mut hashes = std::collections::HashMap::new();
    while let Some(row) = rows.next()? {
        let file_path = row.get(0)?;
        let content_hash = row.get(1)?;
        hashes.insert(file_path, content_hash);
    }
    Ok(hashes)
}

/// Delete a file hash for a specific file.
pub(crate) fn delete_file_hash(conn: &Connection, repo_id: &str, file_path: &str) -> Result<()> {
    conn.execute(q::DELETE_FILE_HASH, (repo_id, file_path))?;
    Ok(())
}

/// Persist entities, relationships, analysis run, and last-commit atomically.
/// Uses `unchecked_transaction` because the connection is accessed through `&self`.
/// Safety: `SqliteStorage` is not `Sync`, so only one thread holds `&Connection` at a time.
pub(crate) fn persist_analysis_batch(
    conn: &Connection,
    entities: &[&Entity],
    relationships: &[Relationship],
    run: &AnalysisRun,
    repo_id: &str,
    last_commit: &str,
    now: &str,
) -> Result<(i64, i64)> {
    // SAFETY: unchecked_transaction() is safe here because SqliteStorage has no
    // reentrant entry points — no other Storage method begins a transaction, and
    // this method holds exclusive access to the connection via the Mutex guard.
    let tx = conn.unchecked_transaction()?;

    let mut entities_upserted = 0i64;
    for entity in entities {
        let tier = entity.tier.to_string();
        tx.execute(
            q::UPSERT_ENTITY,
            (
                &entity.id,
                &entity.project_id,
                &entity.repo_id,
                &tier,
                &entity.parent_id,
                &entity.name,
                &entity.path,
                &entity.language,
                &entity.summary,
                &entity.summary_commit,
                &entity.metrics_json,
                &entity.created_at,
                now,
            ),
        )
        .map_err(|e| {
            tracing::error!(
                entity_id = %entity.id,
                parent_id = ?entity.parent_id,
                tier = %entity.tier,
                repo_id = ?entity.repo_id,
                error = %e,
                "FK constraint failed during entity upsert"
            );
            crate::LievoError::from(e)
        })?;
        entities_upserted += 1;
    }

    // Build set of entity IDs being persisted in this batch.
    // Clean up all stale relationship edges for this repo before upserting new ones.
    // ON CONFLICT DO UPDATE only replaces rows — it does not delete edges that
    // are no longer present, so we must explicitly remove them here.
    let deleted = tx.execute(q::DELETE_RELATIONSHIPS_BY_REPO, (repo_id,))?;
    if deleted > 0 {
        tracing::debug!(deleted, "cleaned up stale relationship edges before upsert");
    }

    // Relationships referencing entities outside the analyzed codebase
    // (e.g. external library imports) are silently dropped to avoid
    // FOREIGN KEY constraint violations.
    let entity_ids: std::collections::HashSet<&str> =
        entities.iter().map(|e| e.id.as_str()).collect();

    let mut rels_upserted = 0i64;
    let mut rels_skipped = 0i64;
    for rel in relationships {
        if !entity_ids.contains(rel.source_id.as_str())
            || !entity_ids.contains(rel.target_id.as_str())
        {
            rels_skipped += 1;
            continue; // skip dangling relationship — endpoint not in this analysis batch
        }
        let rel_type = rel.rel_type.to_string();
        let provenance = rel.provenance.to_string();
        tx.execute(
            q::UPSERT_RELATIONSHIP,
            (
                &rel.source_id,
                &rel.target_id,
                &rel_type,
                rel.weight,
                &rel.evidence_json,
                &provenance,
            ),
        )
        .map_err(|e| {
            tracing::error!(
                source_id = %rel.source_id,
                target_id = %rel.target_id,
                rel_type = %rel_type,
                error = %e,
                "FK constraint failed during relationship upsert"
            );
            crate::LievoError::from(e)
        })?;
        rels_upserted += 1;
    }
    if rels_skipped > 0 {
        tracing::debug!(
            rels_skipped,
            "skipped dangling relationships referencing entities outside the analyzed codebase"
        );
    }

    let status = run.status.to_string();
    tx.execute(
        q::UPDATE_ANALYSIS_RUN,
        (
            run.files_analyzed,
            run.files_changed,
            run.entities_upserted,
            run.relationships_upserted,
            run.duration_ms,
            &status,
            &run.completed_at,
            &run.id,
        ),
    )?;

    tx.execute(q::UPDATE_REPO_LAST_COMMIT, (last_commit, now, repo_id))?;

    // A fresh analysis run just persisted for this repo at `last_commit`: any
    // pending enabled-but-unconfigured marker (issue #788) is now stale, since
    // the new commit is a fresh chance for the user to configure a backend or
    // fix the state. Clear it so the next staleness check re-evaluates from
    // scratch.
    tx.execute(
        q::UPDATE_REPOSITORY_UNCONFIGURED_MARKER,
        (None::<&str>, now, repo_id),
    )?;

    tx.commit()?;

    Ok((entities_upserted, rels_upserted))
}
