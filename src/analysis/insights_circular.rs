// Circular dependency insight detector.
//
// Detects dependency cycles among module-tier entities via DependsOn edges.
// Extracted from insights_arch.rs to keep files under 500 lines.
//
// Uses iterative DFS with gray (visiting) / black (done) node coloring.
// Each unique cycle is reported once, normalized by sorting the full cycle path
// so that rotations and permutations of the same cycle compare equal.

use crate::model::{Entity, Insight, RelType};
use crate::storage::Storage;
use std::collections::{HashMap, HashSet};

use super::insights::{insight_id, now};

/// Detect circular dependency cycles among module-tier entities via DependsOn edges.
///
/// Uses iterative DFS with gray (visiting) / black (done) node coloring.
/// Each unique cycle is reported once, normalized to start at the lexicographically
/// smallest node so that duplicate orderings are deduplicated.
///
/// For Rust projects, circular dependencies are false positives because the Rust
/// compiler enforces acyclicity — any circular use within a crate is a hard error.
/// Skip detection entirely for Rust projects (language == "rust").
pub fn detect_circular_dependencies(
    storage: &dyn Storage,
    project_id: &str,
    modules: &[Entity],
    skip_for_rust: bool,
) -> crate::Result<Vec<Insight>> {
    if modules.is_empty() {
        return Ok(vec![]);
    }

    if skip_for_rust {
        return Ok(vec![]);
    }

    // Build id → name map and adjacency list (DependsOn edges only).
    // Issue #764: the module set is already project-scoped (the caller passes
    // only this project's modules), so an incoming edge that lands outside the
    // set — including a cross-project edge — is dropped. No extra lookup is
    // needed: a genuine same-project edge always lands in the set.
    let id_to_name: HashMap<&str, &str> = modules
        .iter()
        .map(|m| (m.id.as_str(), m.name.as_str()))
        .collect();

    let module_ids: HashSet<&str> = id_to_name.keys().copied().collect();

    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for module in modules {
        let neighbors: Vec<&str> = storage
            .relationships_from(&module.id)?
            .into_iter()
            .filter(|(rel, _)| rel.rel_type == RelType::DependsOn)
            // Project boundary (issue #764): a cross-project endpoint cannot
            // be a project-local module, so filtering it here keeps the
            // cycle traversal inside the project.
            .filter(|(_, target)| target.project_id == project_id)
            .filter_map(|(_, target)| {
                id_to_name
                    .keys()
                    .find(|&&k| k == target.id.as_str())
                    .copied()
            })
            .filter(|tid| module_ids.contains(tid))
            .collect();
        adj.insert(module.id.as_str(), neighbors);
    }

    // Iterative DFS: gray = on current path, black = fully processed.
    let mut gray: HashSet<&str> = HashSet::new();
    let mut black: HashSet<&str> = HashSet::new();
    let mut path: Vec<&str> = Vec::new();
    // Stack entries: (node, neighbor_index)
    let mut stack: Vec<(&str, usize)> = Vec::new();

    let mut seen_cycles: HashSet<String> = HashSet::new();
    let mut insights = Vec::new();

    // Sort for deterministic traversal order.
    let mut start_nodes: Vec<&str> = module_ids.iter().copied().collect();
    start_nodes.sort_unstable();

    for start in start_nodes {
        if black.contains(start) {
            continue;
        }
        gray.insert(start);
        path.push(start);
        stack.push((start, 0));

        'dfs: while let Some((node, idx)) = stack.last_mut() {
            let node = *node;
            let neighbors = adj.get(node).map(|v| v.as_slice()).unwrap_or(&[]);

            if *idx < neighbors.len() {
                let next = neighbors[*idx];
                *idx += 1;

                if black.contains(next) {
                    continue;
                }
                if gray.contains(next) {
                    // Cycle found: extract path from `next` to end of `path`.
                    let cycle_start = path.iter().position(|&n| n == next).unwrap_or(0);
                    let cycle: Vec<&str> = path[cycle_start..].to_vec();
                    let canonical_key = canonical_cycle_key(&cycle);
                    if seen_cycles.insert(canonical_key.clone()) {
                        let names: Vec<&str> = cycle.iter().map(|id| id_to_name[*id]).collect();
                        let entity_ids_json = cycle_ids_json(&cycle);
                        let cycle_display = format!("{} → {}", names.join(" → "), names[0]);
                        let id = insight_id(project_id, "circular_dependency", &canonical_key);
                        let insight = Insight {
                            id,
                            project_id: project_id.to_string(),
                            category: "circular_dependency".to_string(),
                            severity: Some("high".to_string()),
                            title: format!("Circular dependency: {}", names.join(" → ")),
                            description: Some(format!(
                                "Circular dependency: {cycle_display}. Consider breaking the cycle by extracting shared types."
                            )),
                            entity_ids_json: Some(entity_ids_json),
                            detected_at: now(),
                            still_valid: true,
                        };
                        insights.push(insight);
                    }
                    continue 'dfs;
                }

                gray.insert(next);
                path.push(next);
                stack.push((next, 0));
            } else {
                // All neighbors processed: pop.
                black.insert(node);
                gray.remove(node);
                path.pop();
                stack.pop();
            }
        }
    }

    Ok(insights)
}

