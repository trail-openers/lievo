//! File-size CI gate (issue #702, PM policy revision 2026-09-11).
//!
//! Counts physical `wc -l` lines (blanks and comments included — no
//! `#[cfg(test)]` exclusion arithmetic) and enforces per-category budgets:
//! source (`.rs` under `src/`) 500, dedicated test files 800, config files
//! (`Cargo.toml`, `Cargo.lock`, `.github/workflows/*.yml`, `scripts/*.rs`)
//! 200.
//!
//! Grandfathering: files already over their budget at the branch point are
//! listed in the checked-in baseline manifest (`scripts/file_size_baseline.txt`,
//! `path=lines` per line, `#` comments allowed). A grandfathered file may grow
//! at most 10% above its baseline (integer ceiling); the gate fails when a
//! non-grandfathered file exceeds its budget or a grandfathered file exceeds
//! its cap. A grandfathered file shrunk to its budget or below no longer
//! breaches — removing it from the manifest is a follow-up chore.
//!
//! Self-exemption: this gate's own source exceeds the 200-line config budget,
//! so the baseline manifest lists it — the entry is load-bearing for the cap,
//! not a style exception.
//!
//! Standard library only. Run from the crate root:
//!   cargo run --example check_file_size            (gate; exit 0 = compliant)
//! Self-tests:
//!   rustc --edition 2021 --test scripts/check_file_size.rs
//! `--root <dir>` points the gate at a scratch crate root (used by self-tests).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::process::ExitCode;

const SOURCE_BUDGET: usize = 500;
const TEST_BUDGET: usize = 800;
const CONFIG_BUDGET: usize = 200;
const GROWTH_CAP_NUMERATOR: usize = 11;
const GROWTH_CAP_DENOMINATOR: usize = 10;

#[cfg(not(test))]
fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let root = match args.next().as_deref() {
        Some("--root") => {
            let dir = args
                .next()
                .unwrap_or_else(|| panic!("check_file_size: --root requires a directory"));
            PathBuf::from(dir)
        }
        _ => crate_root(),
    };
    let report = check(&root).expect("file-size check should succeed");
    if report.breaches.is_empty() {
        println!("{}", format_report(&report));
        ExitCode::SUCCESS
    } else {
        eprintln!("{}", format_report(&report));
        ExitCode::from(1)
    }
}

/// Crate root: walk up from the current directory to the nearest Cargo.toml
/// so the gate works from the repo root or any subdirectory. Canonicalised
/// via canonicalize() so path-prefix stripping matches the file paths that
/// `fs::read_dir` returns (which are resolved against the real filesystem).
fn crate_root() -> PathBuf {
    let start = env::current_dir().expect("current dir");
    let canonical = start
        .canonicalize()
        .unwrap_or_else(|_| panic!("check_file_size: cannot canonicalize cwd {:?}", start));
    let mut dir = canonical;
    loop {
        if dir.join("Cargo.toml").is_file() {
            return dir;
        }
        if !dir.pop() {
            panic!("check_file_size: run from within the lievo crate (no Cargo.toml found above)");
        }
    }
}

#[derive(Debug)]
struct Breach {
    rel: String,
    lines: usize,
    limit: usize,
    kind: &'static str,
}

struct Report {
    /// Total files checked (drives the OK message count).
    file_count: usize,
    breaches: Vec<Breach>,
}

/// Baseline manifest: `path=lines` per line, `#` comments and blanks
/// skipped. The file may be absent (empty manifest).
fn read_baseline(root: &Path) -> Result<Vec<(String, usize)>, String> {
    let path = root.join("scripts").join("file_size_baseline.txt");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let src = fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((rel, digits)) = line.split_once('=') else {
            return Err(format!("malformed baseline line: {line:?}"));
        };
        let Ok(lines) = digits.trim().parse::<usize>() else {
            return Err(format!("malformed baseline line: {line:?}"));
        };
        out.push((rel.trim().to_string(), lines));
    }
    Ok(out)
}

/// A file is a dedicated test file when any of the rule matches: a `tests/`
/// directory component, a `*_tests.rs` / `*_tests_fixtures.rs` name, or a
/// `*_tests` parent directory.
fn is_test_file(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let base = parts.last().copied().unwrap_or("");
    let stem = base.trim_end_matches(".rs");
    let base_is_test =
        stem == "tests" || stem.ends_with("_tests") || stem.ends_with("_tests_fixtures");
    let dir_is_test = parts
        .iter()
        .rev()
        .any(|p| *p == "tests" || p.ends_with("_tests"));
    base_is_test || dir_is_test
}

