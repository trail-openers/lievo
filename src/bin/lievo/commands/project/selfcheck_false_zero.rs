// Section (b): false-0-callers detection for `lievo admin selfcheck`
// (issue #715). Split from `selfcheck_metrics.rs` to stay within the
// 500-line src budget (AGENTS.md §6).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::selfcheck_metrics::ProbeResult;
use lievo::analysis::rust_mod_parse::is_test_like_file;

/// A file flagged as a false-0-caller: 0 recorded importers, repo-level
/// `unresolved_internal == 0`, yet an on-disk grep-level scan finds >0
/// files referencing its path or module name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalseZeroCaller {
    pub file: String,
    pub on_disk_importers: usize,
}

/// Detect false-0-callers: files with 0 recorded importers, gated on
/// `unresolved_internal == 0` (repo-level; see #715 PM decision — a
/// project-wide, not per-file, gate since `RelationshipBuilder::build`
/// returns only a repo-global `UnresolvedCounts`), whose on-disk grep-level
/// importer count is > 0.
///
/// `physical_to_logical` maps a `#[path]`-diverged physical file to its
/// logical module name (issue #732). A file whose only on-disk importer is
/// a `#[cfg(test)]`-gated `#[path]` declaration has NO recorded importer in
/// the non-test profile no matter what — the production extractor never
/// compiles the gated declaration, so the recorded edge never exists. These
/// files are structurally false-zero; the map exempts them from the scan so
/// the gate can stay at 0 without threshold loosening.
pub fn detect_false_zero_callers(
    all_files: &[String],
    resolved_by_path: &[(String, String)],
    unresolved_internal: u32,
    physical_to_logical: &HashMap<String, String>,
    on_disk_importer_count: impl Fn(&str) -> usize,
) -> Vec<FalseZeroCaller> {
    if unresolved_internal != 0 {
        return Vec::new();
    }
    let imported: HashSet<&str> = resolved_by_path
        .iter()
        .filter(|(s, t)| s != t)
        .map(|(_, t)| t.as_str())
        .collect();

    all_files
        .iter()
        .filter(|f| !imported.contains(f.as_str()))
        .filter(|f| f.starts_with("src/"))
        .filter(|f| !is_test_like_file(f))
        .filter(|f| !physical_to_logical.contains_key(f.as_str()))
        .filter_map(|f| {
            let count = on_disk_importer_count(f);
            if count > 0 {
                Some(FalseZeroCaller {
                    file: f.clone(),
                    on_disk_importers: count,
                })
            } else {
                None
            }
        })
        .collect()
}

/// Generic stems (entry points, module-index files) that legitimately have 0
/// cross-file references — every language's `main` collides on this name
/// regardless of actual dependencies, so they're excluded from the scan.
const GENERIC_STEMS: &[&str] = &["main", "mod", "index"];

/// Grep-level on-disk importer count: files under `repo_root`, SAME
/// extension only (cross-language name collisions like "helper" appearing in
/// unrelated Rust prose are not evidence of a Go import), containing the
/// target's stem as a whole word (word-boundary, not raw substring — avoids
/// false positives on short stems embedded in longer identifiers).
pub fn on_disk_grep_importer_count(
    repo_root: &Path,
    target_file: &str,
    all_files: &[String],
) -> usize {
    let target_path = Path::new(target_file);
    let stem = target_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(target_file);
    if stem.is_empty() || GENERIC_STEMS.contains(&stem) {
        return 0;
    }
    let ext = target_path.extension().and_then(|e| e.to_str());
    all_files
        .iter()
        .filter(|f| f.as_str() != target_file)
        .filter(|f| Path::new(f.as_str()).extension().and_then(|e| e.to_str()) == ext)
        .filter(|f| {
            std::fs::read_to_string(repo_root.join(f.as_str()))
                .map(|content| contains_word(&content, stem))
                .unwrap_or(false)
        })
        .count()
}

/// True when `word` appears in `text` bounded by non-alphanumeric-or-`_`
/// characters on both sides (or start/end of string) — avoids matching a
/// stem that is only a substring of a longer identifier.
fn contains_word(text: &str, word: &str) -> bool {
    let is_boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
    let bytes = text.as_bytes();
    let wlen = word.len();
    if wlen == 0 || wlen > bytes.len() {
        return false;
    }
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + wlen..].chars().next();
        is_boundary(before) && is_boundary(after)
    })
}

// ---------------------------------------------------------------------------
// Thresholds + gate verdict (moved from selfcheck_metrics.rs for the
// 500-line src budget, AGENTS.md §6)
// ---------------------------------------------------------------------------