/// Build a canonical (deduplicated) key for a cycle, normalized to start at the
/// lexicographically smallest node ID so that rotations of the same cycle compare equal.
fn canonical_cycle_key(cycle: &[&str]) -> String {
    let min_pos = cycle
        .iter()
        .enumerate()
        .min_by_key(|&(_, id)| id)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let mut rotated: Vec<&str> = cycle[min_pos..]
        .iter()
        .chain(cycle[..min_pos].iter())
        .copied()
        .collect();
    rotated.sort_unstable(); // sort to handle A→B→C vs A→C→B as same cycle set
    rotated.join(",")
}

/// Serialize a list of node IDs as a JSON array string.
fn cycle_ids_json(cycle: &[&str]) -> String {
    let items: Vec<String> = cycle.iter().map(|id| format!("\"{id}\"")).collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::test_support::{StubStorage, stub_entity};
    use crate::model::EntityTier;
    use crate::model::RelType;

    // ---- Circular dependency tests ----

    #[test]
    fn test_detect_circular_dependencies_simple_cycle() {
        // auth → users → permissions → auth
        let s = StubStorage::default();
        s.add_entity(stub_entity("auth", "auth", EntityTier::Module, None, None));
        s.add_entity(stub_entity(
            "users",
            "users",
            EntityTier::Module,
            None,
            None,
        ));
        s.add_entity(stub_entity(
            "perms",
            "permissions",
            EntityTier::Module,
            None,
            None,
        ));
        s.add_rel("auth", "users", RelType::DependsOn);
        s.add_rel("users", "perms", RelType::DependsOn);
        s.add_rel("perms", "auth", RelType::DependsOn);

        let modules: Vec<_> = s
            .entities
            .borrow()
            .iter()
            .filter(|e| e.tier == EntityTier::Module)
            .cloned()
            .collect();
        let insights = detect_circular_dependencies(&s, "proj", &modules, false).unwrap();
        assert_eq!(insights.len(), 1);
        assert_eq!(insights[0].category, "circular_dependency");
        assert_eq!(insights[0].severity.as_deref(), Some("high"));
        assert!(
            insights[0]
                .description
                .as_deref()
                .unwrap()
                .contains("Consider breaking")
        );
    }

    #[test]
    fn test_detect_circular_dependencies_no_cycle() {
        let s = StubStorage::default();
        s.add_entity(stub_entity("a", "a", EntityTier::Module, None, None));
        s.add_entity(stub_entity("b", "b", EntityTier::Module, None, None));
        s.add_rel("a", "b", RelType::DependsOn);

        let modules: Vec<_> = s
            .entities
            .borrow()
            .iter()
            .filter(|e| e.tier == EntityTier::Module)
            .cloned()
            .collect();
        let insights = detect_circular_dependencies(&s, "proj", &modules, false).unwrap();
        assert!(insights.is_empty());
    }

    #[test]
    fn test_detect_circular_dependencies_ignores_non_depends_on() {
        // Contains edge from a → b should not count as a dependency cycle.
        let s = StubStorage::default();
        s.add_entity(stub_entity("a", "a", EntityTier::Module, None, None));
        s.add_entity(stub_entity("b", "b", EntityTier::Module, None, None));
        s.add_rel("a", "b", RelType::Contains);
        s.add_rel("b", "a", RelType::Contains);

        let modules: Vec<_> = s
            .entities
            .borrow()
            .iter()
            .filter(|e| e.tier == EntityTier::Module)
            .cloned()
            .collect();
        let insights = detect_circular_dependencies(&s, "proj", &modules, false).unwrap();
        assert!(insights.is_empty());
    }

    #[test]
    fn test_detect_circular_dependencies_empty_modules() {
        let s = StubStorage::default();
        let modules: Vec<_> = s
            .entities
            .borrow()
            .iter()
            .filter(|e| e.tier == EntityTier::Module)
            .cloned()
            .collect();
        let insights = detect_circular_dependencies(&s, "proj", &modules, false).unwrap();
        assert!(insights.is_empty());
    }

    #[test]
    fn test_detect_circular_dependencies_distinct_cycles_same_min_node_get_distinct_ids() {
        // Two distinct cycles sharing the same minimum-node "a":
        //   Cycle 1: a → b → a
        //   Cycle 2: a → c → a
        // These must produce two insights with different IDs (Fix E).
        let s = StubStorage::default();
        s.add_entity(stub_entity("a", "a", EntityTier::Module, None, None));
        s.add_entity(stub_entity("b", "b", EntityTier::Module, None, None));
        s.add_entity(stub_entity("c", "c", EntityTier::Module, None, None));
        s.add_rel("a", "b", RelType::DependsOn);
        s.add_rel("b", "a", RelType::DependsOn);
        s.add_rel("a", "c", RelType::DependsOn);
        s.add_rel("c", "a", RelType::DependsOn);

        let modules: Vec<_> = s
            .entities
            .borrow()
            .iter()
            .filter(|e| e.tier == EntityTier::Module)
            .cloned()
            .collect();
        let insights = detect_circular_dependencies(&s, "proj", &modules, false).unwrap();
        // Should find 2 distinct cycles
        assert_eq!(
            insights.len(),
            2,
            "Expected 2 distinct cycles, got: {}",
            insights.len()
        );
        // Each insight must have a unique ID
        let id0 = &insights[0].id;
        let id1 = &insights[1].id;
        assert_ne!(id0, id1, "Distinct cycles must have distinct insight IDs");
    }

    #[test]
    fn test_detect_circular_dependencies_rust_project_returns_empty() {
        // Issue #551/#570: Rust projects should get zero circular dependency insights
        // because Rust's compiler enforces acyclicity — circular deps are hard errors.
        // Caller computes skip_for_rust (>50% Rust files), so pass true here.
        let s = StubStorage::default();
        s.add_entity(stub_entity(
            "auth",
            "auth",
            EntityTier::Module,
            None,
            Some("rust"),
        ));
        s.add_entity(stub_entity(
            "users",
            "users",
            EntityTier::Module,
            None,
            Some("rust"),
        ));
        s.add_rel("auth", "users", RelType::DependsOn);
        s.add_rel("users", "auth", RelType::DependsOn);

        let modules: Vec<_> = s
            .entities
            .borrow()
            .iter()
            .filter(|e| e.tier == EntityTier::Module)
            .cloned()
            .collect();
        let insights = detect_circular_dependencies(&s, "proj", &modules, true).unwrap();
        assert!(
            insights.is_empty(),
            "Predominantly Rust project must not produce circular dependency insights"
        );
    }

    #[test]
    fn test_detect_circular_dependencies_mixed_project_still_detects_cycles() {
        // Issue #570: Mixed-language projects (e.g., Python with Rust FFI) should still
        // get circular dependency detection when not predominantly Rust.
        let s = StubStorage::default();
        s.add_entity(stub_entity(
            "a",
            "a",
            EntityTier::Module,
            None,
            Some("python"),
        ));
        s.add_entity(stub_entity(
            "b",
            "b",
            EntityTier::Module,
            None,
            Some("python"),
        ));
        s.add_rel("a", "b", RelType::DependsOn);
        s.add_rel("b", "a", RelType::DependsOn);

        let modules: Vec<_> = s
            .entities
            .borrow()
            .iter()
            .filter(|e| e.tier == EntityTier::Module)
            .cloned()
            .collect();
        let insights = detect_circular_dependencies(&s, "proj", &modules, false).unwrap();
        assert_eq!(
            insights.len(),
            1,
            "Mixed project must still detect circular dependencies, got {} insights",
            insights.len()
        );
    }

    #[test]
    fn test_detect_circular_dependencies_three_module_cycle_produces_one_insight() {
        // Issue #555: A→B→C→A cycle must produce exactly 1 insight (deduplication).
        let s = StubStorage::default();
        s.add_entity(stub_entity("a", "a", EntityTier::Module, None, None));
        s.add_entity(stub_entity("b", "b", EntityTier::Module, None, None));
        s.add_entity(stub_entity("c", "c", EntityTier::Module, None, None));
        s.add_rel("a", "b", RelType::DependsOn);
        s.add_rel("b", "c", RelType::DependsOn);
        s.add_rel("c", "a", RelType::DependsOn);

        let modules: Vec<_> = s
            .entities
            .borrow()
            .iter()
            .filter(|e| e.tier == EntityTier::Module)
            .cloned()
            .collect();
        let insights = detect_circular_dependencies(&s, "proj", &modules, false).unwrap();
        assert_eq!(
            insights.len(),
            1,
            "3-module cycle A→B→C→A must produce exactly 1 insight, got {}",
            insights.len()
        );
        let names: Vec<_> = insights[0]
            .title
            .split(": ")
            .nth(1)
            .map(|s| s.split(" → ").collect::<Vec<_>>())
            .unwrap_or_default();
        assert_eq!(names.len(), 3, "insight must describe all 3 modules");
    }
}
