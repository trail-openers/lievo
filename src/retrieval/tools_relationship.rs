// Relationship tool implementation.

use serde_json::{Value, json};

use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

use super::super::ListRelationshipsTool;
use super::tools_search::{lock_storage, should_exclude_entity};
use crate::model::RelType;
use crate::retrieval::project_boundary::same_project;

// ---------------------------------------------------------------------------
// ListRelationshipsTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for ListRelationshipsTool<S> {
    fn name(&self) -> &str {
        "list_relationships"
    }

    fn description(&self) -> &str {
        "List what an entity depends on and what depends on it. Useful for coupling and impact analysis. Works on any entity type. rel_type values: 'imports' = file or function uses/depends on another file; 'calls' = function calls another function (function-level only); 'contains' = parent owns child (module→file); 'depends_on' = aggregate dependency (module/subsystem tier); 'implements' = implements interface. The response also carries `unresolved_imports` (count of internal imports in the entity's repo that failed resolution; null when not recorded) and `resolution_coverage` (null when unknown). Each edge also carries `provenance` (\"resolved\" or \"heuristic\") — resolved edges come from exact import-resolution or structural grouping; heuristic edges from name/fn-map matching. An empty `depended_by` together with `unresolved_imports > 0` does NOT mean dead code — absence of dependents may reflect unresolved imports."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "entity_id": {
                    "type": "string",
                    "description": "Entity ID from search_entities or list_subsystems results."
                }
            },
            "required": ["entity_id"]
        })
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let entity_id = input
            .get("entity_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::LievoError::InvalidInput("missing 'entity_id'".into()))?;

        let guard = lock_storage!(self.ctx.storage);
        // guard holds the mutex lock for both get_entity and relationships_from — no TOCTOU possible
        let entity = guard
            .get_entity(entity_id)?
            .ok_or_else(|| crate::LievoError::EntityNotFound(entity_id.to_string()))?;

        // #690: repo-scoped unresolved-import signal. Entity without a repo,
        // or repo with no recorded counts (pre-#681), degrades to null — never
        // a project-wide aggregate and never a false full-coverage claim.
        let signal = crate::query::dependency::ResolutionSignal::from_counts(
            entity
                .repo_id
                .as_deref()
                .and_then(|rid| guard.get_unresolved_counts(rid)),
        );

        // Project boundary (issue #764): the relationships table has no
        // project_id column, so both traversals filter their endpoints
        // against the entity's own project — a cross-project edge must not
        // surface a foreign entity in either direction.
        let project_id = entity.project_id.clone();
        let relationships = guard.relationships_from(entity_id)?;
        let (imports, children): (Vec<_>, Vec<_>) = relationships
            .into_iter()
            .filter(|(_, target)| {
                same_project(&project_id, &target.project_id)
                    && !should_exclude_entity(target.path.as_deref(), &self.ctx.output_dir)
            })
            .partition(|(rel, _)| rel.rel_type != RelType::Contains);

        let imports_json: Vec<Value> = imports
            .into_iter()
            .map(|(rel, target)| {
                json!({
                    "name": target.name,
                    "entity_id": rel.target_id,
                    "rel_type": rel.rel_type.to_string(),
                    "path": target.path,
                    "tier": target.tier.to_string(),
                    "provenance": rel.provenance.to_string()
                })
            })
            .collect();

        let children_json: Vec<Value> = children
            .into_iter()
            .map(|(rel, target)| {
                json!({
                    "name": target.name,
                    "entity_id": rel.target_id,
                    "rel_type": rel.rel_type.to_string(),
                    "path": target.path,
                    "tier": target.tier.to_string(),
                    "provenance": rel.provenance.to_string()
                })
            })
            .collect();

        let depended_by: Vec<Value> = guard
            .relationships_to(entity_id)?
            .into_iter()
            .filter(|(_, source)| {
                same_project(&project_id, &source.project_id)
                    && !should_exclude_entity(source.path.as_deref(), &self.ctx.output_dir)
            })
            .map(|(rel, source)| {
                json!({
                    "name": source.name,
                    "entity_id": rel.source_id,
                    "rel_type": rel.rel_type.to_string(),
                    "path": source.path,
                    "tier": source.tier.to_string(),
                    "provenance": rel.provenance.to_string()
                })
            })
            .collect();

        Ok(json!({
            "imports": imports_json,
            "children": children_json,
            "depended_by": depended_by,
            // #690 additive fields (see ResolutionSignal docs for semantics).
            "unresolved_imports": signal.unresolved_internal.map(serde_json::Value::from).unwrap_or(serde_json::Value::Null),
            "resolution_coverage": crate::retrieval::tools::tools_impl::tools_query::resolution_coverage_value(&signal),
        })
        .to_string())
    }
}
