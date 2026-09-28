// Function entity creation from code units - Phase 1 symbol resolution

use crate::model::{CodeUnit, Entity, EntityTier, RelType, Relationship};
use regex::Regex;
use std::collections::{HashMap, HashSet, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::OnceLock;

// Module-level static regex patterns for cfg(test) detection
static CFG_TEST_FN_RE: OnceLock<Regex> = OnceLock::new();
static CFG_TEST_MOD_RE: OnceLock<Regex> = OnceLock::new();
static FN_IN_MOD_RE: OnceLock<Regex> = OnceLock::new();

/// Scans a Rust source file and returns the set of function names declared
/// inside `#[cfg(test)]` blocks. These are test helper functions in production
/// files that should be excluded from the entity graph.
///
/// Uses a lightweight regex scan — does NOT parse AST. Covers two patterns:
/// 1. `#[cfg(test)]` directly preceding `fn` (including pub, async, unsafe, and intermediate attributes)
/// 2. Functions inside `#[cfg(test)] mod ... { ... }` blocks
///
/// # Arguments
/// * `file_path` - Relative path from repo root (e.g., "src/main.rs")
/// * `repo_root` - Absolute path to the repository root directory
///
/// # Path Resolution
///
/// Relative paths are resolved against `repo_root` to ensure correct file access
/// regardless of the current working directory when lievo is invoked.
pub(crate) fn extract_cfg_test_functions(file_path: &str, repo_root: &Path) -> HashSet<String> {
    // Resolve path against repo_root. Path::join handles absolute paths by replacing the base.
    let abs_path = repo_root.join(file_path);

    let Ok(source) = std::fs::read_to_string(&abs_path) else {
        tracing::warn!(path = %abs_path.display(), "Failed to read file for cfg(test) detection");
        return HashSet::new();
    };
    // Early exit: avoid regex scanning if no cfg(test) blocks present
    if !source.contains("#[cfg(test)]") {
        return HashSet::new();
    }
    let mut result = extract_direct_cfg_test_functions(&source);
    result.extend(extract_mod_cfg_test_functions(&source));
    result
}

/// Extracts function names from `#[cfg(test)] fn ...` patterns directly.
/// Handles pub, async, unsafe, and intermediate attributes.
fn extract_direct_cfg_test_functions(source: &str) -> HashSet<String> {
    let re = CFG_TEST_FN_RE.get_or_init(|| {
        Regex::new(r"#\[cfg\(test\)\](?:\s*#\[[^\]]*\])*\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)")
            .expect("cfg(test) fn regex is valid")
    });

    re.captures_iter(source)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

/// Extracts function names from inside `#[cfg(test)] mod ... { ... }` blocks.
///
/// **Known limitations** (by design — heuristic approach, not full AST parsing):
/// - Brace counting is naive: `{` and `}` inside string literals or comments
///   within the mod block will corrupt the block boundary detection.
/// - `FN_IN_MOD_RE` pattern (`\bfn\s+(\w+)`) may match `fn` in comments or
///   string literals inside the block, causing false-positive exclusions.
/// - Attribute values containing `]` (e.g. `#[doc = "see ]"]`) may confuse
///   the direct-fn regex in `extract_direct_cfg_test_functions`.
///
/// These edge cases are acceptable: false positives (production fn excluded) are
/// impossible in practice for well-formed `#[cfg(test)] mod` blocks, and false
/// negatives (test helper leaks) are equivalent to the state before this fix.
/// A proper fix would require AST parsing (syn crate) — unjustified complexity
/// given the rarity of affected patterns.
fn extract_mod_cfg_test_functions(source: &str) -> HashSet<String> {
    let mod_re = CFG_TEST_MOD_RE.get_or_init(|| {
        Regex::new(r"#\[cfg\(test\)\]\s*(?:pub\s+)?mod\s+\w+\s*\{")
            .expect("cfg(test) mod regex is valid")
    });
    let fn_re = FN_IN_MOD_RE
        .get_or_init(|| Regex::new(r"\bfn\s+(\w+)\s*[({]").expect("fn in mod regex is valid"));

    let mut result = HashSet::new();

    for mod_match in mod_re.find_iter(source) {
        // Find the matching closing brace for this module block
        let block_start = mod_match.end();
        let mut depth = 1usize;
        let mut block_end = block_start;
        for (i, ch) in source[block_start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        block_end = block_start + i;
                        break;
                    }
                }
                _ => {}
            }
        }

        if depth != 0 {
            // Malformed source: cfg(test) mod block has unbalanced braces.
            // Skip this module — functions inside may not be excluded from the graph.
            tracing::warn!(
                "cfg(test) mod block has unbalanced braces, skipping (depth={})",
                depth
            );
            continue; // skip to next mod_match
        }

        let block_content = &source[block_start..block_end];
        for cap in fn_re.captures_iter(block_content) {
            if let Some(name) = cap.get(1) {
                result.insert(name.as_str().to_string());
            }
        }
    }

    result
}

