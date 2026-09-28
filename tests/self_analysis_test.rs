// Self-analysis integration test — exercises the full pipeline against lievo's own source tree.
//
// This test is `#[ignore]`d and runs on a CI schedule (see the `scheduled-full-pipeline`
// job in `.github/workflows/ci.yml`, trigger `schedule: cron 30 4 * * 1-5`), NOT in the
// blocking CI job. Reason for the documented exception: it hard-codes the lievo repo
// path via CARGO_MANIFEST_DIR (full-tree analysis) and downloads the ONNX embedding
// model on first run (~150 MB total; the model is minishlab/potion-code-16M-v2), so
// it is expensive and would add 30+ seconds to every PR.
//
// Run explicitly with:
//   cargo test --test self_analysis_test -- --nocapture
//   cargo test --test self_analysis_test --ignored
use lievo::Lievo;
use lievo::analysis::pipeline::{AnalysisPipeline, PipelineConfig, ReindexMode};
use lievo::storage::sqlite::SqliteStorage;
use tempfile::TempDir;

#[test]
#[ignore = "expensive: downloads ONNX model + analyzes full lievo tree (see comment above; runs on a CI schedule)"]
fn self_analysis_full_pipeline() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n=== Lievo Self-Analysis Test: Analyzing lievo repo itself ===\n");

    // ── Step 1: Create a Lievo instance with a temp database ────────────────
    let tmp = TempDir::new()?;
    let db_path = tmp.path().join("dogfood.db");
    println!("Database: {}", db_path.display());

    let lievo = Lievo::open_at(&db_path)?;
    println!("✓ Opened Lievo instance");

    // ── Step 2: Create project ───────────────────────────────────────────────
    let project = lievo.create_project("lievo")?;
    println!("✓ Created project '{}' (id={})", project.name, project.id);

    let projects = lievo.list_projects()?;
    println!("  Listed {} project(s)", projects.len());
    assert_eq!(projects.len(), 1, "should have exactly one project");

    let fetched = lievo.project("lievo")?.expect("project must be found");
    assert_eq!(fetched.id, project.id);
    println!("  Fetched project by name ✓");

    // ── Step 3: Add the lievo repo (deterministic path via CARGO_MANIFEST_DIR) ─
    let repo_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_path_str = repo_path.to_str().ok_or("repo path not valid UTF-8")?;
    println!("\nRepo path: {}", repo_path_str);

    let repo = lievo.add_repo(&project.id, "lievo", repo_path_str)?;
    println!("✓ Added repo '{}' (id={})", repo.name, repo.id);

    let repos = lievo.list_repos(&project.id)?;
    assert_eq!(repos.len(), 1, "should have exactly one repo");
    println!("  Listed {} repo(s)", repos.len());

    // ── Step 4: Run analysis via AnalysisPipeline ────────────────────────────
    // The Lievo facade doesn't expose analyze() directly; we use the pipeline.
    // We need access to SqliteStorage, so we open it at the same db path.
    println!("\n--- Running analysis pipeline (may download ONNX model ~150MB on first run) ---");

    let storage = SqliteStorage::open_at(&db_path)?;
    let config = PipelineConfig {
        respect_ignore: true,
        reindex: ReindexMode::Incremental,
        no_summarize: false,
        skip_semantic_index: false,
    };
    let analysis_result = AnalysisPipeline::run_project(&storage, &project.id, &config);

    match &analysis_result {
        Ok(result) => {
            println!(
                "\n✓ Analysis completed: {} repo(s) analyzed, {} error(s)",
                result.runs.len(),
                result.repo_errors.len()
            );
            for run in &result.runs {
                println!(
                    "  Repo {}: {} entities, {} relationships ({}ms)",
                    run.repo_id,
                    run.entities_upserted,
                    run.relationships_upserted,
                    run.duration_ms.unwrap_or(0)
                );
            }
            for (repo_name, err) in &result.repo_errors {
                println!("  ERROR in repo '{}': {}", repo_name, err);
            }
        }
        Err(e) => {
            println!("\n✗ Analysis failed: {}", e);
            println!("  This is a valid finding — reporting as test failure.");
            return Err(format!("Analysis pipeline failed: {e}").into());
        }
    }

    let analysis_result = analysis_result.unwrap();

    assert!(
        analysis_result.repo_errors.is_empty(),
        "Analysis had repo errors: {:?}",
        analysis_result.repo_errors
    );

    // ── Step 5: Status / info ────────────────────────────────────────────────
    println!("\n--- Status / Info ---");
    let run_count = analysis_result.runs.len();
    let total_entities: i64 = analysis_result
        .runs
        .iter()
        .map(|r| r.entities_upserted)
        .sum();
    let total_relationships: i64 = analysis_result
        .runs
        .iter()
        .map(|r| r.relationships_upserted)
        .sum();
    assert!(
        total_entities > 0,
        "Expected non-zero entities from self-analysis; \
         code unit _subset_ column fix may have regressed (got 0)"
    );
    println!("  Repos analyzed: {}", run_count);
    println!("  Total entities upserted: {}", total_entities);
    println!("  Total relationships upserted: {}", total_relationships);

    // ── Step 6: Query subsystems, modules, files ─────────────────────────────
    println!("\n--- Querying subsystems ---");
    let subsystems = lievo.subsystems(&project.id)?;
    println!("  Found {} subsystem(s)", subsystems.len());
    for s in &subsystems {
        println!("  Subsystem: {} ({})", s.name, s.id);

        let modules = lievo.modules_in(&s.id)?;
        println!("    └─ {} module(s)", modules.len());

        for m in &modules {
            let files = lievo.files_in(&m.id)?;
            println!("       {} — {} file(s)", m.name, files.len());
            for f in files.iter().take(3) {
                println!(
                    "         • {} [path={}]",
                    f.name,
                    f.path.as_deref().unwrap_or("<none>")
                );
            }
            if files.len() > 3 {
                println!("         … ({} more)", files.len() - 3);
            }
        }
    }

    // ── Step 7: Hotspots ─────────────────────────────────────────────────────
    println!("\n--- Querying hotspots (top 10 by complexity) ---");
    let hotspots = lievo.hotspots(&project.id, 10)?;
    println!("  Found {} hotspot(s)", hotspots.len());
    for (i, h) in hotspots.iter().enumerate() {
        let complexity: f64 = h
            .metrics_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("complexity_max").and_then(|c| c.as_f64()))
            .unwrap_or(0.0);
        println!(
            "  #{}: {} (tier={:?}, complexity={:.1}, path={})",
            i + 1,
            h.name,
            h.tier,
            complexity,
            h.path.as_deref().unwrap_or("<none>")
        );
    }

    // ── Step 8: Insights ─────────────────────────────────────────────────────
    println!("\n--- Running insights ---");
    let insights = lievo.insights(&project.id)?;
    println!("  Detected {} insight(s)", insights.len());
    for ins in &insights {
        println!(
            "  [{}] {} — {}",
            ins.severity.as_deref().unwrap_or("-"),
            ins.title,
            ins.description.as_deref().unwrap_or("<no description>")
        );
    }

    // ── Entity by path spot-check (if entities were found) ───────────────────
    if !subsystems.is_empty() {
        println!("\n--- Entity by path spot-check ---");
        let repo_id = &repos[0].id;
        // Try a known Rust source file path
        let known_path = "src/lib.rs";
        match lievo.entity_by_path(repo_id, known_path)? {
            Some(e) => println!(
                "  Found entity for '{}': {} ({:?})",
                known_path, e.name, e.tier
            ),
            None => println!("  No entity found for '{}' (path may differ)", known_path),
        }
    }

    // ── Summary ──────────────────────────────────────────────────────────────
    println!("\n=== Self-analysis Summary ===");
    println!("  Subsystems:    {}", subsystems.len());
    println!("  Hotspots:      {}", hotspots.len());
    println!("  Insights:      {}", insights.len());
    println!("  Repo errors:   {}", analysis_result.repo_errors.len());
    println!("\n✓ Self-analysis test complete\n");

    Ok(())
}
