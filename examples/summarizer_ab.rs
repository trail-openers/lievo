// Summarizer A/B harness (issue #773).
//
// Measures candidate summarizer backends on lievo's own source: parse-success
// rate across a batch-size sweep, throughput (cold vs steady-state, wall-clock
// per entity), and writes a durable JSON artifact recording the chip, date,
// model revisions and lievo commit.
//
// Run manually (never run in CI):
//
//   cargo run --example summarizer_ab -- --backend apfel \
//       --backend http://127.0.0.1:8080 --http-backend llama-server \
//       --model http://127.0.0.1:8080=Qwen3-4B-Q4 \
//       --batch-sizes 2,4,8,16 --artifact tmp/summarizer_ab_results.json
//
// The candidate set is configured entirely on the command line — no model
// name is hardcoded in this example. `--backend apfel` (the incumbent
// baseline, one-shot `apfel` subprocess, macOS-only) is required: without it
// there is no comparison. HTTP candidates are any OpenAI-compatible endpoint
// (apfel --serve, llama-server, ...) validated by the production health probe.
//
// License filter: a non-redistributable candidate is excluded BEFORE any
// measurement runs (the check runs first; the reason is recorded in the
// artifact and the backend is never contacted).
//
// Faithfulness: NOT scored automatically — an automatic reference-free metric
// does not exist, and similarity against another model's output would merely
// re-measure agreement with that model (the exact flaw in the proposed
// model's benchmark card). The artifact records the stated method and its
// limitations verbatim (FAITHFULNESS_METHOD) and carries per-entity summaries
// for the human pass.
//
// Throughput: the first timed call pays cold start (process spawn + model load
// for the one-shot backend, connection setup for HTTP); it is reported
// separately, and steady-state is the per-entity cost of all later calls —
// the operator's stated preference, since issues #771/#772 remove the fixed
// cost. A missing model or unreachable backend is reported as unavailable,
// never a run failure. No model is ever downloaded by this harness.
//
// This is a dev-only example: it is not a dependency of the library or the
// MCP server, and it does not touch the no-summarizer path.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::Instant;

use lievo::config::SummarizerBackend;
use lievo::summarization::summarizer_ab::sample::{SAMPLE, SampleEntity, validate_sample};
use lievo::summarization::summarizer_ab::scoring::{
    LicenseStatus, build_parse_success_report, check_license,
};
use lievo::summarization::summarizer_backend::{health_validates_for, send_chat_completions_for};
use serde::Serialize;

/// The production batch template (mirrors `batch_summarize` in apfel.rs —
/// the harness builds its own batches independently of `pack_by_char_budget`).
const BATCH_SYSTEM_PROMPT: &str = "You are a concise Rust code summarizer.";
const BATCH_USER_PROMPT: &str =
    "Summarize each function below. Output one line per function: #<n>: <summary>";

/// One-shot apfel subprocess timeout (matches APFEL_TIMEOUT_SECS in apfel.rs).
const APFEL_TIMEOUT_SECS: u64 = 300;

/// Faithfulness method recorded verbatim in the artifact (issue #773: a
/// stated, documented method whose limitations are acknowledged in the
/// output).
const FAITHFULNESS_METHOD: &str = "Manual review: a human reads each entity's summary in this \
artifact against the entity body and judges whether it describes the entity \
(correct name, purpose, no fabrication). Automatic reference-free scoring is \
impossible: similarity against another model's output would merely measure \
agreement with that model (the circularity that invalidated the proposed \
model's own benchmark card). The human judgment is subjective and unrepeatable; \
it should sample at least the batch sizes with the lowest parse-success. No \
faithfulness number is recorded in this artifact.";

/// The durable artifact (issue #773: records the chip, the date, the model
/// revisions and the lievo commit).
#[derive(Debug, Serialize)]
struct Artifact {
    schema: u32,
    date: String,
    chip: String,
    lievo_commit: String,
    batch_sizes: Vec<usize>,
    faithfulness_method: &'static str,
    candidates: Vec<CandidateResult>,
}