/// Note: this intentionally duplicates the logic from `crate::analysis::insights::is_test_file()`
/// rather than sharing it, because that function operates on an Entity struct while this one
/// operates on a raw path string. The extraction layer should not depend on the analysis layer.
/// If the patterns diverge, reconcile them manually.
///
/// Check if a file path indicates a test file.
///
/// Duplicates logic from crate::analysis::insights::is_test_file but operates
/// on a raw path string instead of an Entity. Used in preserve_functions to
/// skip creating Function entities for code in test files.
///
/// Checks both the filename (last path component) for naming patterns and the
/// full path for common test directory names (`/tests/`, `/test/`, `/spec/`,
/// `/__tests__/`). Uses word-boundary patterns on the filename to avoid
/// false positives like "latest", "contest", or "attestation".
///
/// Examples: `user_test.rs`, `test_foo.py`, `bar_spec.rb`, `tests/integration.rs`.
pub fn is_test_file_path(path: &str) -> bool {
    let raw = path;
    let filename = raw.rsplit('/').next().unwrap_or(raw).to_lowercase();
    // Match patterns: test_*.ext, *_test.ext, *_test_*, *_tests.ext (plural), *_spec.ext, *.test.ext, *.spec.ext
    let name_match = filename.starts_with("test_")
        || filename.starts_with("tests_")
        || filename.contains("_test.")
        || filename.contains("_tests.")
        || filename.contains("_test_")
        || filename.contains("_tests_")
        || filename.ends_with("_test")
        || filename.ends_with("_tests")
        || filename.contains("_spec.")
        || filename.contains("_spec_")
        || filename.ends_with("_spec")
        || filename.contains(".test.")
        || filename.contains(".spec.")
        || filename.starts_with("spec_");

    if name_match {
        return true;
    }

    // Also match files inside common test directories.
    // Check if any path segment is a test directory (works for nested directories like src/memory/tests/).
    // Supports both root-relative paths ("tests/foo.rs") and absolute paths ("/project/tests/foo.rs").
    let path_lower = raw.replace('\\', "/").to_lowercase();
    path_lower.split('/').any(|seg| {
        // Strip file extension from segment before matching
        // "tests.rs" → "tests", "test.rs" → "test"
        let stem = seg.split('.').next().unwrap_or(seg);
        matches!(stem, "tests" | "test" | "spec" | "__tests__")
    })
}

/// Check if a code unit represents a function-like entity.
///
/// Tree-sitter extraction emits multiple unit_type values for functions:
/// - "function" - regular functions
/// - "method" - methods defined in classes/structs
/// - "regular_function" - alternative name for regular functions
/// - "closure" - anonymous functions/lambdas
/// - "async_function" - async functions
///
/// Note: This is used for cfg(test) detection only. Type units (struct, enum, etc.) are
/// handled separately by is_type_unit().
pub fn is_function_unit(unit_type: &str) -> bool {
    [
        "function",
        "regular_function",
        "method",
        "closure",
        "async_function",
    ]
    .iter()
    .any(|&t| unit_type.eq_ignore_ascii_case(t))
}

/// Check if a code unit represents a type-like entity (struct, enum, class, etc.).
///
/// The extractor emits unit_type values for types:
/// - "struct" - struct definitions
/// - "class" - class definitions (object-oriented languages)
/// - "enum" - enum definitions
/// - "type" - type aliases
/// - "interface" - interface definitions
/// - "trait" - trait definitions (Rust)
///
/// These are preserved as entities to enable CamelCase search (issue #534).
pub fn is_type_unit(unit_type: &str) -> bool {
    ["struct", "class", "enum", "type", "interface", "trait"]
        .iter()
        .any(|&t| unit_type.eq_ignore_ascii_case(t))
}

/// Check if a code unit should be preserved as an entity.
///
/// Returns true for both function-like and type-like units.
pub fn is_preservable_unit(unit_type: &str) -> bool {
    is_function_unit(unit_type) || is_type_unit(unit_type)
}

