// Extraction coverage metrics and CI gate predicates (issue #679).
//
// Pure computation over CodeUnits + resolved import/call edges — no storage
// or I/O, so the gate runs in CI without network, apfel, or the ONNX
// embedding-model download.
//
// Coverage definition adopted verbatim from CodeGraph's README:
// "Fair coverage = share of symbol-bearing source files with ≥1 resolved
// cross-file dependent".

use crate::extraction::function_preservation::{is_function_unit, is_test_file_path};
use crate::model::CodeUnit;
use std::collections::HashMap;

/// Default max entities per bare function name before the fan-out gate fails
/// (mirrors the `tracing::debug!` at relationships.rs:325-336 which logs at
/// `target_ids.len() > 10`).
pub const FAN_OUT_THRESHOLD: usize = 10;

/// Default number of most-frequently-called names considered by the
/// single-character callee gate.
pub const TOP_N_CALLEES: usize = 20;

/// Per-language extraction coverage metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct LanguageCoverage {
    pub language: String,
    /// Symbol-bearing (non-test) files with ≥1 resolved cross-file dependent.
    pub files_with_dependents: usize,
    /// Symbol-bearing (non-test) files — the denominator.
    pub symbol_files: usize,
    /// Total code units of this language (all, incl. test files).
    pub entities: usize,
    /// Max number of function entities sharing a bare name in this language.
    pub max_fan_out: usize,
    /// The bare name with the max fan-out (None if no function units).
    pub max_fan_out_name: Option<String>,
    /// Top-N callee names that are a single character (chars().count() == 1).
    pub single_char_callees: Vec<String>,
}

/// A coverage gate failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateFailure {
    /// Human-readable reason, e.g. "fan-out 12 for name 'main' (limit 10)".
    pub reason: String,
    /// The language the failure occurred in.
    pub language: String,
}

/// Build per-language coverage metrics from code units.
///
/// `resolved_import_files` is the set of (source_file, target_file) pairs
/// produced by import resolution (RelType::Imports file→file edges); a
/// source file is "covered" if it has ≥1 edge to a DIFFERENT file.
///
/// Test files (per `is_test_file_path`) are excluded from the denominator,
/// matching the call-graph exclusion in the extraction pipeline.
pub fn coverage_by_language(
    code_units: &[CodeUnit],
    resolved_import_files: &[(String, String)],
) -> Vec<LanguageCoverage> {
    let mut by_language: HashMap<&str, Vec<&CodeUnit>> = HashMap::new();
    for unit in code_units {
        by_language
            .entry(unit.language.as_str())
            .or_default()
            .push(unit);
    }

    let mut rows: Vec<LanguageCoverage> = by_language
        .iter()
        .map(|(language, units)| {
            let entity_count = units.len();

            let mut symbol_files: HashMap<&str, usize> = HashMap::new();
            for unit in units {
                if !is_test_file_path(unit.file.as_str()) {
                    *symbol_files.entry(unit.file.as_str()).or_insert(0) += 1;
                }
            }

            let mut covered: std::collections::HashSet<&str> = std::collections::HashSet::new();
            for (src, tgt) in resolved_import_files {
                if src != tgt && symbol_files.contains_key(src.as_str()) {
                    covered.insert(src.as_str());
                }
            }

            let mut fan_out: HashMap<&str, usize> = HashMap::new();
            for unit in units {
                if is_function_unit(&unit.unit_type)
                    && !is_test_file_path(unit.file.as_str())
                    && !is_method_unit(unit)
                {
                    *fan_out.entry(unit.name.as_str()).or_insert(0) += 1;
                }
            }
            let (max_fan_out_name, max_fan_out) = fan_out
                .iter()
                .max_by_key(|(_, count)| *count)
                .map(|(name, count)| (Some(name.to_string()), *count))
                .unwrap_or_else(|| (None, 0));

            LanguageCoverage {
                language: language.to_string(),
                files_with_dependents: covered.len(),
                symbol_files: symbol_files.len(),
                entities: entity_count,
                max_fan_out_name,
                max_fan_out,
                single_char_callees: Vec::new(),
            }
        })
        .collect();

    // Single-char callees: top-N callee names by total call count, filtered to
    // names that are exactly one character (chars().count() == 1). The gate
    // applies to top-N callees only — a repo full of 1-char locals must not
    // trip it.
    for (language, units) in &by_language {
        if let Some(row) = rows.iter_mut().find(|r| r.language == *language) {
            let mut call_counts: HashMap<&str, usize> = HashMap::new();
            for unit in units {
                for call in &unit.calls {
                    *call_counts.entry(call.as_str()).or_insert(0) += 1;
                }
            }
            row.single_char_callees = top_single_char_callees(&call_counts);
        }
    }

    rows.sort_by(|a, b| a.language.cmp(&b.language));
    rows
}