/// Per-candidate harness result, serialised into the artifact.
#[derive(Debug, Serialize)]
struct CandidateResult {
    id: String,
    backend: String,
    license: LicenseStatus,
    /// Model revision (CLI `--model`), if any.
    model: Option<String>,
    available: bool,
    unavailable_reason: Option<String>,
    cold_start_ms_per_entity: Option<f64>,
    steady_state_ms_per_entity: Option<f64>,
    parse_success: Option<ParseSuccessBreakdown>,
    /// Per-entity summaries from the smallest batch, for the human
    /// faithfulness pass.
    summaries: Vec<EntitySummary>,
}

#[derive(Debug, Serialize)]
struct ParseSuccessBreakdown {
    model: String,
    entries: Vec<ParseSuccessEntry>,
    overall: f64,
}

#[derive(Debug, Serialize)]
struct ParseSuccessEntry {
    batch_size: usize,
    percentage: f64,
    successes: usize,
    total: usize,
}

#[derive(Debug, Serialize)]
struct EntitySummary {
    id: &'static str,
    tier: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

/// A configured candidate (the candidate set is CLI-configured, no model
/// name hardcoded).
struct Candidate {
    id: String,
    backend: SummarizerBackend,
    base_url: Option<String>,
    model: Option<String>,
    license: String,
}

impl Candidate {
    /// Label for the artifact: model revision if known, else the id.
    fn model_label(&self) -> String {
        self.model
            .as_deref()
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.id.clone())
    }
}

/// CLI config for the harness run.
struct HarnessConfig {
    batch_sizes: Vec<usize>,
    artifact_path: PathBuf,
    candidates: Vec<Candidate>,
}

/// Parse `#<n>: <summary>` lines into a positional Vec<Option<String>> — the
/// same contract as `parse_batch_response` in apfel.rs (kept local: the
/// harness must not touch or extend the production parser).
fn parse_batch(content: &str, n: usize) -> Vec<Option<String>> {
    let mut results = vec![None; n];
    for line in content.lines() {
        if let Some(hash_pos) = line.find('#') {
            let after_hash = &line[hash_pos + 1..];
            if let Some(colon_pos) = after_hash.find(':') {
                let index_str = &after_hash[..colon_pos].trim();
                if let Ok(index) = index_str.parse::<usize>()
                    && index < n
                {
                    results[index] = Some(after_hash[colon_pos + 1..].trim().to_string());
                }
            }
        }
    }
    results
}

/// Format one batch of sample entities with the production template.
fn format_batch(entities: &[&SampleEntity]) -> String {
    let batch_input = entities
        .iter()
        .enumerate()
        .map(|(i, e)| format!("--- Function: #{}\n{}\n---\n", i, e.body))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{BATCH_USER_PROMPT}\n\n{batch_input}")
}

/// Slice the sample into batches of size `n`, repeating entities to fill the
/// last (partial) batch so every batch exercises exactly `n` entities.
fn slice_batches(n: usize) -> Vec<Vec<&'static SampleEntity>> {
    if n == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut pos = 0usize;
    loop {
        let mut batch = Vec::with_capacity(n);
        for _ in 0..n {
            batch.push(&SAMPLE[pos % SAMPLE.len()]);
            pos += 1;
        }
        out.push(batch);
        if pos >= SAMPLE.len() {
            break;
        }
    }
    out
}

/// Call the apfel one-shot CLI (incumbent baseline). Runs in a spawned thread
/// with a timeout, mirroring `run_apfel` in apfel.rs.
fn call_apfel_cli(prompt: &str) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    let prompt = prompt.to_string();
    std::thread::spawn(move || {
        let output = spawn_apfel_child(&prompt);
        let _ = tx.send(output);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(APFEL_TIMEOUT_SECS)) {
        Ok(Ok(stdout)) => {
            let json_str = String::from_utf8_lossy(&stdout);
            let response: serde_json::Value = serde_json::from_str(json_str.as_ref())
                .map_err(|e| format!("failed to parse apfel JSON response: {e}"))?;
            response
                .get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.trim().to_string())
                .ok_or_else(|| "apfel response missing content field".to_string())
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!("apfel timed out after {APFEL_TIMEOUT_SECS}s")),
    }
}