/// Preserve functions and types as Entity and Relationship records
///
/// Takes a file entity, its contained code units, and a set of cfg(test) function names,
/// creates Function entities for each preservable unit (functions, methods, closures, structs, enums, etc.),
/// and returns all entities paired with their "contains" relationships back to the file.
///
/// # Entity ID Generation
/// Function IDs are deterministically hashed from file_id + function name,
/// ensuring stable identity across runs.
///
/// # Parameters
/// * `file_entity` - The file entity containing the functions/types
/// * `code_units` - Code units detected in the file
/// * `cfg_test_fns` - Set of function names declared inside #[cfg(test)] blocks
///
/// # Note
/// This function performs NO filesystem I/O. The caller (orchestration layer) is responsible
/// for extracting cfg_test_fns from the file system and passing it in.
pub fn preserve_functions(
    file_entity: &Entity,
    code_units: &[CodeUnit],
    cfg_test_fns: &HashSet<String>,
) -> Vec<(Entity, Relationship)> {
    // If the entire file is a test file, skip all its functions.
    // This check is hoisted outside the loop since the result is constant per file.
    if is_test_file_path(file_entity.path.as_deref().unwrap_or("")) {
        return Vec::new();
    }

    let mut result = Vec::new();

    for unit in code_units {
        // Skip units that are neither functions nor types
        if !is_preservable_unit(&unit.unit_type) {
            continue;
        }

        // Type units (struct, enum, etc.) are not subject to test function filtering
        if is_function_unit(&unit.unit_type) {
            // Layer 2: Skip Rust test functions by naming convention
            // Functions named with "test_" prefix are test functions (Rust convention)
            // and should be excluded even if they appear in production files.
            //
            // Trade-off: production functions named test_* (e.g., test_connection()) will
            // also be excluded. This is acceptable — cfg(test) helper detection requires
            // the extractor to tag conditional compilation blocks (tracked separately).
            // Shared predicate: see `crate::summarization::pipeline::is_test_entity`
            // (issue #649) — must stay in sync with the SQL `LIKE 'test_%'` filter in
            // `COUNT_MISSING_SUMMARIES`.
            if crate::summarization::pipeline::is_test_entity(&unit.name) {
                continue;
            }

            // Layer 3: Skip Rust functions declared inside #[cfg(test)] blocks.
            // These are test helper functions in production files that the extractor
            // emits as regular code units since cfg metadata is not exposed.
            if cfg_test_fns.contains(&unit.name) {
                continue;
            }
        }

        // Types reuse EntityTier::Function — see EntityTier docs for rationale
        let function_entity = Entity {
            id: function_id(&file_entity.id, &unit.name),
            project_id: file_entity.project_id.clone(),
            repo_id: file_entity.repo_id.clone(),
            tier: EntityTier::Function,
            parent_id: Some(file_entity.id.clone()),
            name: unit.name.clone(),
            path: file_entity.path.clone(),
            language: Some(unit.language.clone()),
            summary: unit.docstring.clone(),
            summary_commit: None,
            metrics_json: None,
            created_at: file_entity.created_at.clone(),
            updated_at: file_entity.updated_at.clone(),
        };

        let relationship = Relationship {
            source_id: file_entity.id.clone(),
            target_id: function_entity.id.clone(),
            rel_type: RelType::Contains,
            weight: 1.0,
            evidence_json: None,
            // Structural edge (file owns its extracted function) — resolved.
            provenance: crate::model::EdgeProvenance::Resolved,
        };

        result.push((function_entity, relationship));
    }

    result
}

/// Generate stable Function entity ID from file_id and function name.
///
/// Uses hash-based deterministic ID to ensure the same function in the same file
/// always gets the same ID across runs, enabling incremental updates.
///
/// This is `pub` so that the summarization pipeline can look up function entities
/// by constructing their ID from the corresponding file entity ID and function name.
pub fn function_id(file_id: &str, function_name: &str) -> String {
    let mut hasher = DefaultHasher::new();
    file_id.hash(&mut hasher);
    function_name.hash(&mut hasher);
    let hash = hasher.finish();
    format!("fn-{:x}", hash)
}

/// Build a map from function name to list of Function entity IDs.
/// Used for creating Function-level Calls edges within a file.
///
/// Includes ALL functions, even those with ambiguous names.
/// When resolving calls, ambiguous names will match all candidates.
/// Note: common function names (e.g. `new`, `get`) may match many candidates
/// across files, creating multiple CALLS edges per call site. This is intentional
/// to avoid silently dropping edges, but may produce verbose graphs for very
/// common names.
pub fn build_function_map(functions: &[Entity]) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for func in functions {
        if func.parent_id.is_some() {
            map.entry(func.name.clone())
                .or_default()
                .push(func.id.clone());
        }
    }
    map
}

#[path = "function_preservation_tests_core.rs"]
#[cfg(test)]
mod function_preservation_tests_core;

#[path = "function_preservation_tests_ids_and_mapping.rs"]
#[cfg(test)]
mod function_preservation_tests_ids_and_mapping;

#[path = "function_preservation_tests_cfg_test.rs"]
#[cfg(test)]
mod function_preservation_tests_cfg_test;

#[path = "function_preservation_tests_patterns.rs"]
#[cfg(test)]
mod function_preservation_tests_patterns;

#[path = "function_preservation_tests_types.rs"]
#[cfg(test)]
mod function_preservation_tests_types;

#[path = "function_preservation_tests_types_and_paths.rs"]
#[cfg(test)]
mod function_preservation_tests_types_and_paths;
