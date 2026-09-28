use super::project::ProjectId;
use super::project::RepoId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Hierarchical tier of an entity in the project structure.
///
/// Tiers form a containment hierarchy: Subsystem → Module → File → Function.
/// Each tier represents a level of abstraction in code organization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityTier {
    /// Subsystem-level grouping (top-level component or module group, e.g., packages or namespaces)
    Subsystem,
    /// Module-level grouping (logical grouping of related code, typically a file or directory)
    Module,
    /// File-level entity (individual source file containing code)
    File,
    /// Code-unit level below File.
    ///
    /// This tier encompasses both function definitions AND named type definitions
    /// (structs, enums, traits, classes, interfaces, type aliases).
    ///
    /// Rationale: Types and functions share similar semantic significance for
    /// analysis (both are call graph nodes, both can be referenced by name, both
    /// benefit from searchable entity IDs). They occupy the same granularity level
    /// in the containment hierarchy (children of File entities), sharing Function tier
    /// avoids unnecessary tier proliferation.
    Function,
}

impl fmt::Display for EntityTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityTier::Subsystem => write!(f, "subsystem"),
            EntityTier::Module => write!(f, "module"),
            EntityTier::File => write!(f, "file"),
            EntityTier::Function => write!(f, "function"),
        }
    }
}

impl FromStr for EntityTier {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "subsystem" => Ok(EntityTier::Subsystem),
            "module" => Ok(EntityTier::Module),
            "file" => Ok(EntityTier::File),
            "function" => Ok(EntityTier::Function),
            _ => Err(format!("Invalid entity tier: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub project_id: ProjectId,
    pub repo_id: Option<RepoId>,
    pub tier: EntityTier,
    pub parent_id: Option<String>,
    pub name: String,
    pub path: Option<String>,
    pub language: Option<String>,
    pub summary: Option<String>,
    pub summary_commit: Option<String>,
    pub metrics_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Code unit extracted from source code analysis (lievo-owned representation).
/// Keeps only the fields consumed downstream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeUnit {
    pub name: String,
    pub unit_type: String,
    pub file: String,
    pub line: i64,
    pub end_line: i64,
    pub language: String,
    pub signature: Option<String>,
    pub code: Option<String>,
    pub calls: Vec<String>,
    pub imports: Vec<String>,
    pub complexity: i64,
    pub has_branches: bool,
    pub has_loops: bool,
    pub has_error_handling: bool,
    // Kept for downstream usage (relationship resolution, function extraction, tools)
    pub qualified_name: String,
    pub docstring: Option<String>,
    pub parent_class: Option<String>,
}

#[cfg(test)]
impl CodeUnit {
    /// Test helper: create a minimal code unit with common defaults.
    /// Useful for quickly constructing test fixtures without specifying all fields.
    pub fn test_unit(name: &str, file: &str) -> Self {
        Self {
            name: name.to_string(),
            unit_type: "function".to_string(),
            file: file.to_string(),
            line: 1,
            end_line: 10,
            language: "Rust".to_string(),
            signature: None,
            code: None,
            calls: vec![],
            imports: vec![],
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            qualified_name: name.to_string(),
            docstring: None,
            parent_class: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;
    use std::str::FromStr;

    #[test]
    fn test_entity_tier_display() {
        assert_eq!(EntityTier::Subsystem.to_string(), "subsystem");
        assert_eq!(EntityTier::Module.to_string(), "module");
        assert_eq!(EntityTier::File.to_string(), "file");
        assert_eq!(EntityTier::Function.to_string(), "function");
    }

    #[test]
    fn test_entity_tier_fromstr_roundtrip() {
        for tier in [
            EntityTier::Subsystem,
            EntityTier::Module,
            EntityTier::File,
            EntityTier::Function,
        ] {
            let s = tier.to_string();
            let parsed = EntityTier::from_str(&s).unwrap();
            assert_eq!(tier, parsed);
        }
    }

    #[test]
    fn test_entity_tier_fromstr_case_insensitive() {
        assert_eq!(
            EntityTier::from_str("SUBSYSTEM").unwrap(),
            EntityTier::Subsystem
        );
        assert_eq!(
            EntityTier::from_str("Subsystem").unwrap(),
            EntityTier::Subsystem
        );
        assert_eq!(EntityTier::from_str("Module").unwrap(), EntityTier::Module);
    }

    #[test]
    fn test_entity_tier_fromstr_invalid() {
        assert!(EntityTier::from_str("invalid").is_err());
    }

    #[test]
    fn test_entity_serialization() {
        let entity = Entity {
            id: "entity-123".to_string(),
            project_id: "proj-123".to_string(),
            repo_id: Some("repo-123".to_string()),
            tier: EntityTier::Module,
            parent_id: Some("parent-123".to_string()),
            name: "test_module".to_string(),
            path: Some("src/test_module.rs".to_string()),
            language: Some("Rust".to_string()),
            summary: Some("A test module".to_string()),
            summary_commit: Some("abc123".to_string()),
            metrics_json: Some(r#"{"complexity_max": 5}"#.to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&entity).unwrap();
        let deserialized: Entity = serde_json::from_str(&json).unwrap();

        assert_eq!(entity.id, deserialized.id);
        assert_eq!(entity.tier, deserialized.tier);
    }

    #[test]
    fn test_code_unit_serialization() {
        let unit = CodeUnit {
            name: "test_function".to_string(),
            unit_type: "function".to_string(),
            file: "src/module.rs".to_string(),
            line: 10,
            end_line: 50,
            language: "Rust".to_string(),
            signature: Some("fn test_function() -> Result<()>".to_string()),
            code: Some("fn test_function() { ... }".to_string()),
            complexity: 5,
            has_branches: true,
            has_loops: false,
            has_error_handling: true,
            calls: vec!["helper_fn".to_string(), "other_fn".to_string()],
            imports: vec!["std::collections::HashMap".to_string()],
            qualified_name: "module::test_function".to_string(),
            docstring: Some("Test function doc".to_string()),
            parent_class: None,
        };

        // Verify all fields are present and serializable
        let json = serde_json::to_string(&unit).unwrap();
        let deserialized: CodeUnit = serde_json::from_str(&json).unwrap();

        assert_eq!(unit.name, deserialized.name);
        assert_eq!(unit.calls, deserialized.calls);
        assert_eq!(unit.imports, deserialized.imports);
        assert_eq!(unit.qualified_name, deserialized.qualified_name);
        assert_eq!(unit.docstring, deserialized.docstring);
        assert_eq!(unit.parent_class, deserialized.parent_class);
    }
}