/// Spawn the apfel one-shot child, write the prompt to stdin, and collect
/// stdout on success (stderr into the error on failure). Runs inside the
/// timeout-guarded thread from `call_apfel_cli`.
fn spawn_apfel_child(prompt: &str) -> Result<Vec<u8>, String> {
    use std::process::Stdio;
    let mut child = Command::new("apfel")
        .args(["-o", "json", "-s", BATCH_SYSTEM_PROMPT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to invoke apfel: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes());
        let _ = stdin.flush();
    }
    let mut stdout_buf = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut stdout_buf);
    }
    let mut stderr_buf = Vec::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_end(&mut stderr_buf);
    }
    let status = child
        .wait()
        .map_err(|e| format!("failed to wait on apfel: {e}"))?;
    if status.success() {
        Ok(stdout_buf)
    } else {
        Err(format!(
            "apfel failed with status {status}: {}",
            String::from_utf8_lossy(&stderr_buf)
        ))
    }
}

/// Call an OpenAI-compatible HTTP endpoint (apfel --serve, llama-server, ...)
/// via the production chat-completions client.
fn call_http(
    backend: SummarizerBackend,
    base_url: &str,
    model: Option<&str>,
    prompt: &str,
) -> Result<String, String> {
    send_chat_completions_for(
        base_url,
        backend,
        model,
        BATCH_SYSTEM_PROMPT,
        &format!("{BATCH_USER_PROMPT}\n\n{prompt}"),
    )
    .map_err(|e| e.to_string())
}

/// Dispatch one batch prompt to the candidate's transport.
fn call_backend(candidate: &Candidate, prompt: &str) -> Result<String, String> {
    if let Some(url) = &candidate.base_url {
        call_http(candidate.backend, url, candidate.model.as_deref(), prompt)
    } else {
        call_apfel_cli(prompt)
    }
}

