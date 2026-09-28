// Wrong-edge population split for `lievo admin selfcheck` (issue #777).
//
// The production resolver creates file-level `RelType::Imports` edges via two
// distinct paths (src/analysis/relationships_aggregate.rs):
//
//   1. use-statement resolution (import_map / `resolve_import_for`)
//      → `EdgeProvenance::Resolved`
//   2. function-call / mod-declaration resolution (fn_map / bare_name_map)
//      → `EdgeProvenance::Heuristic`
//
// The independent verifier (selfcheck_metrics.rs `verify_edges`) re-resolves
// ONLY import specifiers, so a call-based edge yields no candidate set and
// was previously scored "wrong" despite being correct. Per the operator
// decision (issue #777), call-based edges are no longer scored: they are
// counted and reported as a distinct "unverifiable-by-construction" category
// (the super-probe's confirmed / failures / indeterminate precedent), and
// `wrong_edge_rate` is computed over import-resolved edges only.
//
// Classification signal: `EdgeProvenance` itself — it already carries the
// distinction, and `merge_provenance` (resolved-wins) already decides dual-path
// edges at materialization time. This module mirrors that rule for the
// selfcheck's (src, tgt) edge list: an edge pair present in BOTH lists is
// import-resolved and counted exactly once.

use super::selfcheck_false_zero::{SectionReport, SelfcheckThresholds};
use lievo::model::{EdgeProvenance, RelType, Relationship};
use std::collections::HashSet;

/// The import-edge population split into the two verifier-visible classes.
///
/// The two class lists are dedup'd (src, tgt) pairs and DISJOINT — an edge
/// pair present in both raw lists lands only in `import_resolved` (the
/// resolved-wins rule, see `classify_edges`). `imported` is the union
/// (both classes, dedup'd) — the "has a recorded import edge" set the
/// false_zero_callers section needs, which is unchanged by the split: a
/// file that was imported via a call-based edge before the split must not
/// become a false-zero caller after it (issue #777 regression criterion).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EdgePopulation {
    /// (src, tgt) pairs from use-statement resolution
    /// (`EdgeProvenance::Resolved`) — the population the independent verifier
    /// can actually check, and the denominator of `wrong_edge_rate`.
    pub import_resolved: Vec<(String, String)>,
    /// (src, tgt) pairs from call/mod-declaration resolution
    /// (`EdgeProvenance::Heuristic`) that no Resolved edge covers —
    /// unverifiable by construction, reported but never scored.
    pub call_based: Vec<(String, String)>,
    /// ALL (src, tgt) import-edge pairs from either provenance (dedup'd).
    pub imported: Vec<(String, String)>,
}

/// Split a relationship list into the two import-edge classes.
///
/// Filters to `RelType::Imports`, maps to (source_path, target_path) pairs
/// (dropping edges whose endpoint has no path, as before), and applies the
/// resolved-wins rule: an (src, tgt) pair present in both classes is
/// import-resolved and appears in NO other list. Both output lists are
/// sorted and dedup'd (the same deterministic shape `sample_edges` produces).
///
/// `id_to_path` is a relationship-endpoint-id → repo-relative-path map.
pub fn classify_edges(
    relationships: &[Relationship],
    id_to_path: &std::collections::HashMap<String, String>,
) -> EdgePopulation {
    let mut resolved: Vec<(String, String)> = Vec::new();
    let mut heuristic: Vec<(String, String)> = Vec::new();
    for r in relationships {
        if r.rel_type != RelType::Imports {
            continue;
        }
        let pair = match (id_to_path.get(&r.source_id), id_to_path.get(&r.target_id)) {
            (Some(src), Some(tgt)) => (src.clone(), tgt.clone()),
            _ => continue,
        };
        match r.provenance {
            EdgeProvenance::Resolved => resolved.push(pair),
            EdgeProvenance::Heuristic => heuristic.push(pair),
        }
    }
    EdgePopulation::new(resolved, heuristic)
}

impl EdgePopulation {
    /// Build a populated, disjoint split from raw (unfiltered) per-provenance
    /// pair lists (dual-path rule: present in both → import_resolved only).
    pub fn new(import_resolved: Vec<(String, String)>, call_based: Vec<(String, String)>) -> Self {
        let resolved_set: HashSet<(String, String)> = import_resolved.iter().cloned().collect();
        let mut resolved = import_resolved;
        resolved.sort();
        resolved.dedup();
        let mut calls: Vec<(String, String)> = call_based
            .into_iter()
            .filter(|pair| !resolved_set.contains(pair))
            .collect();
        calls.sort();
        calls.dedup();
        let mut imported: Vec<(String, String)> = resolved
            .iter()
            .cloned()
            .chain(calls.iter().cloned())
            .collect();
        imported.sort();
        imported.dedup();
        Self {
            import_resolved: resolved,
            call_based: calls,
            imported,
        }
    }

    /// Census of ALL call-based (Heuristic, uncovered) import edges — not
    /// limited to the evidenced subset. This is the count the detail string
    /// reports (reported, never scored).
    pub fn call_based_census(&self) -> usize {
        self.call_based.len()
    }
}

/// Count of samples carrying independent evidence
/// (`resolved_target.is_some()`) — the actual denominator of
/// `wrong_edge_rate`.
pub fn count_evidenced(samples: &[super::selfcheck_metrics::EdgeSample]) -> usize {
    samples
        .iter()
        .filter(|s| s.resolved_target.is_some())
        .count()
}

/// Evaluate the edge-correctness gate over the SPLIT population
/// (issue #777).
///
/// `rate` is computed over import-resolved edges only; `evidenced` is the
/// evidenced import-resolved count (the rate's denominator); `population`
/// is the import-resolved population size (the dedup'd census `sample_size=`
/// reports — equals `evidenced` in the default census mode where every edge
/// is verified; with a non-zero `--sample-size` the census stays whole); and
/// `call_based` is the census of ALL Heuristic import edges (reported, never
/// scored).
///
/// The detail string names all three numbers and states what
/// `sample_size=` reports — the import-resolved population only (previously
/// the whole verified edge population, import-resolved + call-based).
pub fn gate_edge_correctness(
    rate: f64,
    evidenced: usize,
    population: usize,
    call_based: usize,
    thresholds: &SelfcheckThresholds,
) -> SectionReport {
    let passed = rate <= thresholds.max_wrong_edge_rate;
    SectionReport {
        name: "edge_correctness",
        passed,
        skipped: false,
        skip_reason: None,
        detail: format!(
            "wrong_edge_rate={:.4} over {} evidenced import-resolved edges \
             (sample_size={} import-resolved population) call_based={} \
             (unverifiable-by-construction, not scored) threshold<= {:.4}",
            rate, evidenced, population, call_based, thresholds.max_wrong_edge_rate
        ),
    }
}

#[cfg(test)]
#[path = "selfcheck_edge_split_tests.rs"]
mod selfcheck_edge_split_tests;
