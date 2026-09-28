use crate::model::{CodeUnit, RelType, Relationship};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub struct MetricsComputer;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityMetrics {
    pub lines: usize,
    pub code_units: usize,
    pub complexity_sum: i64,
    pub complexity_max: i64,
    pub complexity_avg: f64,
    pub has_branches_count: usize,
    pub has_loops_count: usize,
    pub has_error_handling_count: usize,
    pub fan_in: usize,
    pub fan_out: usize,
    #[serde(default)]
    pub module_count: i64,
    #[serde(default)]
    pub file_count: i64,
}

impl MetricsComputer {
    /// Group CodeUnits by file path and compute per-file metrics.
    /// fan_in/fan_out default to 0; set separately via compute_coupling.
    pub fn compute_file_metrics(code_units: &[CodeUnit]) -> HashMap<String, EntityMetrics> {
        let mut by_file: HashMap<String, Vec<&CodeUnit>> = HashMap::new();
        for unit in code_units {
            by_file.entry(unit.file.clone()).or_default().push(unit);
        }

        by_file
            .into_iter()
            .map(|(file, units)| {
                let metrics = Self::metrics_from_units(&units);
                (file, metrics)
            })
            .collect()
    }

    /// Aggregate metrics from a slice of file-level EntityMetrics (module level).
    pub fn aggregate_module_metrics(file_metrics: &[&EntityMetrics]) -> EntityMetrics {
        Self::aggregate(file_metrics)
    }

    /// Aggregate metrics from a slice of module-level EntityMetrics (subsystem level).
    pub fn aggregate_subsystem_metrics(module_metrics: &[&EntityMetrics]) -> EntityMetrics {
        Self::aggregate(module_metrics)
    }

    /// Count fan_in (entity is target) and fan_out (entity is source) for coupling types.
    pub fn compute_coupling(entity_id: &str, relationships: &[Relationship]) -> (usize, usize) {
        let coupling_types = [RelType::Imports, RelType::Calls, RelType::DependsOn];
        let mut fan_in = 0usize;
        let mut fan_out = 0usize;
        for rel in relationships {
            if !coupling_types.contains(&rel.rel_type) {
                continue;
            }
            if rel.source_id == entity_id {
                fan_out += 1;
            }
            if rel.target_id == entity_id {
                fan_in += 1;
            }
        }
        (fan_in, fan_out)
    }

    fn metrics_from_units(units: &[&CodeUnit]) -> EntityMetrics {
        if units.is_empty() {
            return EntityMetrics {
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
            };
        }

        let lines: usize = units
            .iter()
            .map(|u| (u.end_line - u.line + 1).max(0) as usize)
            .sum();
        let code_units = units.len();
        let complexity_sum: i64 = units.iter().map(|u| u.complexity).sum();
        let complexity_max: i64 = units.iter().map(|u| u.complexity).max().unwrap_or(0);
        let complexity_avg = complexity_sum as f64 / code_units as f64;
        let has_branches_count = units.iter().filter(|u| u.has_branches).count();
        let has_loops_count = units.iter().filter(|u| u.has_loops).count();
        let has_error_handling_count = units.iter().filter(|u| u.has_error_handling).count();

        EntityMetrics {
            lines,
            code_units,
            complexity_sum,
            complexity_max,
            complexity_avg,
            has_branches_count,
            has_loops_count,
            has_error_handling_count,
            fan_in: 0,
            fan_out: 0,
            module_count: 0,
            file_count: 0,
        }
    }