/// One-shot apfel CLI availability (macOS-only baseline): the `apfel` binary
/// must be on PATH. Checked on all platforms so the example builds and the
/// dead-code lint stays quiet on CI (Linux) and other OSes.
fn which_apfel() -> bool {
    std::process::Command::new("which")
        .arg("apfel")
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// Availability check: apfel one-shot needs the CLI binary on PATH
/// (macOS-only); HTTP backends need a validating health probe (the production
/// `health_validates_for`). Reported, never fatal.
fn is_backend_available(candidate: &Candidate) -> bool {
    if let Some(url) = &candidate.base_url {
        return health_validates_for(candidate.backend, url);
    }
    which_apfel()
}

fn backend_unavailable_reason(candidate: &Candidate) -> String {
    if let Some(url) = &candidate.base_url {
        return format!("no live backend at {url} (health probe failed) — is it running?");
    }
    #[cfg(target_os = "macos")]
    {
        "apfel CLI binary not found on PATH (macOS-only baseline)".to_string()
    }
    #[cfg(not(target_os = "macos"))]
    {
        "apfel is macOS-only; the one-shot CLI baseline is unavailable on this platform".to_string()
    }
}

/// Detect the host chip (best-effort `sysctl hw.model`, macOS).
fn detect_chip() -> String {
    if let Ok(output) = Command::new("sysctl").arg("hw.model").output()
        && output.status.success()
    {
        let model = String::from_utf8_lossy(&output.stdout);
        if let Some((_, rest)) = model.split_once(':') {
            let chip = rest.trim();
            if !chip.is_empty() {
                return chip.to_string();
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        "Apple Silicon (exact model unknown)".to_string()
    }
    #[cfg(not(target_os = "macos"))]
    {
        "unknown".to_string()
    }
}

/// Detect the lievo commit of this build (best-effort: `git rev-parse HEAD`
/// in the manifest dir, else "unknown").
fn detect_commit(manifest_dir: &Path) -> String {
    let Ok(output) = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(manifest_dir)
        .output()
    else {
        return "unknown".to_string();
    };
    if !output.status.success() {
        return "unknown".to_string();
    }
    let full = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if full.len() >= 7 {
        full[..7].to_string()
    } else {
        "unknown".to_string()
    }
}

/// RFC 3339 UTC timestamp from epoch seconds (avoids pulling chrono into the
/// example; the artifact also stores the commit for exact provenance).
fn timestamp(epoch_secs: u64) -> String {
    let (y, mo, d, h, mi, s) = civil_time(epoch_secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Convert unix epoch seconds to (year, month, day, hour, minute, second).
fn civil_time(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let (h, mi, s) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (mp + if mp < 10 { 3 } else { -9 }) as u32;
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day, h as u32, mi as u32, s as u32)
}

/// Run the full sweep for one candidate: license gate, availability check,
/// warm-up, then per-batch-size measurement (parse + timing).
fn measure_candidate(candidate: &Candidate, batch_sizes: &[usize]) -> CandidateResult {
    let license = check_license(&candidate.license);

    // License gate runs BEFORE any measurement: an excluded candidate never
    // touches a backend (issue #773: checked and recorded per candidate
    // before measurement, with the reason stated).
    if let LicenseStatus::Excluded { reason, .. } = &license {
        let reason = reason.clone();
        return CandidateResult {
            id: candidate.id.clone(),
            backend: candidate.backend.as_str().to_string(),
            license,
            model: candidate.model.clone(),
            available: false,
            unavailable_reason: Some(format!("excluded by license filter: {reason}")),
            cold_start_ms_per_entity: None,
            steady_state_ms_per_entity: None,
            parse_success: None,
            summaries: Vec::new(),
        };
    }

    // Missing/unreachable backend: reported as unavailable, not a failure.
    if !is_backend_available(candidate) {
        return CandidateResult {
            id: candidate.id.clone(),
            backend: candidate.backend.as_str().to_string(),
            license,
            model: candidate.model.clone(),
            available: false,
            unavailable_reason: Some(backend_unavailable_reason(candidate)),
            cold_start_ms_per_entity: None,
            steady_state_ms_per_entity: None,
            parse_success: None,
            summaries: Vec::new(),
        };
    }

    // Warm-up (untimed): a one-shot backend pays process spawn + model load on
    // the first call; an HTTP server pays connection setup. Steady-state
    // timing excludes it.
    for batch in slice_batches(2) {
        let _ = call_backend(candidate, &format_batch(&batch));
    }

    let mut by_size: Vec<(usize, Vec<Vec<Option<String>>>)> = Vec::new();
    let mut cold_ms: f64 = 0.0;
    let mut cold_entities = 0usize;
    let mut steady_ms: f64 = 0.0;
    let mut steady_entities = 0usize;
    let mut first_timed = true;

    for n in batch_sizes {
        let mut per_batch_results: Vec<Vec<Option<String>>> = Vec::new();
        for batch in slice_batches(*n) {
            let prompt = format_batch(&batch);
            let started = Instant::now();
            let outcome = call_backend(candidate, &prompt);
            let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
            match outcome {
                Ok(content) => {
                    if first_timed {
                        cold_ms += elapsed_ms;
                        cold_entities += *n;
                    } else {
                        steady_ms += elapsed_ms;
                        steady_entities += *n;
                    }
                    per_batch_results.push(parse_batch(&content, *n));
                }
                // A failed call scores as total parse failure for the batch —
                // the sweep continues and the run reports it.
                Err(e) => {
                    eprintln!("[{}] batch size {n} call failed: {e}", candidate.id);
                    per_batch_results.push(vec![None; *n]);
                }
            }
            first_timed = false;
        }
        by_size.push((*n, per_batch_results));
    }

    // Throughput: the first timed call covers cold start; the rest are
    // steady-state. With a single timed call, both fields get that call.
    let cold = (cold_entities > 0).then(|| cold_ms / cold_entities as f64);
    let steady = if steady_entities > 0 {
        Some(steady_ms / steady_entities as f64)
    } else {
        cold
    };

    // Parse-success breakdown via the harness scorer (lib module).
    let by_size_refs: Vec<(usize, &Vec<Vec<Option<String>>>)> =
        by_size.iter().map(|p| (p.0, &p.1)).collect();
    let report = build_parse_success_report(&candidate.model_label(), by_size_refs);
    let parse_success = Some(ParseSuccessBreakdown {
        model: report.model,
        entries: report
            .entries
            .into_iter()
            .map(|e| ParseSuccessEntry {
                batch_size: e.batch_size,
                percentage: e.percentage,
                successes: e.successes,
                total: e.total,
            })
            .collect(),
        overall: report.overall,
    });

    // Summaries for the human faithfulness pass: one entry per sample entity,
    // from the smallest configured batch.
    let first_n = batch_sizes.first().copied().unwrap_or(2);
    let first_batch = slice_batches(first_n)
        .into_iter()
        .next()
        .unwrap_or_default();
    let parsed = call_backend(candidate, &format_batch(&first_batch))
        .ok()
        .map(|c| parse_batch(&c, first_n))
        .unwrap_or_default();
    let summaries = first_batch
        .iter()
        .enumerate()
        .map(|(i, e)| EntitySummary {
            id: e.id,
            tier: u32::from(
                e.tier == lievo::summarization::summarizer_ab::sample::SampleTier::Rollup,
            ),
            summary: parsed.get(i).cloned().flatten(),
        })
        .collect();

    CandidateResult {
        id: candidate.id.clone(),
        backend: candidate.backend.as_str().to_string(),
        license,
        model: candidate.model.clone(),
        available: true,
        unavailable_reason: None,
        cold_start_ms_per_entity: cold,
        steady_state_ms_per_entity: steady,
        parse_success,
        summaries,
    }
}

fn parse_cli(args: &[String]) -> Result<HarnessConfig, String> {
    let mut batch_sizes: Vec<usize> = vec![2, 4, 8, 16];
    let mut artifact_path = PathBuf::from("summarizer_ab_results.json");
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut http_backend = SummarizerBackend::Generic;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        match arg.as_str() {
            "--backend" => {
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "missing value for --backend".to_string())?;
                if value == "apfel" {
                    candidates.push(Candidate {
                        id: value.clone(),
                        backend: SummarizerBackend::Apfel,
                        base_url: None,
                        model: None,
                        license: "Apache-2.0".to_string(),
                    });
                } else if value.starts_with("http://") || value.starts_with("https://") {
                    candidates.push(Candidate {
                        id: value.clone(),
                        backend: http_backend,
                        base_url: Some(value),
                        model: None,
                        license: "Apache-2.0".to_string(),
                    });
                } else {
                    return Err(format!(
                        "unknown backend '{value}' (expected 'apfel' or an http(s):// url)"
                    ));
                }
            }
            "--http-backend" => {
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "missing value for --http-backend".to_string())?;
                let parsed = SummarizerBackend::parse(&value)
                    .filter(|b| *b != SummarizerBackend::Apfel)
                    .ok_or_else(|| {
                        format!(
                            "invalid --http-backend '{value}' (expected llama-server or generic)"
                        )
                    })?;
                http_backend = parsed;
                // HTTP candidates registered before the flag inherit it.
                for c in &mut candidates {
                    if c.backend == SummarizerBackend::Generic
                        && parsed == SummarizerBackend::Generic
                    {
                        c.backend = parsed;
                    }
                }
            }
            "--model" => {
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "missing value for --model".to_string())?;
                let (id, model) = value
                    .split_once('=')
                    .ok_or_else(|| format!("--model expects id=model-name, got '{value}'"))?;
                let entry = candidates
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or_else(|| format!("--model references unknown candidate '{id}'"))?;
                entry.model = Some(model.to_string());
            }
            "--license" => {
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "missing value for --license".to_string())?;
                let (id, lic) = value
                    .split_once('=')
                    .ok_or_else(|| format!("--license expects id=license, got '{value}'"))?;
                let entry = candidates
                    .iter_mut()
                    .find(|c| c.id == id)
                    .ok_or_else(|| format!("--license references unknown candidate '{id}'"))?;
                entry.license = lic.to_string();
            }
            "--batch-sizes" => {
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "missing value for --batch-sizes".to_string())?;
                let sizes = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        s.parse::<usize>()
                            .map_err(|_| format!("invalid batch size '{s}'"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if sizes.is_empty() {
                    return Err("--batch-sizes must name at least one size".to_string());
                }
                batch_sizes = sizes;
            }
            "--artifact" => {
                i += 1;
                let value = args
                    .get(i)
                    .cloned()
                    .ok_or_else(|| "missing value for --artifact".to_string())?;
                artifact_path = PathBuf::from(value);
            }
            other => {
                return Err(format!("unknown argument '{other}' (see --help)"));
            }
        }
    }

    if candidates.is_empty() {
        return Err("no candidates configured: pass at least --backend apfel".to_string());
    }
    if !candidates
        .iter()
        .any(|c| c.backend == SummarizerBackend::Apfel)
    {
        return Err("the apfel incumbent baseline is missing: pass --backend apfel".to_string());
    }

    Ok(HarnessConfig {
        batch_sizes,
        artifact_path,
        candidates,
    })
}

