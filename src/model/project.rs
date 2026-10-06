use serde::{Deserialize, Serialize};

pub type ProjectId = String;
pub type RepoId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub description: Option<String>,
    pub output_dirs: Option<Vec<String>>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: RepoId,
    pub project_id: ProjectId,
    pub name: String,
    /// The pre-#31 JSON payloads did not serialize this field; `default`
    /// makes the None-default explicit so old payloads keep deserializing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_url: Option<String>,
    pub local_path: String,
    pub default_branch: String,
    pub last_analyzed_commit: Option<String>,
    pub index_path: Option<String>,
    /// Config fingerprint of the last enabled-but-unconfigured summarization
    /// state (issue #788). `None` when no marker is pending; set to the
    /// repo config's fingerprint when the gate was enabled but no backend was
    /// usable, so `is_stale` can skip re-entering the full pipeline for that
    /// repo until a new commit arrives or the config changes. Not serialized
    /// (DB-only column; defaults to `None` for any external construction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarization_unconfigured: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_project_serialization() {
        let project = Project {
            id: "proj-123".to_string(),
            name: "Test Project".to_string(),
            description: Some("A test project".to_string()),
            output_dirs: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&project).unwrap();
        let deserialized: Project = serde_json::from_str(&json).unwrap();

        assert_eq!(project.id, deserialized.id);
        assert_eq!(project.name, deserialized.name);
        assert_eq!(project.description, deserialized.description);
    }

    #[test]
    fn test_repository_serialization() {
        let repo = Repository {
            id: "repo-123".to_string(),
            project_id: "proj-123".to_string(),
            name: "test-repo".to_string(),
            git_url: Some("https://github.com/test/repo.git".to_string()),
            local_path: "/path/to/repo".to_string(),
            default_branch: "main".to_string(),
            last_analyzed_commit: Some("abc123".to_string()),
            index_path: Some("/home/user/.lievo/indices/repo-123".to_string()),
            summarization_unconfigured: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&repo).unwrap();
        let deserialized: Repository = serde_json::from_str(&json).unwrap();

        assert_eq!(repo.id, deserialized.id);
        assert_eq!(repo.project_id, deserialized.project_id);
        assert_eq!(repo.name, deserialized.name);
    }

    /// A non-None `git_url` (the normalized identity key persisted by
    /// `Storage::set_repo_git_url`, issue #31) must round-trip through
    /// JSON — add_repo still writes None, so this is the only path that
    /// exercises the field as Some.
    #[test]
    fn test_repository_serialization_roundtrips_non_none_git_url() {
        let repo = Repository {
            id: "repo-123".to_string(),
            project_id: "proj-123".to_string(),
            name: "test-repo".to_string(),
            git_url: Some("https://github.com/test/repo.git".to_string()),
            local_path: "/path/to/repo".to_string(),
            default_branch: "main".to_string(),
            last_analyzed_commit: None,
            index_path: None,
            summarization_unconfigured: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&repo).unwrap();
        let deserialized: Repository = serde_json::from_str(&json).unwrap();

        assert_eq!(repo.git_url, deserialized.git_url);
        assert_eq!(
            deserialized.git_url.as_deref(),
            Some("https://github.com/test/repo.git")
        );
        assert_eq!(repo.local_path, deserialized.local_path);
    }

    /// A `Repository` constructed without a `git_url` field in the JSON
    /// (pre-#31 payloads) must still deserialize, defaulting to `None`.
    #[test]
    fn test_repository_deserialization_without_git_url_defaults_to_none() {
        let json = r#"{
            "id": "repo-123",
            "project_id": "proj-123",
            "name": "test-repo",
            "local_path": "/path/to/repo",
            "default_branch": "main",
            "last_analyzed_commit": null,
            "index_path": null,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let deserialized: Repository = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized.git_url, None);
    }
}
