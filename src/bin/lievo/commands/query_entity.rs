use std::path::Path;

use lievo::model::Entity;
use lievo::output::{OutputFormat, ResolutionContext, format_relationships_human};
use lievo::project_resolution::resolve_project_id;
use lievo::retrieval::project_boundary::same_project;
use lievo::storage::Storage;
use lievo::{LievoError, Result};

use serde_json::json;

const DEFAULT_SEARCH_LIMIT: usize = 10;
const SUMMARY_SNIPPET_LEN: usize = 200;
const MAX_ENTITY_RECURSION_DEPTH: u32 = 5;

pub fn entities(
    storage: &dyn Storage,
    project_name: Option<&str>,
    query: &str,
    semantic: bool,
    format: OutputFormat,
) -> Result<()> {
    let project_id = resolve_project_id(storage, project_name)?;
    let items = if semantic {
        let (entities, warning) = search_entities_semantic(storage, &project_id, query)?;
        if let Some(warning) = warning {
            eprintln!("{warning}");
        }
        entities
    } else {
        search_entities(storage, &project_id, query)?
    };

    let output = match format {
        OutputFormat::Human => format_entities_human(&items),
        OutputFormat::Json => format_entities_json(&items),
    };
    println!("{output}");
    Ok(())
}

pub fn entity(storage: &dyn Storage, entity_id: &str, format: OutputFormat) -> Result<()> {
    let entity = storage
        .get_entity(entity_id)?
        .ok_or_else(|| LievoError::EntityNotFound(entity_id.to_string()))?;

    let output = match format {
        OutputFormat::Human => format_entity_human(&entity),
        OutputFormat::Json => format_entity_json(&entity),
    };
    println!("{output}");
    Ok(())
}
pub fn relationships(storage: &dyn Storage, entity_id: &str, format: OutputFormat) -> Result<()> {
    let entity = storage
        .get_entity(entity_id)?
        .ok_or(LievoError::EntityNotFound(entity_id.to_string()))?;

    // Project boundary (#764): filter cross-project edges against the entity's own project.
    let project_id = &entity.project_id;
    let mut depends_on = storage.relationships_from(entity_id)?;
    let mut depended_by = storage.relationships_to(entity_id)?;
    depends_on.retain(|(_, target)| same_project(project_id, &target.project_id));
    depended_by.retain(|(_, source)| same_project(project_id, &source.project_id));

    let resolution = ResolutionContext::unknown();
    match format {
        OutputFormat::Human => println!(
            "{}\n\n{}",
            format_relationships_human(&depends_on, "DEPENDS ON", &resolution),
            format_relationships_human(&depended_by, "DEPENDED BY", &resolution)
        ),
        OutputFormat::Json => {
            for rels in [&depends_on, &depended_by] {
                lievo::output::relationships::format_relationships_json(
                    rels,
                    &resolution,
                    &mut std::io::stdout(),
                )?;
            }
        }
    }
    Ok(())
}

pub fn children(storage: &dyn Storage, entity_id: &str, format: OutputFormat) -> Result<()> {
    let parent = storage
        .get_entity(entity_id)?
        .ok_or(LievoError::EntityNotFound(entity_id.to_string()))?;
    // Project boundary (#764): the parent's project scopes every child's edge display.
    let children = collect_children(storage, &parent.id, &parent.project_id)?;

    let output = match format {
        OutputFormat::Human => format_children_human(&parent, &children),
        OutputFormat::Json => format_children_json(Some(&parent), &children),
    };
    println!("{output}");
    Ok(())
}

fn search_entities(storage: &dyn Storage, project_id: &str, query: &str) -> Result<Vec<Entity>> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Err(LievoError::InvalidInput(
            "query cannot be empty".to_string(),
        ));
    }

    let mut matches: Vec<Entity> = storage
        .list_entities(project_id, None)?
        .into_iter()
        .filter(|entity| matches_query(entity, &needle))
        .collect();

    matches.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    matches.truncate(DEFAULT_SEARCH_LIMIT);
    Ok(matches)
}

