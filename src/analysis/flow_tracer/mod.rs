// Execution flow tracing - identifies entry points and traces call paths depth-first.
// Stores precomputed flows in entity metrics_json field for LLM consumption.

use crate::model::{Entity, EntityTier, RelType, Relationship};
use std::collections::{HashMap, HashSet};

/// Single step in an execution flow - an entity visited in DFS traversal.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FlowStep {
    pub entity_name: String,
    pub entity_id: String,
    pub depth: usize,
}

/// Complete execution flow from entry point through call chain.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ExecutionFlow {
    pub entry_point: String,
    pub entry_point_id: String,
    pub steps: Vec<FlowStep>,
    pub has_cycle: bool,
}

pub struct FlowTracer;

impl FlowTracer {
    /// Trace execution flows from detected entry points.
    ///
    /// Entry points are entities with outgoing DependsOn/Calls but no incoming DependsOn/Calls
    /// (leaf consumers), or entities matching name-based heuristics (main, handle_*, bin/ paths).
    /// Each flow is traced depth-first with cycle detection and max depth of 20 (or `max_depth` if provided).
    ///
    /// Flows are stored as serialized JSON in the metrics_json field of their entry point entities.
    pub fn trace_flows(
        entities: &[Entity],
        relationships: &[Relationship],
        max_depth: Option<usize>,
    ) -> Vec<ExecutionFlow> {
        let max_depth = max_depth.unwrap_or(20);
        if entities.is_empty() {
            return Vec::new();
        }

        // Build graph: entity_id -> list of entity_ids it depends on or calls
        // DependsOn edges always included; Calls edges only between function-tier entities
        let mut call_graph: HashMap<String, Vec<String>> = HashMap::new();
        let mut has_incoming_edges: HashSet<String> = HashSet::new();
        let entities_by_id: std::collections::HashMap<&str, &Entity> =
            entities.iter().map(|e| (e.id.as_str(), e)).collect();

        for rel in relationships {
            let is_depends_on = rel.rel_type == RelType::DependsOn;
            let is_calls = rel.rel_type == RelType::Calls;

            // For Calls edges, only include if both endpoints are function-tier entities
            let source_is_fn = entities_by_id
                .get(rel.source_id.as_str())
                .map(|e| e.tier == EntityTier::Function)
                .unwrap_or(false);
            let target_is_fn = entities_by_id
                .get(rel.target_id.as_str())
                .map(|e| e.tier == EntityTier::Function)
                .unwrap_or(false);

            if is_depends_on || (is_calls && source_is_fn && target_is_fn) {
                call_graph
                    .entry(rel.source_id.clone())
                    .or_default()
                    .push(rel.target_id.clone());
                has_incoming_edges.insert(rel.target_id.clone());
            } else if is_calls {
                tracing::debug!(
                    "Skipping Calls edge {}->{}: not between function-tier entities",
                    rel.source_id,
                    rel.target_id
                );
            }
        }

        // Deduplicate graph targets
        for targets in call_graph.values_mut() {
            targets.sort();
            targets.dedup();
        }

        // Find entry points:
        // 1. Entities with outgoing edges but no incoming edges (leaf consumers)
        // 2. Entities matching name-based heuristics (main, handle_*, bin/ paths)
        let mut entry_points_set: HashSet<&str> = call_graph
            .keys()
            .filter(|entity_id| !has_incoming_edges.contains(*entity_id))
            .map(|s| s.as_str())
            .collect();

        // Add name-based heuristic entry points
        for entity in entities {
            let is_entry_point = entity.name == "main"
                || entity.name.starts_with("handle_")
                || entity.path.as_ref().is_some_and(|p| p.contains("/bin/"));

            if is_entry_point {
                entry_points_set.insert(entity.id.as_str());
            }
        }

        if entry_points_set.is_empty() {
            return Vec::new();
        }

        let mut entry_points: Vec<&str> = entry_points_set.into_iter().collect();
        entry_points.sort();

        // Create name lookup
        let name_by_id: HashMap<&str, &str> = entities
            .iter()
            .map(|e| (e.id.as_str(), e.name.as_str()))
            .collect();

        let mut flows = Vec::new();

        // Trace from each entry point
        for entry_id in entry_points {
            let flow =
                Self::trace_single_flow(entry_id, &call_graph, &name_by_id, entities, max_depth);
            flows.push(flow);
        }

        flows
    }

    fn trace_single_flow(
        entry_id: &str,
        call_graph: &HashMap<String, Vec<String>>,
        name_by_id: &HashMap<&str, &str>,
        _entities: &[Entity],
        max_depth: usize,
    ) -> ExecutionFlow {
        let entry_name = name_by_id
            .get(entry_id)
            .map(|s| s.to_string())
            .unwrap_or_else(|| entry_id.to_string());

        let mut stack: Vec<(String, usize)> = Vec::new();
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut steps: Vec<FlowStep> = vec![FlowStep {
            entity_name: entry_name.clone(),
            entity_id: entry_id.to_string(),
            depth: 0,
        }];
        visited.insert(entry_id.to_string());
        let mut has_cycle = false;

        // Initialize stack with direct callees of entry point
        if let Some(targets) = call_graph.get(entry_id) {
            for target_id in targets {
                stack.push((target_id.clone(), 1));
            }
        }

        while let Some((current_id, depth)) = stack.pop() {
            if depth > max_depth {
                continue;
            }
            visited.insert(current_id.clone());

            let next_name = name_by_id
                .get(current_id.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| current_id.clone());

            steps.push(FlowStep {
                entity_name: next_name,
                entity_id: current_id.clone(),
                depth,
            });

            // Push unvisited callees; mark cycle when target already visited
            if let Some(targets) = call_graph.get(&current_id) {
                for target_id in targets.iter().rev() {
                    if visited.contains(target_id) {
                        has_cycle = true;
                    } else {
                        stack.push((target_id.clone(), depth + 1));
                    }
                }
            }
        }

        ExecutionFlow {
            entry_point: entry_name,
            entry_point_id: entry_id.to_string(),
            steps,
            has_cycle,
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
