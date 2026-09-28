// InsightDetector: detects complexity hotspots, high-coupling modules, coverage gaps,
// circular dependencies, and god modules.
//
// Detection is deterministic and purely synchronous (no I/O beyond storage queries).
// On each run: existing insights are invalidated, then re-detected and upserted.

use crate::model::{Entity, EntityTier, Insight};
use crate::retrieval::project_boundary::same_project;
use crate::storage::Storage;

use super::insights_helpers::{complexity_of, file_stem, test_file_matches_module};
pub use super::insights_helpers::{insight_id, is_test_file, now, severity_order};

use super::insights_circular::detect_circular_dependencies;
use super::insights_god::detect_god_modules;

/// Minimum absolute edge count before a module can be flagged for coupling.
/// Prevents noise in tiny projects with fewer than 10 modules.
const MIN_COUPLING_THRESHOLD: usize = 3;

/// Minimum test ratio (test files / total files) before a module is flagged.
const TEST_RATIO_THRESHOLD: f64 = 0.10;

/// Languages for which test coverage gap detection is applicable.
pub(crate) const TESTABLE_LANGUAGES: &[&str] = &[
    "rust",
    "python",
    "javascript",
    "typescript",
    "jsx",
    "tsx",
    "go",
    "java",
    "kotlin",
    "ruby",
    "c",
    "cpp",
    "c++",
    "swift",
    "scala",
    "php",
];

pub struct InsightDetector<'a> {
    storage: &'a dyn Storage,
    project_id: &'a str,
    repo_root: Option<&'a std::path::Path>,
}

impl<'a> InsightDetector<'a> {
    pub fn new(storage: &'a dyn Storage, project_id: &'a str) -> Self {
        Self {
            storage,
            project_id,
            repo_root: None,
        }
    }

    pub fn with_repo_root(mut self, repo_root: &'a std::path::Path) -> Self {
        self.repo_root = Some(repo_root);
        self
    }

    /// Run all detection algorithms and persist results.
    ///
    /// Marks all previous insights stale, then detects and upserts current ones.
    /// Returns insights sorted by severity (critical first), then by title.
    pub fn detect(&self) -> crate::Result<Vec<Insight>> {
        self.storage.invalidate_insights(self.project_id)?;

        let mut insights = Vec::new();
        insights.extend(self.detect_complexity_hotspots()?);
        insights.extend(self.detect_function_hotspots()?);
        insights.extend(self.detect_high_coupling()?);
        insights.extend(self.detect_coverage_gaps()?);

        // Issue #551/#570: Compute is_predominantly_rust from File entities and pass to
        // detect_circular_dependencies so the algorithm itself stays pure (no I/O).
        let file_entities = self
            .storage
            .list_entities(self.project_id, Some(EntityTier::File))?;
        let total_with_language = file_entities
            .iter()
            .filter(|f| f.language.is_some())
            .count();
        let rust_count = file_entities
            .iter()
            .filter(|f| {
                f.language
                    .as_deref()
                    .is_some_and(|l| l.eq_ignore_ascii_case("rust"))
            })
            .count();
        let skip_for_rust = total_with_language > 0 && (rust_count * 2) > total_with_language;

        let modules = self
            .storage
            .list_entities(self.project_id, Some(EntityTier::Module))?;
        insights.extend(detect_circular_dependencies(
            self.storage,
            self.project_id,
            &modules,
            skip_for_rust,
        )?);

        insights.extend(detect_god_modules(self.storage, self.project_id)?);

        // Sort: critical → high → medium → low, then by title for stability
        insights.sort_by(|a, b| {
            severity_order(a.severity.as_deref())
                .cmp(&severity_order(b.severity.as_deref()))
                .then_with(|| a.title.cmp(&b.title))
        });

        for insight in &insights {
            self.storage.upsert_insight(insight)?;
        }

        // Return from DB so detected_at reflects the original timestamp on re-runs.
        // Re-sort after fetching from DB: list_insights orders by detected_at DESC,
        // but the API contract requires severity (critical first), then title order.
        // Use 100_000 as a safe upper bound — usize::MAX would overflow i64 in SQLite.
        let mut results = self
            .storage
            .list_insights(self.project_id, None, None, 100_000)?;
        results.sort_by(|a, b| {
            severity_order(a.severity.as_deref())
                .cmp(&severity_order(b.severity.as_deref()))
                .then_with(|| a.title.cmp(&b.title))
        });
        Ok(results)
    }

