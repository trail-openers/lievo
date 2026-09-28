// CodeExtractor trait — abstraction over code extraction implementations.
// Currently implemented by TreeSitterExtractor.

use crate::model::CodeUnit;
use std::any::Any;
use std::path::Path;

/// Trait for extracting and indexing code units from a repository.
///
/// This trait provides a clean abstraction boundary over the extraction layer,
/// allowing implementations to be swapped without
/// changing pipeline or retrieval logic.
pub trait CodeExtractor: Any {
    /// Build or rebuild the code index for the repository.
    ///
    /// # Arguments
    /// * `force_rebuild` - If true, force a full reindex regardless of cache state
    ///
    /// # Returns
    /// Ok(()) on success, or an error if indexing failed
    fn index(&mut self, force_rebuild: bool) -> crate::Result<()>;

    /// Read all code units from the index.
    ///
    /// # Returns
    /// A Vec of all CodeUnit structs in the index
    fn read_all_units(&self) -> crate::Result<Vec<CodeUnit>>;

    /// Read code units for a specific file.
    ///
    /// # Arguments
    /// * `file_path` - The file path to query
    ///
    /// # Returns
    /// A Vec of CodeUnit structs for that file
    fn units_for_file(&self, file_path: &str) -> crate::Result<Vec<CodeUnit>>;

    /// List all files that have been indexed.
    ///
    /// # Returns
    /// A Vec of file paths (as strings) in the index
    fn file_paths(&self) -> crate::Result<Vec<String>>;

    /// Get the total count of code units in the index.
    ///
    /// # Returns
    /// The number of code units
    fn unit_count(&self) -> crate::Result<usize>;

    /// Get the path to the index directory, if set.
    ///
    /// # Returns
    /// Some(&Path) if the index has been built, None otherwise
    fn index_dir(&self) -> Option<&Path>;

    /// Build the semantic search index for extracted units.
    /// Default no-op; implementations that support semantic search override this.
    fn build_semantic_index(
        &self,
        _units: &[crate::model::CodeUnit],
        _index_path: &std::path::Path,
    ) -> crate::Result<()> {
        Ok(())
    }

    /// Get `&dyn Any` for downcasting.
    fn as_any(&self) -> &dyn Any;

    /// Get `&mut dyn Any` for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