/// The top-N callee names (ranked by total call count) that are a single
/// character (Unicode-aware: `chars().count() == 1`).
///
/// Top-N is computed over ALL callee names first — a repo full of 1-char
/// locals that never reach the top-N of the whole distribution must not trip
/// the gate.
pub fn top_single_char_callees(call_counts: &HashMap<&str, usize>) -> Vec<String> {
    let mut entries: Vec<(&str, usize)> = call_counts
        .iter()
        .map(|(name, count)| (*name, *count))
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    entries
        .iter()
        .take(TOP_N_CALLEES)
        .filter(|(name, _)| name.chars().count() == 1)
        .map(|(name, _)| (*name).to_string())
        .collect()
}

/// Returns `true` when the code unit is a method (its signature declares a
/// `self` receiver) rather than a free function. Trait/interface method
/// implementations are not a fan-out smell — they are the interface being
/// honoured — so the fan-out gate counts free functions only. Detection keys
/// off the `self` keyword in the unit's source text: `self` is a Rust keyword
/// reserved for method receivers, so its presence in the signature marks a
/// method. (The extractor does not populate `parent_class` for Rust.)
fn is_method_unit(unit: &CodeUnit) -> bool {
    let code = match &unit.code {
        Some(c) => c,
        None => return false,
    };
    // Signature: up to the first `{` (body) or `;` (trait-decl terminator).
    let sig_end = code
        .find('{')
        .map(|b| code.find(';').map_or(b, |s| s.min(b)))
        .or_else(|| code.find(';'))
        .unwrap_or(code.len());
    let sig = &code[..sig_end];
    // `self` is a keyword: it only appears as a method receiver. Match it as
    // a whole token (not a substring of `self_type` etc.).
    let mut rest = sig;
    while let Some(pos) = rest.find("self") {
        let before = if pos == 0 {
            ' ' // boundary
        } else {
            rest.as_bytes()[pos - 1] as char
        };
        let after = rest
            .as_bytes()
            .get(pos + 4)
            .copied()
            .map(|b| b as char)
            .unwrap_or(' ');
        if !before.is_ascii_alphanumeric()
            && before != '_'
            && !after.is_ascii_alphanumeric()
            && after != '_'
        {
            return true;
        }
        rest = &rest[pos + 4..];
    }
    false
}

