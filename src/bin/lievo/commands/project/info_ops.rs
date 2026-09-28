// `admin info` command handler — database statistics.

use std::path::Path;

use super::json_escape;
use lievo::Result;
use lievo::output::OutputFormat;
use lievo::retrieval::project_boundary::same_project;
use lievo::storage::Storage;

pub fn info(storage: &dyn Storage, db_path: &Path, fmt: OutputFormat) -> Result<()> {
    let projects = storage.list_projects()?;
    let project_count = projects.len();

    let repo_count: usize = projects
        .iter()
        .map(|p| storage.list_repos(&p.id).map(|r| r.len()))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum();

    // Aggregate entity and relationship counts across all projects.
    let mut total_entities: usize = 0;
    let mut total_relationships: usize = 0;
    for project in &projects {
        let entities = storage.list_entities(&project.id, None)?;
        // Project boundary (issue #764): an entity's outgoing edge may point
        // to a foreign project. Count only edges whose target belongs to this
        // project so a cross-project edge is never double-counted across two
        // projects' tallies.
        let project_id = &project.id;
        let rel_sum: usize = entities
            .iter()
            .map(|e| -> Result<usize> {
                Ok(storage
                    .relationships_from(&e.id)?
                    .into_iter()
                    .filter(|(_, target)| same_project(project_id, &target.project_id))
                    .count())
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .sum();
        total_entities += entities.len();
        total_relationships += rel_sum;
    }

    let db_size_bytes = std::fs::metadata(db_path).map(|m| m.len()).ok();

    match fmt {
        OutputFormat::Json => {
            let path_str = json_escape(&db_path.display().to_string());
            let size_val = match db_size_bytes {
                Some(b) => b.to_string(),
                None => "null".to_string(),
            };
            println!(
                "{{\"db_path\":\"{}\",\"db_size_bytes\":{},\"projects\":{},\"repositories\":{},\"entities\":{},\"relationships\":{}}}",
                path_str, size_val, project_count, repo_count, total_entities, total_relationships
            );
        }
        OutputFormat::Human => {
            let size_str = match db_size_bytes {
                Some(bytes) => {
                    if bytes < 1024 {
                        format!("{bytes} bytes")
                    } else if bytes < 1024 * 1024 {
                        format!("{:.1} KB", bytes as f64 / 1024.0)
                    } else {
                        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
                    }
                }
                None => "in-memory".to_string(),
            };
            println!("Database path:       {}", db_path.display());
            println!("Database size:       {size_str}");
            println!("Projects:            {project_count}");
            println!("Repositories:        {repo_count}");
            println!("Total entities:      {total_entities}");
            println!("Total relationships: {total_relationships}");
        }
    }
    Ok(())
}
