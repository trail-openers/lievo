use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelType {
    Contains,
    DependsOn,
    Imports,
    Calls,
    Implements,
}

impl fmt::Display for RelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelType::Contains => write!(f, "contains"),
            RelType::DependsOn => write!(f, "depends_on"),
            RelType::Imports => write!(f, "imports"),
            RelType::Calls => write!(f, "calls"),
            RelType::Implements => write!(f, "implements"),
        }
    }
}

impl FromStr for RelType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "contains" => Ok(RelType::Contains),
            "depends_on" => Ok(RelType::DependsOn),
            "imports" => Ok(RelType::Imports),
            "calls" => Ok(RelType::Calls),
            "implements" => Ok(RelType::Implements),
            _ => Err(format!("Invalid relationship type: {}", s)),
        }
    }
}

/// Provenance of a relationship edge: how it was established during analysis.
///
/// `Resolved` marks edges created by exact resolution (normalised-path import
/// resolution, LSP-grade, or structural facts such as containment). `Heuristic`
/// marks edges created by name-matching (bare_name_map/fn_map/tree-sitter call
/// matching) that carry lower confidence. Retrieval consumers (BFS, PPR) apply
/// a ranking discount to hops that traverse `Heuristic` edges.
///
/// Defaults to `Heuristic` for backward compatibility with pre-existing
/// construction sites and as the documented backfill default for rows
/// persisted before this field existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EdgeProvenance {
    Resolved,
    #[default]
    Heuristic,
}

impl fmt::Display for EdgeProvenance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EdgeProvenance::Resolved => write!(f, "resolved"),
            EdgeProvenance::Heuristic => write!(f, "heuristic"),
        }
    }
}

impl EdgeProvenance {
    /// Precedence rank for provenance comparison (issue #714):
    /// `Resolved` (1) outranks `Heuristic` (0). This is the single source of
    /// truth for the resolved-wins order — provenance merging
    /// (`relationships_aggregate::merge_provenance`) delegates to it.
    pub const fn rank(self) -> u8 {
        match self {
            EdgeProvenance::Resolved => 1,
            EdgeProvenance::Heuristic => 0,
        }
    }
}

impl FromStr for EdgeProvenance {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "resolved" => Ok(EdgeProvenance::Resolved),
            "heuristic" => Ok(EdgeProvenance::Heuristic),
            _ => Err(format!("Invalid edge provenance: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub source_id: String,
    pub target_id: String,
    pub rel_type: RelType,
    pub weight: f64,
    pub evidence_json: Option<String>,
    #[serde(default)]
    pub provenance: EdgeProvenance,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;
    use std::str::FromStr;

    #[test]
    fn test_rel_type_display() {
        assert_eq!(RelType::Contains.to_string(), "contains");
        assert_eq!(RelType::DependsOn.to_string(), "depends_on");
        assert_eq!(RelType::Imports.to_string(), "imports");
        assert_eq!(RelType::Calls.to_string(), "calls");
        assert_eq!(RelType::Implements.to_string(), "implements");
    }

    #[test]
    fn test_rel_type_fromstr_roundtrip() {
        for rel_type in [
            RelType::Contains,
            RelType::DependsOn,
            RelType::Imports,
            RelType::Calls,
            RelType::Implements,
        ] {
            let s = rel_type.to_string();
            let parsed = RelType::from_str(&s).unwrap();
            assert_eq!(rel_type, parsed);
        }
    }

    #[test]
    fn test_rel_type_fromstr_case_insensitive() {
        assert_eq!(RelType::from_str("CONTAINS").unwrap(), RelType::Contains);
        assert_eq!(RelType::from_str("Contains").unwrap(), RelType::Contains);
        assert_eq!(RelType::from_str("DEPENDS_ON").unwrap(), RelType::DependsOn);
    }

    #[test]
    fn test_rel_type_fromstr_invalid() {
        assert!(RelType::from_str("invalid").is_err());
    }

    #[test]
    fn test_relationship_serialization() {
        let rel = Relationship {
            source_id: "entity-1".to_string(),
            target_id: "entity-2".to_string(),
            rel_type: RelType::DependsOn,
            weight: 1.5,
            evidence_json: Some(r#"{"file": "src/main.rs", "line": 10}"#.to_string()),
            provenance: EdgeProvenance::Resolved,
        };

        let json = serde_json::to_string(&rel).unwrap();
        let deserialized: Relationship = serde_json::from_str(&json).unwrap();

        assert_eq!(rel.source_id, deserialized.source_id);
        assert_eq!(rel.target_id, deserialized.target_id);
        assert_eq!(rel.rel_type, deserialized.rel_type);
        assert_eq!(rel.weight, deserialized.weight);
        assert_eq!(rel.provenance, deserialized.provenance);
    }

    #[test]
    fn test_edge_provenance_display() {
        assert_eq!(EdgeProvenance::Resolved.to_string(), "resolved");
        assert_eq!(EdgeProvenance::Heuristic.to_string(), "heuristic");
    }

    #[test]
    fn test_edge_provenance_fromstr_roundtrip() {
        for provenance in [EdgeProvenance::Resolved, EdgeProvenance::Heuristic] {
            let s = provenance.to_string();
            let parsed = EdgeProvenance::from_str(&s).unwrap();
            assert_eq!(provenance, parsed);
        }
    }

    #[test]
    fn test_edge_provenance_fromstr_case_insensitive() {
        assert_eq!(
            EdgeProvenance::from_str("RESOLVED").unwrap(),
            EdgeProvenance::Resolved
        );
        assert_eq!(
            EdgeProvenance::from_str("Heuristic").unwrap(),
            EdgeProvenance::Heuristic
        );
    }

    #[test]
    fn test_edge_provenance_fromstr_invalid() {
        assert!(EdgeProvenance::from_str("unknown").is_err());
    }

    #[test]
    fn test_edge_provenance_default_is_heuristic() {
        assert_eq!(EdgeProvenance::default(), EdgeProvenance::Heuristic);
    }

    #[test]
    fn test_edge_provenance_rank_resolved_outranks_heuristic() {
        assert_eq!(EdgeProvenance::Resolved.rank(), 1);
        assert_eq!(EdgeProvenance::Heuristic.rank(), 0);
        assert!(EdgeProvenance::Resolved.rank() > EdgeProvenance::Heuristic.rank());
    }

    #[test]
    fn test_relationship_provenance_field_defaults_when_absent_from_json() {
        // Backward compatibility: JSON payloads produced before this field
        // existed (or from construction sites not yet updated) must still
        // deserialize, defaulting provenance to Heuristic (the documented
        // backfill default).
        let json = r#"{"source_id":"e1","target_id":"e2","rel_type":"Calls","weight":1.0,"evidence_json":null}"#;
        let rel: Relationship = serde_json::from_str(json).unwrap();
        assert_eq!(rel.provenance, EdgeProvenance::Heuristic);
    }
}
