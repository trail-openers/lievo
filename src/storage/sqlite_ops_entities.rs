/// Entity operations: CRUD and batch lookup for entities in SQLite.
/// Extracted from sqlite_ops.rs to maintain 500-line file size limit.
use rusqlite::Connection;

use crate::LievoError;
use crate::Result;
use crate::model::*;
use crate::storage::queries as q;

// parse_err is re-exported by the parent sqlite_ops module
use super::parse_err;

pub(crate) fn upsert_entity(conn: &Connection, entity: &Entity, now: &str) -> Result<()> {
    let tier = entity.tier.to_string();
    conn.execute(
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
    )?;
    Ok(())
}

/// Clear an entity's stored summary and summary_commit in place.
///
/// This is the only path that actively NULLs a single entity's summary. It exists
/// because UPSERT_ENTITY COALESCEs the summary columns (a NULL incoming summary
/// means "no summary to offer" and must not wipe a stored summary), so the
/// deliberate-clear path (`lievo summarize --file`) cannot express a clear via
/// upsert_entity any more.
pub(crate) fn clear_entity_summary(conn: &Connection, entity_id: &str) -> Result<u64> {
    let count = conn.execute(q::CLEAR_ENTITY_SUMMARY, [entity_id])?;
    Ok(count as u64)
}

pub(crate) fn get_entity(conn: &Connection, entity_id: &str) -> Result<Option<Entity>> {
    let mut stmt = conn.prepare_cached(q::GET_ENTITY)?;
    let mut rows = stmt.query([entity_id])?;
    match rows.next()? {
        Some(row) => {
            let tier = row.get::<_, String>(3)?.parse().map_err(parse_err)?;
            Ok(Some(Entity {
                id: row.get(0)?,
                project_id: row.get(1)?,
                repo_id: row.get(2)?,
                tier,
                parent_id: row.get(4)?,
                name: row.get(5)?,
                path: row.get(6)?,
                language: row.get(7)?,
                summary: row.get(8)?,
                summary_commit: row.get(9)?,
                metrics_json: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            }))
        }
        None => Ok(None),
    }
}

