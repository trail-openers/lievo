// Metrics aggregation for entities

use crate::analysis::metrics::{EntityMetrics, MetricsComputer};
use crate::model::{Entity, EntityTier, Relationship};

/// Attach file metrics to file entities and compute coupling
pub fn attach_file_metrics(
    all_entities: &mut [Entity],
    code_units: &[crate::model::CodeUnit],
    relationships: &[Relationship],
) {
    let file_metrics = MetricsComputer::compute_file_metrics(code_units);

    // Serialize file metrics onto file entities
    for entity in all_entities.iter_mut() {
        if entity.tier == EntityTier::File
            && let Some(ref path) = entity.path
        {
            let metrics = file_metrics.get(path).cloned().unwrap_or(
                crate::analysis::metrics::EntityMetrics {
                    lines: 0,
                    code_units: 0,
                    complexity_sum: 0,
                    complexity_max: 0,
                    complexity_avg: 0.0,
                    has_branches_count: 0,
                    has_loops_count: 0,
                    has_error_handling_count: 0,
                    fan_in: 0,
                    fan_out: 0,
                    module_count: 0,
                    file_count: 0,
                },
            );
            entity.metrics_json = serde_json::to_string(&metrics)
                .map_err(|e| {
                    eprintln!(
                        "warning: failed to serialize metrics for {}: {e}",
                        entity.id
                    );
                })
                .ok();
        }
    }

    // Compute coupling (fan_in/fan_out) for file entities
    let file_ids: Vec<String> = all_entities
        .iter()
        .filter(|e| e.tier == EntityTier::File)
        .map(|e| e.id.clone())
        .collect();
    for file_id in &file_ids {
        let (fan_in, fan_out) = MetricsComputer::compute_coupling(file_id, relationships);
        if let Some(entity) = all_entities.iter_mut().find(|e| e.id == *file_id)
            && let Some(ref json) = entity.metrics_json
        {
            match serde_json::from_str::<EntityMetrics>(json) {
                Ok(mut metrics) => {
                    metrics.fan_in = fan_in;
                    metrics.fan_out = fan_out;
                    entity.metrics_json = serde_json::to_string(&metrics)
                        .map_err(|e| {
                            eprintln!(
                                "warning: failed to serialize metrics for {}: {e}",
                                entity.id
                            );
                        })
                        .ok();
                }
                Err(e) => {
                    eprintln!(
                        "warning: skipping entity {} with corrupt metrics_json: {e}",
                        entity.id
                    );
                }
            }
        }
    }
}

/// Attach execution flows to entities
pub fn attach_execution_flows(
    all_entities: &mut [Entity],
    flows: &[crate::analysis::flow_tracer::ExecutionFlow],
) {
    for flow in flows {
        if let Some(entity) = all_entities
            .iter_mut()
            .find(|e| e.id == flow.entry_point_id)
        {
            let mut metrics = if let Some(ref json) = entity.metrics_json {
                match serde_json::from_str::<serde_json::Value>(json) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!(
                            "warning: corrupt metrics_json for entity {}: {e}",
                            entity.id
                        );
                        serde_json::json!({})
                    }
                }
            } else {
                serde_json::json!({})
            };

            if let Some(map) = metrics.as_object_mut() {
                let flows_value = map
                    .entry("execution_flows".to_string())
                    .or_insert_with(|| serde_json::json!([]));
                if let Some(arr) = flows_value.as_array_mut() {
                    match serde_json::to_value(flow) {
                        Ok(value) => arr.push(value),
                        Err(e) => eprintln!(
                            "warning: failed to serialize execution flow for entry {}: {e}",
                            flow.entry_point_id
                        ),
                    }
                }
            }

            match serde_json::to_string(&metrics) {
                Ok(s) => entity.metrics_json = Some(s),
                Err(e) => eprintln!(
                    "warning: failed to serialize metrics for entity {}: {e}",
                    entity.id
                ),
            }
        }
    }
}

/// Aggregate module metrics from children
pub fn aggregate_module_metrics(all_entities: &mut [Entity]) {
    let module_ids: Vec<String> = all_entities
        .iter()
        .filter(|e| e.tier == EntityTier::Module)
        .map(|e| e.id.clone())
        .collect();

    for module_id in &module_ids {
        let children: Vec<&Entity> = all_entities
            .iter()
            .filter(|e| e.parent_id.as_deref() == Some(module_id))
            .collect();
        let file_count = children
            .iter()
            .filter(|e| e.tier == EntityTier::File)
            .count() as i64;
        let child_metrics: Vec<EntityMetrics> = children
            .iter()
            .filter_map(|e| match e.metrics_json.as_deref() {
                Some(j) => match serde_json::from_str(j) {
                    Ok(m) => Some(m),
                    Err(err) => {
                        eprintln!(
                            "warning: skipping entity {} with corrupt metrics_json: {err}",
                            e.id
                        );
                        None
                    }
                },
                None => None,
            })
            .collect();
        let refs: Vec<&EntityMetrics> = child_metrics.iter().collect();
        if !refs.is_empty() {
            let mut agg = MetricsComputer::aggregate_module_metrics(&refs);
            agg.file_count = file_count;
            if let Some(entity) = all_entities.iter_mut().find(|e| e.id == *module_id) {
                entity.metrics_json = serde_json::to_string(&agg)
                    .map_err(|e| {
                        eprintln!(
                            "warning: failed to serialize metrics for {}: {e}",
                            module_id
                        );
                    })
                    .ok();
            }
        }
    }
}

/// Aggregate subsystem metrics from children
pub fn aggregate_subsystem_metrics(all_entities: &mut [Entity]) {
    let subsystem_ids: Vec<String> = all_entities
        .iter()
        .filter(|e| e.tier == EntityTier::Subsystem)
        .map(|e| e.id.clone())
        .collect();

    for subsystem_id in &subsystem_ids {
        let module_children: Vec<&Entity> = all_entities
            .iter()
            .filter(|e| {
                e.tier == EntityTier::Module && e.parent_id.as_deref() == Some(subsystem_id)
            })
            .collect();
        let module_count = module_children.len() as i64;
        let file_count: i64 = module_children
            .iter()
            .map(|m| {
                all_entities
                    .iter()
                    .filter(|e| {
                        e.tier == EntityTier::File && e.parent_id.as_deref() == Some(m.id.as_str())
                    })
                    .count() as i64
            })
            .sum();
        let child_metrics: Vec<EntityMetrics> = module_children
            .iter()
            .filter_map(|e| match e.metrics_json.as_deref() {
                Some(j) => match serde_json::from_str(j) {
                    Ok(m) => Some(m),
                    Err(err) => {
                        eprintln!(
                            "warning: skipping entity {} with corrupt metrics_json: {err}",
                            e.id
                        );
                        None
                    }
                },
                None => None,
            })
            .collect();
        let refs: Vec<&EntityMetrics> = child_metrics.iter().collect();
        if !refs.is_empty() {
            let mut agg = MetricsComputer::aggregate_subsystem_metrics(&refs);
            agg.module_count = module_count;
            agg.file_count = file_count;
            if let Some(entity) = all_entities.iter_mut().find(|e| e.id == *subsystem_id) {
                entity.metrics_json = serde_json::to_string(&agg)
                    .map_err(|e| {
                        eprintln!(
                            "warning: failed to serialize metrics for {}: {e}",
                            subsystem_id
                        );
                    })
                    .ok();
            }
        }
    }
}
