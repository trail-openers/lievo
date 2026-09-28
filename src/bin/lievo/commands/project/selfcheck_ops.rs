// `lievo admin selfcheck` — pinned-repo quality gate (issue #715).
//
// Four independent sections, each with its own pass/fail threshold:
//   (a) edge-correctness   — sampled import edges re-verified by an
//       INDEPENDENT normalised-path resolver (never the stored resolver maps).
//   (b) false-0-callers    — files with 0 recorded importers but
//       unresolved_internal==0 that DO have on-disk importers.
//   (c) retrieval-probes   — configurable probe file, recall@k floors incl. a
//       worst-single-probe floor (storage-backed; skipped if unindexed).
//   (d) payload-bytes      — ExploreTool<S>::call() response byte floor for
//       each probe query (storage-backed; skipped if unindexed).
//
// Sections (a)/(b) are pure re-extraction (no storage), mirroring
// `compute_coverage`. Sections (c)/(d) require an indexed project and are
// SKIPPED (not failed) when no project/index is available, per the PM
// decision recorded in issue #715.
//
// Pure metric/gate functions live in `selfcheck_metrics.rs` (kept separate
// to stay within the 500-line src budget, AGENTS.md §6).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lievo::extraction::code_extractor::CodeExtractor;
use lievo::model::CodeUnit;
use lievo::output::OutputFormat;
use lievo::storage::Storage;
use lievo::{LievoError, Result};

use super::selfcheck_edge_split::{classify_edges, count_evidenced, gate_edge_correctness};
use super::selfcheck_metrics::{
    ProbeResult, SectionReport, SelfcheckThresholds, detect_false_zero_callers,
    gate_false_zero_callers, gate_payload_bytes, gate_retrieval_probes,
    on_disk_grep_importer_count, parse_probes, run_probe, sample_edges, skipped_section,
    verify_edges, wrong_edge_rate,
};
use super::selfcheck_super_probe::{gate_super_probe, probe_super_sites};

/// Flags for `lievo admin selfcheck` (issue #715). A `clap::Args` struct
/// (not inline enum-variant fields) so `AdminCommands` in main.rs stays
/// within its own file-size budget via `#[command(flatten)]`.
#[derive(clap::Args, Debug)]
pub struct SelfcheckArgs {
    /// Repository path to check
    #[arg(long, value_name = "REPO_PATH")]
    pub repo: PathBuf,
    /// Project name (required for the storage-backed probe/payload sections)
    #[arg(long, value_name = "PROJECT")]
    pub project: Option<String>,
    /// Probe file (query<TAB>expected_path lines); enables sections (c)/(d)
    #[arg(long, value_name = "PROBES_PATH")]
    pub probes: Option<PathBuf>,
    /// Fail (exit 1) when any non-skipped section fails its threshold
    #[arg(long)]
    pub gate: bool,
    /// Max wrong-edge rate for the edge-correctness section
    #[arg(long, default_value = "0.0")]
    pub max_wrong_edge_rate: f64,
    /// Max false-0-callers count
    #[arg(long, default_value = "0")]
    pub max_false_zero_callers: usize,
    /// Number of import edges to sample for edge-correctness (0 = census: all)
    #[arg(long, default_value = "0")]
    pub sample_size: usize,
    /// k for recall@k in the retrieval-probes section
    #[arg(long, default_value = "5")]
    pub min_recall_k: usize,
    /// Per-probe recall@k floor
    #[arg(long, default_value = "0.0")]
    pub min_recall_floor: f32,
    /// Worst-single-probe recall@k floor
    #[arg(long, default_value = "0.0")]
    pub min_worst_probe: f32,
    /// Minimum ExploreTool response bytes per probe query
    #[arg(long, default_value = "200")]
    pub min_payload_bytes: usize,
    /// Max confirmed structural super:: failures (the probe's own gate,
    /// issue #744). The default does not break existing CI until the
    /// operator ratchets it (the #733 pattern: explicit flag in the
    /// real-corpus steps).
    #[arg(long)]
    pub max_super_probe_failures: Option<usize>,
}