    // ---- Complexity hotspot detection ----

    fn detect_complexity_hotspots(&self) -> crate::Result<Vec<Insight>> {
        self.detect_hotspots_for_tier(EntityTier::File, "complexity_hotspot", "High complexity")
    }

    fn detect_function_hotspots(&self) -> crate::Result<Vec<Insight>> {
        self.detect_hotspots_for_tier(
            EntityTier::Function,
            "function_complexity_hotspot",
            "Complex function",
        )
    }

    fn detect_hotspots_for_tier(
        &self,
        tier: EntityTier,
        category: &str,
        title_prefix: &str,
    ) -> crate::Result<Vec<Insight>> {
        let entities = self.storage.list_entities(self.project_id, Some(tier))?;

        if entities.is_empty() {
            return Ok(vec![]);
        }

        let total: f64 = entities.iter().map(complexity_of).sum();
        let avg = total / entities.len() as f64;

        if avg <= 0.0 {
            return Ok(vec![]);
        }

        let mut hotspots = Vec::new();
        for entity in &entities {
            let complexity = complexity_of(entity);
            if complexity == 0.0 {
                continue;
            }
            let ratio = complexity / avg;
            let severity = if ratio > 4.0 {
                "critical"
            } else if ratio > 3.0 {
                "high"
            } else if ratio > 2.0 {
                "medium"
            } else if ratio >= 1.5 {
                "low"
            } else {
                continue;
            };

            let name = entity.path.as_deref().unwrap_or(&entity.name);
            let entity_id = &entity.id;
            let insight = Insight {
                id: insight_id(self.project_id, category, entity_id),
                project_id: self.project_id.to_string(),
                category: category.to_string(),
                severity: Some(severity.to_string()),
                title: format!("{}: {}", title_prefix, name),
                description: Some(format!(
                    "Complexity {complexity:.1} is {ratio:.1}x the project average of {avg:.1}"
                )),
                entity_ids_json: Some(format!("[\"{entity_id}\"]")),
                detected_at: now(),
                still_valid: true,
            };
            hotspots.push(insight);
        }

        Ok(hotspots)
    }

    // ---- Coupling analysis ----

