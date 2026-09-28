use super::tree_sitter_utils::ts_index_dir_for_repo;
use crate::error::{LievoError, Result};
use crate::extraction::call_extraction;
use crate::extraction::code_extractor::CodeExtractor;
use crate::extraction::go_imports;
use crate::extraction::ts_metrics;
use crate::model::CodeUnit;
use ignore::WalkBuilder;
use sha2::{Digest, Sha256};

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Parser};

pub struct TreeSitterExtractor {
    repo_path: PathBuf,
    respect_ignore: bool,
    units: Vec<CodeUnit>,
    index_dir: Option<PathBuf>,
    known_hashes: HashMap<String, String>,
    new_hashes: HashMap<String, String>,
    files_parsed: Vec<String>,
    file_set_changed: bool,
    extracted_files: Vec<String>,
}

impl TreeSitterExtractor {
    pub fn new(repo_path: &Path, respect_ignore: bool) -> Result<Self> {
        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            respect_ignore,
            units: Vec::new(),
            index_dir: None,
            known_hashes: HashMap::new(),
            new_hashes: HashMap::new(),
            files_parsed: Vec::new(),
            file_set_changed: false,
            extracted_files: Vec::new(),
        })
    }

    /// Set known file hashes for incremental reindexing.
    pub fn set_known_hashes(&mut self, hashes: HashMap<String, String>) {
        self.known_hashes = hashes;
    }

    /// Take the new file hashes computed during the last index() call.
    pub fn take_new_hashes(&mut self) -> HashMap<String, String> {
        std::mem::take(&mut self.new_hashes)
    }

    /// Get whether the file set changed during the last index() call.
    pub fn file_set_changed(&self) -> bool {
        self.file_set_changed
    }

    /// Get the list of files that were actually parsed during the last index() call.
    pub fn files_parsed(&self) -> &[String] {
        &self.files_parsed
    }

    /// Get every scanned source file (including zero-unit ones) from the last
    /// index() call — used by grouping to seed file entities for zero-unit
    /// files (issue #701: a file whose exports are all wrapped call-expression
    /// forms, or that has no extractable functions at all, must still get a
    /// file entity). Unchanged zero-unit files are re-recorded by each index()
    /// call (including incremental re-indexes), so this list stays complete.
    pub fn extracted_files(&self) -> &[String] {
        &self.extracted_files
    }

    /// Take the per-file hashes recorded during the last index() call and
    /// restore them as the extractor's known hashes — i.e. the state a
    /// subsequent index() call would start from (issue #701 incremental
    /// re-index test seam, mirrors the pipeline's known-hashes handoff).
    pub fn set_last_hash_state(&mut self) {
        self.known_hashes.clear();
        self.known_hashes
            .extend(std::mem::take(&mut self.new_hashes));
    }

    fn get_language(&self, file_path: &Path) -> Option<Language> {
        let ext = file_path.extension().and_then(|s| s.to_str())?;
        match ext {
            "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
            "py" => Some(tree_sitter_python::LANGUAGE.into()),
            "js" | "jsx" | "mjs" | "ts" | "tsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
            "go" => Some(tree_sitter_go::LANGUAGE.into()),
            _ => None,
        }
    }

    fn language_name(ext: &str) -> &'static str {
        match ext {
            "rs" => "Rust",
            "py" => "Python",
            "js" | "jsx" | "mjs" => "JavaScript",
            "ts" | "tsx" => "TypeScript",
            "go" => "Go",
            _ => "Unknown",
        }
    }

    /// Compute content hash for a file using SHA-256.
    fn compute_file_hash(&self, file_path: &Path) -> Option<String> {
        let bytes = fs::read(file_path).ok()?;
        let digest = Sha256::digest(&bytes);
        Some(digest.iter().map(|b| format!("{:02x}", b)).collect())
    }

    fn extract_from_file(&mut self, file_path: &Path) -> Result<Vec<CodeUnit>> {
        let ext = file_path
            .extension()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                LievoError::InvalidInput("Could not determine file extension".to_string())
            })?;

        let language = self
            .get_language(file_path)
            .ok_or_else(|| LievoError::InvalidInput(format!("Unsupported file type: {}", ext)))?;

        let source_code = fs::read_to_string(file_path).map_err(LievoError::Io)?;

        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| LievoError::InvalidInput(format!("Failed to set language: {:?}", e)))?;

        let tree = parser
            .parse(&source_code, None)
            .ok_or_else(|| LievoError::InvalidInput("Failed to parse file".to_string()))?;

        let mut units = Vec::new();
        // Use repo-relative path (strip the repo root prefix) so that
        // CodeUnit.file and UsearchSearcher metadata carry repo-relative
        // paths, matching the canonical form used by probes and entity
        // storage (Decision 4 — issue #665).
        let file_path_str = file_path
            .strip_prefix(&self.repo_path)
            .ok()
            .and_then(|p| p.to_str())
            .unwrap_or(file_path.to_str().unwrap_or(""))
            .to_string();
        let lang_name = Self::language_name(ext);

        // Extract file-level imports once
        let file_imports =
            self.extract_imports_from_file(tree.root_node(), &source_code, lang_name);

        // Simple tree walk to extract functions
        self.extract_functions_from_node(
            tree.root_node(),
            &source_code,
            lang_name,
            &file_path_str,
            &file_imports,
            &mut units,
        );

        // `export default <wrapped form>` has no variable binding, so the
        // walk above cannot name it — extract it as a single unit named after
        // the file stem (issue #701).
        self.extract_wrapped_default_export(
            tree.root_node(),
            &source_code,
            lang_name,
            &file_path_str,
            &file_imports,
            &mut units,
        );

        // #706 root-cause B: a file with zero function-shaped units (e.g. a
        // barrel of re-exports, or a file whose only content is top-level
        // import statements) has no unit to carry the file-level import list,
        // so the builder's per-unit `unit.imports` walk never sees them — no
        // edge is produced and unresolved_internal reports 0, defeating the
        // #690 false-orphan guard. Emit a synthetic marker unit that carries
        // the file-level import list so the builder walks it. The marker is
        // distinguished by `unit_type == "file_imports_marker"` so downstream
        // consumers (function-level edge building, coverage, etc.) can skip
        // it; it has no name, no calls, no code, and no metrics.
        if units.is_empty() && !file_imports.is_empty() {
            units.push(CodeUnit {
                name: String::new(),
                unit_type: "file_imports_marker".to_string(),
                file: file_path_str.clone(),
                line: 1,
                end_line: 1,
                language: lang_name.to_string(),
                signature: None,
                code: None,
                calls: Vec::new(),
                imports: file_imports.clone(),
                complexity: 0,
                has_branches: false,
                has_loops: false,
                has_error_handling: false,
                qualified_name: format!(
                    "{}::file_imports_marker",
                    file_path_str.replace('/', "::")
                ),
                docstring: None,
                parent_class: None,
            });
        }

        Ok(units)
    }

    fn extract_imports_from_file(
        &self,
        root_node: tree_sitter::Node,
        source_code: &str,
        language: &str,
    ) -> Vec<String> {
        match language {
            "Go" => {
                // Iterate over top-level nodes only (Go imports are always at file level)
                go_imports::extract_go_imports(root_node, source_code)
            }
            _ => {
                let mut imports = Vec::new();
                let mut seen = std::collections::HashSet::new();

                let mut cursor = root_node.walk();
                for child in root_node.children(&mut cursor) {
                    let specifier = if matches!(language, "JavaScript" | "TypeScript") {
                        // `source` field of import_statement, e.g. `import x from "mod"` → mod
                        (child.kind() == "import_statement")
                            .then(|| child.child_by_field_name("source"))
                            .flatten()
                    } else if language == "Rust" {
                        // `argument` field of use_declaration, e.g. `use std::io;` → std::io
                        (child.kind() == "use_declaration")
                            .then(|| child.child_by_field_name("argument"))
                            .flatten()
                    } else {
                        // Python (and unknown): no specifier field — whole-statement text
                        import_kinds_for(language)
                            .contains(&child.kind())
                            .then_some(child)
                    };

                    if let Some(text) =
                        specifier.and_then(|n| n.utf8_text(source_code.as_bytes()).ok())
                    {
                        let import_text = if matches!(language, "JavaScript" | "TypeScript") {
                            // The `source` field is a quoted string node — strip the quotes
                            text.trim().trim_matches('"').trim_matches('\'').to_string()
                        } else {
                            text.trim().to_string()
                        };
                        if !import_text.is_empty() && seen.insert(import_text.clone()) {
                            imports.push(import_text);
                        }
                    }
                }
                imports
            }
        }
    }
}

