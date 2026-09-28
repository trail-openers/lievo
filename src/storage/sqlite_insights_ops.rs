/// Insight and convention operations on SqliteStorage.
/// Extracted from sqlite_ops to keep files under 500 lines.
use rusqlite::Connection;

use crate::Result;
use crate::model::*;
use crate::storage::queries as q;

pub(crate) fn upsert_insight(conn: &Connection, insight: &Insight) -> Result<()> {
    conn.execute(
        q::UPSERT_INSIGHT,
        (
            &insight.id,
            &insight.project_id,
            &insight.category,
            &insight.severity,
            &insight.title,
            &insight.description,
            &insight.entity_ids_json,
            &insight.detected_at,
            insight.still_valid,
        ),
    )?;
    Ok(())
}

pub(crate) fn list_insights(
    conn: &Connection,
    project_id: &str,
    category: Option<&str>,
    severity: Option<&str>,
    limit: usize,
) -> Result<Vec<Insight>> {
    let mut stmt = conn.prepare_cached(q::LIST_INSIGHTS)?;
    let rows = stmt.query_map((project_id, category, severity, limit as i64), |row| {
        Ok(Insight {
            id: row.get(0)?,
            project_id: row.get(1)?,
            category: row.get(2)?,
            severity: row.get(3)?,
            title: row.get(4)?,
            description: row.get(5)?,
            entity_ids_json: row.get(6)?,
            detected_at: row.get(7)?,
            still_valid: row.get(8)?,
        })
    })?;
    super::collect_rows(rows)
}

pub(crate) fn invalidate_insights(conn: &Connection, project_id: &str) -> Result<()> {
    conn.execute(q::INVALIDATE_INSIGHTS, [project_id])?;
    Ok(())
}

pub(crate) fn upsert_convention(conn: &Connection, convention: &Convention) -> Result<()> {
    conn.execute(
        q::UPSERT_CONVENTION,
        (
            &convention.id,
            &convention.project_id,
            &convention.category,
            &convention.title,
            &convention.description,
            &convention.example_code,
            convention.confidence,
            &convention.entity_ids_json,
            &convention.detected_at,
            convention.still_valid,
        ),
    )?;
    Ok(())
}

pub(crate) fn list_conventions(
    conn: &Connection,
    project_id: &str,
    category: Option<&str>,
) -> Result<Vec<Convention>> {
    let mut stmt = conn.prepare_cached(q::LIST_CONVENTIONS)?;
    let rows = stmt.query_map((project_id, category), |row| {
        Ok(Convention {
            id: row.get(0)?,
            project_id: row.get(1)?,
            category: row.get(2)?,
            title: row.get(3)?,
            description: row.get(4)?,
            example_code: row.get(5)?,
            confidence: row.get(6)?,
            entity_ids_json: row.get(7)?,
            detected_at: row.get(8)?,
            still_valid: row.get(9)?,
        })
    })?;
    super::collect_rows(rows)
}