pub(crate) fn list_entities(
    conn: &Connection,
    project_id: &str,
    tier: Option<EntityTier>,
) -> Result<Vec<Entity>> {
    let mut stmt = conn.prepare_cached(q::LIST_ENTITIES)?;
    let tier_str = tier.map(|t| t.to_string());
    let rows = stmt.query_map((project_id, tier_str.as_deref()), |row| {
        Ok(Entity {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repo_id: row.get(2)?,
            tier: row.get::<_, String>(3)?.parse().map_err(parse_err)?,
            parent_id: row.get(4)?,
            name: row.get(5)?,
            path: row.get(6)?,
            language: row.get(7)?,
            summary: row.get(8)?,
            summary_commit: row.get(9)?,
            metrics_json: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    })?;
    super::collect_rows(rows)
}

/// Build search parameters for `search_entities_by_name`.
///
/// Returns owned `Box<dyn ToSql>` values because rusqlite's dynamic parameter binding
/// requires owned values, not references. This function centralizes the parameter
/// construction logic to improve readability in the calling function.
pub(crate) fn search_entities_by_name(
    conn: &Connection,
    project_id: &str,
    words: &[&str],
    limit: usize,
    tier: Option<&str>,
) -> Result<Vec<Entity>> {
    if words.is_empty() {
        return Err(LievoError::InvalidInput(
            "search_entities_by_name requires at least one search word".into(),
        ));
    }
    let query = q::build_entity_search_query_with_tier(words.len(), tier);
    let mut stmt = conn.prepare_cached(&query)?;

    // Build parameter vector: project_id, words, tier, limit
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(project_id.to_string())];
    for word in words {
        params.push(Box::new(format!("%{}%", word.to_lowercase())));
    }
    if let Some(t) = tier {
        params.push(Box::new(t.to_string()));
    }
    params.push(Box::new(limit as i64));

    // Convert to reference slice for query
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(Entity {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repo_id: row.get(2)?,
            tier: row.get::<_, String>(3)?.parse().map_err(parse_err)?,
            parent_id: row.get(4)?,
            name: row.get(5)?,
            path: row.get(6)?,
            language: row.get(7)?,
            summary: row.get(8)?,
            summary_commit: row.get(9)?,
            metrics_json: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    })?;

    super::collect_rows(rows)
}

pub(crate) fn entities_by_repo(
    conn: &Connection,
    repo_id: &str,
    tier: Option<EntityTier>,
) -> Result<Vec<Entity>> {
    let mut stmt = conn.prepare_cached(q::ENTITIES_BY_REPO)?;
    let tier_str = tier.map(|t| t.to_string());
    let rows = stmt.query_map((repo_id, tier_str.as_deref()), |row| {
        Ok(Entity {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repo_id: row.get(2)?,
            tier: row.get::<_, String>(3)?.parse().map_err(parse_err)?,
            parent_id: row.get(4)?,
            name: row.get(5)?,
            path: row.get(6)?,
            language: row.get(7)?,
            summary: row.get(8)?,
            summary_commit: row.get(9)?,
            metrics_json: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    })?;
    super::collect_rows(rows)
}

pub(crate) fn entities_by_parent(conn: &Connection, parent_id: &str) -> Result<Vec<Entity>> {
    let mut stmt = conn.prepare_cached(q::ENTITIES_BY_PARENT)?;
    let rows = stmt.query_map([parent_id], |row| {
        Ok(Entity {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repo_id: row.get(2)?,
            tier: row.get::<_, String>(3)?.parse().map_err(parse_err)?,
            parent_id: row.get(4)?,
            name: row.get(5)?,
            path: row.get(6)?,
            language: row.get(7)?,
            summary: row.get(8)?,
            summary_commit: row.get(9)?,
            metrics_json: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    })?;
    super::collect_rows(rows)
}

pub(crate) fn entity_by_path_projectwide(
    conn: &Connection,
    project_id: &str,
    path: &str,
) -> Result<Vec<Entity>> {
    let query = r#"
        SELECT id, project_id, repo_id, tier, parent_id, name, path, language,
               summary, summary_commit, metrics_json, created_at, updated_at
        FROM entities
        WHERE project_id = ?1 AND path = ?2
    "#;
    let mut stmt = conn.prepare_cached(query)?;
    let rows = stmt.query_map((project_id, path), |row| {
        let tier = row.get::<_, String>(3)?.parse().map_err(parse_err)?;
        Ok(Entity {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repo_id: row.get(2)?,
            tier,
            parent_id: row.get(4)?,
            name: row.get(5)?,
            path: row.get(6)?,
            language: row.get(7)?,
            summary: row.get(8)?,
            summary_commit: row.get(9)?,
            metrics_json: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    })?;
    super::collect_rows(rows)
}

pub(crate) fn entity_by_path(
    conn: &Connection,
    repo_id: &str,
    path: &str,
) -> Result<Option<Entity>> {
    let mut stmt = conn.prepare_cached(q::ENTITY_BY_PATH)?;
    let mut rows = stmt.query((repo_id, path))?;
    match rows.next()? {
        Some(row) => {
            let tier = row.get::<_, String>(3)?.parse().map_err(parse_err)?;
            Ok(Some(Entity {
                id: row.get(0)?,
                project_id: row.get(1)?,
                repo_id: row.get(2)?,
                tier,
                parent_id: row.get(4)?,
                name: row.get(5)?,
                path: row.get(6)?,
                language: row.get(7)?,
                summary: row.get(8)?,
                summary_commit: row.get(9)?,
                metrics_json: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            }))
        }
        None => Ok(None),
    }
}

/// Batch lookup of entity IDs by paths for a single repository.
/// Returns a HashMap mapping path to entity_id for efficient O(1) lookups.
/// Paths that don't exist are simply absent from the map.
///
/// Uses chunking (900 items per batch) to avoid SQLite's bind-variable limit (~999),
/// which prevents crashes on directories with 1000+ files (e.g., node_modules, vendor).
pub(crate) fn entity_ids_for_paths(
    conn: &Connection,
    repo_id: &str,
    paths: &[&str],
) -> Result<std::collections::HashMap<String, String>> {
    if paths.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let mut result = std::collections::HashMap::new();
    const CHUNK_SIZE: usize = 900;

    for chunk in paths.chunks(CHUNK_SIZE) {
        let placeholders = chunk.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let query = format!(
            "SELECT id, path FROM entities WHERE repo_id = ?1 AND path IN ({placeholders}) AND tier = 'file'"
        );

        let params: Vec<&dyn rusqlite::ToSql> = std::iter::once(&repo_id as &dyn rusqlite::ToSql)
            .chain(chunk.iter().map(|p| p as &dyn rusqlite::ToSql))
            .collect();

        let mut stmt = conn.prepare(&query)?;
        let rows = stmt.query_map(params.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        for row in rows {
            let (entity_id, path) = row?;
            result.insert(path, entity_id);
        }
    }
    Ok(result)
}

/// Delete all entities for a repository.
///
/// CASCADE deletes are implemented via:
/// - relationships.source_id and relationships.target_id: FK with ON DELETE CASCADE
/// - entities.parent_id: BEFORE DELETE trigger with recursive CTE (see schema.rs)
///
/// This function performs a single deletion operation at the repo level, and CASCADE handles
/// cleanup of all related data (child entities in hierarchies, and relationships referencing deleted entities).
/// Read the unresolved-import counter persisted on the repository row's
/// `unresolved_internal`/`unresolved_external` columns (#856: the counts
/// used to be routed through a file entity whose path equals the repo name,
/// which no real index contains). NULL columns mean the counts were never
/// recorded (pre-fix index) — `None`, never a fabricated zero.
pub(crate) fn get_unresolved_counts(
    conn: &Connection,
    repo_id: &str,
) -> Result<Option<(u64, u64)>> {
    // An unknown repo id is "not recorded", not an error — the query layer
    // maps None to the null shape without a false zero.
    let row: Option<(Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT unresolved_internal, unresolved_external FROM repositories WHERE id = ?1",
            [repo_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    Ok(match row {
        Some((Some(internal), Some(external))) => Some((internal as u64, external as u64)),
        _ => None,
    })
}

pub(crate) fn delete_entities_by_repo(conn: &Connection, repo_id: &str) -> Result<u64> {
    let count = conn.execute(q::DELETE_ENTITIES_BY_REPO, [repo_id])?;
    Ok(count as u64)
}

pub(crate) fn delete_entities_by_paths(
    conn: &Connection,
    repo_id: &str,
    exclude_paths: &[String],
) -> Result<u64> {
    use crate::extraction::grouping_filter;

    // Load entities to filter
    let mut stmt = conn.prepare_cached(q::ENTITIES_BY_REPO)?;
    let entities = stmt.query_map((repo_id, None::<&str>), |row| {
        Ok(Entity {
            id: row.get(0)?,
            project_id: row.get(1)?,
            repo_id: row.get(2)?,
            tier: row.get::<_, String>(3)?.parse().map_err(parse_err)?,
            parent_id: row.get(4)?,
            name: row.get(5)?,
            path: row.get(6)?,
            language: row.get(7)?,
            summary: row.get(8)?,
            summary_commit: row.get(9)?,
            metrics_json: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    })?;

    let to_delete: Vec<String> = super::collect_rows(entities)?
        .into_iter()
        .filter(|e| {
            e.path
                .as_ref()
                .is_some_and(|p| grouping_filter::should_exclude_path(p, exclude_paths))
        })
        .map(|e| e.id)
        .collect();

    if to_delete.is_empty() {
        return Ok(0);
    }

    // Collect all entities (direct + cascaded) to delete
    let mut all_to_delete: std::collections::HashSet<String> = to_delete.iter().cloned().collect();
    let mut to_check: Vec<String> = to_delete.clone();
    let delete_set: std::collections::HashSet<String> = to_delete.iter().cloned().collect();

    while !to_check.is_empty() {
        let placeholders = to_check.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let params: Vec<&dyn rusqlite::ToSql> =
            to_check.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let mut stmt = conn.prepare(&format!(
            "SELECT id FROM entities WHERE parent_id IN ({placeholders})"
        ))?;

        let rows = stmt.query_map(params.as_slice(), |row| row.get::<_, String>(0))?;
        to_check.clear();
        for child_id in rows.flatten() {
            if all_to_delete.insert(child_id.clone()) {
                to_check.push(child_id);
            }
        }
    }

    let total_count = all_to_delete.len();

    // Delete relationships referencing entities about to be deleted
    // This must happen before FK is disabled to ensure proper cleanup
    if !all_to_delete.is_empty() {
        let placeholders = all_to_delete
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let mut rel_params: Vec<&dyn rusqlite::ToSql> = all_to_delete
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        // Duplicate params for source_id and target_id conditions
        rel_params.extend(all_to_delete.iter().map(|s| s as &dyn rusqlite::ToSql));
        let rels_deleted = conn.execute(
            &format!("DELETE FROM relationships WHERE source_id IN ({placeholders}) OR target_id IN ({placeholders})"),
            rel_params.as_slice(),
        )?;
        if rels_deleted > 0 {
            tracing::debug!(
                rels_deleted,
                "deleted orphaned relationships before entity deletion"
            );
        }
    }

    // Disable FK for batch delete to avoid CASCADE trigger conflicts
    let was_enabled = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
        .unwrap_or(0);
    conn.pragma_update(None, "foreign_keys", false)?;

    // Delete direct matches
    let placeholders = to_delete.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let params: Vec<&dyn rusqlite::ToSql> = to_delete
        .iter()
        .map(|s| s as &dyn rusqlite::ToSql)
        .collect();
    conn.execute(
        &format!("DELETE FROM entities WHERE id IN ({placeholders})"),
        params.as_slice(),
    )
    .map_err(|e| {
        if was_enabled != 0 {
            // Intentionally ignored: PRAGMA result is informational only
            let _ = conn.pragma_update(None, "foreign_keys", true);
        }
        crate::error::LievoError::Database(e)
    })?;

    // Delete cascaded entities
    let cascaded: Vec<String> = all_to_delete
        .into_iter()
        .filter(|id| !delete_set.contains(id))
        .collect();
    if !cascaded.is_empty() {
        let placeholders = cascaded.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let params: Vec<&dyn rusqlite::ToSql> =
            cascaded.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        conn.execute(
            &format!("DELETE FROM entities WHERE id IN ({placeholders})"),
            params.as_slice(),
        )?;
    }

    if was_enabled != 0 {
        conn.pragma_update(None, "foreign_keys", true)?;
    }

    Ok(total_count as u64)
}