/// Import statement node kinds that carry no specifier field (whole-statement fallback).
fn import_kinds_for(language: &str) -> &'static [&'static str] {
    match language {
        "Rust" => &["use_declaration"],
        "Python" => &["import_statement", "import_from_statement"],
        "JavaScript" | "TypeScript" => &["import_statement"],
        _ => &[],
    }
}

// Function extraction methods (`extract_functions_from_node`, `is_function_node`,
// `extract_function_unit`, `build_function_unit`, `extract_function_name`,
// `js_function_name`) live in `ts_function_extraction.rs`, included at module
// level (issue #702: keep this file under the 500-line limit).
include!("ts_function_extraction.rs");

// Wrapped-export helpers (`is_wrapped_component_binding`, `value_is_wrapped_form`,
// `extract_wrapped_default_export`, `default_export_name`, `is_styled_callee`,
// `is_wrapper_callee`, `call_takes_inline_function`) live in
// `ts_wrapped_exports.rs`, included at module level (the batch_ops pattern —
// issue #702: keep this file under the 500-line limit). The items keep their
// original paths: `Self::…` from the extractor, `super::…` from the tests.
include!("ts_wrapped_exports.rs");

impl CodeExtractor for TreeSitterExtractor {
    fn index(&mut self, force_rebuild: bool) -> Result<()> {
        // Reset per-call tracking state.
        self.units.clear();
        self.new_hashes.clear();
        self.files_parsed.clear();
        self.extracted_files.clear();
        self.file_set_changed = false;

        let mut all_units = Vec::new();
        let mut current_paths: Vec<String> = Vec::new();

        let walker = WalkBuilder::new(&self.repo_path)
            .standard_filters(self.respect_ignore)
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if let Some(ext) = path.extension()
                && let Some(ext_str) = ext.to_str()
                && matches!(
                    ext_str,
                    "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "mjs" | "go"
                )
            {
                let file_hash = self.compute_file_hash(path);
                let file_path_rel = path
                    .strip_prefix(&self.repo_path)
                    .ok()
                    .and_then(|p| p.to_str())
                    .unwrap_or(path.to_str().unwrap_or(""))
                    .to_string();

                current_paths.push(file_path_rel.clone());
                // Record every scanned source path (issue #701) —
                // including files that will produce zero code units —
                // so grouping can seed file entities. Done for EVERY
                // candidate file, regardless of whether it is re-parsed:
                // an incremental re-index that skips unchanged files
                // must still report zero-unit files.
                self.extracted_files.push(file_path_rel.clone());

                if let Some(hash) = file_hash {
                    self.new_hashes.insert(file_path_rel.clone(), hash.clone());

                    let should_parse = force_rebuild
                        || !self.known_hashes.contains_key(&file_path_rel)
                        || self.known_hashes.get(&file_path_rel) != Some(&hash);

                    if should_parse {
                        self.files_parsed.push(file_path_rel.clone());
                        match self.extract_from_file(path) {
                            Ok(units) => all_units.extend(units),
                            Err(e) => {
                                eprintln!("  warning: skipped {} ({})", path.display(), e)
                            }
                        }
                    }
                } else {
                    eprintln!("  warning: could not hash {}, skipping", path.display());
                }
            }
        }

