// Project-level operations: create, list, delete projects

use std::collections::HashMap;

use lievo::extraction::code_extractor::CodeExtractor;
use lievo::model::RelType;
use lievo::output::OutputFormat;
use lievo::storage::Storage;
use lievo::{LievoError, Result};

pub fn create_project(storage: &dyn Storage, name: &str) -> Result<()> {
    let project = storage.create_project(name, None)?;
    println!("Created project '{}' (id: {})", project.name, project.id);
    Ok(())
}

pub fn list_projects(storage: &dyn Storage) -> Result<()> {
    let projects = storage.list_projects()?;
    if projects.is_empty() {
        println!("No projects found. Run `lievo create-project <name>` to get started.");
        return Ok(());
    }
    println!("{:<36}  {:<30}  Created", "ID", "Name");
    println!("{}", "-".repeat(80));
    for p in &projects {
        // Show only the date portion of the ISO-8601 timestamp for readability.
        let date = p.created_at.get(..10).unwrap_or(&p.created_at);
        println!("{:<36}  {:<30}  {}", p.id, p.name, date);
    }
    Ok(())
}

pub fn delete_project(storage: &dyn Storage, name: &str, force: bool) -> Result<()> {
    let project = storage
        .get_project(name)?
        .ok_or_else(|| lievo::LievoError::ProjectNotFound(name.to_string()))?;

    if !force {
        let repos = storage.list_repos(&project.id)?;
        if !repos.is_empty() {
            eprintln!(
                "Project '{}' has {} repositories. Use --force to delete anyway.",
                project.name,
                repos.len()
            );
            eprintln!("Your local repos will NOT be deleted, only the lievo metadata.");
            return Err(lievo::LievoError::InvalidInput(
                "Please confirm with --force".to_string(),
            ));
        }
    }

    storage.delete_project(&project.id)?;
    println!("Deleted project '{}'", project.name);
    Ok(())
}

/// Print per-language extraction coverage metrics for a project's repos
/// (issue #679).
///
/// Coverage definition (adopted verbatim from CodeGraph's README): share of
/// symbol-bearing source files with ≥1 resolved cross-file dependent.
///
/// With `gate`, the command exits non-zero (via a `LievoError`) when any
/// language violates the fan-out or single-character-callee gate, so the CI
/// step `lievo admin coverage --project X --gate` fails the build. Without
/// `gate`, the command only reports and always exits 0 on successful runs —
/// JS/TS coverage may legitimately be 0% until import resolution (#681) is
/// complete, and the gate must tolerate that in reporting mode.
pub fn coverage(
    storage: &dyn Storage,
    project_name: Option<&str>,
    repo_path: Option<&std::path::Path>,
    gate: bool,
    fan_out_threshold: usize,
    fmt: OutputFormat,
) -> Result<()> {
    // With `repo_path` the command measures a pinned path directly (the CI
    // gate runs against tests/fixtures/sample_repo, which is not a registered
    // git repo). Otherwise it measures every repo of the given project (or
    // every project).
    if let Some(repo_path) = repo_path {
        let (rows, failures) = compute_coverage(repo_path, fan_out_threshold)?;
        let name = repo_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| repo_path.display().to_string());
        print_report(&name, &rows, &failures, gate, fmt)?;
        if gate && !failures.is_empty() {
            return Err(LievoError::InvalidInput(format!(
                "coverage gate failed for {} violation(s); run without --gate to inspect",
                failures.len()
            )));
        }
        return Ok(());
    }

    let projects = if let Some(name) = project_name {
        let p = storage
            .get_project(name)?
            .ok_or_else(|| LievoError::ProjectNotFound(name.to_string()))?;
        vec![p]
    } else {
        storage.list_projects()?
    };

    if projects.is_empty() {
        println!("No projects found. Run `lievo admin create-project <name>` to get started.");
        return Ok(());
    }

    for project in &projects {
        let repos = storage.list_repos(&project.id)?;
        let mut all_rows: Vec<lievo::analysis::coverage::LanguageCoverage> = Vec::new();
        let mut all_failures: Vec<lievo::analysis::coverage::GateFailure> = Vec::new();

        for repo in &repos {
            let (rows, failures) =
                compute_coverage(std::path::Path::new(&repo.local_path), fan_out_threshold)?;
            all_failures.extend(failures);
            all_rows.extend(rows);
        }

        if repos.is_empty() && fmt != OutputFormat::Json {
            println!("Project: {}", project.name);
            println!("  (no repositories)");
            continue;
        }
        print_report(&project.name, &all_rows, &all_failures, gate, fmt)?;
        if gate && !all_failures.is_empty() {
            return Err(LievoError::InvalidInput(format!(
                "coverage gate failed for {} violation(s); run without --gate to inspect",
                all_failures.len()
            )));
        }
    }
    Ok(())
}

