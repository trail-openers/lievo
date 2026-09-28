use thiserror::Error;

/// Error codes for structured machine-readable error output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    SubprocessIndexFailed,
    Database,
    DatabaseLocked,
    MigrationFailed,
    ProjectNotFound,
    RepoNotFound,
    InvalidRepoPath,
    RepoAlreadyLinked,
    Git,
    NoAnalysisRun,
    InvalidConfig,
    EntityNotFound,
    EntityIdTooLong,
    InvalidProjectId,
    InvalidRepoName,
    InvalidInput,
    PathNotFound,
    JsonParse,
    YamlParse,
    Io,
    RetrievalError,
    SummarizationFailed,
}

impl ErrorCode {
    /// Returns the error code as uppercase snake_case string.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::SubprocessIndexFailed => "SUBPROCESS_INDEX_FAILED",
            ErrorCode::Database => "DATABASE",
            ErrorCode::DatabaseLocked => "DATABASE_LOCKED",
            ErrorCode::MigrationFailed => "MIGRATION_FAILED",
            ErrorCode::ProjectNotFound => "PROJECT_NOT_FOUND",
            ErrorCode::RepoNotFound => "REPO_NOT_FOUND",
            ErrorCode::InvalidRepoPath => "INVALID_REPO_PATH",
            ErrorCode::RepoAlreadyLinked => "REPO_ALREADY_LINKED",
            ErrorCode::Git => "GIT",
            ErrorCode::NoAnalysisRun => "NO_ANALYSIS_RUN",
            ErrorCode::InvalidConfig => "INVALID_CONFIG",
            ErrorCode::EntityNotFound => "ENTITY_NOT_FOUND",
            ErrorCode::EntityIdTooLong => "ENTITY_ID_TOO_LONG",
            ErrorCode::InvalidProjectId => "INVALID_PROJECT_ID",
            ErrorCode::InvalidRepoName => "INVALID_REPO_NAME",
            ErrorCode::InvalidInput => "INVALID_INPUT",
            ErrorCode::PathNotFound => "PATH_NOT_FOUND",
            ErrorCode::JsonParse => "JSON_PARSE",
            ErrorCode::YamlParse => "YAML_PARSE",
            ErrorCode::Io => "IO",
            ErrorCode::RetrievalError => "RETRIEVAL_ERROR",
            ErrorCode::SummarizationFailed => "SUMMARIZATION_FAILED",
        }
    }

    /// Returns the error kind (variant name).
    pub fn kind(self) -> &'static str {
        match self {
            ErrorCode::SubprocessIndexFailed => "SubprocessIndexFailed",
            ErrorCode::Database => "Database",
            ErrorCode::DatabaseLocked => "DatabaseLocked",
            ErrorCode::MigrationFailed => "MigrationFailed",
            ErrorCode::ProjectNotFound => "ProjectNotFound",
            ErrorCode::RepoNotFound => "RepoNotFound",
            ErrorCode::InvalidRepoPath => "InvalidRepoPath",
            ErrorCode::RepoAlreadyLinked => "RepoAlreadyLinked",
            ErrorCode::Git => "Git",
            ErrorCode::NoAnalysisRun => "NoAnalysisRun",
            ErrorCode::InvalidConfig => "InvalidConfig",
            ErrorCode::EntityNotFound => "EntityNotFound",
            ErrorCode::EntityIdTooLong => "EntityIdTooLong",
            ErrorCode::InvalidProjectId => "InvalidProjectId",
            ErrorCode::InvalidRepoName => "InvalidRepoName",
            ErrorCode::InvalidInput => "InvalidInput",
            ErrorCode::PathNotFound => "PathNotFound",
            ErrorCode::JsonParse => "JsonParse",
            ErrorCode::YamlParse => "YamlParse",
            ErrorCode::Io => "Io",
            ErrorCode::RetrievalError => "RetrievalError",
            ErrorCode::SummarizationFailed => "SummarizationFailed",
        }
    }

    /// Returns whether the error is retryable (transient).
    pub fn is_retryable(self) -> bool {
        matches!(self, ErrorCode::DatabaseLocked | ErrorCode::Io)
    }
}