/// All configurable thresholds for the four sections (CLI flags only, no
/// config file, per the PM decision).
#[derive(Debug, Clone)]
pub struct SelfcheckThresholds {
    pub max_wrong_edge_rate: f64,
    pub max_false_zero_callers: usize,
    pub sample_size: usize,
    pub min_recall_k: usize,
    pub min_recall_floor: f32,
    pub min_worst_probe: f32,
    pub min_payload_bytes: usize,
    /// Max confirmed structural super:: failures (issue #744) — the
    /// threshold is ratcheted per the #733 pattern: explicit flag in the
    /// real-corpus CI steps, lenient default for fixture/CLI use.
    pub max_super_probe_failures: usize,
}

impl Default for SelfcheckThresholds {
    fn default() -> Self {
        Self {
            max_wrong_edge_rate: 0.0,
            max_false_zero_callers: 0,
            sample_size: 0,
            min_recall_k: 5,
            min_recall_floor: 0.0,
            min_worst_probe: 0.0,
            min_payload_bytes: 200,
            max_super_probe_failures: usize::MAX,
        }
    }
}

/// One section's report: metric value(s) plus pass/fail, or `skipped` with a
/// reason (storage-backed sections when no project/index is available).
#[derive(Debug, Clone)]
pub struct SectionReport {
    pub name: &'static str,
    pub passed: bool,
    pub skipped: bool,
    pub skip_reason: Option<String>,
    pub detail: String,
}

// The edge-correctness gate lives in `selfcheck_edge_split.rs` (issue #777:
// the wrong-edge rate is computed over import-resolved edges only, and the
// detail string names the call-based/unverifiable category). Call sites
// import it via `selfcheck_metrics::gate_edge_correctness` (re-exported
// there), so no re-export is needed here.

/// Evaluate the false-0-callers gate.
pub fn gate_false_zero_callers(
    flagged: &[FalseZeroCaller],
    thresholds: &SelfcheckThresholds,
) -> SectionReport {
    let passed = flagged.len() <= thresholds.max_false_zero_callers;
    let files: Vec<&str> = flagged.iter().map(|f| f.file.as_str()).take(5).collect();
    let files_desc = if files.is_empty() {
        String::new()
    } else {
        format!(" files={:?}", files)
    };
    SectionReport {
        name: "false_zero_callers",
        passed,
        skipped: false,
        skip_reason: None,
        detail: format!(
            "count={} threshold<= {}{}",
            flagged.len(),
            thresholds.max_false_zero_callers,
            files_desc
        ),
    }
}

/// The probe with the lowest recall (diagnostic naming for the worst-probe floor).
fn worst_probe(results: &[ProbeResult]) -> Option<&ProbeResult> {
    results.iter().min_by(|a, b| a.recall.total_cmp(&b.recall))
}

/// Evaluate the retrieval-probes gate (worst-probe floor + per-probe floor).
pub fn gate_retrieval_probes(
    results: &[ProbeResult],
    thresholds: &SelfcheckThresholds,
) -> SectionReport {
    let worst = super::selfcheck_metrics::worst_probe_recall(results);
    let all_pass_floor = results
        .iter()
        .all(|r| r.recall >= thresholds.min_recall_floor);
    let passed = worst >= thresholds.min_worst_probe && all_pass_floor;
    let worst_desc = worst_probe(results)
        .map(|p| format!(" worst_probe={:?}->{:?}", p.query, p.expected_path))
        .unwrap_or_default();
    SectionReport {
        name: "retrieval_probes",
        passed,
        skipped: false,
        skip_reason: None,
        detail: format!(
            "probes={} worst_recall={:.4} (floor {:.4}) per_probe_floor={:.4}{}",
            results.len(),
            worst,
            thresholds.min_worst_probe,
            thresholds.min_recall_floor,
            worst_desc,
        ),
    }
}

/// Evaluate the payload-bytes floor gate.
pub fn gate_payload_bytes(
    results: &[ProbeResult],
    thresholds: &SelfcheckThresholds,
) -> SectionReport {
    let min_bytes = results.iter().map(|r| r.payload_bytes).min().unwrap_or(0);
    let passed = results
        .iter()
        .all(|r| r.payload_bytes >= thresholds.min_payload_bytes);
    SectionReport {
        name: "payload_bytes",
        passed,
        skipped: false,
        skip_reason: None,
        detail: format!(
            "min_payload_bytes={} threshold>= {}",
            min_bytes, thresholds.min_payload_bytes
        ),
    }
}

pub fn skipped_section(name: &'static str, reason: &str) -> SectionReport {
    SectionReport {
        name,
        passed: true,
        skipped: true,
        skip_reason: Some(reason.to_string()),
        detail: String::new(),
    }
}

#[cfg(test)]
#[path = "selfcheck_false_zero_tests.rs"]
mod selfcheck_false_zero_tests;