    fn detect_high_coupling(&self) -> crate::Result<Vec<Insight>> {
        let modules = self
            .storage
            .list_entities(self.project_id, Some(EntityTier::Module))?;

        let total = modules.len();

        // Collect (fan_in, fan_out) for every module, excluding Contains edges.
        let mut module_edges: Vec<(&crate::model::Entity, usize, usize)> =
            Vec::with_capacity(total);
        for module in &modules {
            // Project boundary (issue #764): a cross-project edge must not
            // inflate a module's fan-in/fan-out. The module set is already
            // project-scoped; the edge endpoint's project must match.
            let project_id = &module.project_id;
            let fan_out = self
                .storage
                .relationships_from(&module.id)?
                .iter()
                .filter(|(rel, target)| {
                    rel.rel_type != crate::model::RelType::Contains
                        && same_project(project_id, &target.project_id)
                })
                .count();
            let fan_in = self
                .storage
                .relationships_to(&module.id)?
                .iter()
                .filter(|(rel, source)| {
                    rel.rel_type != crate::model::RelType::Contains
                        && same_project(project_id, &source.project_id)
                })
                .count();
            module_edges.push((module, fan_in, fan_out));
        }

        // Compute percentile thresholds.
        // For small projects (< 10 modules), use a fixed fallback threshold of 5.
        let (high_threshold, medium_threshold, low_threshold) = if total < 10 {
            (5, 5, 5)
        } else {
            // Sort all edge counts combined to find percentile cutoffs.
            let mut fan_ins: Vec<usize> = module_edges.iter().map(|(_, fi, _)| *fi).collect();
            let mut fan_outs: Vec<usize> = module_edges.iter().map(|(_, _, fo)| *fo).collect();
            fan_ins.sort_unstable();
            fan_outs.sort_unstable();

            // Top 5%: index at 95th percentile; top 10%: index at 90th; top 20%: index at 80th.
            let p95_idx = ((total * 95) / 100).min(total - 1);
            let p90_idx = ((total * 90) / 100).min(total - 1);
            let p80_idx = ((total * 80) / 100).min(total - 1);

            let high_fi = fan_ins[p95_idx].max(MIN_COUPLING_THRESHOLD);
            let high_fo = fan_outs[p95_idx].max(MIN_COUPLING_THRESHOLD);
            let med_fi = fan_ins[p90_idx].max(MIN_COUPLING_THRESHOLD);
            let med_fo = fan_outs[p90_idx].max(MIN_COUPLING_THRESHOLD);
            let low_fi = fan_ins[p80_idx].max(MIN_COUPLING_THRESHOLD);
            let low_fo = fan_outs[p80_idx].max(MIN_COUPLING_THRESHOLD);
            // Use the more conservative (lower) of fan-in and fan-out thresholds
            // so either axis alone can trigger the insight.
            (high_fi.min(high_fo), med_fi.min(med_fo), low_fi.min(low_fo))
        };

        // Pre-sort both axes once before the flagging loop (used for rank computation).
        let sorted_fan_ins: Vec<usize> = {
            let mut v: Vec<usize> = module_edges.iter().map(|(_, fi, _)| *fi).collect();
            v.sort_unstable();
            v
        };
        let sorted_fan_outs: Vec<usize> = {
            let mut v: Vec<usize> = module_edges.iter().map(|(_, _, fo)| *fo).collect();
            v.sort_unstable();
            v
        };

        let mut insights = Vec::new();
        for (module, fan_in, fan_out) in &module_edges {
            let max_edges = fan_in.max(fan_out);

            // Skip modules below the low severity threshold.
            if *max_edges < low_threshold {
                continue;
            }

            // Assign severity based on percentile boundaries (inclusive: >= threshold).
            let severity = if *max_edges >= high_threshold {
                "high"
            } else if *max_edges >= medium_threshold {
                "medium"
            } else {
                "low"
            };

            // Compute rank for the dominant axis (fan-in or fan-out).
            let (dominant_label, dominant_val, sorted_axis) = if fan_in >= fan_out {
                ("fan-in", *fan_in, &sorted_fan_ins)
            } else {
                ("fan-out", *fan_out, &sorted_fan_outs)
            };
            // Rank: how many modules have a lower count (1-indexed from top).
            let below = sorted_axis.partition_point(|&x| x < dominant_val);
            let rank = total - below; // modules at or above this value
            let pct = (rank as f64 / total as f64 * 100.0).round() as usize;

            let description = format!(
                "fan-in={fan_in}, fan-out={fan_out} — {dominant_label} {dominant_val} is in the top {pct}% of modules (rank {rank} of {total})"
            );
            let insight = Insight {
                id: insight_id(self.project_id, "high_coupling", &module.id),
                project_id: self.project_id.to_string(),
                category: "high_coupling".to_string(),
                severity: Some(severity.to_string()),
                title: format!("High coupling: {}", module.name),
                description: Some(description),
                entity_ids_json: Some(format!("[\"{}\"]", module.id)),
                detected_at: now(),
                still_valid: true,
            };
            insights.push(insight);
        }

        Ok(insights)
    }

    // ---- Coverage gap detection ----

    /// Return true if a Rust source file has inline test functions.
    /// Uses `extract_cfg_test_functions` to detect `#[cfg(test)]` blocks in the source.
    pub(crate) fn file_has_inline_tests(&self, entity: &Entity) -> bool {
        // Only applicable to Rust files when repo_root is available
        let repo_root = match self.repo_root {
            Some(r) => r,
            None => return false,
        };
        let file_path = match entity.path.as_deref() {
            Some(p) if p.ends_with(".rs") => p,
            _ => return false,
        };
        // extract_cfg_test_functions returns non-empty set if file has cfg(test) content
        !crate::extraction::function_preservation::extract_cfg_test_functions(file_path, repo_root)
            .is_empty()
    }