        // Detect file set changes: new files, deleted files, or hash mismatches
        let known_paths: std::collections::HashSet<_> = self.known_hashes.keys().cloned().collect();
        let current_paths_set: std::collections::HashSet<_> =
            current_paths.iter().cloned().collect();

        self.file_set_changed = known_paths != current_paths_set
            || self.files_parsed.iter().any(|path| {
                self.new_hashes
                    .get(path)
                    .and_then(|new_hash| self.known_hashes.get(path).map(|old| old != new_hash))
                    .unwrap_or(true)
            });

        self.units = all_units;

        let ts_index_dir = ts_index_dir_for_repo(&self.repo_path)?;
        fs::create_dir_all(&ts_index_dir).map_err(LievoError::Io)?;
        self.index_dir = Some(ts_index_dir);

        Ok(())
    }

    fn read_all_units(&self) -> Result<Vec<CodeUnit>> {
        Ok(self.units.clone())
    }

    fn units_for_file(&self, file_path: &str) -> Result<Vec<CodeUnit>> {
        Ok(self
            .units
            .iter()
            .filter(|u| u.file == file_path)
            .cloned()
            .collect())
    }

    fn file_paths(&self) -> Result<Vec<String>> {
        let mut paths = self
            .units
            .iter()
            .map(|u| u.file.clone())
            .collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn unit_count(&self) -> Result<usize> {
        Ok(self.units.len())
    }

    fn index_dir(&self) -> Option<&Path> {
        self.index_dir.as_deref()
    }

    fn build_semantic_index(
        &self,
        units: &[CodeUnit],
        index_path: &std::path::Path,
    ) -> crate::Result<()> {
        crate::retrieval::usearch_searcher::build_usearch_searcher(units, index_path)?;
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
#[path = "tree_sitter_extractor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tree_sitter_extractor_separate_tests.rs"]
mod separate_tests;