/// Physical line count (`wc -l` semantics): 1 per `\n`, plus 1 when the file
/// ends without a trailing newline.
fn count_lines(src: &str) -> usize {
    let nl = src.bytes().filter(|b| *b == b'\n').count();
    if nl > 0 && !src.ends_with('\n') {
        nl + 1
    } else {
        nl
    }
}

/// Grandfathering growth cap: `ceil(baseline × 1.1)`, floored at the source
/// budget so a small grandfathered file can still grow to its category budget.
fn growth_cap(baseline: usize) -> usize {
    let grown = baseline
        .saturating_mul(GROWTH_CAP_NUMERATOR)
        .div_ceil(GROWTH_CAP_DENOMINATOR);
    grown.max(SOURCE_BUDGET)
}

fn check(root: &Path) -> Result<Report, String> {
    let baseline = read_baseline(root)?;
    let mut entries: Vec<(String, usize, usize, bool)> = Vec::new();
    for (rel, path) in collect_files(root, &root.join("src")) {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let test = is_test_file(&rel);
        let limit = if test { TEST_BUDGET } else { SOURCE_BUDGET };
        entries.push((rel, count_lines(&read(&path)?), limit, test));
    }
    for (rel, path) in collect_files(root, root) {
        let is_config = rel == "Cargo.toml"
            || rel == "Cargo.lock"
            || (rel.starts_with("scripts/") && rel.ends_with(".rs"))
            || (rel.starts_with(".github/workflows/")
                && (rel.ends_with(".yml") || rel.ends_with(".yaml")));
        if is_config {
            entries.push((rel, count_lines(&read(&path)?), CONFIG_BUDGET, false));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut breaches = Vec::new();
    let mut kept_count: usize = 0;
    for (rel, lines, limit, test) in entries {
        let Some((_, base)) = baseline.iter().find(|(r, _)| r == &rel) else {
            let breached = lines > limit;
            if breached {
                breaches.push(Breach {
                    rel: rel.clone(),
                    lines,
                    limit,
                    kind: category(&rel, test),
                });
            }
            kept_count += 1;
            continue;
        };
        let allowed = growth_cap(*base);
        if lines > allowed {
            breaches.push(Breach {
                rel: rel.clone(),
                lines,
                limit: allowed,
                kind: category(&rel, test),
            });
        }
        kept_count += 1;
    }
    Ok(Report {
        file_count: kept_count,
        breaches,
    })
}

/// Recursively collect every file under `dir` as `(rel, path)` pairs,
/// sorted by path. A missing directory yields an empty list. Returns the
/// crate-relative path (`root` stripped) as the key.
fn collect_files(root: &Path, dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_files(root, &path));
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
                .unwrap_or_default();
            out.push((rel, path));
        }
    }
    out.sort();
    out
}

fn category(rel: &str, test: bool) -> &'static str {
    if test {
        "test file"
    } else if rel.starts_with("src/") {
        "source file"
    } else {
        "config file"
    }
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))
}

