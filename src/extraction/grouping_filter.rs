// Path filtering for grouping heuristic - excludes documentation output directories

/// Check if a file path matches any excluded path (e.g., doc output directory).
/// Normalizes paths by removing leading `./` and trailing `/` before comparison.
pub fn should_exclude_path(file_path: &str, exclude_paths: &[String]) -> bool {
    let normalized = file_path.trim_start_matches("./");
    exclude_paths.iter().any(|excluded| {
        let excl = excluded.trim_start_matches("./").trim_end_matches('/');
        normalized == excl || normalized.starts_with(&format!("{}/", excl))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_exclude_path() {
        let exclude_paths = vec!["docs".to_string(), "lievo_docs".to_string()];
        assert!(should_exclude_path("docs", &exclude_paths));
        assert!(should_exclude_path("docs/index.md", &exclude_paths));
        assert!(should_exclude_path("lievo_docs/README.md", &exclude_paths));
        assert!(!should_exclude_path("src/main.rs", &exclude_paths));
        assert!(!should_exclude_path("src/docs/utils.rs", &exclude_paths));
    }

    #[test]
    fn test_should_exclude_path_with_trailing_slash() {
        let exclude_paths = vec!["docs/".to_string(), "./lievo_docs/".to_string()];
        assert!(should_exclude_path("docs", &exclude_paths));
        assert!(should_exclude_path("docs/index.md", &exclude_paths));
        assert!(should_exclude_path("lievo_docs/README.md", &exclude_paths));
    }

    #[test]
    fn test_should_exclude_path_with_leading_dotslash() {
        let exclude_paths = vec!["./docs".to_string(), "lievo_docs".to_string()];
        assert!(should_exclude_path("./docs", &exclude_paths));
        assert!(should_exclude_path("docs", &exclude_paths));
        assert!(should_exclude_path("./docs/index.md", &exclude_paths));
        assert!(should_exclude_path("docs/index.md", &exclude_paths));
    }

    #[test]
    fn test_should_exclude_path_mixed_normalizations() {
        let exclude_paths = vec!["./docs/".to_string()];
        assert!(should_exclude_path("docs", &exclude_paths));
        assert!(should_exclude_path("./docs", &exclude_paths));
        assert!(should_exclude_path("docs/", &exclude_paths));
        assert!(should_exclude_path("./docs/", &exclude_paths));
        assert!(should_exclude_path("docs/index.md", &exclude_paths));
        assert!(should_exclude_path("./docs/index.md", &exclude_paths));
    }
}
