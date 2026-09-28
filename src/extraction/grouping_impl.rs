// Internal implementation helpers for the grouping heuristic

use crate::error::Result;
use crate::extraction::entity_id::entity_id;
use crate::extraction::grouping_filter::should_exclude_path;
use crate::extraction::grouping_helpers::{
    determine_module_path, find_parent_subsystem_for_module, find_subsystem_for_file,
    most_common_language,
};
use crate::model::{CodeUnit, Entity, EntityTier};
use std::collections::{HashMap, HashSet};

/// Map a source file extension to the display language used on file
/// entities. Zero-unit seed paths (issue #701) have no code units to borrow
/// a language from, so their entity language is derived from the extension.
pub(crate) fn language_for_path(file_path: &str) -> Option<String> {
    let ext = file_path.rsplit('.').next()?;
    match ext {
        "rs" => Some("Rust".to_string()),
        "py" => Some("Python".to_string()),
        "js" | "jsx" | "mjs" => Some("JavaScript".to_string()),
        "ts" | "tsx" => Some("TypeScript".to_string()),
        "go" => Some("Go".to_string()),
        _ => None,
    }
}

pub(crate) fn group_into_files(
    code_units: &[CodeUnit],
    scanned_paths: &[String],
    project_id: &str,
    repo_name: &str,
    repo_id: &str,
    exclude_paths: &[String],
) -> Result<Vec<Entity>> {
    let excluded_unit_files: HashSet<&str> = code_units
        .iter()
        .filter(|unit| should_exclude_path(&unit.file, exclude_paths))
        .map(|unit| unit.file.as_str())
        .collect();

    let mut file_map: HashMap<&str, Vec<&CodeUnit>> = HashMap::new();
    for unit in code_units {
        // Skip excluded paths
        if !excluded_unit_files.contains(unit.file.as_str()) {
            file_map.entry(&unit.file).or_default().push(unit);
        }
    }

    // Seed every scanned path that produced no units (issue #701): a file
    // whose exports are all wrapped forms, or that has no extractable
    // functions at all, must still get a file entity. Paths that were
    // excluded via unit-file matching are skipped (same filter as above).
    for path in scanned_paths {
        if !should_exclude_path(path, exclude_paths) && !excluded_unit_files.contains(path.as_str())
        {
            file_map.entry(path.as_str()).or_default();
        }
    }

    let mut files = Vec::new();
    for (file_path, units) in file_map {
        // Units always agree on the language of the file they came from, so
        // the most-common language is the first unit's language; zero-unit
        // seed paths have no units and fall back to the extension.
        let language = match units.first() {
            Some(unit) => most_common_language(&[unit]),
            None => language_for_path(file_path),
        };
        let id = entity_id(project_id, repo_name, EntityTier::File, file_path)?;
        let name = file_path
            .rsplit('/')
            .next()
            .unwrap_or(file_path)
            .to_string();

        files.push(Entity {
            id,
            project_id: project_id.to_string(),
            repo_id: Some(repo_id.to_string()),
            tier: EntityTier::File,
            parent_id: None,
            name,
            path: Some(file_path.to_string()),
            language,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    files.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(files)
}

pub(crate) fn group_into_modules(
    files: &[Entity],
    subsystem_map: &HashMap<String, String>,
    module_depth: usize,
    project_id: &str,
    repo_name: &str,
    repo_id: &str,
) -> Result<(Vec<Entity>, Vec<Entity>)> {
    let mut module_map: HashMap<String, Vec<&Entity>> = HashMap::new();
    let mut file_to_module: HashMap<String, String> = HashMap::new();

    for file in files {
        // Invariant: file entities are always created with path: Some(...) in group_into_files
        let file_path = file.path.as_ref().ok_or_else(|| {
            crate::error::LievoError::EntityNotFound(format!(
                "entity '{}' missing path field",
                file.id
            ))
        })?;
        if let Some((subsystem_path, _)) = find_subsystem_for_file(file_path, subsystem_map) {
            let module_path = determine_module_path(file_path, &subsystem_path, module_depth);
            file_to_module.insert(file.id.clone(), module_path.clone());
            module_map.entry(module_path).or_default().push(file);
        } else {
            file_to_module.insert(file.id.clone(), ".".to_string());
            module_map.entry(".".to_string()).or_default().push(file);
        }
    }

    let mut modules = Vec::new();
    for (module_path, _) in module_map {
        let id = entity_id(project_id, repo_name, EntityTier::Module, &module_path)?;
        let name = if module_path == "." {
            "root".to_string()
        } else {
            module_path
                .split('/')
                .next_back()
                .unwrap_or(&module_path)
                .to_string()
        };
        let parent_path = find_parent_subsystem_for_module(&module_path, subsystem_map);
        let parent_id = if subsystem_map.contains_key(&parent_path) {
            Some(entity_id(
                project_id,
                repo_name,
                EntityTier::Subsystem,
                &parent_path,
            )?)
        } else {
            None
        };

        modules.push(Entity {
            id,
            project_id: project_id.to_string(),
            repo_id: Some(repo_id.to_string()),
            tier: EntityTier::Module,
            parent_id,
            name,
            path: Some(module_path),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    modules.sort_by(|a, b| a.id.cmp(&b.id));

    let mut updated_files: Vec<Entity> = files
        .iter()
        .map(|file| {
            let mut f = file.clone();
            if let Some(module_path) = file_to_module.get(&file.id) {
                let parent_id = entity_id(project_id, repo_name, EntityTier::Module, module_path)?;
                f.parent_id = Some(parent_id);
            }
            Ok(f)
        })
        .collect::<Result<Vec<_>>>()?;

    updated_files.sort_by(|a, b| a.id.cmp(&b.id));

    Ok((modules, updated_files))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_module_has_no_parent_id_when_dot_not_in_subsystem_map() {
        // Arrange: a file at the root level, subsystem_map has no "." entry
        let project_id = "test-project";
        let repo_name = "test-repo";
        let repo_id = "test-repo-id";
        let file_id = entity_id(project_id, repo_name, EntityTier::File, "main.py").unwrap();
        let files = vec![Entity {
            id: file_id,
            project_id: project_id.to_string(),
            repo_id: Some(repo_id.to_string()),
            tier: EntityTier::File,
            parent_id: None,
            name: "main.py".to_string(),
            path: Some("main.py".to_string()),
            language: Some("Python".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }];
        // subsystem_map does NOT contain "." — simulates Python project with named packages
        let subsystem_map = HashMap::from([("myapp".to_string(), "myapp".to_string())]);

        // Act
        let (modules, _updated_files) =
            group_into_modules(&files, &subsystem_map, 1, project_id, repo_name, repo_id).unwrap();

        // Assert: the root "." module should have parent_id: None (deterministic)
        let root_module = modules.iter().find(|m| m.path.as_deref() == Some("."));
        assert!(
            root_module.is_some(),
            "Expected a root '.' module to be created"
        );
        assert!(
            root_module.unwrap().parent_id.is_none(),
            "Root module should have parent_id: None when '.' not in subsystem_map"
        );
    }

    #[test]
    fn test_python_package_with_root_files_hierarchy_integrity() {
        // Arrange: Python project with myapp package + root-level config.py
        // subsystem_map has "myapp" but no "." (avoiding catch-all matcher)
        let project_id = "test-project";
        let repo_name = "test-repo";
        let repo_id = "test-repo-id";
        let config_id = entity_id(project_id, repo_name, EntityTier::File, "config.py").unwrap();
        let app_id = entity_id(project_id, repo_name, EntityTier::File, "myapp/main.py").unwrap();
        let files = vec![
            Entity {
                id: config_id,
                project_id: project_id.to_string(),
                repo_id: Some(repo_id.to_string()),
                tier: EntityTier::File,
                parent_id: None,
                name: "config.py".to_string(),
                path: Some("config.py".to_string()),
                language: Some("Python".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            Entity {
                id: app_id,
                project_id: project_id.to_string(),
                repo_id: Some(repo_id.to_string()),
                tier: EntityTier::File,
                parent_id: None,
                name: "main.py".to_string(),
                path: Some("myapp/main.py".to_string()),
                language: Some("Python".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        ];
        // Simulate Python detection with named packages only (no ".")
        let subsystem_map = HashMap::from([("myapp".to_string(), "myapp".to_string())]);

        // Act
        let (modules, _updated_files) =
            group_into_modules(&files, &subsystem_map, 1, project_id, repo_name, repo_id).unwrap();

        // Assert: root module "." should have parent_id: None, "myapp" module should have parent
        let root_module = modules.iter().find(|m| m.path.as_deref() == Some("."));
        let myapp_module = modules.iter().find(|m| m.path.as_deref() == Some("myapp"));

        assert!(
            root_module.is_some(),
            "Expected a root '.' module to be created for config.py"
        );
        assert!(
            root_module.unwrap().parent_id.is_none(),
            "Root module '.' should have parent_id: None when not in subsystem_map (deterministic)"
        );

        assert!(
            myapp_module.is_some(),
            "Expected a 'myapp' module to be created for myapp/main.py"
        );
        assert!(
            myapp_module.unwrap().parent_id.is_some(),
            "myapp module should have parent_id: Some (links to myapp subsystem)"
        );
    }

    #[test]
    fn test_module_has_parent_id_when_subsystem_exists() {
        // Arrange: a file inside src/, subsystem_map has "src" entry
        let project_id = "test-project";
        let repo_name = "test-repo";
        let repo_id = "test-repo-id";
        let file_id = entity_id(project_id, repo_name, EntityTier::File, "src/main.rs").unwrap();
        let files = vec![Entity {
            id: file_id,
            project_id: project_id.to_string(),
            repo_id: Some(repo_id.to_string()),
            tier: EntityTier::File,
            parent_id: None,
            name: "main.rs".to_string(),
            path: Some("src/main.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }];
        let subsystem_map = HashMap::from([("src".to_string(), "src".to_string())]);

        // Act
        let (modules, _updated_files) =
            group_into_modules(&files, &subsystem_map, 1, project_id, repo_name, repo_id).unwrap();

        // Assert: the src module should have parent_id: Some(...)
        let src_module = modules.iter().find(|m| m.path.as_deref() == Some("src"));
        assert!(
            src_module.is_some(),
            "Expected a 'src' module to be created"
        );
        assert!(
            src_module.unwrap().parent_id.is_some(),
            "src module parent_id should be Some when 'src' is in subsystem_map"
        );
    }

    #[test]
    fn test_deterministic_parent_assignment_no_hashmap_order_dependency() {
        // Arrange: Python project with multiple named packages + root file
        let project_id = "test-project";
        let repo_name = "test-repo";
        let repo_id = "test-repo-id";
        let main_id = entity_id(project_id, repo_name, EntityTier::File, "main.py").unwrap();
        let app_id = entity_id(project_id, repo_name, EntityTier::File, "app/main.py").unwrap();
        let utils_id =
            entity_id(project_id, repo_name, EntityTier::File, "utils/helper.py").unwrap();
        let files = vec![
            Entity {
                id: main_id,
                project_id: project_id.to_string(),
                repo_id: Some(repo_id.to_string()),
                tier: EntityTier::File,
                parent_id: None,
                name: "main.py".to_string(),
                path: Some("main.py".to_string()),
                language: Some("Python".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            Entity {
                id: app_id,
                project_id: project_id.to_string(),
                repo_id: Some(repo_id.to_string()),
                tier: EntityTier::File,
                parent_id: None,
                name: "main.py".to_string(),
                path: Some("app/main.py".to_string()),
                language: Some("Python".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            Entity {
                id: utils_id,
                project_id: project_id.to_string(),
                repo_id: Some(repo_id.to_string()),
                tier: EntityTier::File,
                parent_id: None,
                name: "helper.py".to_string(),
                path: Some("utils/helper.py".to_string()),
                language: Some("Python".to_string()),
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        ];

        // Create subsystem_maps with different insertion orders (HashMap iteration order varies)
        let subsystem_map_1 = {
            let mut map = HashMap::new();
            map.insert("app".to_string(), "app".to_string());
            map.insert("utils".to_string(), "utils".to_string());
            map
        };

        let subsystem_map_2 = {
            let mut map = HashMap::new();
            map.insert("utils".to_string(), "utils".to_string());
            map.insert("app".to_string(), "app".to_string());
            map
        };

        // Act: run with both maps
        let (modules_1, _) =
            group_into_modules(&files, &subsystem_map_1, 1, project_id, repo_name, repo_id)
                .unwrap();
        let (modules_2, _) =
            group_into_modules(&files, &subsystem_map_2, 1, project_id, repo_name, repo_id)
                .unwrap();

        // Sort modules by ID for consistent comparison
        let mut sorted_1 = modules_1.clone();
        let mut sorted_2 = modules_2.clone();
        sorted_1.sort_by(|a, b| a.path.cmp(&b.path));
        sorted_2.sort_by(|a, b| a.path.cmp(&b.path));

        // Assert: results must be identical regardless of HashMap iteration order
        assert_eq!(sorted_1.len(), sorted_2.len(), "Module counts should match");

        for (m1, m2) in sorted_1.iter().zip(sorted_2.iter()) {
            assert_eq!(m1.path, m2.path, "Module paths should match");
            assert_eq!(
                m1.parent_id, m2.parent_id,
                "Module parent_id should be deterministic (not dependent on HashMap order) for path {:?}",
                m1.path
            );
        }

        // Verify root module has no parent (deterministic, not random first subsystem)
        let root_m1 = sorted_1.iter().find(|m| m.path.as_deref() == Some("."));
        let root_m2 = sorted_2.iter().find(|m| m.path.as_deref() == Some("."));
        assert!(root_m1.is_some(), "Should have root module");
        assert!(root_m2.is_some(), "Should have root module");
        assert_eq!(
            root_m1.unwrap().parent_id,
            None,
            "Root module should have no parent (deterministic)"
        );
        assert_eq!(
            root_m2.unwrap().parent_id,
            None,
            "Root module should have no parent (deterministic)"
        );
    }
}
