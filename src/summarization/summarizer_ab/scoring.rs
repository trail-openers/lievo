// Parse-success scorer and license gate for the summarizer A/B harness
// (issue #773).
//
// `compute_parse_success` is the new harness-specific scorer: it takes the
// per-batch parse results (each a `Vec<Option<String>>` produced by
// `parse_batch_response`) and aggregates them into a percentage. This is
// distinct from `parse_batch_response` itself, which is the per-batch
// `#<n>:` line parser.
//
// The license gate (`LicenseStatus`, `check_license`) is a pure function:
// it takes a licence identifier and returns whether the candidate is
// admissible. The harness calls this before any measurement so that
// non-redistributable models are excluded with the reason stated.

use serde::Serialize;

/// The outcome of a licence check for a candidate summarizer.
///
/// `Admissible` means the licence permits redistribution in an open-source
/// CLI (e.g. Apache-2.0, MIT). `Excluded` means the licence does not
/// (e.g. research-only, non-commercial, or unknown).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status")]
pub enum LicenseStatus {
    /// The licence permits redistribution in an open-source CLI.
    Admissible {
        /// The licence name, e.g. `"Apache-2.0"`.
        license: String,
    },
    /// The licence does not permit redistribution.
    Excluded {
        /// The licence name, e.g. `"qwen-research"`.
        license: String,
        /// Human-readable reason for exclusion.
        reason: String,
    },
}

/// Check whether a licence identifier permits redistribution in an
/// open-source CLI.
///
/// This is a pure, deterministic function: no network calls, no model
/// downloads. The harness calls it before any measurement so that
/// non-redistributable candidates are excluded with the reason stated.
///
/// The set of known licences is intentionally small. Any licence not in
/// this list returns `Excluded` with the reason `"unrecognised licence"`
/// — the operator can extend the list as new candidates are evaluated.
///
/// # Known permissive licences
/// - `"Apache-2.0"` / `"apache-2.0"` — Apache License 2.0
/// - `"MIT"` / `"mit"` — MIT License
/// - `"BSD-3-Clause"` / `"bsd-3-clause"` — BSD 3-Clause
/// - `"BSD-2-Clause"` / `"bsd-2-clause"` — BSD 2-Clause
/// - `"ISC"` / `"isc"` — ISC License
/// - `"Unlicense"` / `"unlicense"` — Unlicense (public domain)
///
/// # Known restrictive licences
/// - `"qwen-research"` / `"Qwen RESEARCH LICENSE AGREEMENT"` — non-commercial
/// - `"GPL-3.0"` / `"gpl-3.0"` — copyleft, incompatible with proprietary redistribution
///
/// # Returns
/// `Admissible` for permissive licences, `Excluded` for restrictive or
/// unknown licences.
pub fn check_license(license: &str) -> LicenseStatus {
    let lower = license.to_lowercase();
    let normalized = lower.trim();
    let permissive = [
        "apache-2.0",
        "mit",
        "bsd-3-clause",
        "bsd-2-clause",
        "isc",
        "unlicense",
    ];
    let restrictive = [
        (
            "qwen-research",
            "Qwen RESEARCH LICENSE AGREEMENT — non-commercial only",
        ),
        (
            "qwen research license agreement",
            "Qwen RESEARCH LICENSE AGREEMENT — non-commercial only",
        ),
        (
            "gpl-3.0",
            "GPL-3.0 — copyleft, incompatible with proprietary redistribution",
        ),
        (
            "agpl-3.0",
            "AGPL-3.0 — copyleft, incompatible with proprietary redistribution",
        ),
    ];

    if permissive.contains(&normalized) {
        return LicenseStatus::Admissible {
            license: license.to_string(),
        };
    }

    if let Some((_, reason)) = restrictive.iter().find(|(id, _)| *id == normalized) {
        return LicenseStatus::Excluded {
            license: license.to_string(),
            reason: reason.to_string(),
        };
    }

    LicenseStatus::Excluded {
        license: license.to_string(),
        reason: "unrecognised licence — cannot verify redistribution rights".to_string(),
    }
}

