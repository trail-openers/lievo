// Fixed, checked-in entity sample for the summarizer A/B harness (issue #773).
//
// The sample is a `&'static const` set of entities drawn from lievo's own
// source. It is designed to span:
//   - function-tier: short functions (a few lines) AND long functions (40+ lines)
//   - rollup-tier:  File / Module / Subsystem shapes (3 child-summary lines + header)
//
// This prevents the harness from flattering every model (tiny functions only)
// or missing the rollup prompt shape that issue #771 is changing.
//
// Stability: the sample is a const — no randomness, no filesystem reads.
// A unit test asserts `SAMPLE` is identical across two successive reads.

use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

/// The tier of an entity in the A/B sample, matching `EntityTier` in the
/// production model. Used only for labeling in the artifact; the harness
/// treats all tiers uniformly for parse-success purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SampleTier {
    /// A function-tier entity: the body is the function source code.
    Function,
    /// A rollup-tier entity: the body is a pre-built prompt with a header
    /// and up to three child-summary lines, matching the shape used by
    /// `rollup_batch_summarize` in `pipeline_rollup.rs`.
    Rollup,
}

/// One entity in the fixed A/B sample.
#[derive(Debug, Clone, Serialize)]
pub struct SampleEntity {
    /// Stable identifier, e.g. `"fn:parse_batch_response"`.
    pub id: &'static str,
    /// Display name for the artifact, e.g. `"parse_batch_response"`.
    pub name: &'static str,
    /// Tier: function-tier or rollup-tier.
    pub tier: SampleTier,
    /// The entity body.
    /// - Function tier: the function source code (Rust).
    /// - Rollup tier: the pre-built prompt string (header + child lines).
    pub body: &'static str,
}

/// The fixed A/B sample. All entries are `&'static str` — no heap allocation.
///
/// The sample is deliberately small (8 entities) so the harness runs quickly
/// but large enough to test batch sizes 2, 4, and 8. For batch size 16,
/// the harness will repeat the sample to fill the batch.
pub const SAMPLE: &[SampleEntity] = &[
    // ── Function-tier: short (< 100 chars body) ─────────────────────────
    SampleEntity {
        id: "fn:is_function_unit",
        name: "is_function_unit",
        tier: SampleTier::Function,
        body: "fn is_function_unit(t: &str) -> bool {\n    t == \"function\" || t == \"method\"\n}",
    },
    SampleEntity {
        id: "fn:is_test_entity",
        name: "is_test_entity",
        tier: SampleTier::Function,
        body: "fn is_test_entity(name: &str) -> bool {\n    name.starts_with(\"test_\")\n}",
    },
    // ── Function-tier: medium (~200–500 chars) ──────────────────────────
    SampleEntity {
        id: "fn:normalise_path",
        name: "normalise_path",
        tier: SampleTier::Function,
        body: "pub fn normalise_path(path: &str, repo_root: &str) -> String {\n    let lower = |s: &str| s.replace('\\\\', \"/\");\n    let mut p = lower(path);\n    let root = lower(repo_root);\n    let stripped = p.strip_prefix(&format!(\"{root}/\")).map(String::from);\n    let stripped = stripped.or_else(|| p.strip_prefix(&root).map(String::from));\n    if let Some(s) = stripped { p = s; }\n    while let Some(s) = p.strip_prefix(\"./\") { p = s.to_string(); }\n    while p.starts_with('/') { p = p.trim_start_matches('/').to_string(); }\n    p\n}",
    },
    SampleEntity {
        id: "fn:resolve_input_char_budget",
        name: "resolve_input_char_budget",
        tier: SampleTier::Function,
        body: "pub fn resolve_input_char_budget(\n    transport: Option<&BackendTransport>,\n    config: Option<&SummarizerBackend>,\n) -> usize {\n    match transport {\n        Some(t) => backend_profile::resolve_for_backend(t.backend, config),\n        None => backend_profile::APFEL_INPUT_CHAR_BUDGET,\n    }\n}",
    },
    // ── Function-tier: long (> 500 chars) ───────────────────────────────
    SampleEntity {
        id: "fn:pack_by_char_budget",
        name: "pack_by_char_budget",
        tier: SampleTier::Function,
        body: "pub fn pack_by_char_budget(\n    snippets: &[(String, String, String)],\n    budget: usize,\n) -> Vec<Vec<(String, String, String)>> {\n    let mut batches = Vec::new();\n    let mut current_batch = Vec::new();\n    let mut current_size = 0usize;\n    let prompt_overhead = 81 + 2;\n    let effective_budget = budget.saturating_sub(prompt_overhead);\n    for (file, name, code) in snippets {\n        let entry_size = 25 + code.len();\n        if current_size > 0 && current_size + entry_size > effective_budget {\n            batches.push(current_batch);\n            current_batch = Vec::new();\n            current_size = 0;\n        }\n        current_batch.push((file.clone(), name.clone(), code.clone()));\n        current_size += entry_size;\n    }\n    if !current_batch.is_empty() { batches.push(current_batch); }\n    batches\n}",
    },
    SampleEntity {
        id: "fn:parse_batch_response",
        name: "parse_batch_response",
        tier: SampleTier::Function,
        body: "pub(crate) fn parse_batch_response(content: &str, num_snippets: usize) -> Vec<Option<String>> {\n    let mut results = vec![None; num_snippets];\n    for line in content.lines() {\n        if let Some(hash_pos) = line.find('#') {\n            let after_hash = &line[hash_pos + 1..];\n            if let Some(colon_pos) = after_hash.find(':') {\n                let index_str = &after_hash[..colon_pos].trim();\n                if let Ok(index) = index_str.parse::<usize>() {\n                    let summary = after_hash[colon_pos + 1..].trim().to_string();\n                    if index < num_snippets { results[index] = Some(summary); }\n                }\n            }\n        }\n    }\n    results\n}",
    },
    // ── Rollup-tier: File shape ─────────────────────────────────────────
    // Matches the template in pipeline_rollup.rs for EntityTier::File:
    // "Based on the following components of the file '{name}'..."
    SampleEntity {
        id: "rollup:file:summarization",
        name: "summarization",
        tier: SampleTier::Rollup,
        body: "Based on the following components of the file 'summarization', write a concise 1-2 sentence architectural description of what this file is responsible for.\nFocus on PURPOSE and ROLE. Do NOT describe individual functions.\nRespond with only the description, no preamble.\n\nComponents:\n- parse_batch_response (src/summarization/apfel.rs): Parses #n: lines from batch responses into positional result vectors\n- pack_by_char_budget (src/summarization/apfel.rs): Packs code snippets into batches respecting a character budget\n- batch_summarize (src/summarization/apfel.rs): Sends a batch of code snippets to apfel and returns per-snippet summaries",
    },
    // ── Rollup-tier: Module shape ───────────────────────────────────────
    // Matches the template in pipeline_rollup.rs for EntityTier::Module:
    // "Describe the role of Module '{name}' in the codebase."
    SampleEntity {
        id: "rollup:module:extraction",
        name: "extraction",
        tier: SampleTier::Rollup,
        body: "Describe the role of Module 'extraction' in the codebase.\nComponents:\n- TreeSitterExtractor (src/extraction/tree_sitter_extractor.rs): Indexes a repository using tree-sitter parsers for Rust, Python, JS, and Go\n- CodeExtractor (src/extraction/code_extractor.rs): Trait for code extraction strategies; produces CodeUnit and entity records\n- GroupingHeuristics (src/extraction/grouping.rs): Heuristic functions that group related code units into entities",
    },
];

