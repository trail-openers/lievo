// Query command handlers — expose query functions through the CLI.
//
// Each function corresponds to one `lievo query <subcommand>` and prints
// to stdout using the requested OutputFormat. Errors bubble up as LievoError.

use lievo::model::EntityTier;
use lievo::output::{
    OutputFormat, ResolutionContext, format_conventions_human, format_conventions_json,
    format_entities_human, format_entities_json, format_relationships_human,
    format_relationships_json,
};
use lievo::query::{dependency, entity_queries};
use lievo::storage::Storage;
use lievo::{LievoError, Result};

use lievo::project_resolution::resolve_project_id;

// ---------------------------------------------------------------------------
// query subsystems
// ---------------------------------------------------------------------------

pub fn subsystems(
    storage: &dyn Storage,
    project_name: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    let project_id = resolve_project_id(storage, project_name)?;
    let items = entity_queries::subsystems(storage, &project_id)?;

    match format {
        OutputFormat::Human => println!("{}", format_entities_human(&items, EntityTier::Subsystem)),
        OutputFormat::Json => format_entities_json(&items, &mut std::io::stdout())?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// query modules
// ---------------------------------------------------------------------------

pub fn modules(storage: &dyn Storage, subsystem_id: &str, format: OutputFormat) -> Result<()> {
    let items = entity_queries::modules_in(storage, subsystem_id)?;

    match format {
        OutputFormat::Human => println!("{}", format_entities_human(&items, EntityTier::Module)),
        OutputFormat::Json => format_entities_json(&items, &mut std::io::stdout())?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// query files
// ---------------------------------------------------------------------------

pub fn files(storage: &dyn Storage, module_id: &str, format: OutputFormat) -> Result<()> {
    let items = entity_queries::files_in(storage, module_id)?;

    match format {
        OutputFormat::Human => println!("{}", format_entities_human(&items, EntityTier::File)),
        OutputFormat::Json => format_entities_json(&items, &mut std::io::stdout())?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// query deps
// ---------------------------------------------------------------------------

pub fn deps(storage: &dyn Storage, entity_id: &str, format: OutputFormat) -> Result<()> {
    let items = dependency::dependencies_of(storage, entity_id)?;
    let resolution = resolution_context(storage, entity_id);

    let direction = format!("Dependencies of {entity_id}");
    match format {
        OutputFormat::Human => println!(
            "{}",
            format_relationships_human(&items, &direction, &resolution)
        ),
        OutputFormat::Json => {
            format_relationships_json(&items, &resolution, &mut std::io::stdout())?
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// query dependents
// ---------------------------------------------------------------------------

pub fn dependents(storage: &dyn Storage, entity_id: &str, format: OutputFormat) -> Result<()> {
    let items = dependency::dependents_of(storage, entity_id)?;
    let resolution = resolution_context(storage, entity_id);

    let direction = format!("Dependents of {entity_id}");
    match format {
        OutputFormat::Human => println!(
            "{}",
            format_relationships_human(&items, &direction, &resolution)
        ),
        OutputFormat::Json => {
            format_relationships_json(&items, &resolution, &mut std::io::stdout())?
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// query impact
// ---------------------------------------------------------------------------

/// Impact analysis for changed files. Delegates to `query_impact::impact`
/// (split out to keep this module under the 500-line limit).
pub use super::query_impact::impact;

// ---------------------------------------------------------------------------
// query hotspots
// ---------------------------------------------------------------------------

pub fn hotspots(
    storage: &dyn Storage,
    project_name: Option<&str>,
    limit: usize,
    format: OutputFormat,
) -> Result<()> {
    let project_id = resolve_project_id(storage, project_name)?;
    let items = entity_queries::hotspots(storage, &project_id, limit)?;

    // Hotspots are entities at any tier — use File tier for column layout
    // (complexity is the primary metric; tier is shown per-row)
    match format {
        OutputFormat::Human => {
            if items.is_empty() {
                println!("No hotspots found.");
            } else {
                println!("{}", format_entities_human(&items, EntityTier::File));
            }
        }
        OutputFormat::Json => format_entities_json(&items, &mut std::io::stdout())?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// query conventions
// ---------------------------------------------------------------------------

/// List conventions for a project, optionally filtered by category.
///
/// Returns an error if `category` is not one of the recognised convention categories.
pub fn conventions(
    storage: &dyn Storage,
    project_name: Option<&str>,
    category: Option<&str>,
    format: OutputFormat,
) -> Result<()> {
    const VALID_CATEGORIES: &[&str] = &[
        "naming",
        "error_handling",
        "testing",
        "architecture",
        "documentation",
    ];
    if let Some(cat) = category
        && !VALID_CATEGORIES.contains(&cat)
    {
        return Err(LievoError::InvalidInput(format!(
            "Unknown convention category '{}'. Valid: {:?}",
            cat, VALID_CATEGORIES
        )));
    }
    let project_id = resolve_project_id(storage, project_name)?;
    let items = storage.list_conventions(&project_id, category)?;

    match format {
        OutputFormat::Human => println!("{}", format_conventions_human(&items)),
        OutputFormat::Json => format_conventions_json(&items, &mut std::io::stdout())?,
    }
    Ok(())
}

/// Build a `ResolutionContext` for the entity's repo scope by reading the
/// persisted unresolved-import counts (#681 side-channel).
///
/// Returns `ResolutionContext::unknown()` when the entity has no repo
/// (e.g. pre-#681 data) or the repo has no recorded counts.
fn resolution_context(storage: &dyn Storage, entity_id: &str) -> ResolutionContext {
    let Some(entity) = storage.get_entity(entity_id).ok().flatten() else {
        return ResolutionContext::unknown();
    };
    let Some(repo_id) = entity.repo_id else {
        return ResolutionContext::unknown();
    };
    match storage.get_unresolved_counts(&repo_id) {
        Some((internal, external)) => ResolutionContext {
            unresolved_internal: Some(internal),
            unresolved_external: Some(external),
        },
        None => ResolutionContext::unknown(),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use lievo::model::{Convention, Entity, EntityTier};
    use lievo::storage::sqlite::SqliteStorage;

    #[test]
    fn test_resolve_project_id_by_name() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("my-proj", None).unwrap();
        let id = resolve_project_id(&storage, Some("my-proj")).unwrap();
        assert_eq!(id, project.id);
    }

    #[test]
    fn test_resolve_project_id_unknown_name_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let result = resolve_project_id(&storage, Some("ghost"));
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_project_id_no_name_single_project_auto_resolves() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("only-proj", None).unwrap();
        let id = resolve_project_id(&storage, None).unwrap();
        assert_eq!(id, project.id);
    }

    #[test]
    fn test_resolve_project_id_no_name_multiple_projects_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage.create_project("proj-a", None).unwrap();
        storage.create_project("proj-b", None).unwrap();
        let result = resolve_project_id(&storage, None);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("specify --project"), "got: {msg}");
    }

    #[test]
    fn test_resolve_project_id_no_name_no_projects_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let result = resolve_project_id(&storage, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_project_id_empty_name_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let result = resolve_project_id(&storage, Some(""));
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_project_id_whitespace_only_name_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let result = resolve_project_id(&storage, Some("   "));
        assert!(result.is_err());
    }

    fn insert_convention(storage: &dyn Storage, project_id: &str, category: &str) {
        let conv = Convention {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.to_string(),
            category: category.to_string(),
            title: format!("{category} convention"),
            description: Some("A convention".to_string()),
            example_code: None,
            confidence: 0.8,
            entity_ids_json: None,
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            still_valid: true,
        };
        storage.upsert_convention(&conv).unwrap();
    }

    #[test]
    fn test_conventions_returns_all_when_no_category_filter() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test-proj", None).unwrap();
        insert_convention(&storage, &project.id, "naming");
        insert_convention(&storage, &project.id, "testing");

        let result = conventions(&storage, Some("test-proj"), None, OutputFormat::Human);
        assert!(result.is_ok());
    }

    #[test]
    fn test_conventions_filters_by_category() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test-proj", None).unwrap();
        insert_convention(&storage, &project.id, "naming");
        insert_convention(&storage, &project.id, "testing");

        let items = storage
            .list_conventions(&project.id, Some("naming"))
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].category, "naming");
    }

    #[test]
    fn test_conventions_empty_project_shows_no_conventions() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let _project = storage.create_project("empty-proj", None).unwrap();

        let result = conventions(&storage, Some("empty-proj"), None, OutputFormat::Human);
        assert!(result.is_ok());
    }

    #[test]
    fn test_conventions_json_format() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("test-proj", None).unwrap();
        insert_convention(&storage, &project.id, "architecture");

        let result = conventions(&storage, Some("test-proj"), None, OutputFormat::Json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_conventions_unknown_project_fails() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let result = conventions(&storage, Some("no-such-proj"), None, OutputFormat::Human);
        assert!(result.is_err());
    }

    #[test]
    fn test_conventions_unknown_category_returns_error() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage.create_project("test-proj", None).unwrap();
        let result = conventions(
            &storage,
            Some("test-proj"),
            Some("bogus_cat"),
            OutputFormat::Human,
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("bogus_cat"),
            "error should mention the invalid category, got: {msg}"
        );
    }

    // -----------------------------------------------------------------------
    // resolution_context tests (#690)
    // -----------------------------------------------------------------------

    fn make_file_entity(id: &str, project_id: &str, repo_id: Option<&str>) -> Entity {
        Entity {
            id: id.to_string(),
            project_id: project_id.to_string(),
            repo_id: repo_id.map(|s| s.to_string()),
            tier: EntityTier::File,
            parent_id: None,
            name: id.to_string(),
            path: Some("src/test.rs".to_string()),
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_resolution_context_unknown_for_entity_without_repo() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let entity = make_file_entity("e1", &project.id, None);
        storage.upsert_entity(&entity).unwrap();

        let ctx = resolution_context(&storage, "e1");
        assert_eq!(ctx, ResolutionContext::unknown());
    }

    #[test]
    fn test_resolution_context_unknown_for_repo_without_counts() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let repo = storage.add_repo(&project.id, "repo", "/path").unwrap();
        let entity = make_file_entity("e1", &project.id, Some(&repo.id));
        storage.upsert_entity(&entity).unwrap();

        // No repo file entity with metrics_json → get_unresolved_counts returns None
        let ctx = resolution_context(&storage, "e1");
        assert_eq!(ctx, ResolutionContext::unknown());
    }

    #[test]
    fn test_resolution_context_reads_persisted_counts() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let repo = storage.add_repo(&project.id, "repo", "/path").unwrap();

        // Record the counts on the repository row's columns (#856)
        storage.record_unresolved_counts(&repo.id, 5, 12).unwrap();

        // Insert a regular file entity in the same repo
        let entity = make_file_entity("e1", &project.id, Some(&repo.id));
        storage.upsert_entity(&entity).unwrap();

        let ctx = resolution_context(&storage, "e1");
        assert_eq!(ctx.unresolved_internal, Some(5));
        assert_eq!(ctx.unresolved_external, Some(12));
        assert!(ctx.has_caveat());
    }

    #[test]
    fn test_resolution_context_zero_counts_no_caveat() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project("proj", None).unwrap();
        let repo = storage.add_repo(&project.id, "repo", "/path").unwrap();

        // Recorded zeros must still read as Some((0, 3)) — never None (#856)
        storage.record_unresolved_counts(&repo.id, 0, 3).unwrap();

        let entity = make_file_entity("e1", &project.id, Some(&repo.id));
        storage.upsert_entity(&entity).unwrap();

        let ctx = resolution_context(&storage, "e1");
        assert_eq!(ctx.unresolved_internal, Some(0));
        assert!(
            !ctx.has_caveat(),
            "zero internal unresolved must not trigger caveat"
        );
    }

    #[test]
    fn test_resolution_context_nonexistent_entity_returns_unknown() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let ctx = resolution_context(&storage, "nonexistent");
        assert_eq!(ctx, ResolutionContext::unknown());
    }
}