/// Semantic entity search using the vector index built by `lievo refresh`.
///
/// Looks up the project's repository vector index, loads a semantic searcher,
/// and runs a direct vector search with path→entity resolution. Falls back to
/// keyword matching (with a warning) when no index is available.
fn search_entities_semantic(
    storage: &dyn Storage,
    project_id: &str,
    query: &str,
) -> Result<(Vec<Entity>, Option<String>)> {
    let repos = storage.list_repos(project_id)?;
    let index_path = repos.iter().find_map(|repo| repo.index_path.as_deref());

    let searcher: Option<Box<dyn lievo::retrieval::semantic_searcher::SemanticSearcher>> =
        match index_path {
            Some(index_path) => match lievo::retrieval::usearch_searcher::UsearchSearcher::load(
                Path::new(index_path),
            ) {
                Ok(searcher) => Some(Box::new(searcher)
                    as Box<dyn lievo::retrieval::semantic_searcher::SemanticSearcher>),
                Err(e) => {
                    tracing::warn!(
                        "Failed to load vector index at {index_path}: {e}. Falling back to keyword matching."
                    );
                    eprintln!(
                        "Semantic search unavailable: vector index at {index_path} could not be loaded. Results are from keyword matching only. Run `lievo refresh` to rebuild the vector index (rebuilds are triggered automatically when the on-disk index was built with a different embedding model)."
                    );
                    None
                }
            },
            None => None,
        };

    search_entities_semantic_with_searcher(storage, project_id, query, searcher)
}

/// Shared implementation of semantic entity search with an injectable
/// semantic searcher (used by tests to avoid loading the real index).
///
/// A `None` searcher forces the keyword-fallback path with its guidance
/// warning.
fn search_entities_semantic_with_searcher(
    storage: &dyn Storage,
    project_id: &str,
    query: &str,
    searcher: Option<Box<dyn lievo::retrieval::semantic_searcher::SemanticSearcher>>,
) -> Result<(Vec<Entity>, Option<String>)> {
    let needle = query.trim();
    if needle.is_empty() {
        return Err(LievoError::InvalidInput(
            "query cannot be empty".to_string(),
        ));
    }

    let Some(searcher) = searcher else {
        // keyword fallback
        let entities = search_entities(storage, project_id, needle)?;
        let warning = "Semantic search unavailable: no vector index found for this project or the index failed to load. Results are from keyword matching only. Run `lievo refresh` to build the vector index for full semantic search (it is rebuilt automatically when the index is missing or was built with a different embedding model)";
        return Ok((entities, Some(warning.to_string())));
    };

    // `resolve_semantic_hits` drops hits that resolve to no stored entity
    // before truncating to the limit, so every id we get back here is
    // fetchable via `get_entity` — no post-filter needed.
    let raw = searcher.search(needle, DEFAULT_SEARCH_LIMIT.saturating_mul(2))?;
    let entities = storage.list_entities(project_id, None)?;
    let results = lievo::retrieval::resolve_semantic_hits(raw, &entities, DEFAULT_SEARCH_LIMIT);

    let entities: Vec<Entity> = results
        .into_iter()
        .filter_map(|r| {
            storage
                .get_entity(&r.entity_id)
                .ok()?
                .filter(|entity| entity.id == r.entity_id)
        })
        .collect();

    Ok((entities, None))
}

fn matches_query(entity: &Entity, needle: &str) -> bool {
    entity.name.to_lowercase().contains(needle)
        || entity
            .path
            .as_deref()
            .map(|path| path.to_lowercase().contains(needle))
            .unwrap_or(false)
        || entity
            .summary
            .as_deref()
            .map(|summary| summary.to_lowercase().contains(needle))
            .unwrap_or(false)
}

