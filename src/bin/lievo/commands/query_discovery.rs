use std::path::Path;

use lievo::Result;
use lievo::analysis::flow_tracer::ExecutionFlow;
use lievo::output::OutputFormat;
use lievo::project_resolution::resolve_project_id;
use lievo::retrieval::doc_discovery::discover_existing_docs;
use lievo::storage::Storage;

use serde_json::json;

pub fn flows(
    storage: &dyn Storage,
    project_name: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    let project_id = resolve_project_id(storage, project_name)?;
    let flows = collect_execution_flows(storage, &project_id)?;
    let output = match format {
        OutputFormat::Human => format_flows_human(&flows),
        OutputFormat::Json => {
            serde_json::to_string(&flows).map_err(lievo::LievoError::JsonParse)?
        }
    };
    println!("{output}");
    Ok(())
}

pub fn docs(storage: &dyn Storage, project_name: Option<&str>, format: OutputFormat) -> Result<()> {
    let project_id = resolve_project_id(storage, project_name)?;
    let docs = collect_docs(storage, &project_id)?;
    let output = match format {
        OutputFormat::Human => format_docs_human(&docs),
        OutputFormat::Json => format_docs_json(&docs),
    };
    println!("{output}");
    Ok(())
}

fn collect_execution_flows(storage: &dyn Storage, project_id: &str) -> Result<Vec<ExecutionFlow>> {
    let mut flows = Vec::new();
    for entity in storage.list_entities(project_id, None)? {
        let Some(metrics_json) = entity.metrics_json.as_deref() else {
            continue;
        };
        let metrics: serde_json::Value = match serde_json::from_str(metrics_json) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "Warning: corrupt metrics_json for entity {}: {e}",
                    entity.id
                );
                continue;
            }
        };
        // Most entities have metrics_json without execution_flows — this is normal; skip silently.
        let Some(raw_flows) = metrics.get("execution_flows") else {
            continue;
        };
        let parsed: Vec<ExecutionFlow> = match serde_json::from_value(raw_flows.clone()) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "Warning: could not parse execution_flows for entity {}: {e}",
                    entity.id
                );
                continue;
            }
        };
        flows.extend(parsed);
    }

    flows.sort_by(|a, b| {
        a.entry_point
            .cmp(&b.entry_point)
            .then_with(|| a.entry_point_id.cmp(&b.entry_point_id))
    });
    Ok(flows)
}

fn collect_docs(storage: &dyn Storage, project_id: &str) -> Result<Vec<DiscoveredDocRow>> {
    let mut rows = Vec::new();
    for repo in storage.list_repos(project_id)? {
        if !Path::new(&repo.local_path).is_dir() {
            continue;
        }
        for doc in discover_existing_docs(Path::new(&repo.local_path))? {
            rows.push(DiscoveredDocRow {
                path: doc.path.to_string_lossy().to_string(),
                size_bytes: doc.size_bytes,
            });
        }
    }

    rows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(rows)
}

#[derive(Debug, Clone)]
struct DiscoveredDocRow {
    path: String,
    size_bytes: u64,
}

fn format_flows_human(flows: &[ExecutionFlow]) -> String {
    if flows.is_empty() {
        return "No execution flows found.".to_string();
    }

    let mut lines = vec!["EXECUTION FLOWS".to_string(), "━".repeat(15)];
    for flow in flows {
        lines.push(format!(
            "  {} ({})",
            flow.entry_point.as_str(),
            flow.entry_point_id.as_str()
        ));
        lines.push(format!(
            "    cycle: {}",
            if flow.has_cycle { "yes" } else { "no" }
        ));
        lines.push("    steps:".to_string());
        for step in &flow.steps {
            lines.push(format!(
                "      {}: {} ({})",
                step.depth,
                step.entity_name.as_str(),
                step.entity_id.as_str()
            ));
        }
    }
    lines.join("\n")
}

