// Impact report formatting.

use crate::model::Entity;
use crate::query::dependency::{ImpactReport, ResolutionSignal};

/// Format an impact report as human-readable text sections.
///
/// When `signal.caveat_active()` is true (unresolved *internal* imports are
/// recorded and non-zero), appends an explicit caveat line distinguishing
/// "0 downstream dependents" from "unresolved imports in the repo" so an
/// agent (or human) does not misread the absence of dependents as the
/// dead-code signal. Pre-#681 (`None` fields) and external-only counts
/// never fire the caveat.
pub fn format_impact_human(report: &ImpactReport, signal: &ResolutionSignal) -> String {
    let mut lines = Vec::new();
    lines.push("Impact Analysis".to_string());
    lines.push("━".repeat(40));

    lines.push(format!("Changed files ({}):", report.changed_files.len()));
    for e in &report.changed_files {
        lines.push(format!("  {} ({})", e.name, e.id));
    }

    lines.push(format!(
        "Affected modules ({}):",
        report.affected_modules.len()
    ));
    for e in &report.affected_modules {
        lines.push(format!("  {} ({})", e.name, e.id));
    }

    lines.push(format!(
        "Affected subsystems ({}):",
        report.affected_subsystems.len()
    ));
    for e in &report.affected_subsystems {
        lines.push(format!("  {} ({})", e.name, e.id));
    }

    lines.push(format!(
        "Downstream dependents ({}):",
        report.downstream_dependents.len()
    ));
    for e in &report.downstream_dependents {
        lines.push(format!("  {} ({})", e.name, e.id));
    }

    lines.push(format!(
        "Affected callers ({}):",
        report.affected_functions.len()
    ));
    for e in &report.affected_functions {
        lines.push(format!("  {} ({})", e.name, e.id));
    }

    if signal.caveat_active() {
        let internal = signal.unresolved_internal.unwrap_or(0);
        let external = signal.unresolved_external.unwrap_or(0);
        lines.push(String::new());
        lines.push(format!(
            "warning: {} internal import(s) in this repo could not be resolved ({} external).",
            internal, external
        ));
        lines.push(
            "absence of dependents below may reflect unresolved imports, not dead code."
                .to_string(),
        );
    }

    lines.join("\n")
}

