// Grouping heuristic - clusters extracted code units into entity hierarchy
use crate::config::RepoConfig;
use crate::error::Result;
use crate::extraction::detectors::detect_subsystems;
use crate::extraction::grouping_helpers::create_subsystem_entities;
use crate::extraction::grouping_impl::{group_into_files, group_into_modules};
use crate::model::{CodeUnit, Entity, Relationship};
use std::path::Path;

/// Configuration for the grouping heuristic
pub struct GroupingConfig<'a> {
    pub code_units: &'a [CodeUnit],
    /// Every source file the extractor scanned during the last index() call —
    /// including files that produced zero code units (issue #701). Grouping
    /// seeds file entities for these paths so a file with no extractable
    /// functions still gets a file entity. `&[]` for extractors that cannot
    /// report their scanned set (existing behavior: file entities only for
    /// files with units).
    pub scanned_file_paths: &'a [String],
    pub project_id: &'a str,
    pub repo_name: &'a str,
    pub repo_id: &'a str,
    pub repo_path: &'a Path,
    pub config: Option<&'a RepoConfig>,
    pub exclude_paths: &'a [String],
}

/// Result of the grouping heuristic
#[derive(Debug)]
pub struct GroupingResult {
    pub subsystems: Vec<Entity>,
    pub modules: Vec<Entity>,
    pub files: Vec<Entity>,
    pub preserve_function_entities: bool,
}

/// Main grouping function - clusters code units into entities.
/// # Arguments
/// * `config` - GroupingConfig containing all parameters
/// # Returns
/// GroupingResult with entities at three tiers
pub fn group_code_units(config: &GroupingConfig) -> Result<GroupingResult> {
    let file_entities = group_into_files(
        config.code_units,
        config.scanned_file_paths,
        config.project_id,
        config.repo_name,
        config.repo_id,
        config.exclude_paths,
    )?;
    let (subsystem_map, detected_module_depth, _framework_profile) =
        detect_subsystems(config.repo_path)?;

    // Merge config subsystems overrides into subsystem_map (config takes precedence)
    let mut subsystem_map = subsystem_map;
    if let Some(repo_config) = config.config {
        for subsystem in &repo_config.subsystems {
            for path in &subsystem.paths {
                subsystem_map.insert(path.clone(), subsystem.name.clone());
            }
        }
    }

    let module_depth = config
        .config
        .and_then(|c| c.module_depth)
        .unwrap_or(detected_module_depth);
    let (module_entities, updated_file_entities) = group_into_modules(
        &file_entities,
        &subsystem_map,
        module_depth,
        config.project_id,
        config.repo_name,
        config.repo_id,
    )?;
    let subsystem_entities = create_subsystem_entities(
        &subsystem_map,
        &module_entities,
        config.project_id,
        config.repo_name,
        config.repo_id,
    )?;

    let preserve_function_entities = config
        .config
        .as_ref()
        .is_none_or(|c| c.preserve_function_entities);

    Ok(GroupingResult {
        subsystems: subsystem_entities,
        modules: module_entities,
        files: updated_file_entities,
        preserve_function_entities,
    })
}

/// Extract function entities and relationships when preserve_function_entities is enabled.
/// Maps code units to files and delegates to function_preservation module.
/// This is the orchestration layer where filesystem I/O (cfg(test) scanning) happens.
/// The transformation layer (preserve_functions) receives the pre-extracted cfg(test)
/// function names and performs only pure transformations.
pub fn extract_function_entities(
    code_units: &[CodeUnit],
    file_entities: &[Entity],
    repo_root: &Path,
) -> (Vec<Entity>, Vec<Relationship>) {
    use crate::extraction::function_preservation::{
        extract_cfg_test_functions, preserve_functions,
    };
    use std::collections::HashSet;
    let mut functions = Vec::new();
    let mut relationships = Vec::new();
    for file in file_entities {
        if let Some(file_path) = &file.path {
            let units: Vec<&CodeUnit> =
                code_units.iter().filter(|u| u.file == *file_path).collect();
            let unit_refs: Vec<CodeUnit> = units.iter().map(|&u| u.clone()).collect();

            // Extract cfg(test) function names from Rust source files (I/O happens here)
            let cfg_test_fns = if file_path.ends_with(".rs") {
                extract_cfg_test_functions(file_path, repo_root)
            } else {
                HashSet::new()
            };

            let results = preserve_functions(file, &unit_refs, &cfg_test_fns);
            for (entity, rel) in results {
                functions.push(entity);
                relationships.push(rel);
            }
        }
    }
    (functions, relationships)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