fn format_docs_human(docs: &[DiscoveredDocRow]) -> String {
    if docs.is_empty() {
        return "No documentation files found.".to_string();
    }

    let path_w = docs
        .iter()
        .map(|doc| doc.path.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let header = format!("  {:<path_w$}  {:>10}", "Path", "Bytes");
    let sep = "━".repeat(header.chars().count());
    let mut lines = vec!["DOCUMENTATION FILES".to_string(), sep, header];
    for doc in docs {
        lines.push(format!(
            "  {:<path_w$}  {:>10}",
            doc.path.as_str(),
            doc.size_bytes
        ));
    }
    lines.join("\n")
}

fn format_docs_json(docs: &[DiscoveredDocRow]) -> String {
    json!(
        docs.iter()
            .map(|doc| json!({
                "path": doc.path.as_str(),
                "size_bytes": doc.size_bytes,
            }))
            .collect::<Vec<_>>()
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lievo::analysis::flow_tracer::{ExecutionFlow, FlowStep};
    use lievo::model::{Entity, EntityTier};
    use lievo::storage::sqlite::SqliteStorage; // For test helpers
    use std::fs;
    use tempfile::tempdir;

    fn entity_with_flows(metrics_json: Option<String>) -> Entity {
        Entity {
            id: "e1".to_string(),
            project_id: "proj".to_string(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "main".to_string(),
            path: Some("src/main.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_collect_execution_flows_reads_metrics_json() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let project_id = project.id.clone();
        let flow = ExecutionFlow {
            entry_point: "main".to_string(),
            entry_point_id: "e1".to_string(),
            steps: vec![FlowStep {
                entity_name: "main".to_string(),
                entity_id: "e1".to_string(),
                depth: 0,
            }],
            has_cycle: false,
        };
        let entity = Entity {
            project_id,
            ..entity_with_flows(Some(json!({"execution_flows": [flow]}).to_string()))
        };
        storage.upsert_entity(&entity).unwrap();

        let flows = collect_execution_flows(&storage, &entity.project_id).unwrap();
        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].entry_point, "main");
    }

    #[test]
    fn test_format_flows_human_has_header() {
        let flow = ExecutionFlow {
            entry_point: "main".to_string(),
            entry_point_id: "e1".to_string(),
            steps: vec![],
            has_cycle: false,
        };
        let human = format_flows_human(&[flow]);
        assert!(human.contains("EXECUTION FLOWS"));
        assert!(human.contains("main"));
    }

    #[test]
    fn test_format_docs_json_valid() {
        let json = format_docs_json(&[DiscoveredDocRow {
            path: "/tmp/README.md".to_string(),
            size_bytes: 10,
        }]);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["path"], "/tmp/README.md");
    }

    #[test]
    fn test_flows_handler_with_seeded_flows_returns_ok() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test_proj", None).unwrap();
        let flow = ExecutionFlow {
            entry_point: "main".to_string(),
            entry_point_id: "e1".to_string(),
            steps: vec![],
            has_cycle: false,
        };
        let entity = Entity {
            id: "e1".to_string(),
            project_id: project.id.clone(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "main".to_string(),
            path: Some("src/main.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: Some(serde_json::json!({"execution_flows": [flow]}).to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        storage.upsert_entity(&entity).unwrap();

        let flows_found = collect_execution_flows(&storage, &project.id).unwrap();
        assert_eq!(flows_found.len(), 1);
        assert_eq!(flows_found[0].entry_point, "main");

        let result = flows(&storage, Some("test_proj"), OutputFormat::Human);
        assert!(result.is_ok());
    }

    #[test]
    fn test_docs_handler_empty_returns_ok() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage.create_project("test_proj", None).unwrap();

        let result = docs(&storage, Some("test_proj"), OutputFormat::Human);
        assert!(result.is_ok());
    }

    #[test]
    fn test_format_docs_human_empty_reports_no_files() {
        assert_eq!(format_docs_human(&[]), "No documentation files found.");
    }

    #[test]
    fn test_docs_handler_with_seeded_docs_returns_ok() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test_proj", None).unwrap();
        let repo_dir = tempdir().unwrap();
        fs::write(repo_dir.path().join("README.md"), "# Test Project").unwrap();
        storage
            .add_repo(&project.id, "repo", repo_dir.path().to_str().unwrap())
            .unwrap();

        let docs_found = collect_docs(&storage, &project.id).unwrap();
        assert_eq!(docs_found.len(), 1);
        assert!(docs_found[0].path.ends_with("README.md"));

        let human = format_docs_human(&docs_found);
        assert!(human.contains("DOCUMENTATION FILES"));
        assert!(human.contains("README.md"));

        let result = docs(&storage, Some("test_proj"), OutputFormat::Human);
        assert!(result.is_ok());
    }

    #[test]
    fn test_collect_execution_flows_warns_on_corrupt_metrics_json() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test_proj", None).unwrap();
        let entity = Entity {
            id: "e1".to_string(),
            project_id: project.id.clone(),
            repo_id: None,
            tier: EntityTier::File,
            parent_id: None,
            name: "bad".to_string(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: Some("not valid json".to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };
        storage.upsert_entity(&entity).unwrap();

        let result = collect_execution_flows(&storage, &project.id);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_flows_handler_returns_error_for_unknown_project() {
        let storage = SqliteStorage::open_in_memory().unwrap();

        let result = flows(&storage, Some("nonexistent"), OutputFormat::Human);
        assert!(result.is_err());
    }
}