    fn detect_coverage_gaps(&self) -> crate::Result<Vec<Insight>> {
        let modules = self
            .storage
            .list_entities(self.project_id, Some(EntityTier::Module))?;

        // Collect all File-tier entities that are test files, for cross-directory attribution.
        // Integration tests in tests/ are grouped under root, not under the module they test.
        let all_files = self
            .storage
            .list_entities(self.project_id, Some(EntityTier::File))?;
        let project_test_files: Vec<&Entity> =
            all_files.iter().filter(|f| is_test_file(f)).collect();

        // Single pass: collect (module, files) pairs, calling entities_by_parent once per module.
        let mut module_files: Vec<(Entity, Vec<Entity>)> = Vec::new();
        for m in modules {
            let files = self.storage.entities_by_parent(&m.id)?;
            module_files.push((m, files));
        }

        // Pre-compute inline test cache across ALL files (deduped by path) BEFORE the
        // module processing loop so each unique file path is read at most once.
        let mut inline_test_cache: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for (_, files) in &module_files {
            for f in files {
                if let Some(p) = f.path.as_deref()
                    && self.file_has_inline_tests(f)
                {
                    inline_test_cache.insert(p.to_string());
                }
            }
        }

        let mut insights = Vec::new();
        for (module, files) in module_files {
            if files.is_empty() {
                continue;
            }

            // Skip modules where no files have a testable language — whether remaining
            // files are known non-testable (e.g., YAML) or have unknown (None) language.
            let testable_count = files
                .iter()
                .filter(|f| {
                    f.language.as_deref().is_some_and(|l| {
                        TESTABLE_LANGUAGES
                            .iter()
                            .any(|tl| l.eq_ignore_ascii_case(tl))
                    })
                })
                .count();
            if testable_count == 0 {
                continue;
            }

            // Count direct-child test files (by name/path) plus Rust files with inline tests
            let direct_child_ids: std::collections::HashSet<&str> =
                files.iter().map(|f| f.id.as_str()).collect();
            let direct_test_count = files
                .iter()
                .filter(|f| {
                    is_test_file(f)
                        || f.path
                            .as_deref()
                            .is_some_and(|p| inline_test_cache.contains(p))
                })
                .count();

            // Collect stems of non-test source files in this module (4+ chars to avoid
            // broad matches on short names like "mod" or "lib").
            let child_stems: Vec<String> = files
                .iter()
                .filter(|f| {
                    !is_test_file(f)
                        && !f
                            .path
                            .as_deref()
                            .is_some_and(|p| inline_test_cache.contains(p))
                })
                .filter_map(file_stem)
                .filter(|s| s.len() >= 4)
                .collect();

            // Count cross-directory test files that reference this module or its children,
            // excluding files already counted as direct children.
            let cross_dir_count = project_test_files
                .iter()
                .filter(|tf| !direct_child_ids.contains(tf.id.as_str()))
                .filter(|tf| {
                    file_stem(tf).is_some_and(|stem| {
                        test_file_matches_module(&stem, &module.name, &child_stems)
                    })
                })
                .count();

            let test_count = direct_test_count + cross_dir_count;

            if test_count == 0 || (test_count as f64 / files.len() as f64) < TEST_RATIO_THRESHOLD {
                let severity = if test_count == 0 { "high" } else { "medium" };
                let description = if cross_dir_count > 0 {
                    format!(
                        "{direct_test_count} direct test file(s) + {cross_dir_count} cross-directory test(s) out of {} total file(s) in module",
                        files.len()
                    )
                } else {
                    format!(
                        "{test_count} test file(s) out of {} total file(s) in module",
                        files.len()
                    )
                };
                let insight = Insight {
                    id: insight_id(self.project_id, "coverage_gap", &module.id),
                    project_id: self.project_id.to_string(),
                    category: "coverage_gap".to_string(),
                    severity: Some(severity.to_string()),
                    title: format!("Low test coverage: {}", module.name),
                    description: Some(description),
                    entity_ids_json: Some(format!("[\"{}\"]", module.id)),
                    detected_at: now(),
                    still_valid: true,
                };
                insights.push(insight);
            }
        }

        Ok(insights)
    }
}
