// Rollup skip outcome types (issue #793).
//
// Extracted from pipeline.rs to keep that file under the 500-line limit
// (AGENTS.md §6). This is an `include!`d file (see
// `mod pipeline_outcome { include!(...) }` in pipeline.rs), so `super`
// refers to the pipeline module's scope. The skip counters make the
// rollup "no child summaries" path a first-class `SummaryOutcome` field
// instead of a `tracing::warn!` line that no report surfaced.

use crate::model::{Entity, EntityTier};
use super::is_test_file_path;

/// Outcome of one `rollup_to_tier` pass (issue #793): entities updated plus
/// the first-class skip breakdown.
#[derive(Debug, Clone, Default)]
pub(crate) struct RollupOutcome {
    pub(crate) updated: usize,
    pub(crate) skipped: SkippedRollup,
}

/// Classify a rollup-tier entity skipped for lacking child summaries
/// (issue #793): deliberate policy skip vs failure vs orphaned.
///
/// A path marked as a test file is a DELIBERATE policy skip (#531) — the
/// file's functions are never extracted, so "no summarized child" is the
/// expected state, not a failure. A non-test entity with children is a
/// child-summary failure; with no children it is an orphan (#791-style).
pub(crate) fn classify_rollup_skip(entity: &Entity, child_count: usize) -> SkippedRollup {
    let mut bucket = SkippedRollup::default();
    if entity.path.as_deref().map(is_test_file_path).unwrap_or(false) {
        bucket.policy = 1;
    } else if child_count > 0 {
        bucket.child_summary_missing = 1;
    } else {
        bucket.no_children = 1;
    }
    bucket
}

/// Breakdown of rollup-tier entities skipped because no child had a summary
/// (issue #793). The buckets are distinct by construction: a policy-skipped
/// entity (e.g. a test file, #531) must not share a bucket with an entity
/// whose children exist but carry no summary.
#[derive(Debug, Clone, Default)]
pub struct SkippedRollup {
    /// Entities skipped because their own path marks a test file — an
    /// intentional policy skip (#531), not a failure.
    pub policy: u64,
    /// Entities with children, none of which had a summary.
    pub child_summary_missing: u64,
    /// Entities with no children at all (orphaned, e.g. a file whose
    /// functions were never extracted — #791), and no summary.
    /// File-tier childless entities are first offered a source-text fallback
    /// (issue #827); they land here only when the fallback has no source to
    /// read (empty/missing/unreadable), or when the fallback summarizer call
    /// fails.
    pub no_children: u64,
}

impl SkippedRollup {
    /// Total skipped entities in this bucket.
    pub fn total(&self) -> u64 {
        self.policy + self.child_summary_missing + self.no_children
    }
}

impl std::ops::Add for SkippedRollup {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        self.policy += other.policy;
        self.child_summary_missing += other.child_summary_missing;
        self.no_children += other.no_children;
        self
    }
}

/// Per-tier rollup skip counters, one row per rollup tier (issue #793).
#[derive(Debug, Clone, Default)]
pub struct RollupSkips {
    pub file: SkippedRollup,
    pub module: SkippedRollup,
    pub subsystem: SkippedRollup,
}

impl RollupSkips {
    /// Total skipped entities across all tiers and buckets.
    pub fn total(&self) -> u64 {
        self.file.total() + self.module.total() + self.subsystem.total()
    }

    /// Record one skip on the named tier (issue #793).
    pub(crate) fn record(&mut self, tier: EntityTier, bucket: SkippedRollup) {
        match tier {
            EntityTier::File => self.file = self.file.clone() + bucket,
            EntityTier::Module => self.module = self.module.clone() + bucket,
            EntityTier::Subsystem => self.subsystem = self.subsystem.clone() + bucket,
            EntityTier::Function => {}
        }
    }
}

impl std::ops::Add for RollupSkips {
    type Output = Self;

    fn add(mut self, other: Self) -> Self {
        self.file = self.file + other.file;
        self.module = self.module + other.module;
        self.subsystem = self.subsystem + other.subsystem;
        self
    }
}

/// Result of the summarization pipeline: total entities summarized, functions
/// skipped for exceeding apfel's context window (issue #649), functions
/// whose output did not honour the `#<n>:` contract (issue #776), and the
/// rollup skip counters (issue #793). Rollup skips are a first-class outcome:
/// they used to be a `tracing::warn!` that no counter or report surfaced.
#[derive(Debug, Clone, Default)]
pub struct SummaryOutcome {
    /// Total entities summarized (functions + files + modules + subsystems,
    /// excluding rollup-skipped entities).
    pub summarized: usize,
    /// Functions skipped because their code exceeds apfel's context window.
    pub skipped_oversized: u64,
    /// Functions whose model output did not honour the `#<n>:` contract.
    pub parse_failures: u64,
    /// Rollup-tier entities skipped because no child had a summary, per tier.
    pub rollup_skipped: RollupSkips,
}
