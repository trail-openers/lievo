// Entity tool implementations.

use serde_json::{Value, json};

use crate::model::EntityTier;
use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

use super::super::{GetEntityTool, ListSubsystemsTool};
use super::tools_search::{lock_storage, should_exclude_entity, truncate};

// GetEntityTool helpers

fn build_module_children_response<S: Storage>(
    children: Vec<crate::model::Entity>,
    storage: &S,
    output_dir: &Option<String>,
) -> Result<Vec<Value>, crate::LievoError> {
    let mut children_json: Vec<Value> = Vec::new();

    for child in children {
        if should_exclude_entity(child.path.as_deref(), output_dir) {
            continue;
        }

        // For File-tier children, fetch and include their Function children
        let child_obj = if child.tier == EntityTier::File {
            let grandchildren = storage.entities_by_parent(&child.id).unwrap_or_default();
            let total_count = grandchildren.len();

            let functions: Vec<_> = grandchildren
                .into_iter()
                .filter(|e| e.tier == EntityTier::Function)
                .take(50)
                .collect();

            let functions_json: Vec<Value> = functions
                .into_iter()
                .filter(|f| !should_exclude_entity(f.path.as_deref(), output_dir))
                .map(|f| {
                    json!({
                        "id": f.id,
                        "name": f.name,
                        "tier": f.tier.to_string(),
                        "path": f.path
                    })
                })
                .collect();

            let mut obj = json!({
                "id": child.id,
                "name": child.name,
                "tier": child.tier.to_string(),
                "path": child.path
            });

            if !functions_json.is_empty() {
                obj["functions"] = json!(functions_json);
            }

            if total_count > 50 {
                obj["functions_truncated"] = json!(true);
            }

            obj
        } else {
            // Non-File children: simple object without grandchildren
            json!({
                "id": child.id,
                "name": child.name,
                "tier": child.tier.to_string(),
                "path": child.path
            })
        };

        children_json.push(child_obj);
    }

    Ok(children_json)
}

// ---------------------------------------------------------------------------
// GetEntityTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for GetEntityTool<S> {
    fn name(&self) -> &str {
        "get_entity"
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let entity_id = input
            .get("entity_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::LievoError::InvalidInput("missing 'entity_id'".into()))?;

        let include_children = input
            .get("include_children")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let guard = lock_storage!(self.ctx.storage);
        match guard.get_entity(entity_id)? {
            Some(e) => {
                if should_exclude_entity(e.path.as_deref(), &self.ctx.output_dir) {
                    return Ok(json!({
                        "error": "This entity is the generated documentation output directory and should not be documented"
                    })
                    .to_string());
                }

                // Parse metrics_json to a JSON value. If null or invalid, use Null.
                let metrics = e
                    .metrics_json
                    .as_ref()
                    .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
                    .unwrap_or(serde_json::Value::Null);

                let mut response = json!({
                    "entity_id": e.id,
                    "name": e.name,
                    "path": e.path,
                    "tier": e.tier.to_string(),
                    "language": e.language,
                    "summary": e.summary,
                    "metrics": metrics,
                    "parent_id": e.parent_id,
                    "created_at": e.created_at,
                    "updated_at": e.updated_at
                });

                if include_children {
                    let children = guard.entities_by_parent(entity_id)?;

                    // For Module-tier entities, nest Function grandchildren under File children
                    if e.tier == EntityTier::Module {
                        let children_json = build_module_children_response(
                            children,
                            &*guard,
                            &self.ctx.output_dir,
                        )?;
                        let actual_child_count = children_json.len();
                        response["children"] = json!(children_json);
                        response["child_count"] = json!(actual_child_count);
                    } else {
                        // Non-module entities: return single-level children (existing behavior)
                        let children_json: Vec<Value> = children
                            .into_iter()
                            .filter(|child| {
                                !should_exclude_entity(child.path.as_deref(), &self.ctx.output_dir)
                            })
                            .map(|child| {
                                json!({
                                    "id": child.id,
                                    "name": child.name,
                                    "tier": child.tier.to_string(),
                                    "path": child.path
                                })
                            })
                            .collect();

                        let actual_child_count = children_json.len();
                        response["children"] = json!(children_json);
                        response["child_count"] = json!(actual_child_count);
                    }
                }

                Ok(response.to_string())
            }
            None => Ok(json!({"error": "entity not found"}).to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// ListSubsystemsTool
// ---------------------------------------------------------------------------

impl<S: Storage + Send> Tool for ListSubsystemsTool<S> {
    fn name(&self) -> &str {
        "list_subsystems"
    }

    fn call(&self, _input: Value) -> crate::Result<String> {
        let guard = lock_storage!(self.ctx.storage);
        let subsystems = guard.list_entities(&self.ctx.project_id, Some(EntityTier::Subsystem))?;
        let result: Vec<Value> = subsystems
            .into_iter()
            .filter(|e| !should_exclude_entity(e.path.as_deref(), &self.ctx.output_dir))
            .map(|e| {
                json!({
                    "entity_id": e.id,
                    "name": e.name,
                    "path": e.path,
                    "summary": e.summary.as_deref().map(|s| truncate(s, 200))
                })
            })
            .collect();
        Ok(json!(result).to_string())
    }
}