    fn aggregate(metrics_list: &[&EntityMetrics]) -> EntityMetrics {
        if metrics_list.is_empty() {
            return EntityMetrics {
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
            };
        }

        let lines: usize = metrics_list.iter().map(|m| m.lines).sum();
        let code_units: usize = metrics_list.iter().map(|m| m.code_units).sum();
        let complexity_sum: i64 = metrics_list.iter().map(|m| m.complexity_sum).sum();
        let complexity_max: i64 = metrics_list
            .iter()
            .map(|m| m.complexity_max)
            .max()
            .unwrap_or(0);
        // Weighted average by unit count; fall back to 0.0 if no units
        let complexity_avg = if code_units > 0 {
            complexity_sum as f64 / code_units as f64
        } else {
            0.0
        };
        let has_branches_count: usize = metrics_list.iter().map(|m| m.has_branches_count).sum();
        let has_loops_count: usize = metrics_list.iter().map(|m| m.has_loops_count).sum();
        let has_error_handling_count: usize = metrics_list
            .iter()
            .map(|m| m.has_error_handling_count)
            .sum();
        let fan_in: usize = metrics_list.iter().map(|m| m.fan_in).sum();
        let fan_out: usize = metrics_list.iter().map(|m| m.fan_out).sum();

        EntityMetrics {
            lines,
            code_units,
            complexity_sum,
            complexity_max,
            complexity_avg,
            has_branches_count,
            has_loops_count,
            has_error_handling_count,
            fan_in,
            fan_out,
            module_count: 0,
            file_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CodeUnit, RelType, Relationship};

    fn make_unit(
        file: &str,
        line: i64,
        end_line: i64,
        complexity: i64,
        has_branches: bool,
        has_loops: bool,
        has_error_handling: bool,
    ) -> CodeUnit {
        CodeUnit {
            name: "fn".to_string(),
            qualified_name: "mod::fn".to_string(),
            unit_type: "function".to_string(),
            file: file.to_string(),
            line,
            end_line,
            language: "Rust".to_string(),
            signature: None,
            code: None,
            docstring: None,
            parent_class: None,
            complexity,
            has_branches,
            has_loops,
            has_error_handling,
            calls: vec![],
            imports: vec![],
        }
    }

    fn make_rel(source: &str, target: &str, rel_type: RelType) -> Relationship {
        Relationship {
            source_id: source.to_string(),
            target_id: target.to_string(),
            rel_type,
            weight: 1.0,
            evidence_json: None,
            provenance: crate::model::EdgeProvenance::default(),
        }
    }

    // --- compute_file_metrics ---

    #[test]
    fn test_compute_file_metrics_empty_returns_empty_map() {
        let result = MetricsComputer::compute_file_metrics(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_compute_file_metrics_single_file_single_unit() {
        let units = vec![make_unit("src/a.rs", 1, 11, 3, true, false, true)];
        let result = MetricsComputer::compute_file_metrics(&units);
        assert_eq!(result.len(), 1);
        let m = result.get("src/a.rs").unwrap();
        assert_eq!(m.lines, 11);
        assert_eq!(m.code_units, 1);
        assert_eq!(m.complexity_sum, 3);
        assert_eq!(m.complexity_max, 3);
        assert!((m.complexity_avg - 3.0).abs() < f64::EPSILON);
        assert_eq!(m.has_branches_count, 1);
        assert_eq!(m.has_loops_count, 0);
        assert_eq!(m.has_error_handling_count, 1);
        assert_eq!(m.fan_in, 0);
        assert_eq!(m.fan_out, 0);
    }

    #[test]
    fn test_compute_file_metrics_single_file_multiple_units() {
        let units = vec![
            make_unit("src/a.rs", 1, 11, 2, true, false, false),
            make_unit("src/a.rs", 12, 22, 4, false, true, true),
        ];
        let result = MetricsComputer::compute_file_metrics(&units);
        assert_eq!(result.len(), 1);
        let m = result.get("src/a.rs").unwrap();
        assert_eq!(m.lines, 22);
        assert_eq!(m.code_units, 2);
        assert_eq!(m.complexity_sum, 6);
        assert_eq!(m.complexity_max, 4);
        assert!((m.complexity_avg - 3.0).abs() < f64::EPSILON);
        assert_eq!(m.has_branches_count, 1);
        assert_eq!(m.has_loops_count, 1);
        assert_eq!(m.has_error_handling_count, 1);
    }

    #[test]
    fn test_compute_file_metrics_multiple_files() {
        let units = vec![
            make_unit("src/a.rs", 1, 6, 2, true, false, false),
            make_unit("src/b.rs", 1, 11, 5, false, true, true),
        ];
        let result = MetricsComputer::compute_file_metrics(&units);
        assert_eq!(result.len(), 2);
        let a = result.get("src/a.rs").unwrap();
        assert_eq!(a.code_units, 1);
        assert_eq!(a.complexity_max, 2);
        let b = result.get("src/b.rs").unwrap();
        assert_eq!(b.code_units, 1);
        assert_eq!(b.complexity_max, 5);
    }

    // --- aggregate_module_metrics ---

    #[test]
    fn test_aggregate_module_metrics_empty_returns_zeros() {
        let result = MetricsComputer::aggregate_module_metrics(&[]);
        assert_eq!(result.lines, 0);
        assert_eq!(result.code_units, 0);
        assert_eq!(result.complexity_sum, 0);
        assert_eq!(result.complexity_max, 0);
        assert!((result.complexity_avg - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_aggregate_module_metrics_sums_and_max() {
        let m1 = EntityMetrics {
            lines: 100,
            code_units: 5,
            complexity_sum: 15,
            complexity_max: 7,
            complexity_avg: 3.0,
            has_branches_count: 3,
            has_loops_count: 1,
            has_error_handling_count: 2,
            fan_in: 2,
            fan_out: 4,
            module_count: 0,
            file_count: 0,
        };
        let m2 = EntityMetrics {
            lines: 50,
            code_units: 2,
            complexity_sum: 6,
            complexity_max: 4,
            complexity_avg: 3.0,
            has_branches_count: 1,
            has_loops_count: 0,
            has_error_handling_count: 1,
            fan_in: 1,
            fan_out: 3,
            module_count: 0,
            file_count: 0,
        };
        let result = MetricsComputer::aggregate_module_metrics(&[&m1, &m2]);
        assert_eq!(result.lines, 150);
        assert_eq!(result.code_units, 7);
        assert_eq!(result.complexity_sum, 21);
        assert_eq!(result.complexity_max, 7);
        // weighted avg: 21 / 7 = 3.0
        assert!((result.complexity_avg - 3.0).abs() < f64::EPSILON);
        assert_eq!(result.has_branches_count, 4);
        assert_eq!(result.has_loops_count, 1);
        assert_eq!(result.has_error_handling_count, 3);
        assert_eq!(result.fan_in, 3);
        assert_eq!(result.fan_out, 7);
    }

    // --- aggregate_subsystem_metrics ---

    #[test]
    fn test_aggregate_subsystem_metrics_same_logic_as_module() {
        let m = EntityMetrics {
            lines: 200,
            code_units: 10,
            complexity_sum: 30,
            complexity_max: 8,
            complexity_avg: 3.0,
            has_branches_count: 5,
            has_loops_count: 2,
            has_error_handling_count: 4,
            fan_in: 3,
            fan_out: 6,
            module_count: 0,
            file_count: 0,
        };
        let result = MetricsComputer::aggregate_subsystem_metrics(&[&m]);
        assert_eq!(result.lines, 200);
        assert_eq!(result.complexity_max, 8);
        assert_eq!(result.fan_out, 6);
    }

    // --- compute_coupling ---

    #[test]
    fn test_compute_coupling_empty_relationships() {
        let (fan_in, fan_out) = MetricsComputer::compute_coupling("entity-a", &[]);
        assert_eq!(fan_in, 0);
        assert_eq!(fan_out, 0);
    }

    #[test]
    fn test_compute_coupling_counts_coupling_types() {
        let rels = vec![
            make_rel("entity-a", "entity-b", RelType::Imports),
            make_rel("entity-a", "entity-c", RelType::Calls),
            make_rel("entity-a", "entity-d", RelType::DependsOn),
            make_rel("entity-x", "entity-a", RelType::Imports),
            make_rel("entity-y", "entity-a", RelType::Calls),
        ];
        let (fan_in, fan_out) = MetricsComputer::compute_coupling("entity-a", &rels);
        assert_eq!(fan_out, 3);
        assert_eq!(fan_in, 2);
    }

    #[test]
    fn test_compute_coupling_ignores_non_coupling_types() {
        let rels = vec![
            make_rel("entity-a", "entity-b", RelType::Contains),
            make_rel("entity-a", "entity-c", RelType::Implements),
            make_rel("entity-x", "entity-a", RelType::Contains),
        ];
        let (fan_in, fan_out) = MetricsComputer::compute_coupling("entity-a", &rels);
        assert_eq!(fan_in, 0);
        assert_eq!(fan_out, 0);
    }

    #[test]
    fn test_compute_coupling_unrelated_entity_returns_zeros() {
        let rels = vec![make_rel("entity-x", "entity-y", RelType::Calls)];
        let (fan_in, fan_out) = MetricsComputer::compute_coupling("entity-z", &rels);
        assert_eq!(fan_in, 0);
        assert_eq!(fan_out, 0);
    }

    // --- EntityMetrics serialization ---

    #[test]
    fn test_entity_metrics_serde_roundtrip() {
        let m = EntityMetrics {
            lines: 100,
            code_units: 5,
            complexity_sum: 15,
            complexity_max: 7,
            complexity_avg: 3.0,
            has_branches_count: 3,
            has_loops_count: 1,
            has_error_handling_count: 2,
            fan_in: 2,
            fan_out: 4,
            module_count: 0,
            file_count: 0,
        };
        let json = serde_json::to_string(&m).unwrap();
        let deserialized: EntityMetrics = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.lines, m.lines);
        assert_eq!(deserialized.code_units, m.code_units);
        assert_eq!(deserialized.complexity_max, m.complexity_max);
    }

    #[test]
    fn test_entity_metrics_serde_default_fills_new_fields() {
        // Old JSON without module_count/file_count should deserialize with defaults of 0.
        let old_json = r#"{"lines":10,"code_units":2,"complexity_sum":5,"complexity_max":3,"complexity_avg":2.5,"has_branches_count":1,"has_loops_count":0,"has_error_handling_count":1,"fan_in":0,"fan_out":0}"#;
        let m: EntityMetrics = serde_json::from_str(old_json).unwrap();
        assert_eq!(m.module_count, 0);
        assert_eq!(m.file_count, 0);
    }

    #[test]
    fn test_single_line_code_unit_yields_one_line() {
        let units = vec![make_unit("src/a.rs", 5, 5, 1, false, false, false)];
        let result = MetricsComputer::compute_file_metrics(&units);
        let m = result.get("src/a.rs").unwrap();
        assert_eq!(m.lines, 1);
    }

    #[test]
    fn test_compute_coupling_entity_is_both_source_and_target() {
        let rels = vec![make_rel("entity-a", "entity-a", RelType::Calls)];
        let (fan_in, fan_out) = MetricsComputer::compute_coupling("entity-a", &rels);
        assert_eq!(fan_in, 1);
        assert_eq!(fan_out, 1);
    }
}
