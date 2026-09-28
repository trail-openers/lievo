use rusqlite::Connection;
use uuid::Uuid;

use crate::Result;
use crate::model::Project;
use crate::storage::queries as q;
use serde_json;

/// Project-related database operations.
/// Handles creation, retrieval, and listing of projects.
///
/// Uses INSERT OR IGNORE + SELECT to atomically ensure idempotency:
/// - If a project with the same name exists, the INSERT is ignored and the
///   existing row is returned, preserving the original project_id.
/// - If no such project exists, a new one is inserted and returned.
///   This guarantees project_id stability even under concurrent calls.
pub(super) fn create_project(
    conn: &Connection,
    now: &str,
    name: &str,
    description: Option<&str>,
) -> Result<Project> {
    // INSERT OR IGNORE: if a project with this name already exists, the insert
    // is silently skipped. Then we SELECT the row — whether just inserted or
    // pre-existing. Atomic because UNIQUE constraint + OR IGNORE is a single
    // SQLite operation; no separate transaction needed.
    conn.execute(
        r#"INSERT OR IGNORE INTO projects (id, name, description, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5)"#,
        (Uuid::new_v4().to_string(), name, description, now, now),
    )?;
    get_project(conn, name)?.ok_or_else(|| {
        crate::LievoError::RetrievalError("project missing after INSERT OR IGNORE".into())
    })
}

pub(super) fn get_project(conn: &Connection, name: &str) -> Result<Option<Project>> {
    let mut stmt = conn.prepare_cached(q::GET_PROJECT)?;
    let mut rows = stmt.query([name])?;
    match rows.next()? {
        Some(row) => {
            let output_dirs_json: Option<String> = row.get(3)?;
            let output_dirs = output_dirs_json.and_then(|j| serde_json::from_str(&j).ok());
            Ok(Some(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                output_dirs,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            }))
        }
        None => Ok(None),
    }
}

pub(super) fn get_project_by_id(conn: &Connection, project_id: &str) -> Result<Option<Project>> {
    let mut stmt = conn.prepare_cached(q::GET_PROJECT_BY_ID)?;
    let mut rows = stmt.query([project_id])?;
    match rows.next()? {
        Some(row) => {
            let output_dirs_json: Option<String> = row.get(3)?;
            let output_dirs = output_dirs_json.and_then(|j| serde_json::from_str(&j).ok());
            Ok(Some(Project {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                output_dirs,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            }))
        }
        None => Ok(None),
    }
}

pub(super) fn list_projects(conn: &Connection) -> Result<Vec<Project>> {
    let mut stmt = conn.prepare_cached(q::LIST_PROJECTS)?;
    let rows = stmt.query_map([], |row| {
        let output_dirs_json: Option<String> = row.get(3)?;
        let output_dirs = output_dirs_json.and_then(|j| serde_json::from_str(&j).ok());
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            output_dirs,
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

pub(super) fn add_output_dir(conn: &Connection, project_id: &str, dir: &str) -> Result<()> {
    // Get current output_dirs
    let current: Option<Vec<String>> = conn.query_row(
        "SELECT output_dirs FROM projects WHERE id = ?1",
        [project_id],
        |row| {
            let json: Option<String> = row.get(0)?;
            Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
        },
    )?;

    // Merge with new dir
    let mut dirs = current.unwrap_or_default();
    if !dirs.contains(&dir.to_string()) {
        dirs.push(dir.to_string());
    }

    // Store back
    let json = serde_json::to_string(&dirs)?;
    conn.execute(
        "UPDATE projects SET output_dirs = ?1 WHERE id = ?2",
        (json, project_id),
    )?;

    Ok(())
}

pub(super) fn get_output_dirs(conn: &Connection, project_id: &str) -> Result<Vec<String>> {
    let dirs: Option<String> = conn.query_row(
        "SELECT output_dirs FROM projects WHERE id = ?1",
        [project_id],
        |row| row.get(0),
    )?;

    Ok(dirs
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default())
}
