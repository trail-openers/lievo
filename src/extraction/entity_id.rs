use crate::error::{LievoError, Result};
use crate::model::EntityTier;

const MAX_ENTITY_ID_LENGTH: usize = 512;

/// Generates a deterministic entity ID.
/// Format: `<project_id>:<repo_name>:<tier>:<path>`
/// Example: `myproject:myrepo:subsystem:crates/myproject-core`
///
/// # Arguments
/// * `project_id` - Project identifier
/// * `repo_name` - Repository name (not path)
/// * `tier` - Entity tier (Subsystem, Module, File)
/// * `path` - Path relative to repo root, or "." for root
///
/// # Returns
/// Entity ID string
///
/// # Errors
/// Returns `LievoError::InvalidProjectId` if project_id is empty or contains ':'
/// Returns `LievoError::InvalidRepoName` if repo_name is empty or contains ':'
/// Returns `LievoError::EntityIdTooLong` if the generated ID exceeds 512 characters.
pub fn entity_id(
    project_id: &str,
    repo_name: &str,
    tier: EntityTier,
    path: &str,
) -> Result<String> {
    // Validate inputs
    if project_id.is_empty() {
        return Err(LievoError::InvalidProjectId(project_id.to_string()));
    }
    if project_id.contains(':') {
        return Err(LievoError::InvalidProjectId(project_id.to_string()));
    }
    if repo_name.is_empty() {
        return Err(LievoError::InvalidRepoName(repo_name.to_string()));
    }
    if repo_name.contains(':') {
        return Err(LievoError::InvalidRepoName(repo_name.to_string()));
    }

    // Normalize path: replace backslashes with forward slashes, strip trailing slashes
    let normalized_path = normalize_path(path);

    let id = format!("{}:{}:{}:{}", project_id, repo_name, tier, normalized_path);

    if id.len() > MAX_ENTITY_ID_LENGTH {
        return Err(LievoError::EntityIdTooLong(id));
    }

    Ok(id)
}

/// Normalizes a path for entity ID generation and cross-module path comparison.
/// - Replaces backslashes with forward slashes
/// - Strips a leading `./` prefix
/// - Collapses double slashes to single slashes
/// - Strips trailing slashes
/// - Empty / root path becomes "."
///
/// `pub` (not `pub(crate)`) because the binary-local `summarize` command
/// resolves file paths against it; the lib/bin split keeps `crate` distinct
/// between the two targets.
pub fn normalize_path(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);

    // If empty after trimming, it's the root
    if trimmed.is_empty() {
        return ".".to_string();
    }

    // Replace backslashes with forward slashes
    let mut result = trimmed.replace('\\', "/");

    // Collapse double slashes to single slashes
    while result.contains("//") {
        result = result.replace("//", "/");
    }

    // Strip a leading `./` prefix (e.g. git sometimes reports paths with it)
    if let Some(stripped) = result.strip_prefix("./") {
        result = stripped.to_string();
    }

    result
}

