use super::project::RepoId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnalysisStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl fmt::Display for AnalysisStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnalysisStatus::Pending => write!(f, "pending"),
            AnalysisStatus::Running => write!(f, "running"),
            AnalysisStatus::Completed => write!(f, "completed"),
            AnalysisStatus::Failed => write!(f, "failed"),
        }
    }
}

impl FromStr for AnalysisStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(AnalysisStatus::Pending),
            "running" => Ok(AnalysisStatus::Running),
            "completed" => Ok(AnalysisStatus::Completed),
            "failed" => Ok(AnalysisStatus::Failed),
            _ => Err(format!("Invalid analysis status: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRun {
    pub id: String,
    pub repo_id: RepoId,
    pub commit_hash: String,
    pub files_analyzed: i64,
    pub files_changed: i64,
    pub entities_upserted: i64,
    pub relationships_upserted: i64,
    pub duration_ms: Option<i64>,
    pub status: AnalysisStatus,
    pub completed_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;
    use std::str::FromStr;

    #[test]
    fn test_analysis_status_display() {
        assert_eq!(AnalysisStatus::Pending.to_string(), "pending");
        assert_eq!(AnalysisStatus::Running.to_string(), "running");
        assert_eq!(AnalysisStatus::Completed.to_string(), "completed");
        assert_eq!(AnalysisStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn test_analysis_status_fromstr_roundtrip() {
        for status in [
            AnalysisStatus::Pending,
            AnalysisStatus::Running,
            AnalysisStatus::Completed,
            AnalysisStatus::Failed,
        ] {
            let s = status.to_string();
            let parsed = AnalysisStatus::from_str(&s).unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[test]
    fn test_analysis_status_fromstr_case_insensitive() {
        assert_eq!(
            AnalysisStatus::from_str("PENDING").unwrap(),
            AnalysisStatus::Pending
        );
        assert_eq!(
            AnalysisStatus::from_str("Pending").unwrap(),
            AnalysisStatus::Pending
        );
        assert_eq!(
            AnalysisStatus::from_str("RUNNING").unwrap(),
            AnalysisStatus::Running
        );
    }

    #[test]
    fn test_analysis_status_fromstr_invalid() {
        assert!(AnalysisStatus::from_str("invalid").is_err());
    }

    #[test]
    fn test_analysis_run_serialization() {
        let run = AnalysisRun {
            id: "run-123".to_string(),
            repo_id: "repo-123".to_string(),
            commit_hash: "abc123".to_string(),
            files_analyzed: 100,
            files_changed: 10,
            entities_upserted: 50,
            relationships_upserted: 200,
            duration_ms: Some(5000),
            status: AnalysisStatus::Completed,
            completed_at: Some("2024-01-01T00:05:00Z".to_string()),
        };

        let json = serde_json::to_string(&run).unwrap();
        let deserialized: AnalysisRun = serde_json::from_str(&json).unwrap();

        assert_eq!(run.id, deserialized.id);
        assert_eq!(run.status, deserialized.status);
        assert_eq!(run.files_analyzed, deserialized.files_analyzed);
    }
}
