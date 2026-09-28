use rusqlite::Connection;

use crate::Result;
use crate::model::*;
use crate::storage::queries as q;

// parse_err is re-exported by the parent sqlite_ops module
use super::parse_err;

pub(crate) fn upsert_relationship(conn: &Connection, rel: &Relationship) -> Result<()> {
    let rel_type = rel.rel_type.to_string();
    let provenance = rel.provenance.to_string();
    conn.execute(
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
            "FK constraint failed during direct relationship upsert"
        );
        crate::LievoError::from(e)
    })?;
    Ok(())
}

fn map_rel_entity(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Relationship, Entity)> {
    Ok((
        Relationship {
            source_id: row.get(0)?,
            target_id: row.get(1)?,
            rel_type: row.get::<_, String>(2)?.parse().map_err(parse_err)?,
            weight: row.get(3)?,
            evidence_json: row.get(4)?,
            provenance: row.get::<_, String>(5)?.parse().map_err(parse_err)?,
        },
        Entity {
            id: row.get(6)?,
            project_id: row.get(7)?,
            repo_id: row.get(8)?,
            tier: row.get::<_, String>(9)?.parse().map_err(parse_err)?,
            parent_id: row.get(10)?,
            name: row.get(11)?,
            path: row.get(12)?,
            language: row.get(13)?,
            summary: row.get(14)?,
            summary_commit: row.get(15)?,
            metrics_json: row.get(16)?,
            created_at: row.get(17)?,
            updated_at: row.get(18)?,
        },
    ))
}

pub(crate) fn relationships_from(
    conn: &Connection,
    source_id: &str,
) -> Result<Vec<(Relationship, Entity)>> {
    let mut stmt = conn.prepare_cached(q::RELATIONSHIPS_FROM)?;
    let rows = stmt.query_map([source_id], map_rel_entity)?;
    super::collect_rows(rows)
}

pub(crate) fn relationships_to(
    conn: &Connection,
    target_id: &str,
) -> Result<Vec<(Relationship, Entity)>> {
    let mut stmt = conn.prepare_cached(q::RELATIONSHIPS_TO)?;
    let rows = stmt.query_map([target_id], map_rel_entity)?;
    super::collect_rows(rows)
}

pub(crate) fn delete_relationships_by_source(conn: &Connection, source_id: &str) -> Result<u64> {
    let count = conn.execute(q::DELETE_RELATIONSHIPS_BY_SOURCE, [source_id])?;
    Ok(count as u64)
}

/// List all relationships for a project using a single bulk query.
/// More efficient than calling `relationships_from` per-entity (avoids N+1).
pub(crate) fn list_all_relationships(
    conn: &Connection,
    project_id: &str,
) -> crate::Result<Vec<Relationship>> {
    let mut stmt = conn.prepare_cached(
        "SELECT r.source_id, r.target_id, r.rel_type, r.weight, r.evidence_json, r.provenance \
         FROM relationships r \
         JOIN entities e1 ON r.source_id = e1.id \
         JOIN entities e2 ON r.target_id = e2.id \
         WHERE e1.project_id = ? AND e2.project_id = ?",
    )?;

    let rows = stmt.query_map([project_id, project_id], |row| {
        Ok(Relationship {
            source_id: row.get(0)?,
            target_id: row.get(1)?,
            rel_type: row.get::<_, String>(2)?.parse().map_err(parse_err)?,
            weight: row.get(3)?,
            evidence_json: row.get(4)?,
            provenance: row.get::<_, String>(5)?.parse().map_err(parse_err)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}