fn format_report(report: &Report) -> String {
    if report.breaches.is_empty() {
        return format!(
            "check_file_size: OK — {} files checked (src budgets {SOURCE_BUDGET}/{TEST_BUDGET}; \
             config {CONFIG_BUDGET}); grandfathered cap = baseline × 1.1.",
            report.file_count
        );
    }
    let mut out = format!(
        "check_file_size: FAILED — {} file(s) over budget (src {SOURCE_BUDGET}, test {TEST_BUDGET}, \
         config {CONFIG_BUDGET}; grandfathered cap = baseline × 1.1):\n",
        report.breaches.len()
    );
    for b in &report.breaches {
        out.push_str(&format!(
            "  {0} — {1} lines (limit {2}, {3})\n    fix: split the file, or (if over the grandfathered \
             cap) trim it back toward its baseline.\n",
            b.rel, b.lines, b.limit, b.kind
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_root(tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = env::temp_dir().join(format!("check_file_size_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for (path, content) in files {
            let p = dir.join(path);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(p, content).unwrap();
        }
        dir
    }

    fn with_root(
        tag: &str,
        files: &[(&str, &str)],
        f: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        let dir = tmp_root(tag, files);
        let result = f(&dir);
        let _ = fs::remove_dir_all(&dir);
        result
    }

    // ── counting (wc -l semantics) ──────────────────────────────────────

    #[test]
    fn count_lines_matches_wc_l_for_trailing_newline() {
        assert_eq!(count_lines("fn a() {}\nfn b() {}\n"), 2);
    }

    #[test]
    fn count_lines_counts_final_unterminated_line() {
        assert_eq!(count_lines("fn a() {}\nfn b() {}"), 2);
    }

    #[test]
    fn count_lines_empty_file_is_zero() {
        assert_eq!(count_lines(""), 0);
    }

    #[test]
    fn blanks_and_comments_count() {
        let src = "// comment\n\nfn a() {}\n";
        assert_eq!(count_lines(src), 3);
    }

    // ── budgets ─────────────────────────────────────────────────────────

    #[test]
    fn test_file_detection_covers_name_and_dir() {
        assert!(is_test_file("src/retrieval/hybrid_tests.rs"));
        assert!(is_test_file("src/retrieval/hybrid_tests_fixtures.rs"));
        assert!(is_test_file("src/extraction/grouping/tests.rs"));
        assert!(is_test_file(
            "src/analysis/convention_detector_tests/mod.rs"
        ));
        assert!(!is_test_file("src/refresh.rs"));
        assert!(!is_test_file("scripts/check_file_size.rs"));
    }

    #[test]
    fn source_at_500_passes_at_501_fails() {
        let at: String = (0..500).map(|i| format!("fn f{i}() {{}}\n")).collect();
        let over: String = (0..501).map(|i| format!("fn f{i}() {{}}\n")).collect();
        with_root(
            "src_at",
            &[("src/lib.rs", "mod a;\n"), ("src/a.rs", &at)],
            |root| {
                assert!(check(root).unwrap().breaches.is_empty());
                Ok(())
            },
        )
        .unwrap();
        with_root(
            "src_over",
            &[("src/lib.rs", "mod a;\n"), ("src/a.rs", &over)],
            |root| {
                let r = check(root).unwrap();
                assert_eq!(r.breaches.len(), 1);
                assert_eq!(r.breaches[0].rel, "src/a.rs");
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn test_file_at_800_passes_at_801_fails() {
        let at: String = (0..800).map(|i| format!("fn t{i}() {{}}\n")).collect();
        let over: String = (0..801).map(|i| format!("fn t{i}() {{}}\n")).collect();
        with_root(
            "test_at",
            &[("src/lib.rs", "mod a;\n"), ("src/a_tests.rs", &at)],
            |root| {
                assert!(check(root).unwrap().breaches.is_empty());
                Ok(())
            },
        )
        .unwrap();
        with_root(
            "test_over",
            &[("src/lib.rs", "mod a;\n"), ("src/a_tests.rs", &over)],
            |root| {
                let r = check(root).unwrap();
                assert_eq!(r.breaches.len(), 1);
                assert_eq!(r.breaches[0].limit, TEST_BUDGET);
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn config_file_over_200_fails() {
        let big: String = (0..201).map(|i| format!("x{i}\n")).collect();
        with_root("cfg_over", &[("Cargo.toml", "[package]\n")], |root| {
            fs::write(root.join("Cargo.toml"), &big).unwrap();
            let r = check(root).unwrap();
            assert!(
                r.breaches
                    .iter()
                    .any(|b| b.rel == "Cargo.toml" && b.limit == CONFIG_BUDGET)
            );
            Ok(())
        })
        .unwrap();
    }

    // ── grandfathering ──────────────────────────────────────────────────

    #[test]
    fn growth_cap_math() {
        // Baselines below ~455 are floored at the source budget (500).
        assert_eq!(growth_cap(0), 500);
        assert_eq!(growth_cap(10), 500);
        assert_eq!(growth_cap(100), 500); // 110 < 500 → floor applies
        // Baselines above ~455: the 1.1× cap kicks in.
        assert_eq!(growth_cap(500), 550);
        assert_eq!(growth_cap(1205), 1326); // ceil(1325.5)
        assert_eq!(growth_cap(1338), 1472); // ceil(1471.8)
    }

    #[test]
    fn grandfathered_file_within_10_percent_passes() {
        // Baseline 600 → cap ceil(660) = 660; 660 passes, 661 fails.
        let at_cap: String = (0..660).map(|i| format!("fn f{i}() {{}}\n")).collect();
        let over: String = (0..661).map(|i| format!("fn f{i}() {{}}\n")).collect();
        with_root(
            "gf_at",
            &[
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", &at_cap),
                ("scripts/file_size_baseline.txt", "src/a.rs=600\n"),
            ],
            |root| {
                assert!(
                    check(root).unwrap().breaches.is_empty(),
                    "660 lines at baseline 600 must pass"
                );
                Ok(())
            },
        )
        .unwrap();
        with_root(
            "gf_over",
            &[
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", &over),
                ("scripts/file_size_baseline.txt", "src/a.rs=600\n"),
            ],
            |root| {
                let r = check(root).unwrap();
                assert_eq!(r.breaches.len(), 1);
                assert_eq!(r.breaches[0].limit, 660);
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn grandfather_cap_floors_at_source_budget() {
        // A grandfathered small file (baseline 10) may still grow to its
        // category budget (500) — the cap is max(budget, 1.1×baseline).
        let at: String = (0..500).map(|i| format!("fn f{i}() {{}}\n")).collect();
        with_root(
            "gf_floor",
            &[
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", &at),
                ("scripts/file_size_baseline.txt", "src/a.rs=10\n"),
            ],
            |root| {
                assert!(
                    check(root).unwrap().breaches.is_empty(),
                    "500 lines at baseline 10 must pass (cap floors at 500)"
                );
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn shrunk_grandfathered_file_below_budget_passes() {
        let small: String = (0..100).map(|i| format!("fn f{i}() {{}}\n")).collect();
        with_root(
            "gf_shrunk",
            &[
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", &small),
                ("scripts/file_size_baseline.txt", "src/a.rs=1205\n"),
            ],
            |root| {
                assert!(
                    check(root).unwrap().breaches.is_empty(),
                    "100 lines (well under baseline 1205) must pass"
                );
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn non_grandfathered_file_over_budget_fails_even_if_baseline_has_other_files() {
        let big: String = (0..600).map(|i| format!("fn f{i}() {{}}\n")).collect();
        with_root(
            "ngf_fail",
            &[
                ("src/lib.rs", "mod a;\n"),
                ("src/a.rs", &big),
                ("src/b.rs", "fn b() {}\n"),
                ("scripts/file_size_baseline.txt", "src/b.rs=400\n"),
            ],
            |root| {
                let r = check(root).unwrap();
                assert_eq!(r.breaches.len(), 1);
                assert_eq!(r.breaches[0].rel, "src/a.rs");
                assert_eq!(r.breaches[0].limit, SOURCE_BUDGET);
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn baseline_file_absent_means_empty_manifest() {
        with_root(
            "no_manifest",
            &[("src/lib.rs", "mod a;\n"), ("src/a.rs", "fn a() {}\n")],
            |root| {
                assert!(read_baseline(root).unwrap().is_empty());
                assert!(check(root).unwrap().breaches.is_empty());
                Ok(())
            },
        )
        .unwrap();
    }

    #[test]
    fn malformed_baseline_line_is_an_error() {
        with_root("bad_manifest", &[("src/lib.rs", "mod a;\n")], |root| {
            fs::create_dir_all(root.join("scripts")).unwrap();
            fs::write(
                root.join("scripts/file_size_baseline.txt"),
                "not-a-baseline-line\n",
            )
            .unwrap();
            let err = read_baseline(root).unwrap_err();
            assert!(err.contains("malformed"), "got: {err}");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn report_names_each_breach_with_count_and_limit() {
        let big: String = (0..600).map(|i| format!("fn f{i}() {{}}\n")).collect();
        with_root(
            "report_breach",
            &[("src/lib.rs", "mod a;\n"), ("src/a.rs", &big)],
            |root| {
                let report = check(root).unwrap();
                let text = format_report(&report);
                assert!(text.contains("src/a.rs"), "breach path must be listed");
                assert!(text.contains("600 lines"));
                assert!(text.contains("limit 500"));
                Ok(())
            },
        )
        .unwrap();
    }
}