/// Aggregate per-batch parse results into a parse-success percentage.
///
/// # Arguments
/// - `per_batch_results`: a slice of `Vec<Option<String>>`, one inner vec
///   per batch. Each inner vec is the result of `parse_batch_response` for
///   that batch: `Some(summary)` at the position of a successfully parsed
///   entity, `None` at a position where the model failed to produce a
///   correctly attributed `#<n>:` line.
///
/// # Returns
/// A floating-point percentage in `[0.0, 100.0]`. If there are no entities
/// across all batches (i.e. all inner vecs are empty), returns `0.0`.
///
/// # Examples
/// ```
/// # use lievo::summarization::summarizer_ab::scoring::compute_parse_success;
/// // Known-good batch: all Some
/// let good = vec![vec![Some("a".into()), Some("b".into())]];
/// assert_eq!(compute_parse_success(&good), 100.0);
///
/// // Known-malformed batch: all None (markdown bullets)
/// let bad = vec![vec![None, None]];
/// assert_eq!(compute_parse_success(&bad), 0.0);
///
/// // Mixed batch: 2 of 3 Some
/// let mixed = vec![vec![Some("a".into()), None, Some("c".into())]];
/// assert!((compute_parse_success(&mixed) - 66.67).abs() < 0.01);
/// ```
pub fn compute_parse_success(per_batch_results: &[Vec<Option<String>>]) -> f64 {
    let total: usize = per_batch_results.iter().map(|v| v.len()).sum();
    if total == 0 {
        return 0.0;
    }
    let successful: usize = per_batch_results
        .iter()
        .map(|v| v.iter().filter(|o| o.is_some()).count())
        .sum();
    (successful as f64 / total as f64) * 100.0
}

/// A single batch-size entry in the parse-success breakdown.
#[derive(Debug, Clone, Serialize)]
pub struct ParseSuccessEntry {
    /// The batch size (number of entities per batch).
    pub batch_size: usize,
    /// The parse-success percentage for this batch size.
    pub percentage: f64,
    /// Number of entities that were successfully parsed.
    pub successes: usize,
    /// Total number of entities attempted at this batch size.
    pub total: usize,
}

/// The full parse-success breakdown across multiple batch sizes.
///
/// The harness sweeps batch sizes (e.g. 2, 4, 8, 16) and records the
/// percentage for each. This allows detection of models that honour
/// `#<n>:` for small batches but degrade as batch size grows.
#[derive(Debug, Clone, Serialize)]
pub struct ParseSuccessReport {
    /// The model name / identifier.
    pub model: String,
    /// One entry per batch size tested.
    pub entries: Vec<ParseSuccessEntry>,
    /// Overall parse-success percentage across all batch sizes.
    pub overall: f64,
}

/// Build a `ParseSuccessReport` from per-batch results grouped by batch size.
///
/// # Arguments
/// - `model`: the model identifier.
/// - `by_batch_size`: a `Vec<(batch_size, &[Vec<Option<String>>])` where the
///   second element is the per-batch results for that batch size.
pub fn build_parse_success_report(
    model: &str,
    by_batch_size: Vec<(usize, &Vec<Vec<Option<String>>>)>,
) -> ParseSuccessReport {
    let mut entries = Vec::new();
    let mut total_successes = 0usize;
    let mut total_entities = 0usize;

    for (batch_size, results) in &by_batch_size {
        let successes = results
            .iter()
            .map(|v| v.iter().filter(|o| o.is_some()).count())
            .sum::<usize>();
        let total = results.iter().map(|v| v.len()).sum::<usize>();
        let percentage = if total == 0 {
            0.0
        } else {
            (successes as f64 / total as f64) * 100.0
        };
        total_successes += successes;
        total_entities += total;
        entries.push(ParseSuccessEntry {
            batch_size: *batch_size,
            percentage,
            successes,
            total,
        });
    }

    let overall = if total_entities == 0 {
        0.0
    } else {
        (total_successes as f64 / total_entities as f64) * 100.0
    };

    ParseSuccessReport {
        model: model.to_string(),
        entries,
        overall,
    }
}