/// Validates an entity ID (max 512 characters).
///
/// # Arguments
/// * `id` - Entity ID to validate
///
/// # Errors
/// Returns `LievoError::EntityIdTooLong` if the ID exceeds 512 characters.
pub fn validate_entity_id(id: &str) -> Result<()> {
    if id.len() > MAX_ENTITY_ID_LENGTH {
        return Err(LievoError::EntityIdTooLong(id.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_id_subsystem() {
        let id = entity_id(
            "myproject",
            "myrepo",
            EntityTier::Subsystem,
            "crates/myproject-core",
        )
        .unwrap();
        assert_eq!(id, "myproject:myrepo:subsystem:crates/myproject-core");
    }

    #[test]
    fn test_entity_id_module() {
        let id = entity_id("myproject", "myrepo", EntityTier::Module, "src/main.rs").unwrap();
        assert_eq!(id, "myproject:myrepo:module:src/main.rs");
    }

    #[test]
    fn test_entity_id_file() {
        let id = entity_id("myproject", "myrepo", EntityTier::File, "src/main.rs").unwrap();
        assert_eq!(id, "myproject:myrepo:file:src/main.rs");
    }

    #[test]
    fn test_entity_id_root_path() {
        let id = entity_id("project", "repo", EntityTier::Subsystem, ".").unwrap();
        assert_eq!(id, "project:repo:subsystem:.");
    }

    #[test]
    fn test_entity_id_normalizes_forward_slashes() {
        let id = entity_id("project", "repo", EntityTier::Module, "src/module.rs").unwrap();
        assert_eq!(id, "project:repo:module:src/module.rs");
    }

    #[test]
    fn test_entity_id_normalizes_backslashes_to_forward() {
        let id = entity_id("project", "repo", EntityTier::Module, "src\\module.rs").unwrap();
        assert_eq!(id, "project:repo:module:src/module.rs");
    }

    #[test]
    fn test_entity_id_strips_trailing_slashes() {
        let id1 = entity_id("project", "repo", EntityTier::Module, "src/module/").unwrap();
        assert_eq!(id1, "project:repo:module:src/module");

        let id2 = entity_id("project", "repo", EntityTier::Module, "src/module\\").unwrap();
        assert_eq!(id2, "project:repo:module:src/module");
    }

    #[test]
    fn test_entity_id_empty_path_becomes_root() {
        let id1 = entity_id("project", "repo", EntityTier::Subsystem, "").unwrap();
        assert_eq!(id1, "project:repo:subsystem:.");

        let id2 = entity_id("project", "repo", EntityTier::Subsystem, "/").unwrap();
        assert_eq!(id2, "project:repo:subsystem:.");
    }

    #[test]
    fn test_entity_id_deterministic() {
        let inputs = ("project", "repo", EntityTier::Module, "src/test.rs");

        let id1 = entity_id(inputs.0, inputs.1, inputs.2, inputs.3).unwrap();
        let id2 = entity_id(inputs.0, inputs.1, inputs.2, inputs.3).unwrap();

        assert_eq!(id1, id2);
    }

    #[test]
    fn test_entity_id_human_readable() {
        let id = entity_id(
            "my-project",
            "my-repo",
            EntityTier::File,
            "src/lib/my_module.rs",
        )
        .unwrap();
        assert!(id.contains("my-project"));
        assert!(id.contains("my-repo"));
        assert!(id.contains("file"));
        assert!(id.contains("src/lib/my_module.rs"));
        assert!(id.is_ascii()); // No special chars
    }

    #[test]
    fn test_entity_id_too_long() {
        // Create a path that will exceed 512 total characters
        let long_path = "a/".repeat(500); // ~1000 chars
        let result = entity_id("project", "repo", EntityTier::File, &long_path);
        assert!(matches!(result, Err(LievoError::EntityIdTooLong(_))));
    }

    #[test]
    fn test_entity_id_exactly_max_length() {
        // Create an ID that is exactly 512 characters
        // Format: project_id:repo_name:tier:path
        // "project:repo:file:" is 18 characters, so path needs 494
        let path = "a".repeat(494);
        let result = entity_id("project", "repo", EntityTier::File, &path);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 512);
    }

    #[test]
    fn test_entity_id_one_char_over_max() {
        // Create an ID that is 513 characters
        let path = "a".repeat(495);
        let result = entity_id("project", "repo", EntityTier::File, &path);
        assert!(matches!(result, Err(LievoError::EntityIdTooLong(_))));
    }
    #[test]
    fn test_validate_entity_id_valid() {
        assert!(validate_entity_id("project:repo:module:src/test.rs").is_ok());
    }

    #[test]
    fn test_validate_entity_id_too_long() {
        let long_id = "a".repeat(513);
        let result = validate_entity_id(&long_id);
        assert!(matches!(result, Err(LievoError::EntityIdTooLong(_))));
    }

    #[test]
    fn test_validate_entity_id_exactly_max_length() {
        let id = "a".repeat(512);
        assert!(validate_entity_id(&id).is_ok());
    }

    #[test]
    fn test_entity_id_windows_paths() {
        let id1 = entity_id("project", "repo", EntityTier::File, "src\\utils\\file.rs").unwrap();
        let id2 = entity_id("project", "repo", EntityTier::File, "src/utils/file.rs").unwrap();
        assert_eq!(id1, id2);
        assert_eq!(id1, "project:repo:file:src/utils/file.rs");
    }

    #[test]
    fn test_entity_id_complex_path_normalization() {
        // Mixed slashes, multiple trailing slashes
        let id = entity_id(
            "project",
            "repo",
            EntityTier::Module,
            "src//module\\\\submodule///",
        )
        .unwrap();
        // Note: All backslashes (including internal) are replaced with forward slashes
        // Double slashes are collapsed to single slashes
        // Only TRAILING slashes are stripped
        assert_eq!(id, "project:repo:module:src/module/submodule");
    }

    #[test]
    fn test_entity_id_empty_project_id() {
        let result = entity_id("", "repo", EntityTier::Module, "src/test.rs");
        assert!(matches!(result, Err(LievoError::InvalidProjectId(_))));
    }

    #[test]
    fn test_entity_id_empty_repo_name() {
        let result = entity_id("project", "", EntityTier::Module, "src/test.rs");
        assert!(matches!(result, Err(LievoError::InvalidRepoName(_))));
    }

    #[test]
    fn test_entity_id_project_id_with_colon() {
        let result = entity_id("pro:ject", "repo", EntityTier::Module, "src/test.rs");
        assert!(matches!(result, Err(LievoError::InvalidProjectId(_))));
    }

    #[test]
    fn test_entity_id_repo_name_with_colon() {
        let result = entity_id("project", "re:po", EntityTier::Module, "src/test.rs");
        assert!(matches!(result, Err(LievoError::InvalidRepoName(_))));
    }

    #[test]
    fn test_entity_id_double_slashes_collapsed() {
        let id1 = entity_id("project", "repo", EntityTier::Module, "src//module.rs").unwrap();
        assert_eq!(id1, "project:repo:module:src/module.rs");

        let id2 = entity_id("project", "repo", EntityTier::Module, "src///module.rs").unwrap();
        assert_eq!(id2, "project:repo:module:src/module.rs");

        let id3 = entity_id("project", "repo", EntityTier::Module, "src////module.rs").unwrap();
        assert_eq!(id3, "project:repo:module:src/module.rs");
    }

    #[test]
    fn test_entity_id_mixed_double_slashes_collapsed() {
        let id = entity_id(
            "project",
            "repo",
            EntityTier::Module,
            "src//lib///utils//file.rs",
        )
        .unwrap();
        assert_eq!(id, "project:repo:module:src/lib/utils/file.rs");
    }

    #[test]
    fn test_entity_id_backslashes_and_double_slashes() {
        let id = entity_id(
            "project",
            "repo",
            EntityTier::Module,
            "src\\lib//utils\\//file.rs",
        )
        .unwrap();
        assert_eq!(id, "project:repo:module:src/lib/utils/file.rs");
    }
}
