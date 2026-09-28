//! Project resolution shared by the MCP server and the CLI binary.
//!
//! Resolves an optional project name to a project ID, auto-selecting when
//! exactly one project exists.

use crate::storage::Storage;
use crate::{LievoError, Result};

/// Resolve a project ID from an optional project name.
///
/// - If `project_name` is given, look it up by name.
/// - If omitted and exactly one project exists, use it automatically.
/// - If omitted and zero or multiple projects exist, return an error.
pub fn resolve_project_id(storage: &dyn Storage, project_name: Option<&str>) -> Result<String> {
    match project_name {
        Some(name) => {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(LievoError::InvalidProjectId(name.to_string()));
            }
            let project = storage
                .get_project(trimmed)?
                .ok_or_else(|| LievoError::ProjectNotFound(trimmed.to_string()))?;
            Ok(project.id)
        }
        None => {
            let projects = storage.list_projects()?;
            match projects.len() {
                0 => Err(LievoError::ProjectNotFound(
                    "no projects exist — create one first with `lievo admin create-project`"
                        .to_string(),
                )),
                1 => {
                    let project = projects
                        .into_iter()
                        .next()
                        .ok_or_else(|| LievoError::ProjectNotFound("internal error".to_string()))?;
                    Ok(project.id)
                }
                _ => Err(LievoError::ProjectNotFound(
                    "multiple projects exist — specify --project <name>".to_string(),
                )),
            }
        }
    }
}