impl From<&SelfcheckArgs> for SelfcheckThresholds {
    fn from(args: &SelfcheckArgs) -> Self {
        Self {
            max_wrong_edge_rate: args.max_wrong_edge_rate,
            max_false_zero_callers: args.max_false_zero_callers,
            sample_size: args.sample_size,
            min_recall_k: args.min_recall_k,
            min_recall_floor: args.min_recall_floor,
            min_worst_probe: args.min_worst_probe,
            min_payload_bytes: args.min_payload_bytes,
            max_super_probe_failures: args.max_super_probe_failures.unwrap_or(usize::MAX),
        }
    }
}

// ---------------------------------------------------------------------------
// Orchestration: re-run extraction (no storage), mirroring compute_coverage.
// ---------------------------------------------------------------------------

/// Extraction context shared by sections (a) and (b): code units, resolved
/// file→file import edges (by path), the full scanned-file set, and the
/// repo-global unresolved-import counter.
struct SelfcheckExtraction {
    code_units: Vec<CodeUnit>,
    /// ALL file-level import edges (both provenances, issue #777) with their
    /// provenance: the raw relationship list must survive to the sampling seam
    /// instead of being flattened to bare path pairs (that flattening dropped
    /// r.provenance). The edge-split (classify_edges) re-filters by RelType
    /// and splits by provenance; the false-zero section consumes the union
    /// (both classes) so its numbers are unchanged by the split.
    import_relationships: Vec<lievo::model::Relationship>,
    id_to_path: HashMap<String, String>,
    all_files: Vec<String>,
    unresolved_internal: u32,
}

fn run_extraction(repo_path: &Path) -> Result<SelfcheckExtraction> {
    let mut extractor =
        lievo::extraction::tree_sitter_extractor::TreeSitterExtractor::new(repo_path, true)?;
    extractor.index(false)?;
    let code_units = extractor.read_all_units()?;
    let all_files = extractor.extracted_files().to_vec();

    if code_units.is_empty() {
        return Ok(SelfcheckExtraction {
            code_units,
            import_relationships: Vec::new(),
            id_to_path: HashMap::new(),
            all_files,
            unresolved_internal: 0,
        });
    }

    let grouping_config = lievo::extraction::grouping::GroupingConfig {
        code_units: &code_units,
        scanned_file_paths: &all_files,
        project_id: "selfcheck",
        repo_name: "selfcheck",
        repo_id: "selfcheck",
        repo_path,
        config: None,
        exclude_paths: &[],
    };
    let grouping = lievo::extraction::grouping::group_code_units(&grouping_config)?;

    let (relationships, unresolved) = lievo::analysis::relationships::RelationshipBuilder::build(
        &code_units,
        &grouping,
        "selfcheck",
        "selfcheck",
        repo_path,
    )?;

    let id_to_path: HashMap<String, String> = grouping
        .files
        .iter()
        .filter_map(|f| f.path.as_ref().map(|p| (f.id.clone(), p.clone())))
        .collect();

    Ok(SelfcheckExtraction {
        code_units,
        import_relationships: relationships,
        id_to_path,
        all_files,
        unresolved_internal: unresolved.internal,
    })
}

/// Full selfcheck result: one report per section, in a fixed order.
pub struct SelfcheckReport {
    pub repo_name: String,
    pub sections: Vec<SectionReport>,
}

impl SelfcheckReport {
    pub fn gate_failed(&self) -> bool {
        self.sections.iter().any(|s| !s.skipped && !s.passed)
    }
}

