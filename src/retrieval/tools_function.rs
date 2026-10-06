// GetFunctionTool implementation.

use tracing;

use crate::extraction::code_extractor::CodeExtractor;
use crate::extraction::tree_sitter_extractor::TreeSitterExtractor;
use crate::model::{CodeUnit, EntityTier, RelType};
use crate::retrieval::project_boundary::same_project;
use crate::retrieval::tool_trait::Tool;
use crate::storage::Storage;

use serde_json::{Value, json};

use super::super::GetFunctionTool;
use super::tools_search::lock_storage;

fn no_function_entities_error() -> String {
    json!({
        "error": "No function entities indexed. Re-run 'lievo refresh' (or 'lievo refresh --force') to index function-level entities. This is required after upgrading from a version where function indexing was disabled."
    }).to_string()
}

impl<S: Storage + Send> Tool for GetFunctionTool<S> {
    fn name(&self) -> &str {
        "get_function"
    }

    fn call(&self, input: Value) -> crate::Result<String> {
        let entity_id = input
            .get("entity_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::LievoError::InvalidInput("missing 'entity_id'".into()))?;

        let guard = lock_storage!(self.ctx.storage);

        // Look up entity first — avoid full table scan on the hot path.
        let entity = match guard.get_entity(entity_id)? {
            Some(e) => e,
            None => {
                // Entity not found — check if any function entities exist for a helpful hint.
                let has_any = !guard
                    .list_entities(&self.ctx.project_id, Some(EntityTier::Function))?
                    .is_empty();
                if !has_any {
                    return Ok(no_function_entities_error());
                }
                return Ok(json!({"error": "Entity not found"}).to_string());
            }
        };

        // Verify entity tier is Function
        if entity.tier != EntityTier::Function {
            // Check if any function entities exist for a better error message.
            let has_any = !guard
                .list_entities(&self.ctx.project_id, Some(EntityTier::Function))?
                .is_empty();
            if !has_any {
                return Ok(no_function_entities_error());
            }
            return Ok(
                json!({"error": "Entity is not a function or class. Use get_entity for file/module entities."})
                    .to_string(),
            );
        }

        // Get file path from entity
        let file_path = match &entity.path {
            Some(p) => p.clone(),
            None => {
                return Ok(json!({"error": "Entity has no file path"}).to_string());
            }
        };

        // Get relationships directly from storage for function_call type.
        // Project boundary (issue #764): the relationships table has no
        // project_id column, so both traversals filter their endpoints
        // against the entity's own project at the single point of use.
        let project_id = &entity.project_id;
        let relationships = guard.relationships_from(entity_id)?;
        let incoming_relationships = guard.relationships_to(entity_id)?;

        // Process 'calls' relationships (outgoing)
        let calls_data: Vec<serde_json::Value> = relationships
            .iter()
            .filter(|(rel, target)| {
                rel.rel_type == RelType::Calls && same_project(project_id, &target.project_id)
            })
            .map(|(rel, target)| {
                json!({
                    "name": target.name,
                    "entity_id": rel.target_id.clone()
                })
            })
            .collect();

        // Process 'called_by' relationships (incoming)
        let called_by_data: Vec<serde_json::Value> = incoming_relationships
            .iter()
            .filter(|(rel, source)| {
                rel.rel_type == RelType::Calls && same_project(project_id, &source.project_id)
            })
            .map(|(_rel, source)| {
                json!({
                    "name": source.name,
                    "entity_id": source.id.clone()
                })
            })
            .collect();

        // Clone data we need before dropping the guard
        let entity_id_owned = entity.id.clone();
        let entity_name = entity.name.clone();
        // Get the file entity ID (parent of function entities) for computing method IDs
        let file_entity_id = entity.parent_id.clone();
        drop(guard);

        // Query tree-sitter index for CodeUnit data — index() must be called before units_for_file()
        // to populate index_dir; skipping it causes IndexNotFound
        let mut extractor = TreeSitterExtractor::new(&self.ctx.repo_path, true)?;
        extractor.index(false)?;
        let units = extractor.units_for_file(&file_path)?;

        // Find matching CodeUnit by entity name
        let matching_unit = units.iter().find(|u| u.name == entity_name);

        match matching_unit {
            Some(unit) => {
                // For struct/enum/type/trait entities, aggregate method calls relationships.
                // Methods are code units in the same file with parent_class matching this entity.
                let is_type = matches!(
                    unit.unit_type.as_str(),
                    "struct" | "enum" | "type" | "trait" | "class" | "interface"
                );

                let (mut aggregated_calls, mut aggregated_called_by) =
                    (calls_data.clone(), called_by_data.clone());

                if is_type {
                    // Find all method entities of this struct/type
                    let methods: Vec<&CodeUnit> = units
                        .iter()
                        .filter(|u| u.parent_class.as_deref() == Some(&entity_name))
                        .collect();

                    // Re-acquire storage lock to fetch method relationships
                    let guard = lock_storage!(self.ctx.storage);
                    for method in &methods {
                        // Use the file_entity_id (parent) to compute method entity ID
                        let method_id = if let Some(ref file_id) = file_entity_id {
                            crate::extraction::function_preservation::function_id(
                                file_id,
                                &method.name,
                            )
                        } else {
                            continue;
                        };
                        // Get calls FROM this method
                        match guard.relationships_from(&method_id) {
                            Ok(method_rels) => {
                                for (rel, target) in method_rels {
                                    if rel.rel_type == RelType::Calls
                                        && same_project(project_id, &target.project_id)
                                    {
                                        aggregated_calls.push(json!({
                                            "name": target.name,
                                            "entity_id": rel.target_id.clone(),
                                            "via_method": method.name.clone()
                                        }));
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to fetch outgoing relationships for method {}: {}",
                                    method_id,
                                    e
                                );
                            }
                        }
                        // Get calls TO this method (incoming)
                        match guard.relationships_to(&method_id) {
                            Ok(incoming) => {
                                for (rel, source) in incoming {
                                    if rel.rel_type == RelType::Calls
                                        && same_project(project_id, &source.project_id)
                                    {
                                        aggregated_called_by.push(json!({
                                            "name": source.name,
                                            "entity_id": source.id.clone(),
                                            "via_method": method.name.clone()
                                        }));
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "failed to fetch incoming relationships for method {}: {}",
                                    method_id,
                                    e
                                );
                            }
                        }
                    }
                }

                Ok(json!({
                    "entity_id": entity_id_owned,
                    "name": unit.name,
                    "qualified_name": unit.qualified_name,
                    "unit_type": unit.unit_type,
                    "file": unit.file,
                    "line": unit.line,
                    "end_line": unit.end_line,
                    "signature": unit.signature,
                    "code": unit.code,
                    "docstring": unit.docstring,
                    "complexity": unit.complexity,
                    "calls": aggregated_calls,
                    "called_by": aggregated_called_by
                })
                .to_string())
            }
            None => {
                Ok(json!({"error": "Function not indexed. Run lievo refresh first."}).to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::function_preservation::function_id;
    use crate::model::{EdgeProvenance, Entity, EntityTier, Relationship};
    use crate::retrieval::tools::ToolContext;
    use crate::storage::sqlite::SqliteStorage;

    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    fn make_test_entity(id: &str, project_id: &str, tier: EntityTier) -> Entity {
        Entity {
            id: id.to_string(),
            project_id: project_id.to_string(),
            repo_id: None,
            tier,
            parent_id: None,
            name: id.to_string(),
            path: Some(format!("src/{id}.rs")),
            language: Some("Rust".to_string()),
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_context(repo_path: PathBuf, project_name: &str) -> Arc<ToolContext<SqliteStorage>> {
        let storage = SqliteStorage::open_in_memory().unwrap();
        let project = storage.create_project(project_name, None).unwrap();
        Arc::new(ToolContext {
            storage: Arc::new(Mutex::new(storage)),
            project_id: project.id,
            repo_path,
            output_dir: None,
            zero_repo_guidance: None,
        })
    }

    #[test]
    fn get_function_returns_error_when_no_function_entities_indexed() {
        // Project with only a file entity — no function entities at all.
        // This simulates repos analyzed before v0.9 where preserve_function_entities was false.
        let ctx = make_context(PathBuf::new(), "test-no-funcs-project");
        let storage = ctx.storage.lock().unwrap();
        let project_id = ctx.project_id.clone();

        let file_entity = make_test_entity("file-456", &project_id, EntityTier::File);
        storage.upsert_entity(&file_entity).unwrap();
        drop(storage);

        let tool = GetFunctionTool { ctx };

        // Calling with a non-existent entity_id: lookup returns None, then
        // the zero-function check fires with a helpful error.
        let result = tool.call(json!({"entity_id": "file-456"})).unwrap();
        assert!(result.contains("No function entities indexed"));
        assert!(result.contains("lievo refresh"));
    }

    #[test]
    fn get_function_returns_error_for_unknown_entity_id() {
        // Need at least one function entity so the zero-function check passes.
        let ctx = make_context(PathBuf::new(), "test-get-function-1-project");
        let storage = ctx.storage.lock().unwrap();
        let project_id = ctx.project_id.clone();

        let func_entity = make_test_entity("func-placeholder", &project_id, EntityTier::Function);
        storage.upsert_entity(&func_entity).unwrap();
        drop(storage);

        let tool = GetFunctionTool { ctx };

        let result = tool.call(json!({"entity_id": "unknown-id"})).unwrap();
        assert!(result.contains("Entity not found"));
    }

    #[test]
    fn get_function_returns_error_for_non_function_entity() {
        let ctx = make_context(PathBuf::new(), "test-get-function-2-non-func");
        let storage = ctx.storage.lock().unwrap();
        let project_id = ctx.project_id.clone();

        // Add a function entity so the zero-function check passes,
        // then test that a file entity correctly gets the "not a function or class" error.
        let func_entity = make_test_entity("func-placeholder-2", &project_id, EntityTier::Function);
        storage.upsert_entity(&func_entity).unwrap();

        let file_entity = make_test_entity("file-123", &project_id, EntityTier::File);

        storage.upsert_entity(&file_entity).unwrap();
        drop(storage);

        let tool = GetFunctionTool { ctx };

        let result = tool.call(json!({"entity_id": "file-123"})).unwrap();
        assert!(result.contains("not a function or class"));
    }

    #[test]
    fn get_function_returns_error_for_entity_without_path() {
        let ctx = make_context(PathBuf::new(), "test-get-function-3-no-path");
        let storage = ctx.storage.lock().unwrap();
        let project_id = ctx.project_id.clone();

        let func_entity = make_test_entity("func-no-path", &project_id, EntityTier::Function);
        let mut func_entity = func_entity;
        func_entity.path = None;
        func_entity.language = None;
        func_entity.name = "test_function".to_string();

        storage.upsert_entity(&func_entity).unwrap();
        drop(storage);

        let tool = GetFunctionTool { ctx };

        let result = tool.call(json!({"entity_id": "func-no-path"})).unwrap();
        assert!(result.contains("no file path"));
    }

    #[test]
    fn get_function_tool_has_correct_schema() {
        let ctx = make_context(PathBuf::new(), "test-get-function-4");
        let tool = GetFunctionTool { ctx };

        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["entity_id"].is_object());
        assert_eq!(schema["properties"]["entity_id"]["type"], "string");
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"entity_id".into())
        );
    }

    #[test]
    fn called_by_returns_correct_entity_ids() {
        // Regression test for #514: called_by should return caller IDs, not callee IDs.
        // This test verifies the storage layer's relationships_to returns correct caller IDs.
        let ctx = make_context(PathBuf::new(), "test-called-by-fix");
        let storage = ctx.storage.lock().unwrap();
        let project_id = ctx.project_id.clone();

        // Create two function entities: caller and callee
        let caller = make_test_entity("caller-func", &project_id, EntityTier::Function);
        let callee = make_test_entity("callee-func", &project_id, EntityTier::Function);
        storage.upsert_entity(&caller).unwrap();
        storage.upsert_entity(&callee).unwrap();

        // Create a Calls relationship: caller → callee
        let rel = Relationship {
            source_id: "caller-func".to_string(),
            target_id: "callee-func".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        };
        storage.upsert_relationship(&rel).unwrap();

        // Verify relationships_to (incoming) returns the caller, not the callee
        let incoming_rels = storage.relationships_to("callee-func").unwrap();
        assert_eq!(incoming_rels.len(), 1);
        let (rel, source_entity) = &incoming_rels[0];

        // The relationship should be a Calls relationship
        assert_eq!(rel.rel_type, RelType::Calls);

        // The source entity should be the caller, not the callee
        assert_eq!(source_entity.id, "caller-func");
        assert_eq!(source_entity.name, "caller-func");

        // Verify the relationship target is the callee
        assert_eq!(rel.target_id, "callee-func");
        assert_eq!(rel.source_id, "caller-func");

        drop(storage);
    }

    #[test]
    fn method_entity_relationships_queryable_by_function_id() {
        // Verifies that struct impl method relationships can be looked up
        // using the function_id() formula — the key lookup used in get_function aggregation.
        let ctx = make_context(PathBuf::new(), "test-struct-method-rels");
        let storage = ctx.storage.lock().unwrap();
        let project_id = ctx.project_id.clone();

        // Create a file entity (parent of both struct and method)
        let file_entity = make_test_entity("file-struct-test", &project_id, EntityTier::File);
        storage.upsert_entity(&file_entity).unwrap();

        // Create the struct entity
        let mut struct_entity = make_test_entity("MyStruct", &project_id, EntityTier::Function);
        struct_entity.parent_id = Some("file-struct-test".to_string());
        struct_entity.name = "MyStruct".to_string();
        storage.upsert_entity(&struct_entity).unwrap();

        // Create the method entity — its ID must match function_id(file_id, method_name)
        let method_id = function_id("file-struct-test", "new");
        let mut method_entity = make_test_entity(&method_id, &project_id, EntityTier::Function);
        method_entity.parent_id = Some("file-struct-test".to_string());
        method_entity.name = "new".to_string();
        storage.upsert_entity(&method_entity).unwrap();

        // Create a target entity (what the method calls)
        let target_entity = make_test_entity("SomeService", &project_id, EntityTier::Function);
        storage.upsert_entity(&target_entity).unwrap();

        // Create Calls relationship: method → target
        let rel = Relationship {
            source_id: method_id.clone(),
            target_id: "SomeService".to_string(),
            rel_type: RelType::Calls,
            weight: 1.0,
            evidence_json: None,
            provenance: EdgeProvenance::Heuristic,
        };
        storage.upsert_relationship(&rel).unwrap();
        drop(storage);

        // Verify the relationship is queryable by method_id
        let storage = ctx.storage.lock().unwrap();
        let rels = storage.relationships_from(&method_id).unwrap();
        assert_eq!(
            rels.len(),
            1,
            "method should have 1 outgoing Calls relationship"
        );
        let (rel, target) = &rels[0];
        assert_eq!(rel.rel_type, RelType::Calls);
        assert_eq!(target.name, "SomeService");
    }
}