/// Print one project's coverage report (Human or Json) and, in gate mode,
/// the gate verdict. The caller maps gate failures to a non-zero exit.
fn print_report(
    name: &str,
    rows: &[lievo::analysis::coverage::LanguageCoverage],
    failures: &[lievo::analysis::coverage::GateFailure],
    gate: bool,
    fmt: OutputFormat,
) -> Result<()> {
    if fmt == OutputFormat::Json {
        let mut parts = vec![format!("\"project\":\"{}\"", super::json_escape(name))];
        let rows_json = rows
            .iter()
            .map(format_coverage_row_json)
            .collect::<Vec<_>>()
            .join(",");
        parts.push(format!("\"languages\":[{}]", rows_json));
        let failures_json = failures
            .iter()
            .map(|f| {
                format!(
                    "{{\"language\":\"{}\",\"reason\":\"{}\"}}",
                    super::json_escape(&f.language),
                    super::json_escape(&f.reason)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        parts.push(format!("\"gates\":[{}]", failures_json));
        parts.push(format!("\"gate_failed\":{}", !failures.is_empty()));
        println!("{{{}}}", parts.join(","));
        return Ok(());
    }

    println!("Project: {}", name);
    if rows.is_empty() {
        println!("  (no code units — run `lievo refresh` to index)");
        return Ok(());
    }
    println!(
        "  {:<14} {:>10} {:>10} {:>14} {:>12}",
        "Language", "Cov %", "Files", "Entities", "Max fan-out"
    );
    for row in rows {
        let cov_pct = if row.symbol_files > 0 {
            100.0 * row.files_with_dependents as f64 / row.symbol_files as f64
        } else {
            0.0
        };
        println!(
            "  {:<14} {:>8.1}% {:>6}/{} {:>14} {:>12}",
            row.language,
            cov_pct,
            row.files_with_dependents,
            row.symbol_files,
            row.entities,
            row.max_fan_out
        );
        if !row.single_char_callees.is_empty() {
            println!(
                "  {:<14} top single-char callees: {}",
                row.language,
                row.single_char_callees.join(", ")
            );
        }
    }
    if gate {
        if failures.is_empty() {
            println!("  Gate: PASS");
        } else {
            for f in failures {
                println!("  Gate: FAIL — {}", f.reason);
            }
        }
    }
    Ok(())
}

fn format_coverage_row_json(row: &lievo::analysis::coverage::LanguageCoverage) -> String {
    let cov_pct = if row.symbol_files > 0 {
        100.0 * row.files_with_dependents as f64 / row.symbol_files as f64
    } else {
        0.0
    };
    let callees = row
        .single_char_callees
        .iter()
        .map(|c| format!("\"{}\"", super::json_escape(c)))
        .collect::<Vec<_>>()
        .join(",");
    let fan_out_name = match &row.max_fan_out_name {
        Some(n) => format!("\"{}\"", super::json_escape(n)),
        None => "null".to_string(),
    };
    format!(
        "{{\"language\":\"{}\",\"coverage_pct\":{:.1},\"files_with_dependents\":{},\"symbol_files\":{},\"entities\":{},\"max_fan_out\":{},\"max_fan_out_name\":{},\"single_char_callees\":[{}]}}",
        super::json_escape(&row.language),
        cov_pct,
        row.files_with_dependents,
        row.symbol_files,
        row.entities,
        row.max_fan_out,
        fan_out_name,
        callees
    )
}

/// Compute coverage rows + gate failures by re-running extraction over a
/// repository path (no storage read — the coverage metric needs code units
/// and resolved import edges, which live in the extractor and
/// RelationshipBuilder, not in the persisted graph).
pub fn compute_coverage(
    repo_path: &std::path::Path,
    fan_out_threshold: usize,
) -> Result<(
    Vec<lievo::analysis::coverage::LanguageCoverage>,
    Vec<lievo::analysis::coverage::GateFailure>,
)> {
    let mut extractor =
        lievo::extraction::tree_sitter_extractor::TreeSitterExtractor::new(repo_path, true)?;
    extractor.index(false)?;
    let code_units = extractor.read_all_units()?;

    if code_units.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let grouping_config = lievo::extraction::grouping::GroupingConfig {
        code_units: &code_units,
        scanned_file_paths: extractor.extracted_files(),
        project_id: "coverage",
        repo_name: "coverage",
        repo_id: "coverage",
        repo_path,
        config: None,
        exclude_paths: &[],
    };
    let grouping = lievo::extraction::grouping::group_code_units(&grouping_config)?;

    // Resolve file→file import edges (cross-file dependency = coverage numerator).
    let (relationships, _unresolved) = lievo::analysis::relationships::RelationshipBuilder::build(
        &code_units,
        &grouping,
        "coverage",
        "coverage",
        repo_path,
    )?;
    let resolved_files: Vec<(String, String)> = relationships
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .map(|r| (r.source_id.clone(), r.target_id.clone()))
        .collect();

    // File→file edges reference file entity IDs, not paths — translate via
    // the grouping file map so coverage_by_language can join on paths.
    let id_to_path: HashMap<String, String> = grouping
        .files
        .iter()
        .filter_map(|f| f.path.as_ref().map(|p| (f.id.clone(), p.clone())))
        .collect();
    let resolved_by_path: Vec<(String, String)> = resolved_files
        .iter()
        .filter_map(|(s, t)| Some((id_to_path.get(s)?.clone(), id_to_path.get(t)?.clone())))
        .collect();

    let rows = lievo::analysis::coverage::coverage_by_language(&code_units, &resolved_by_path);
    let failures = lievo::analysis::coverage::gate_failures(&rows, fan_out_threshold);
    Ok((rows, failures))
}
