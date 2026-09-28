// Private helper functions for the grouping heuristic

use crate::error::Result;
use crate::extraction::entity_id::entity_id;
use crate::model::{CodeUnit, Entity, EntityTier};
use std::collections::HashMap;

/// Picks the most frequent language among a set of code units.
pub(crate) fn most_common_language(units: &[&CodeUnit]) -> Option<String> {
    if units.is_empty() {
        return None;
    }
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for unit in units {
        *counts.entry(&unit.language).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(lang, _)| lang.to_string())
}

/// Returns the deepest subsystem that contains `file_path`, or `None`.
pub(crate) fn find_subsystem_for_file(
    file_path: &str,
    subsystem_map: &HashMap<String, String>,
) -> Option<(String, String)> {
    let mut best_match: Option<(String, String)> = None;
    let mut best_len = 0;

    for (subsystem_path, subsystem_name) in subsystem_map {
        let is_under = if subsystem_path == "." {
            true
        } else {
            file_path.starts_with(&format!("{subsystem_path}/")) || file_path == subsystem_path
        };

        if is_under {
            let len = subsystem_path.len();
            if len > best_len {
                best_len = len;
                best_match = Some((subsystem_path.clone(), subsystem_name.clone()));
            }
        }
    }

    best_match
}

/// Returns the parent subsystem path for a module.
///
/// For "." module path, returns "." only if it exists in subsystem_map.
/// Returns a matching subsystem path for other modules, or "." for root fallback.
pub(crate) fn find_parent_subsystem_for_module(
    module_path: &str,
    subsystem_map: &HashMap<String, String>,
) -> String {
    if module_path == "." {
        // Root module: if "." is not in subsystem_map, return "." to trigger parent_id: None
        return ".".to_string();
    }

    for subsystem_path in subsystem_map.keys() {
        if subsystem_path == "." {
            continue;
        }
        if module_path == subsystem_path || module_path.starts_with(&format!("{subsystem_path}/")) {
            return subsystem_path.clone();
        }
    }

    ".".to_string()
}

/// Returns a module path: up to `module_depth` directory levels below the subsystem root,
/// prefixed with the subsystem path. Returns the subsystem path (or ".") when the file
/// sits directly in the subsystem root with no sub-directories to take.
pub(crate) fn determine_module_path(
    file_path: &str,
    subsystem_path: &str,
    module_depth: usize,
) -> String {
    let relative = if subsystem_path == "." {
        file_path.to_string()
    } else {
        file_path
            .strip_prefix(&format!("{subsystem_path}/"))
            .unwrap_or(file_path)
            .to_string()
    };
    let parts: Vec<&str> = relative.split('/').collect();
    let dir_components = if parts.len() > 1 {
        &parts[..parts.len() - 1]
    } else {
        &[][..]
    };
    let taken = dir_components.len().min(module_depth);
    if taken == 0 {
        if subsystem_path == "." {
            ".".to_string()
        } else {
            subsystem_path.to_string()
        }
    } else {
        let sub = dir_components[..taken].join("/");
        if subsystem_path == "." {
            sub
        } else {
            format!("{subsystem_path}/{sub}")
        }
    }
}

/// Builds `Entity` structs at the `Subsystem` tier for each entry in `subsystem_map`.
pub(crate) fn create_subsystem_entities(
    subsystem_map: &HashMap<String, String>,
    _modules: &[Entity],
    project_id: &str,
    repo_name: &str,
    repo_id: &str,
) -> Result<Vec<Entity>> {
    let mut subsystems = subsystem_map
        .iter()
        .map(|(path, name)| {
            let id = entity_id(project_id, repo_name, EntityTier::Subsystem, path)?;
            Ok(Entity {
                id,
                project_id: project_id.to_string(),
                repo_id: Some(repo_id.to_string()),
                tier: EntityTier::Subsystem,
                parent_id: None,
                name: name.clone(),
                path: Some(path.clone()),
                language: None,
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    subsystems.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(subsystems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CodeUnit;

    fn make_code_unit(file: &str, language: &str) -> CodeUnit {
        CodeUnit {
            name: "test".to_string(),
            unit_type: "function".to_string(),
            file: file.to_string(),
            line: 1,
            end_line: 10,
            language: language.to_string(),
            signature: None,
            code: None,
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: vec![],
            imports: vec![],
            qualified_name: "test::test".to_string(),
            docstring: None,
            parent_class: None,
        }
    }

    #[test]
    fn test_most_common_language() {
        let u1 = make_code_unit("test.rs", "Rust");
        let u2 = make_code_unit("test.rs", "Rust");
        let u3 = make_code_unit("test.rs", "Python");
        let units = vec![&u1, &u2, &u3];
        assert_eq!(most_common_language(&units), Some("Rust".to_string()));
    }

    #[test]
    fn test_most_common_language_empty() {
        assert_eq!(most_common_language(&[]), None);
    }

    #[test]
    fn test_find_subsystem_for_file_cargo_workspace() {
        let mut map = HashMap::new();
        map.insert("crates/conductor".to_string(), "conductor".to_string());
        map.insert("crates/agents".to_string(), "agents".to_string());
        map.insert(".".to_string(), "root".to_string());

        let result = find_subsystem_for_file("crates/conductor/src/pipeline.rs", &map).unwrap();
        assert_eq!(
            result,
            ("crates/conductor".to_string(), "conductor".to_string())
        );
    }

    #[test]
    fn test_find_subsystem_for_file_root() {
        let mut map = HashMap::new();
        map.insert(".".to_string(), "root".to_string());

        let result = find_subsystem_for_file("any/path/file.rs", &map).unwrap();
        assert_eq!(result, (".".to_string(), "root".to_string()));
    }
}
