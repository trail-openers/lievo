use super::entity::Entity;
use super::project::ProjectId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub id: String,
    pub project_id: ProjectId,
    pub category: String,
    pub severity: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub entity_ids_json: Option<String>,
    pub detected_at: String,
    pub still_valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Convention {
    pub id: String,
    pub project_id: ProjectId,
    pub category: String,
    pub title: String,
    pub description: Option<String>,
    pub example_code: Option<String>,
    pub confidence: f64,
    pub entity_ids_json: Option<String>,
    pub detected_at: String,
    pub still_valid: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityMetrics {
    pub total_files: usize,
    pub total_lines: usize,
    pub avg_complexity: f64,
    pub max_complexity: f64,
    pub avg_coupling: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityReport {
    pub project_name: String,
    pub metrics: QualityMetrics,
    pub hotspot_count: usize,
    pub convention_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactReport {
    pub changed_files: Vec<String>,
    pub affected_modules: Vec<Entity>,
    pub affected_subsystems: Vec<Entity>,
    pub downstream_dependents: Vec<Entity>,
    pub affected_functions: Vec<Entity>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::entity::{Entity, EntityTier};
    use serde_json;

    #[test]
    fn test_insight_serialization() {
        let insight = Insight {
            id: "insight-123".to_string(),
            project_id: "proj-123".to_string(),
            category: "hotspot".to_string(),
            severity: Some("critical".to_string()),
            title: "Complex function detected".to_string(),
            description: Some("Function has high complexity".to_string()),
            entity_ids_json: Some(r#"["entity-1", "entity-2"]"#.to_string()),
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            still_valid: true,
        };

        let json = serde_json::to_string(&insight).unwrap();
        let deserialized: Insight = serde_json::from_str(&json).unwrap();

        assert_eq!(insight.id, deserialized.id);
        assert_eq!(insight.category, deserialized.category);
        assert_eq!(insight.still_valid, deserialized.still_valid);
    }

    #[test]
    fn test_convention_serialization() {
        let convention = Convention {
            id: "conv-123".to_string(),
            project_id: "proj-123".to_string(),
            category: "error_handling".to_string(),
            title: "Result type for errors".to_string(),
            description: Some("Use Result<T, E> for errors".to_string()),
            example_code: Some("fn foo() -> Result<()>".to_string()),
            confidence: 0.95,
            entity_ids_json: Some(r#"["entity-1"]"#.to_string()),
            detected_at: "2024-01-01T00:00:00Z".to_string(),
            still_valid: true,
        };

        let json = serde_json::to_string(&convention).unwrap();
        let deserialized: Convention = serde_json::from_str(&json).unwrap();

        assert_eq!(convention.id, deserialized.id);
        assert_eq!(convention.category, deserialized.category);
        assert_eq!(convention.confidence, deserialized.confidence);
    }

    #[test]
    fn test_quality_metrics_default() {
        let metrics = QualityMetrics::default();

        assert_eq!(metrics.total_files, 0);
        assert_eq!(metrics.total_lines, 0);
        assert_eq!(metrics.avg_complexity, 0.0);
        assert_eq!(metrics.max_complexity, 0.0);
        assert_eq!(metrics.avg_coupling, 0.0);
    }

    #[test]
    fn test_quality_report_serialization() {
        let report = QualityReport {
            project_name: "test-project".to_string(),
            metrics: QualityMetrics {
                total_files: 100,
                total_lines: 10000,
                avg_complexity: 5.5,
                max_complexity: 15.0,
                avg_coupling: 3.2,
            },
            hotspot_count: 5,
            convention_count: 10,
        };

        let json = serde_json::to_string(&report).unwrap();
        let deserialized: QualityReport = serde_json::from_str(&json).unwrap();

        assert_eq!(report.project_name, deserialized.project_name);
        assert_eq!(report.hotspot_count, deserialized.hotspot_count);
    }

    #[test]
    fn test_impact_report_serialization() {
        let report = ImpactReport {
            changed_files: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
            affected_modules: vec![Entity {
                id: "entity-1".to_string(),
                project_id: "proj-123".to_string(),
                repo_id: None,
                tier: EntityTier::Module,
                parent_id: None,
                name: "module1".to_string(),
                path: Some("src/module1.rs".to_string()),
                language: None,
                summary: None,
                summary_commit: None,
                metrics_json: None,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
            }],
            affected_subsystems: vec![],
            downstream_dependents: vec![],
            affected_functions: vec![],
        };

        let json = serde_json::to_string(&report).unwrap();
        let deserialized: ImpactReport = serde_json::from_str(&json).unwrap();

        assert_eq!(report.changed_files, deserialized.changed_files);
        assert_eq!(
            report.affected_modules.len(),
            deserialized.affected_modules.len()
        );
    }
}