#[derive(Debug)]
struct ChildDetails {
    entity: Entity,
    depends_on: Vec<(lievo::model::Relationship, Entity)>,
    depended_by: Vec<(lievo::model::Relationship, Entity)>,
}
fn collect_children(
    storage: &dyn Storage,
    entity_id: &str,
    project_id: &str,
) -> Result<Vec<ChildDetails>> {
    fn collect_recursive_children(
        storage: &dyn Storage,
        parent_id: &str,
        project_id: &str,
        depth: u32,
    ) -> Result<Vec<ChildDetails>> {
        let mut children = storage.entities_by_parent(parent_id)?;
        children.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });

        let mut result = Vec::with_capacity(children.len());
        for child in children {
            // Project boundary (#764): filter each child's edge lists against the parent's project.
            let mut depends_on = storage.relationships_from(&child.id)?;
            let mut depended_by = storage.relationships_to(&child.id)?;
            depends_on.retain(|(_, target)| same_project(project_id, &target.project_id));
            depended_by.retain(|(_, source)| same_project(project_id, &source.project_id));
            let child_details = ChildDetails {
                entity: child.clone(),
                depends_on,
                depended_by,
            };

            if depth > 0 && child_details.entity.tier != lievo::model::EntityTier::Function {
                let sub_children =
                    collect_recursive_children(storage, &child.id, project_id, depth - 1)?;
                result.extend(sub_children);
            }

            result.push(child_details);
        }
        Ok(result)
    }

    collect_recursive_children(storage, entity_id, project_id, MAX_ENTITY_RECURSION_DEPTH)
}