/// Evaluate the fan-out and single-character gates over all language rows.
///
/// Returns one `GateFailure` per violated gate (empty vec = pass).
/// The caller decides the exit code; CI turns any failure into a build
/// failure while `lievo admin coverage` (without `--gate`) only reports.
pub fn gate_failures(rows: &[LanguageCoverage], fan_out_threshold: usize) -> Vec<GateFailure> {
    let mut failures = Vec::new();
    for row in rows {
        if row.max_fan_out > fan_out_threshold {
            let name = row.max_fan_out_name.as_deref().unwrap_or("?");
            failures.push(GateFailure {
                reason: format!(
                    "fan-out {} for bare name '{}' exceeds threshold {} (language '{}')",
                    row.max_fan_out, name, fan_out_threshold, row.language
                ),
                language: row.language.clone(),
            });
        }
        if !row.single_char_callees.is_empty() {
            failures.push(GateFailure {
                reason: format!(
                    "single-character callee names in top-N callees: {} (language '{}')",
                    row.single_char_callees
                        .iter()
                        .map(|n| format!("'{}'", n))
                        .collect::<Vec<_>>()
                        .join(", "),
                    row.language
                ),
                language: row.language.clone(),
            });
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CodeUnit;

    fn unit(name: &str, file: &str, language: &str, unit_type: &str, calls: &[&str]) -> CodeUnit {
        CodeUnit {
            name: name.to_string(),
            qualified_name: name.to_string(),
            unit_type: unit_type.to_string(),
            file: file.to_string(),
            line: 1,
            end_line: 5,
            language: language.to_string(),
            signature: None,
            code: None,
            docstring: None,
            parent_class: None,
            complexity: 1,
            has_branches: false,
            has_loops: false,
            has_error_handling: false,
            calls: calls.iter().map(|s| s.to_string()).collect(),
            imports: vec![],
        }
    }

    fn row(
        language: &str,
        covered: usize,
        symbol: usize,
        entities: usize,
        fan_out: usize,
        fan_out_name: Option<&str>,
        single_char: &[&str],
    ) -> LanguageCoverage {
        LanguageCoverage {
            language: language.to_string(),
            files_with_dependents: covered,
            symbol_files: symbol,
            entities,
            max_fan_out: fan_out,
            max_fan_out_name: fan_out_name.map(|s| s.to_string()),
            single_char_callees: single_char.iter().map(|s| s.to_string()).collect(),
        }
    }

    // --- CodeGraph coverage formula ---------------------------------------

    #[test]
    fn coverage_formula_file_without_dependents_not_in_numerator() {
        // a.rs and b.rs both bear symbols; only a.rs has a resolved cross-file
        // dependent. b.rs must not count toward the numerator.
        let units = vec![
            unit("alpha", "a.rs", "Rust", "function", &[]),
            unit("beta", "b.rs", "Rust", "function", &[]),
        ];
        let resolved: Vec<(String, String)> = vec![("a.rs".into(), "b.rs".into())];
        let rows = coverage_by_language(&units, &resolved);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol_files, 2, "both files bear symbols");
        assert_eq!(
            rows[0].files_with_dependents, 1,
            "only the source file with a resolved cross-file dependent counts"
        );
    }

    #[test]
    fn coverage_formula_zero_symbol_file_excluded_from_denominator() {
        // c.rs has no code units at all — it cannot appear in the symbol-file
        // set, so it is excluded from the denominator entirely.
        let units = vec![unit("alpha", "a.rs", "Rust", "function", &[])];
        let rows = coverage_by_language(&units, &[]);
        assert_eq!(
            rows[0].symbol_files, 1,
            "empty files are not symbol-bearing"
        );
        assert_eq!(rows[0].files_with_dependents, 0);
    }

    #[test]
    fn coverage_formula_test_files_excluded_from_denominator() {
        let units = vec![
            unit("alpha", "src/a.rs", "Rust", "function", &[]),
            unit("test_one", "tests/a_test.rs", "Rust", "function", &[]),
        ];
        let rows = coverage_by_language(&units, &[]);
        assert_eq!(
            rows[0].symbol_files, 1,
            "test files must not inflate the denominator"
        );
    }

    #[test]
    fn coverage_formula_self_edges_do_not_count() {
        // A file importing itself has no CROSS-file dependent.
        let units = vec![unit("alpha", "a.rs", "Rust", "function", &[])];
        let resolved: Vec<(String, String)> = vec![("a.rs".into(), "a.rs".into())];
        let rows = coverage_by_language(&units, &resolved);
        assert_eq!(rows[0].files_with_dependents, 0);
    }

    // --- Fan-out ------------------------------------------------------------

    #[test]
    fn gate_fan_out_fails_at_eleven_and_passes_at_ten() {
        let bad = row("Rust", 1, 1, 5, 11, Some("main"), &[]);
        let good = row("Rust", 1, 1, 5, 10, Some("main"), &[]);
        let failures = gate_failures(std::slice::from_ref(&bad), FAN_OUT_THRESHOLD);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].reason.contains("fan-out 11"));
        assert!(gate_failures(std::slice::from_ref(&good), FAN_OUT_THRESHOLD).is_empty());
    }

    #[test]
    fn fan_out_measured_from_function_units_across_files() {
        // 11 same-named function units in 11 distinct files → fan-out 11.
        let units: Vec<CodeUnit> = (0..11)
            .map(|i| unit("main", &format!("f{i}.rs"), "Rust", "function", &[]))
            .collect();
        let rows = coverage_by_language(&units, &[]);
        assert_eq!(rows[0].max_fan_out, 11);
        assert_eq!(rows[0].max_fan_out_name.as_deref(), Some("main"));
        let failures = gate_failures(&rows, FAN_OUT_THRESHOLD);
        assert_eq!(
            failures.len(),
            1,
            "11 same-named targets must fail the gate"
        );
    }

    // --- Trait/interface method exclusion (issue #763) ----------------------

    fn mu(name: &str, file: &str, code: &str) -> CodeUnit {
        let mut u = unit(name, file, "Rust", "function", &[]);
        u.code = Some(code.to_string());
        u
    }

    #[test]
    fn is_method_unit_detects_self_receiver() {
        assert!(is_method_unit(&mu("f", "a.rs", "fn f(&self) -> u32 { 1 }")));
        assert!(is_method_unit(&mu("f", "a.rs", "fn f(&mut self) { }")));
        assert!(is_method_unit(&mu("f", "a.rs", "fn f(self) -> u32 { 1 }")));
        assert!(is_method_unit(&mu("f", "a.rs", "fn f(&self) -> u32;"))); // trait decl
    }

    #[test]
    fn is_method_unit_rejects_non_methods() {
        assert!(!is_method_unit(&mu("f", "a.rs", "fn f() { }")));
        assert!(!is_method_unit(&mu("f", "a.rs", "fn f(&self_type) { }"))); // token guard
        assert!(!is_method_unit(&unit("f", "a.rs", "Rust", "function", &[]))); // code: None
    }

    #[test]
    fn fan_out_excludes_trait_impl_methods() {
        let mut units: Vec<CodeUnit> = (0..15)
            .map(|i| {
                mu(
                    "input_schema",
                    &format!("t{i}.rs"),
                    "fn input_schema(&self) -> u32 { 1 }",
                )
            })
            .collect();
        units.push(unit("helper", "free1.rs", "Rust", "function", &[]));
        let rows = coverage_by_language(&units, &[]);
        assert_eq!(
            rows[0].max_fan_out, 1,
            "methods excluded → only free `helper` remains"
        );
        assert_eq!(rows[0].max_fan_out_name.as_deref(), Some("helper"));
        assert!(gate_failures(&rows, FAN_OUT_THRESHOLD).is_empty());
    }

    #[test]
    fn fan_out_still_counts_free_functions_above_threshold() {
        let units: Vec<CodeUnit> = (0..11)
            .map(|i| unit("main", &format!("free_{i}.rs"), "Rust", "function", &[]))
            .collect();
        let rows = coverage_by_language(&units, &[]);
        assert_eq!(rows[0].max_fan_out, 11);
        assert_eq!(gate_failures(&rows, FAN_OUT_THRESHOLD).len(), 1);
    }

    #[test]
    fn fan_out_counts_mixed_methods_and_free_functions() {
        let mut units: Vec<CodeUnit> = (0..12)
            .map(|i| mu("run", &format!("m{i}.rs"), "fn run(&self) { }"))
            .collect();
        units.push(unit("run", "fa.rs", "Rust", "function", &[]));
        units.push(unit("run", "fb.rs", "Rust", "function", &[]));
        let rows = coverage_by_language(&units, &[]);
        assert_eq!(
            rows[0].max_fan_out, 2,
            "12 methods excluded, 2 free fns counted"
        );
    }

    // --- Single-character callees -------------------------------------------

    #[test]
    fn gate_single_char_fails_on_a_and_x_passes_on_two_char() {
        let bad = row("JavaScript", 1, 1, 3, 1, None, &["a", "x"]);
        let good = row("JavaScript", 1, 1, 3, 1, None, &[]);
        let failures = gate_failures(std::slice::from_ref(&bad), FAN_OUT_THRESHOLD);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].reason.contains("'a'") && failures[0].reason.contains("'x'"));
        assert!(gate_failures(std::slice::from_ref(&good), FAN_OUT_THRESHOLD).is_empty());
    }

    #[test]
    fn top_single_char_uses_char_count_not_len() {
        // A multibyte 1-grapheme callee (e.g. "µ") has chars().count() == 1 and
        // len() == 2 — it must be flagged.
        let mut counts: HashMap<&str, usize> = HashMap::new();
        counts.insert("µ", 5);
        counts.insert("ab", 50); // two chars: never flagged
        let top = top_single_char_callees(&counts);
        assert_eq!(top, vec!["µ".to_string()]);
    }

    #[test]
    fn top_single_char_only_considers_top_n_callees() {
        // A single-char callee with a lower count than TOP_N two-char callees
        // must not be flagged (the gate applies to top-N only).
        let mut names: Vec<String> = Vec::new();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for i in 0..TOP_N_CALLEES {
            let name = format!("f{i}");
            names.push(name);
        }
        for name in &names {
            counts.insert(name.as_str(), 100); // 2+ char names, higher counts
        }
        counts.insert("z", 1);
        let top = top_single_char_callees(&counts);
        assert!(
            top.is_empty(),
            "a single-char callee ranked below top-N must not trip the gate"
        );
    }

    #[test]
    fn top_single_char_ranks_by_call_count() {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        counts.insert("a", 3);
        counts.insert("b", 7);
        counts.insert("c", 1);
        let top = top_single_char_callees(&counts);
        assert_eq!(top, vec!["b".to_string(), "a".to_string(), "c".to_string()]);
    }

    #[test]
    fn gate_passes_when_no_language_violates() {
        let rows = vec![
            row("Rust", 1, 2, 10, 3, Some("main"), &[]),
            row("JavaScript", 0, 4, 8, 2, Some("x"), &[]),
        ];
        assert!(gate_failures(&rows, FAN_OUT_THRESHOLD).is_empty());
    }
}