fn print_help() {
    println!(
        "summarizer A/B harness (issue #773) — measures candidate summarizer backends on lievo's own source.\n\n\
Usage: cargo run --example summarizer_ab -- [options]\n\n\
Options:\n\
  --backend apfel               incumbent baseline (one-shot apfel CLI, macOS-only)\n\
  --backend <http(s)://url>     an OpenAI-compatible HTTP endpoint (apfel --serve, llama-server, ...)\n\
  --http-backend <name>         backend name for HTTP candidates: llama-server | generic (default: generic)\n\
  --model <id>=<model-name>     model revision for a candidate (recorded in the artifact)\n\
  --license <id>=<license>      license filter; non-redistributable candidates are excluded before measurement\n\
  --batch-sizes 2,4,8,16        parse-success sweep sizes (default: 2,4,8,16)\n\
  --artifact <path>             where to write the JSON artifact (default: summarizer_ab_results.json)\n\n\
Examples:\n\
  cargo run --example summarizer_ab -- --backend apfel --backend http://127.0.0.1:8080 \\\n\
      --http-backend llama-server --model http://127.0.0.1:8080=Qwen3-4B-Q4\n\
\n\
The faithfulness method and its limitations are recorded in the artifact.\n\
No model name is hardcoded; the candidate set is entirely CLI-configured."
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        std::process::exit(0);
    }
    let config = parse_cli(&args).unwrap_or_else(|e| {
        eprintln!("error: {e}\n");
        print_help();
        std::process::exit(2);
    });

    // Validate the fixed sample before any measurement (CI-safe, no model).
    let problems = validate_sample();
    if !problems.is_empty() {
        eprintln!("sample validation failed:\n{}", problems.join("\n"));
        std::process::exit(1);
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut candidates_results = Vec::new();
    for candidate in &config.candidates {
        println!(
            "=== candidate: {} (backend: {}) ===",
            candidate.id,
            candidate.backend.as_str()
        );
        let result = measure_candidate(candidate, &config.batch_sizes);
        match &result.parse_success {
            Some(report) => {
                println!("  parse-success: {:.1}% overall", report.overall);
                for entry in &report.entries {
                    println!(
                        "    batch size {:<3} -> {:5.1}% ({}/{} entities)",
                        entry.batch_size, entry.percentage, entry.successes, entry.total
                    );
                }
                if let Some(ms) = result.steady_state_ms_per_entity {
                    println!("  throughput (steady-state): {ms:.1} ms/entity");
                }
                if let Some(ms) = result.cold_start_ms_per_entity {
                    println!("  throughput (cold start): {ms:.1} ms/entity");
                }
            }
            None => {
                if let Some(reason) = &result.unavailable_reason {
                    println!("  not measured: {reason}");
                }
            }
        }
        candidates_results.push(result);
    }

    let artifact = Artifact {
        schema: 1,
        date: timestamp(now_secs),
        chip: detect_chip(),
        lievo_commit: detect_commit(&manifest_dir),
        batch_sizes: config.batch_sizes.clone(),
        faithfulness_method: FAITHFULNESS_METHOD,
        candidates: candidates_results,
    };

    let json = serde_json::to_string_pretty(&artifact).expect("artifact serialization");
    if let Some(parent) = config.artifact_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        eprintln!("failed to create artifact directory: {e}");
        std::process::exit(1);
    }
    if let Err(e) = std::fs::write(&config.artifact_path, format!("{json}\n")) {
        eprintln!(
            "failed to write artifact at {}: {e}",
            config.artifact_path.display()
        );
        std::process::exit(1);
    }
    println!("\nartifact written to {}", config.artifact_path.display());
    println!(
        "chip: {}  lievo commit: {}  date: {}",
        artifact.chip, artifact.lievo_commit, artifact.date
    );
}
