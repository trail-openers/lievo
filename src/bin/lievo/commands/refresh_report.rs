// Per-tier summary coverage for the refresh report (issue #793).
//
// The pre-#793 report counted the function tier only, so a whole tier
// failing to summarize was invisible behind a high function-tier number.
// The per-tier view makes the work that was actually attempted visible at
// every tier; the function-tier value stays byte-identical to the old
// formula (total = non-test_ functions, missing = count_missing_summaries)
// so historical comparisons remain valid.
//
// This file is a `#[path]` sibling of `refresh.rs`: the module lives at the
// same crate path as `refresh.rs`'s own items, so `use super::*` resolves
// to the refresh command's scope.

use lievo::model::EntityTier;
use lievo::output::OutputFormat;
use lievo::storage::Storage;
use lievo::summarization::pipeline::is_test_entity;

/// Coverage of one tier across a project's repositories: the denominator
/// (entities in the summarization population) and the missing count
/// (NULL summary). The test exclusion is tier-specific: the function tier
/// excludes by name (`is_test_entity`), the file tier by path
/// (`is_test_file_path`); the module and subsystem tiers carry no test
/// exclusion (issue #793 review, finding 4).
#[derive(Debug, Clone, Copy, Default)]
pub struct TierCoverage {
    pub total: u64,
    pub missing: u64,
}

impl TierCoverage {
    /// Entities with a summary: `total - missing`.
    pub fn summarized(self) -> u64 {
        self.total.saturating_sub(self.missing)
    }

    /// Percentage covered as `f64` rounded to one decimal (e.g. 52.8).
    /// Zero total yields 0.0 rather than dividing by zero (the pre-#793
    /// `SummaryCoverage` had the same behaviour; the refresh report tests
    /// pin it).
    pub fn pct(self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.summarized() as f64 / self.total as f64 * 1000.0).round() / 10.0
    }
}

/// The function tier's coverage across a project.
///
/// The value is byte-identical to the pre-#793 formula: total is non-`test_`
/// functions per repo, missing is `count_missing_summaries` (which excludes
/// `test_` names), so historical pct values remain comparable (issue #793).
pub fn compute_coverage(storage: &dyn Storage, project_id: &str) -> lievo::Result<TierCoverage> {
    let mut total: u64 = 0;
    let mut missing: u64 = 0;
    for repo in storage.list_repos(project_id)? {
        // `count_missing_summaries` excludes `test_` names; the total must
        // match that population exactly, otherwise pct would be skewed.
        let functions = storage.entities_by_repo(&repo.id, Some(EntityTier::Function))?;
        let total_fn = functions
            .iter()
            .filter(|e| !is_test_entity(&e.name))
            .count() as u64;
        total += total_fn;
        missing += storage.count_missing_summaries(&repo.id)?;
    }
    Ok(TierCoverage { total, missing })
}

/// Per-tier coverage for every tier the refresh report shows: function,
/// file, module, subsystem (issue #793).
///
/// The function tier is computed via the unchanged formula
/// ([`compute_coverage`]). The rollup tiers use the tier-parameterized
/// `count_missing_summaries_tier`, whose test exclusion mirrors the
/// pipeline's notion of a test entity (functions by name, files by path).
pub fn compute_tier_coverage(
    storage: &dyn Storage,
    project_id: &str,
) -> lievo::Result<(TierCoverage, TierCoverage, TierCoverage, TierCoverage)> {
    let function = compute_coverage(storage, project_id)?;
    let repos = storage.list_repos(project_id)?;
    let mut file = TierCoverage::default();
    let mut module = TierCoverage::default();
    let mut subsystem = TierCoverage::default();
    for repo in &repos {
        let files = storage.entities_by_repo(&repo.id, Some(EntityTier::File))?;
        file.total += files.len() as u64;
        file.missing += storage.count_missing_summaries_tier(&repo.id, "file")?;

        let modules = storage.entities_by_repo(&repo.id, Some(EntityTier::Module))?;
        module.total += modules.len() as u64;
        module.missing += storage.count_missing_summaries_tier(&repo.id, "module")?;

        let subsystems = storage.entities_by_repo(&repo.id, Some(EntityTier::Subsystem))?;
        subsystem.total += subsystems.len() as u64;
        subsystem.missing += storage.count_missing_summaries_tier(&repo.id, "subsystem")?;
    }
    Ok((function, file, module, subsystem))
}

