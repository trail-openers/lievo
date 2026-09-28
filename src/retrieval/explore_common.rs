//! Shared leaf helpers for the retrieval tool implementations (issue #712).
//!
//! `tools_explore.rs` and `tools_explore_scope.rs` both needed
//! `continuation_pointer` and `should_exclude_entity`; before this module
//! existed they imported them from each other, forming a two-file import
//! cycle. `tools_search.rs` separately carried its own private copy of the
//! `lock_storage!` macro. This module is a genuine leaf (no imports from any
//! `tools_*` module) so every caller depends on it one-directionally.
//!
//! `should_exclude_entity` is a general output-directory filter used across
//! several tools (search, entity, relationship, explore) — it is not
//! scope-listing-specific and does not belong in `tools_explore_scope.rs`.

/// Lock a Mutex, mapping poison errors to `LievoError::RetrievalError`.
macro_rules! lock_storage {
    ($mutex:expr) => {
        $mutex
            .lock()
            .map_err(|_| crate::LievoError::RetrievalError("storage lock poisoned".into()))?
    };
}
pub(crate) use lock_storage;

/// Compose the continuation pointer text: `returned: N, total: M, next`.
pub(crate) fn continuation_pointer(returned: usize, total: usize, next: &str) -> String {
    format!("returned: {returned}, total: {total}, next: \"{next}\"")
}

/// Check if an entity's path matches the output directory to be excluded.
/// Returns true if the entity path matches exactly or is a child of the
/// output directory. General-purpose filter, not scope-listing-specific.
pub(crate) fn should_exclude_entity(
    entity_path: Option<&str>,
    output_dir: &Option<String>,
) -> bool {
    match (entity_path, output_dir.as_deref()) {
        (Some(p), Some(d)) => p == d || p.starts_with(&format!("{d}/")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_pointer_formats_returned_total_next() {
        assert_eq!(
            continuation_pointer(2, 5, "lievo_explore(offset=2)"),
            "returned: 2, total: 5, next: \"lievo_explore(offset=2)\""
        );
    }

    #[test]
    fn should_exclude_entity_matches_exact_and_nested() {
        let out = Some("docs_out".to_string());
        assert!(should_exclude_entity(Some("docs_out"), &out));
        assert!(should_exclude_entity(Some("docs_out/readme.md"), &out));
        assert!(!should_exclude_entity(Some("docs_out2/readme.md"), &out));
        assert!(!should_exclude_entity(Some("src/main.rs"), &out));
    }

    #[test]
    fn should_exclude_entity_false_when_no_output_dir_or_path() {
        assert!(!should_exclude_entity(Some("src/main.rs"), &None));
        assert!(!should_exclude_entity(None, &Some("docs_out".to_string())));
    }
}