/// Run the pure sections (edge-correctness, false-0-callers) against a repo
/// path. No storage read — mirrors `compute_coverage`'s re-extraction shape.
pub fn run_pure_sections(
    repo_path: &Path,
    thresholds: &SelfcheckThresholds,
) -> Result<Vec<SectionReport>> {
    let extraction = run_extraction(repo_path)?;
    let repo_name = repo_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo_path.display().to_string());

    let known_paths: HashSet<String> = extraction.all_files.iter().cloned().collect();
    // `#[path]` logical→physical module map (issue #732): computed once per
    // run. The logical→physical half feeds the independent resolver (edge
    // verification) so `crate::{logical}` specifiers for divergent modules
    // resolve to the physical file the production resolver recorded the edge
    // against; the physical→logical half feeds the false-zero section to
    // exempt `#[cfg(test)]`-gated `#[path]` test files whose only importer
    // is a declaration the non-test profile never compiles.
    let module_map = lievo::analysis::module_map::scan_rust_module_map(repo_path);
    let logical_to_physical = &module_map.logical_to_physical;
    // Split the import-edge population by provenance (issue #777): the
    // verifier only checks import-resolved (use-statement) edges; call-based
    // edges are reported as unverifiable-by-construction and never scored.
    let population = classify_edges(&extraction.import_relationships, &extraction.id_to_path);
    let sampled = sample_edges(&population.import_resolved, thresholds.sample_size);
    let samples = verify_edges(
        &sampled,
        &extraction.code_units,
        &known_paths,
        &repo_name,
        logical_to_physical,
    );
    let rate = wrong_edge_rate(&samples);
    let evidenced = count_evidenced(&samples);
    let mut edge_report = gate_edge_correctness(
        rate,
        evidenced,
        population.import_resolved.len(),
        population.call_based_census(),
        thresholds,
    );
    if module_map.skipped > 0 {
        edge_report.detail.push_str(&format!(
            " (degraded: {} mod-map skips)",
            module_map.skipped
        ));
    }

    // The false-zero section keys off the FULL import-edge set (both
    // provenances, issue #777 regression criterion): a file imported only
    // via a call-based (Heuristic) edge before the split must not become a
    // false-zero caller after it.
    let flagged = detect_false_zero_callers(
        &extraction.all_files,
        &population.imported,
        extraction.unresolved_internal,
        &module_map.physical_to_logical,
        |f| on_disk_grep_importer_count(repo_path, f, &extraction.all_files),
    );
    let false_zero_report = gate_false_zero_callers(&flagged, thresholds);

    // Section (e): structural super:: probe (issue #744) — independent
    // evidence read from the source tree itself (never via the resolvers).
    // The `#[path]` alias map is consulted for target LOCATION only
    // (issue #758): `physical_to_logical` turns the importing file's
    // physical path into its logical module path; `logical_to_physical`
    // maps a walked logical path to its (possibly divergent) physical file.
    // The map is a structural fact from `mod` declarations, not resolver
    // output — independence is preserved (see the probe module's import
    // list).
    let super_results = probe_super_sites(
        &extraction.code_units,
        &extraction.all_files,
        repo_path,
        logical_to_physical,
        &module_map.physical_to_logical,
    );
    let super_report = gate_super_probe(&super_results, thresholds);

    Ok(vec![edge_report, false_zero_report, super_report])
}

/// Storage-backed sections (retrieval-probes, payload-bytes): drives each
/// probe through `ExploreTool::call()` for the project's single repo.
fn run_probe_sections(
    storage: &lievo::storage::sqlite::SqliteStorage,
    repo_path: &Path,
    project_name: &str,
    probes_path: &Path,
    thresholds: &SelfcheckThresholds,
) -> Result<Vec<SectionReport>> {
    let content = std::fs::read_to_string(probes_path).map_err(|e| {
        LievoError::InvalidInput(format!(
            "failed to read probes file {}: {e}",
            probes_path.display()
        ))
    })?;
    let probes = parse_probes(&content)?;

    let project_id = lievo::project_resolution::resolve_project_id(storage, Some(project_name))?;
    let repos = storage.list_repos(&project_id)?;
    let output_dir = storage.get_output_dirs(&project_id)?.into_iter().next();
    let tool_repo_path = repos
        .first()
        .map(|r| PathBuf::from(&r.local_path))
        .unwrap_or_else(|| repo_path.to_path_buf());

    let probe_storage = lievo::storage::sqlite::SqliteStorage::open()?;
    let ctx = std::sync::Arc::new(lievo::retrieval::tools::ToolContext {
        storage: std::sync::Arc::new(std::sync::Mutex::new(probe_storage)),
        project_id,
        repo_path: tool_repo_path.clone(),
        output_dir,
        zero_repo_guidance: None,
    });
    // `ExploreTool` has no public constructor outside the lievo lib crate
    // (its `ctx` field is `pub(crate)`); `create_tools()` is the public seam
    // that returns the full tool set (issue #715 AC: the payload floor must
    // exercise `call()`, the same seam MCP/agents use).
    let tools = lievo::retrieval::tools::create_tools(ctx);
    let tool = tools
        .iter()
        .find(|t| t.name() == "explore")
        .ok_or_else(|| {
            LievoError::InvalidInput("explore tool not registered in create_tools()".to_string())
        })?;
    let repo_root = tool_repo_path.to_string_lossy().into_owned();

    let results: Vec<ProbeResult> = probes
        .iter()
        .map(|p| run_probe(tool.as_ref(), p, &repo_root, thresholds.min_recall_k))
        .collect::<Result<Vec<_>>>()?;

    Ok(vec![
        gate_retrieval_probes(&results, thresholds),
        gate_payload_bytes(&results, thresholds),
    ])
}