fn format_entities_human(entities: &[Entity]) -> String {
    if entities.is_empty() {
        return "No entities found.".to_string();
    }

    let name_w = entities
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let path_w = entities
        .iter()
        .map(|e| e.path.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(4)
        .max(4);
    let tier_w = entities
        .iter()
        .map(|e| e.tier.to_string().len())
        .max()
        .unwrap_or(4)
        .max(4);

    let header = format!(
        "  {:<name_w$}  {:<path_w$}  {:<tier_w$}  Summary",
        "Name", "Path", "Tier"
    );
    let sep = "━".repeat(header.chars().count());

    let mut lines = vec!["ENTITIES".to_string(), sep, header];
    for entity in entities {
        let path = entity.path.as_deref().unwrap_or("-");
        lines.push(format!(
            "  {:<name_w$}  {:<path_w$}  {:<tier_w$}  {}",
            entity.name.as_str(),
            path,
            entity.tier,
            truncate(
                entity.summary.as_deref().unwrap_or("-"),
                SUMMARY_SNIPPET_LEN
            )
        ));
    }
    lines.join("\n")
}

fn format_entities_json(entities: &[Entity]) -> String {
    json!(entities.iter().map(entity_json).collect::<Vec<_>>()).to_string()
}

fn format_entity_human(entity: &Entity) -> String {
    let mut lines = vec!["ENTITY".to_string(), "━".repeat(6)];
    lines.push(format!("  id: {}", entity.id.as_str()));
    lines.push(format!("  name: {}", entity.name.as_str()));
    lines.push(format!("  tier: {}", entity.tier));
    lines.push(format!("  path: {}", entity.path.as_deref().unwrap_or("-")));
    lines.push(format!(
        "  language: {}",
        entity.language.as_deref().unwrap_or("-")
    ));
    lines.push(format!("  project_id: {}", entity.project_id.as_str()));
    lines.push(format!(
        "  repo_id: {}",
        entity.repo_id.as_deref().unwrap_or("-")
    ));
    lines.push(format!(
        "  parent_id: {}",
        entity.parent_id.as_deref().unwrap_or("-")
    ));
    lines.push(format!(
        "  summary: {}",
        entity.summary.as_deref().unwrap_or("-")
    ));
    lines.push(format!(
        "  metrics_json: {}",
        entity.metrics_json.as_deref().unwrap_or("-")
    ));
    lines.push(format!("  created_at: {}", entity.created_at.as_str()));
    lines.push(format!("  updated_at: {}", entity.updated_at.as_str()));
    lines.join("\n")
}

fn format_entity_json(entity: &Entity) -> String {
    entity_json(entity).to_string()
}

fn entity_json(entity: &Entity) -> serde_json::Value {
    json!({
        "entity_id": entity.id.as_str(),
        "name": entity.name.as_str(),
        "path": entity.path.as_deref(),
        "tier": entity.tier.to_string(),
        "language": entity.language.as_deref(),
        "summary": entity.summary.as_deref(),
        "metrics_json": entity.metrics_json.as_deref(),
        "parent_id": entity.parent_id.as_deref(),
        "project_id": entity.project_id.as_str(),
        "repo_id": entity.repo_id.as_deref(),
        "created_at": entity.created_at.as_str(),
        "updated_at": entity.updated_at.as_str(),
    })
}

fn format_children_human(parent: &Entity, children: &[ChildDetails]) -> String {
    let mut lines = vec!["CHILDREN".to_string(), "━".repeat(8)];
    lines.push(format!(
        "  parent: {} ({})",
        parent.name.as_str(),
        parent.id.as_str()
    ));
    lines.push(format!("  path: {}", parent.path.as_deref().unwrap_or("-")));
    lines.push(format!(
        "  summary: {}",
        parent.summary.as_deref().unwrap_or("-")
    ));
    lines.push(format!("  child_count: {}", children.len()));

    for child in children {
        lines.push(String::new());
        lines.push(format!(
            "  - {} ({})",
            child.entity.name.as_str(),
            child.entity.tier
        ));
        lines.push(format!("    id: {}", child.entity.id.as_str()));
        lines.push(format!(
            "    path: {}",
            child.entity.path.as_deref().unwrap_or("-")
        ));
        lines.push(format!(
            "    summary: {}",
            child.entity.summary.as_deref().unwrap_or("-")
        ));
        lines.push(format_relationship_block(
            "depends_on",
            &child.depends_on,
            4,
        ));
        lines.push(format_relationship_block(
            "depended_by",
            &child.depended_by,
            4,
        ));
    }

    lines.join("\n")
}

fn format_children_json(parent: Option<&Entity>, children: &[ChildDetails]) -> String {
    json!({
        "parent": parent.map(entity_json),
        "children": children
            .iter()
            .map(|child| {
                json!({
                    "entity_id": child.entity.id.as_str(),
                    "name": child.entity.name.as_str(),
                    "path": child.entity.path.as_deref(),
                    "tier": child.entity.tier.to_string(),
                    "summary": child.entity.summary.as_deref(),
                    "metrics_json": child.entity.metrics_json.as_deref(),
                    "depends_on": relationships_to_json_array(&child.depends_on, true),
                    "depended_by": relationships_to_json_array(&child.depended_by, false),
                })
            })
            .collect::<Vec<_>>(),
        "child_count": children.len(),
    })
    .to_string()
}

fn relationships_to_json_array(
    rels: &[(lievo::model::Relationship, Entity)],
    is_outgoing: bool,
) -> Vec<serde_json::Value> {
    rels.iter()
        .map(|(rel, entity)| {
            json!({
                "name": entity.name.as_str(),
                "entity_id": if is_outgoing { rel.target_id.as_str() } else { rel.source_id.as_str() },
                "path": entity.path.as_deref(),
                "tier": entity.tier.to_string(),
                "rel_type": rel.rel_type.to_string(),
                "weight": rel.weight,
            })
        })
        .collect()
}

fn format_relationship_block(
    label: &str,
    rels: &[(lievo::model::Relationship, Entity)],
    indent: usize,
) -> String {
    let pad = " ".repeat(indent);
    if rels.is_empty() {
        return format!("{pad}{label}: (none)");
    }

    let mut lines = vec![format!("{pad}{label}:")];
    for (rel, entity) in rels {
        lines.push(format!(
            "{pad}  - {} ({}, weight {:.1})",
            entity.name.as_str(),
            rel.rel_type,
            rel.weight
        ));
    }
    lines.join("\n")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }

    let cut = s
        .char_indices()
        .take_while(|(i, _)| *i <= max.saturating_sub(3))
        .last()
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("{}...", &s[..cut])
}

#[cfg(test)]
#[path = "query_entity_tests.rs"]
mod tests;