/// Format an impact report as NDJSON with one section per line written to `writer`.
/// Writes each section line immediately — no full-collection buffering.
///
/// The five section lines carry the labels `changed_files`, `affected_modules`,
/// `affected_subsystems`, `downstream_dependents`, `affected_callers`. A final
/// metadata line is appended with `section` = `"resolution"` and the
/// unresolved-import counters. `None` fields are emitted as JSON `null`
/// (pre-#681 degradation). Existing consumers that only read the first five
/// section lines are unaffected (additive).
pub fn format_impact_json(
    report: &ImpactReport,
    signal: &ResolutionSignal,
    writer: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    let sections: &[(&str, &[Entity])] = &[
        ("changed_files", &report.changed_files),
        ("affected_modules", &report.affected_modules),
        ("affected_subsystems", &report.affected_subsystems),
        ("downstream_dependents", &report.downstream_dependents),
        ("affected_callers", &report.affected_functions),
    ];

    for (label, entities) in sections {
        writeln!(
            writer,
            "{}",
            serde_json::json!({
                "section": label,
                "entities": entities.iter().map(|e| serde_json::json!({
                    "id": e.id,
                    "name": e.name,
                    "tier": e.tier.to_string(),
                })).collect::<Vec<_>>(),
            })
        )?;
    }

    let internal_val = signal
        .unresolved_internal
        .map(|n| serde_json::json!(n))
        .unwrap_or(serde_json::Value::Null);
    let external_val = signal
        .unresolved_external
        .map(|n| serde_json::json!(n))
        .unwrap_or(serde_json::Value::Null);
    writeln!(
        writer,
        "{}",
        serde_json::json!({
            "section": "resolution",
            "unresolved_internal": internal_val,
            "unresolved_external": external_val,
        })
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EntityTier;

    fn make_entity(id: &str, name: &str, tier: EntityTier) -> Entity {
        Entity {
            id: id.to_string(),
            project_id: "proj".to_string(),
            repo_id: None,
            tier,
            parent_id: None,
            name: name.to_string(),
            path: None,
            language: None,
            summary: None,
            summary_commit: None,
            metrics_json: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn make_impact_report() -> ImpactReport {
        ImpactReport {
            changed_files: vec![make_entity("f1", "main.rs", EntityTier::File)],
            affected_modules: vec![make_entity("m1", "core", EntityTier::Module)],
            affected_subsystems: vec![make_entity("s1", "api", EntityTier::Subsystem)],
            downstream_dependents: vec![],
            affected_functions: vec![],
        }
    }

    #[test]
    fn test_format_impact_human_has_sections() {
        let report = make_impact_report();
        let signal = ResolutionSignal::from_counts(None);
        let result = format_impact_human(&report, &signal);
        assert!(result.contains("Impact Analysis"));
        assert!(result.contains("Changed files (1)"));
        assert!(result.contains("Affected modules (1)"));
        assert!(result.contains("Affected subsystems (1)"));
        assert!(result.contains("Downstream dependents (0)"));
        assert!(result.contains("Affected callers (0)"));
    }

    #[test]
    fn test_format_impact_human_shows_entity_names() {
        let report = make_impact_report();
        let signal = ResolutionSignal::from_counts(None);
        let result = format_impact_human(&report, &signal);
        assert!(result.contains("main.rs"));
        assert!(result.contains("core"));
        assert!(result.contains("api"));
    }

    fn write_json(report: &ImpactReport) -> String {
        let signal = ResolutionSignal::from_counts(None);
        let mut buf = Vec::new();
        format_impact_json(report, &signal, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    fn write_json_with_signal(report: &ImpactReport, signal: &ResolutionSignal) -> String {
        let mut buf = Vec::new();
        format_impact_json(report, signal, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn test_format_impact_json_six_lines_with_resolution() {
        let report = make_impact_report();
        let result = write_json(&report);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(
            lines.len(),
            6,
            "expected 5 section lines + 1 resolution line"
        );
        for line in &lines {
            serde_json::from_str::<serde_json::Value>(line).expect("each line must be valid JSON");
        }
    }

    #[test]
    fn test_format_impact_json_sections_have_correct_labels() {
        let report = make_impact_report();
        let result = write_json(&report);
        let lines: Vec<&str> = result.trim().lines().collect();
        let sections: Vec<String> = lines
            .iter()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["section"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(
            sections,
            vec![
                "changed_files",
                "affected_modules",
                "affected_subsystems",
                "downstream_dependents",
                "affected_callers",
                "resolution"
            ]
        );
    }

    #[test]
    fn test_format_impact_json_entities_included() {
        let report = make_impact_report();
        let result = write_json(&report);
        let first_line = result.trim().lines().next().unwrap();
        let v: serde_json::Value = serde_json::from_str(first_line).unwrap();
        let entities = v["entities"].as_array().unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0]["name"], "main.rs");
    }

    #[test]
    fn test_format_impact_json_empty_report() {
        let empty_report = ImpactReport {
            changed_files: vec![],
            affected_modules: vec![],
            affected_subsystems: vec![],
            downstream_dependents: vec![],
            affected_functions: vec![],
        };
        let result = write_json(&empty_report);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(
            lines.len(),
            6,
            "should always output 5 section lines + 1 resolution line"
        );
        for line in &lines {
            serde_json::from_str::<serde_json::Value>(line).expect("each line must be valid JSON");
        }
    }

    // --- #690: unresolved-imports caveat + resolution JSON line ---

    fn signal_with(internal: Option<u64>, external: Option<u64>) -> ResolutionSignal {
        ResolutionSignal {
            unresolved_internal: internal,
            unresolved_external: external,
        }
    }

    #[test]
    fn test_format_impact_human_caveat_when_unresolved_internal_positive() {
        let report = make_impact_report();
        let signal = signal_with(Some(3), Some(7));
        let result = format_impact_human(&report, &signal);
        assert!(
            result.contains("3 internal import(s)"),
            "should mention the unresolved_internal count"
        );
        assert!(
            result.contains("7 external"),
            "should mention the unresolved_external count"
        );
        assert!(
            result.contains("may reflect unresolved imports, not dead code"),
            "should include the caveat"
        );
    }

    #[test]
    fn test_format_impact_human_no_caveat_when_signal_none() {
        let report = make_impact_report();
        let signal = ResolutionSignal::from_counts(None);
        let result = format_impact_human(&report, &signal);
        assert!(
            !result.contains("unresolved imports"),
            "no caveat when signal is pre-#681 (None fields)"
        );
    }

    #[test]
    fn test_format_impact_human_no_caveat_when_zero_internal() {
        let report = make_impact_report();
        let signal = signal_with(Some(0), Some(5));
        let result = format_impact_human(&report, &signal);
        assert!(
            !result.contains("unresolved imports"),
            "no caveat when unresolved_internal is 0 (external only)"
        );
    }

    #[test]
    fn test_format_impact_json_resolution_line_present_with_signal() {
        let report = make_impact_report();
        let signal = signal_with(Some(2), Some(1));
        let result = write_json_with_signal(&report, &signal);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(lines.len(), 6, "5 section lines + 1 resolution line");
        let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert_eq!(last["section"], "resolution");
        assert_eq!(last["unresolved_internal"], 2);
        assert_eq!(last["unresolved_external"], 1);
    }

    #[test]
    fn test_format_impact_json_resolution_line_null_when_pre681() {
        let report = make_impact_report();
        let signal = ResolutionSignal::from_counts(None);
        let result = write_json_with_signal(&report, &signal);
        let lines: Vec<&str> = result.trim().lines().collect();
        assert_eq!(lines.len(), 6, "5 section lines + 1 resolution line");
        let last: serde_json::Value = serde_json::from_str(lines.last().unwrap()).unwrap();
        assert_eq!(last["section"], "resolution");
        assert!(
            last["unresolved_internal"].is_null(),
            "pre-#681: unresolved_internal must be null"
        );
        assert!(
            last["unresolved_external"].is_null(),
            "pre-#681: unresolved_external must be null"
        );
    }

    #[test]
    fn test_format_impact_json_existing_five_lines_unchanged_with_signal() {
        let report = make_impact_report();
        let signal = signal_with(Some(1), Some(0));
        let result = write_json_with_signal(&report, &signal);
        let lines: Vec<&str> = result.trim().lines().collect();
        let sections: Vec<String> = lines
            .iter()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["section"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(
            sections,
            vec![
                "changed_files",
                "affected_modules",
                "affected_subsystems",
                "downstream_dependents",
                "affected_callers",
                "resolution"
            ]
        );
    }
}