impl LievoError {
    /// Returns the structured error code for machine-readable output.
    pub fn error_code(&self) -> ErrorCode {
        match self {
            LievoError::SubprocessIndexFailed { .. } => ErrorCode::SubprocessIndexFailed,
            LievoError::Database(_) => ErrorCode::Database,
            LievoError::DatabaseLocked => ErrorCode::DatabaseLocked,
            LievoError::MigrationFailed { .. } => ErrorCode::MigrationFailed,
            LievoError::ProjectNotFound(_) => ErrorCode::ProjectNotFound,
            LievoError::RepoNotFound(_) => ErrorCode::RepoNotFound,
            LievoError::InvalidRepoPath(_) => ErrorCode::InvalidRepoPath,
            LievoError::RepoAlreadyLinked { .. } => ErrorCode::RepoAlreadyLinked,
            LievoError::Git(_) => ErrorCode::Git,
            LievoError::NoAnalysisRun(_) => ErrorCode::NoAnalysisRun,
            LievoError::InvalidConfig { .. } => ErrorCode::InvalidConfig,
            LievoError::EntityNotFound(_) => ErrorCode::EntityNotFound,
            LievoError::EntityIdTooLong(_) => ErrorCode::EntityIdTooLong,
            LievoError::InvalidProjectId(_) => ErrorCode::InvalidProjectId,
            LievoError::InvalidRepoName(_) => ErrorCode::InvalidRepoName,
            LievoError::InvalidInput(_) => ErrorCode::InvalidInput,
            LievoError::PathNotFound(_) => ErrorCode::PathNotFound,
            LievoError::JsonParse(_) => ErrorCode::JsonParse,
            LievoError::YamlParse(_) => ErrorCode::YamlParse,
            LievoError::Io(_) => ErrorCode::Io,
            LievoError::RetrievalError(_) => ErrorCode::RetrievalError,
            LievoError::SummarizationFailed(_) => ErrorCode::SummarizationFailed,
        }
    }
}

#[derive(Debug, Error)]
pub enum LievoError {
    #[error("Subprocess indexing failed for {repo_path} with exit code {exit_code}: {stderr}")]
    SubprocessIndexFailed {
        repo_path: String,
        exit_code: i32,
        stderr: String,
    },
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Database locked — is another lievo process running?")]
    DatabaseLocked,
    #[error("Schema migration failed from v{from} to v{to}: {reason}")]
    MigrationFailed { from: u32, to: u32, reason: String },
    #[error("Project '{0}' not found")]
    ProjectNotFound(String),
    #[error("Repository '{0}' not found")]
    RepoNotFound(String),
    #[error("Repository path '{0}' does not exist or is not a git repo")]
    InvalidRepoPath(String),
    #[error("Repository '{repo}' is already linked to project '{project}'")]
    RepoAlreadyLinked { repo: String, project: String },
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("No analysis has been run for repo '{0}'. Run `lievo refresh` first.")]
    NoAnalysisRun(String),
    #[error("Invalid .lievo/config.yaml at '{path}': {reason}")]
    InvalidConfig { path: String, reason: String },
    #[error("Entity '{0}' not found")]
    EntityNotFound(String),
    #[error("Entity ID exceeds maximum length of 512 characters: {0}")]
    EntityIdTooLong(String),
    #[error("Invalid project_id '{0}': cannot be empty or contain ':'")]
    InvalidProjectId(String),
    #[error("Invalid repo_name '{0}': cannot be empty or contain ':'")]
    InvalidRepoName(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("no entity found for path: {0}")]
    PathNotFound(String),
    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),
    #[error("YAML parse error: {0}")]
    YamlParse(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Retrieval error: {0}")]
    RetrievalError(String),
    #[error("Summarization failed: {0}")]
    SummarizationFailed(String),
}

pub type Result<T> = std::result::Result<T, LievoError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_variants() {
        assert!(matches!(
            rusqlite::Error::QueryReturnedNoRows.into(),
            LievoError::Database(_)
        ));
        assert!(matches!(
            git2::Error::from_str("e").into(),
            LievoError::Git(_)
        ));
        assert!(matches!(
            serde_json::from_str::<serde_json::Value>("{x}")
                .unwrap_err()
                .into(),
            LievoError::JsonParse(_)
        ));
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "e");
        assert!(matches!(io_err.into(), LievoError::Io(_)));
    }

    #[test]
    fn test_display_messages() {
        assert!(LievoError::DatabaseLocked.to_string().contains("locked"));
        assert!(
            LievoError::ProjectNotFound("p".into())
                .to_string()
                .contains("Project")
        );
        assert!(
            LievoError::NoAnalysisRun("r".into())
                .to_string()
                .contains("lievo refresh")
        );
    }

    #[test]
    fn test_retrieval_error() {
        let err = LievoError::RetrievalError("semantic index failed".into());
        assert!(err.to_string().contains("Retrieval error"));
        assert!(err.to_string().contains("semantic index failed"));
    }

    #[test]
    fn test_result_alias() {
        fn ok_fn() -> Result<()> {
            Ok(())
        }
        fn err_fn() -> Result<()> {
            Err(LievoError::EntityNotFound("e".into()))
        }
        assert!(ok_fn().is_ok() && err_fn().is_err());
    }

    #[test]
    fn test_debug_formatting() {
        assert!(
            format!("{:?}", LievoError::ProjectNotFound("x".into())).contains("ProjectNotFound")
        );
    }
}