/// Build the refresh success report (JSON object / human lines).
///
/// `summarization_enabled` mirrors the shared pipeline gate
/// (`summarization::pipeline::summarization_enabled`, issue #786) — the
/// enablement rule lives there, not here. When disabled, the report carries
/// `summarization_disabled: true` (JSON) or a "disabled" line (human) and
/// must NOT imply incompleteness. When enabled with missing summaries, it
/// carries `summaries_incomplete: true` plus a warning line.
///
/// Per-tier output stays bounded (issue #793): at most four coverage lines
/// (one per tier with entities), never one line per skipped entity.
pub fn build_report(
    stats: &lievo::refresh::RefreshStats,
    tiers: (TierCoverage, TierCoverage, TierCoverage, TierCoverage),
    summarization_enabled: bool,
    fmt: OutputFormat,
) -> String {
    let (function, file, module, subsystem) = tiers;
    let incomplete = summarization_enabled
        && (function.missing > 0
            || file.missing > 0
            || module.missing > 0
            || subsystem.missing > 0);

    let mut value = serde_json::json!({
        "was_stale": true,
        "repos_refreshed": stats.repos_refreshed,
        "elapsed_secs": stats.elapsed_secs
    });
    let Some(object) = value.as_object_mut() else {
        // Unreachable: json! object literal always produces an object. Return
        // the bare value rather than panicking on a JSON invariant we do not
        // control from here.
        return value.to_string();
    };
    if summarization_enabled {
        // Aggregate coverage across repos for multi-repo projects. The
        // function tier keeps its pre-#793 field names so existing consumers
        // and historical comparisons stay valid.
        object.insert(
            "summary_coverage".to_string(),
            serde_json::json!({
                "total": function.total,
                "summarized": function.summarized(),
                "missing": function.missing,
                "pct": function.pct(),
            }),
        );
        // Per-tier coverage (issue #793): a whole tier failing to summarize
        // can no longer hide behind the function tier's number.
        object.insert(
            "tier_coverage".to_string(),
            serde_json::json!({
                "function": {
                    "total": function.total,
                    "summarized": function.summarized(),
                    "missing": function.missing,
                    "pct": function.pct(),
                },
                "file": {
                    "total": file.total,
                    "summarized": file.summarized(),
                    "missing": file.missing,
                    "pct": file.pct(),
                },
                "module": {
                    "total": module.total,
                    "summarized": module.summarized(),
                    "missing": module.missing,
                    "pct": module.pct(),
                },
                "subsystem": {
                    "total": subsystem.total,
                    "summarized": subsystem.summarized(),
                    "missing": subsystem.missing,
                    "pct": subsystem.pct(),
                },

            }),
        );
    } else {
        // apfel absent or --no-summarize: no summaries by design.
        object.insert(
            "summarization_disabled".to_string(),
            serde_json::json!(true),
        );
    }
    if incomplete {
        object.insert("summaries_incomplete".to_string(), serde_json::json!(true));
    }

    if fmt == OutputFormat::Json {
        return value.to_string();
    }

    // Human output
    let mut out = format!(
        "Refresh complete: {} repo(s) re-analyzed in {}s.\n",
        stats.repos_refreshed, stats.elapsed_secs
    );
    if summarization_enabled {
        out.push_str(&format!(
            "Summarized {}/{} functions ({:.1}%); {} missing\n",
            function.summarized(),
            function.total,
            function.pct(),
            function.missing
        ));
        // One line per tier with entities (issue #793): bounded by the tier
        // count, never by the entity count.
        for (tier_name, tier) in [
            ("file", &file),
            ("module", &module),
            ("subsystem", &subsystem),
        ] {
            if tier.total > 0 {
                out.push_str(&format!(
                    "  {}: {}/{} summarized ({:.1}%); {} missing\n",
                    tier_name,
                    tier.summarized(),
                    tier.total,
                    tier.pct(),
                    tier.missing
                ));
            }
        }
        if incomplete {
            out.push_str("warning: summary coverage is incomplete — re-run `lievo refresh` to resume summarization.\n");
        }
    } else {
        out.push_str("Summarization disabled (apfel not in PATH or --no-summarize).\n");
    }
    out
}