/// Validate that the sample is well-formed:
/// - all IDs are unique
/// - all names are non-empty
/// - all bodies are non-empty
/// - the sample contains at least one function-tier AND one rollup-tier entity
/// - the sample contains both short (< 100 chars) and long (> 500 chars) bodies
///
/// Returns an empty `Vec` on success, or a list of human-readable problems.
pub fn validate_sample() -> Vec<String> {
    let mut problems: Vec<String> = Vec::new();

    // Uniqueness of IDs
    let mut seen_ids: HashSet<&str> = HashSet::new();
    for entity in SAMPLE {
        if !seen_ids.insert(entity.id) {
            problems.push(format!("duplicate entity id: {}", entity.id));
        }
    }

    // Non-empty fields
    for entity in SAMPLE {
        if entity.name.is_empty() {
            problems.push(format!("entity {} has empty name", entity.id));
        }
        if entity.body.is_empty() {
            problems.push(format!("entity {} has empty body", entity.id));
        }
    }

    // Must span both tiers
    if !SAMPLE.iter().any(|e| e.tier == SampleTier::Function) {
        problems.push("sample has no function-tier entities".to_string());
    }
    if !SAMPLE.iter().any(|e| e.tier == SampleTier::Rollup) {
        problems.push("sample has no rollup-tier entities".to_string());
    }

    // Must span short and long bodies (function-tier only — rollup bodies
    // are all > 500 chars by design and would trivially satisfy the long check)
    let fn_bodies: Vec<&SampleEntity> = SAMPLE
        .iter()
        .filter(|e| e.tier == SampleTier::Function)
        .collect();
    if !fn_bodies.iter().any(|e| e.body.len() < 100) {
        problems.push("sample has no short function-tier entities (<100 chars)".to_string());
    }
    if !fn_bodies.iter().any(|e| e.body.len() > 500) {
        problems.push("sample has no long function-tier entities (>500 chars)".to_string());
    }

    problems
}

/// Validate that the sample is internally consistent (no path-based checks —
/// the sample is self-contained const data). This is the CI-safe check:
/// it can run without a tree-sitter index or any model.
///
/// Follows the pattern from `retrieval_eval::check` in spirit, but the
/// summarizer sample has no file-path references — the bodies are inline
/// const strings, not references to live source files.
///
/// `repo_root` is accepted for API symmetry with `retrieval_eval::check`
/// but is not currently used.
#[allow(dead_code)]
pub fn check_sample(_repo_root: &Path) -> Vec<String> {
    validate_sample()
}