/// Top-level `lievo admin selfcheck` handler.
///
/// Storage-backed sections (retrieval-probes, payload-bytes) run only when
/// `args.project` is given AND `args.probes` is passed; they SKIP (not fail)
/// otherwise, per the PM decision.
pub fn selfcheck(
    storage: &lievo::storage::sqlite::SqliteStorage,
    args: SelfcheckArgs,
    fmt: OutputFormat,
) -> Result<()> {
    let thresholds = SelfcheckThresholds::from(&args);
    let mut sections = run_pure_sections(&args.repo, &thresholds)?;

    match (args.project.as_deref(), args.probes.as_deref()) {
        (Some(name), Some(probes_path)) => {
            sections.extend(run_probe_sections(
                storage,
                &args.repo,
                name,
                probes_path,
                &thresholds,
            )?);
        }
        (None, Some(_)) => {
            sections.push(skipped_section(
                "retrieval_probes",
                "--probes given but no --project — storage-backed sections need an indexed project",
            ));
            sections.push(skipped_section(
                "payload_bytes",
                "--probes given but no --project — storage-backed sections need an indexed project",
            ));
        }
        _ => {
            sections.push(skipped_section(
                "retrieval_probes",
                "no --probes file given",
            ));
            sections.push(skipped_section("payload_bytes", "no --probes file given"));
        }
    }

    let repo_name = args
        .repo
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| args.repo.display().to_string());
    let report = SelfcheckReport {
        repo_name,
        sections,
    };

    print_report(&report, fmt);

    if args.gate && report.gate_failed() {
        let failed: Vec<&str> = report
            .sections
            .iter()
            .filter(|s| !s.skipped && !s.passed)
            .map(|s| s.name)
            .collect();
        return Err(LievoError::InvalidInput(format!(
            "selfcheck gate failed: {}",
            failed.join(", ")
        )));
    }
    Ok(())
}

fn print_report(report: &SelfcheckReport, fmt: OutputFormat) {
    if fmt == OutputFormat::Json {
        let sections_json = report
            .sections
            .iter()
            .map(|s| {
                format!(
                    "{{\"name\":\"{}\",\"passed\":{},\"skipped\":{},\"skip_reason\":{},\"detail\":\"{}\"}}",
                    super::json_escape(s.name),
                    s.passed,
                    s.skipped,
                    s.skip_reason
                        .as_deref()
                        .map(|r| format!("\"{}\"", super::json_escape(r)))
                        .unwrap_or_else(|| "null".to_string()),
                    super::json_escape(&s.detail),
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        println!(
            "{{\"repo\":\"{}\",\"gate_failed\":{},\"sections\":[{}]}}",
            super::json_escape(&report.repo_name),
            report.gate_failed(),
            sections_json
        );
        return;
    }

    println!("Selfcheck: {}", report.repo_name);
    for s in &report.sections {
        if s.skipped {
            println!(
                "  {:<20} SKIPPED — {}",
                s.name,
                s.skip_reason.as_deref().unwrap_or("")
            );
        } else {
            let verdict = if s.passed { "PASS" } else { "FAIL" };
            println!("  {:<20} {} — {}", s.name, verdict, s.detail);
        }
    }
}

#[cfg(test)]
#[path = "selfcheck_ops_tests.rs"]
pub(crate) mod selfcheck_ops_tests;
