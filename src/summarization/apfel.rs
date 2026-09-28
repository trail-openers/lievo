// apfel transport — calls Apple Foundation Model for code summarization.
//
// When `apfel_endpoint` is configured (via `.lievo/config.yaml` or
// `LIEVO_APFEL_ENDPOINT`), summarization goes through the persistent apfel
// server over HTTP (see `summarizer_backend.rs`). Otherwise, the legacy
// one-shot `apfel` CLI subprocess is used as a fallback.

use crate::config::SummarizerBackend;
use crate::error::Result;
use serde::Deserialize;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

const SYSTEM_PROMPT: &str =
    "You are a concise Rust code summarizer. Output only the summary, no preamble.";
/// The summarize_code user prompt (issue #827): `pub(crate)` so the
/// file-tier source-text fallback in `pipeline_rollup` reuses it verbatim
/// without touching `summarize_code`'s own code path.
pub(crate) const USER_PROMPT: &str =
    "Summarize what this function does in 1-2 sentences. Focus on purpose and behaviour.";
const BATCH_SYSTEM_PROMPT: &str = "You are a concise Rust code summarizer.";
const BATCH_USER_PROMPT: &str =
    "Summarize each function below. Output one line per function: #<n>: <summary>";

use crate::summarization::backend_profile;
use crate::summarization::pipeline::BUDGET_EXCEEDED_MARKER;

/// Apfel's input budget (chars) — the per-backend default for `apfel`
/// (issue #792). The other backends carry their own defaults; the
/// pipeline resolves the budget via [`resolve_input_char_budget`].
///
pub(crate) const APFEL_INPUT_CHAR_BUDGET: usize = backend_profile::APFEL_INPUT_CHAR_BUDGET;

/// Per-invocation timeout for a single apfel subprocess call (in seconds).
///
/// Justification:
/// - A single `summarize_code` handles one function (a few hundred chars of code).
/// - A single `batch_summarize` handles up to APFEL_INPUT_CHAR_BUDGET (8000 chars) of
///   snippets in one LLM invocation.
/// - 300 seconds matches the MODEL_DOWNLOAD_TIMEOUT_SECS precedent in model_cache.rs
///   (a single LLM inference call is expected to complete well within 5 minutes).
/// - The timeout is PER INVOCATION: summarize_functions calls batch_summarize once per
///   batch, so the effective total bound is (number of batches) × APFEL_TIMEOUT_SECS.
///   This is intentional — a per-call bound keeps the error message precise and lets
///   the pipeline salvage remaining batches; an overall-run deadline would require
///   additional coordination not in scope for this fix.
const APFEL_TIMEOUT_SECS: u64 = 300;

/// Effective per-invocation deadline in seconds.
///
/// Production uses `APFEL_TIMEOUT_SECS` (300s). Test builds (`cargo test`) read
/// `LIEVO_APFEL_TIMEOUT_SECS` first so a test can drive a short deadline through
/// the public API path (`summarize_functions` → `batch_summarize` → `run_apfel`)
/// without touching the 300s production constant (issue #811 task-a). The env
/// var is only read under `#[cfg(test)]`, so the production binary never consults
/// it.
fn apfel_timeout_secs() -> u64 {
    #[cfg(test)]
    if let Some(secs) = std::env::var("LIEVO_APFEL_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return secs.max(1);
    }
    APFEL_TIMEOUT_SECS
}

/// Max rollup entities per batch. Guards against output truncation: the on-device
/// window holds input+output combined (min 4096 tokens); 40 one-line summaries
/// (~400 tokens) fit the output half with margin.
pub(crate) const ROLLUP_MAX_ENTITIES_PER_BATCH: usize = 40;

/// Result from apfel invocation.
#[derive(Debug)]
pub struct ApfelSummaryResult {
    /// The summary text extracted from JSON response
    pub content: String,
}

#[derive(Debug, Deserialize)]
struct ApfelResponse {
    content: String,
}

static APFEL_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Check if apfel is available in PATH.
///
/// Result is cached process-wide — the subprocess is only spawned once regardless
/// of how many repos are analyzed in a single run.
pub fn is_apfel_available() -> bool {
    *APFEL_AVAILABLE.get_or_init(|| {
        Command::new("which")
            .arg("apfel")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// Backend transport for the HTTP path: the server URL, the named backend
/// that serves it (issue #776 — the `model` field of the request must be
/// backend-aware), and the user-configured model name (if any).
///
/// The transport is passed explicitly down the call path from
/// `SummarizationPipeline::run` (issue #783); no ambient state carries it.
/// The bearer token is not a field here — the HTTP path resolves it at
/// request time via `summarizer_backend::summarizer_token()` (issue #780),
/// which is the single source of truth.
#[derive(Clone, Debug)]
pub struct BackendTransport {
    pub url: String,
    pub backend: SummarizerBackend,
    pub model_name: Option<String>,
}

/// Resolve the input budget (chars) for a transport (issue #792). HTTP
/// transports resolve to the per-backend default; the no-transport CLI
/// fallback resolves to apfel's budget.
pub fn resolve_input_char_budget(
    transport: Option<&BackendTransport>,
    configured_override: Option<usize>,
) -> usize {
    match transport {
        Some(transport) => {
            backend_profile::input_char_budget(transport.backend, configured_override)
        }
        None => APFEL_INPUT_CHAR_BUDGET,
    }
}

/// Call apfel to summarize a single code snippet.
///
/// `transport` is `None` for the one-shot CLI fallback; `Some` routes the
/// call through the HTTP transport (issue #783: explicit, not ambient).
pub fn summarize_code(
    code: &str,
    transport: Option<&BackendTransport>,
) -> Result<ApfelSummaryResult> {
    let prompt = format!("{}\n\n{}", USER_PROMPT, code);
    let budget = resolve_input_char_budget(transport, None);

    // Defensive check: reject over-budget input before spawning a subprocess
    // (mirrors the guard in batch_summarize). The packer still sends an
    // over-budget snippet alone on purpose; the budget check turns that into a
    // deterministic offline `SummarizationFailed` (classified as
    // `skipped_oversized` by the pipeline, issue #649) instead of spawning a
    // subprocess that would fail with a context overflow.
    if prompt.len() > budget {
        return Err(crate::error::LievoError::SummarizationFailed(format!(
            "summarize_code: input {len} {marker} {budget}",
            len = prompt.len(),
            marker = BUDGET_EXCEEDED_MARKER
        )));
    }

    let content = match transport {
        Some(transport) => {
            let backend = transport.backend;
            let model_name = transport.model_name.clone();
            crate::summarization::summarizer_backend::send_chat_completions_for(
                &transport.url,
                backend,
                model_name.as_deref(),
                SYSTEM_PROMPT,
                &prompt,
            )?
        }
        None => run_apfel(
            "apfel",
            apfel_timeout_secs(),
            &["-o", "json", "-s", SYSTEM_PROMPT],
            &prompt,
        )?
        .parse_single()?,
    };

    Ok(ApfelSummaryResult { content })
}

/// Helper to parse a single (non-batch) apfel response.
trait ParseSingle {
    fn parse_single(self) -> Result<String>;
}

impl ParseSingle for (bool, Vec<u8>, Vec<u8>) {
    fn parse_single(self) -> Result<String> {
        let (success, stdout, stderr) = self;
        if !success {
            // NOTE (issue #649): `is_overflow_error` (pipeline.rs) classifies
            // `SummarizationFailed` messages by matching apfel's overflow markers.
            // It assumes stderr is apfel's own output and never contains the raw
            // code being summarized. If this wrapping is ever changed to include
            // code content (e.g. echoing the prompt on failure), the classifier's
            // guarantee that a code body containing "context overflow" or
            // "exceeds budget" does not cause a false positive silently breaks.
            let stderr = String::from_utf8_lossy(&stderr);
            return Err(crate::error::LievoError::SummarizationFailed(format!(
                "apfel failed with status: {stderr}"
            )));
        }
        let json_str = String::from_utf8_lossy(&stdout);
        let response: ApfelResponse = serde_json::from_str(&json_str).map_err(|e| {
            crate::error::LievoError::SummarizationFailed(format!(
                "Failed to parse apfel JSON response: {e}"
            ))
        })?;
        Ok(response.content.trim().to_string())
    }
}

/// Spawn the apfel subprocess on a background thread with a bounded wait.
///
/// The spawned thread owns the full child lifecycle (spawn + write to stdin +
/// read stdout/stderr + wait) and only sends the *parsed* result over the
/// channel. This ensures `recv_timeout` truly bounds the caller's wait.
///
/// On timeout, the main thread calls `child.kill()` via the shared `Arc<Mutex<Option<Child>>>`.
/// The kill is non-ignorable (SIGKILL on Unix), so the child terminates and
/// its pipes close, unblocking the spawned thread's pipe reads. The spawned
/// thread then sends its result via best-effort `let _ = tx.send(...)` — if the
/// receiver is already gone (timeout fired), the send fails silently and the
/// thread exits. No second unbounded wait is introduced.
///
/// Returns `(status_success, stdout_bytes, stderr_bytes)`.
/// On timeout, returns `Err(SummarizationFailed("apfel timed out after N seconds"))`.
///
/// `pub(crate)` so the test modules (apfel_oneshot_tests / apfel_rollup_tests)
/// can exercise the bounded-wait machinery directly with a fake binary.
pub(crate) fn run_apfel(
    binary: &str,
    timeout_secs: u64,
    args: &[&str],
    prompt: &str,
) -> Result<(bool, Vec<u8>, Vec<u8>)> {
    let (tx, rx) = mpsc::channel();

    // Shared child handle: the spawned thread writes the Child into this Arc
    // after spawning, and the main thread reads + kills it on timeout.
    // Using Mutex<Option<Child>> because Child is !Sync.
    let child_handle: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));

    // Record the deadline so a hung run is diagnosable (debug-only; production
    // cost is one extra format on the slow path — the timeout branch already
    // builds a message).
    #[cfg(test)]
    let deadline_note = format!(" (deadline: {timeout_secs}s)");
    #[cfg(not(test))]
    let deadline_note = "";

    let binary = binary.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let prompt = prompt.to_string();
    let child_handle_thread = Arc::clone(&child_handle);

    std::thread::spawn(move || {
        let result = run_apfel_in_thread(&binary, &args, &prompt, child_handle_thread);
        // Best-effort send: if the receiver dropped (timeout fired), this fails
        // silently and the thread exits. No second unbounded wait.
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_secs(timeout_secs)) {
        Ok(Ok((success, stdout, stderr))) => Ok((success, stdout, stderr)),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Timeout: kill the child so it doesn't linger as an orphan.
            // The spawned thread's pipe reads will unblock once the pipes close.
            if let Ok(mut guard) = child_handle.lock()
                && let Some(child) = guard.as_mut()
            {
                let _ = child.kill();
            }
            Err(crate::error::LievoError::SummarizationFailed(format!(
                "apfel timed out after {timeout_secs} seconds{deadline_note}"
            )))
        }
    }
}

/// Internal: spawn the child, store the handle in the shared Arc, write to
/// stdin, read stdout/stderr, wait. Runs on the spawned thread.
///
/// The child handle is stored in `child_handle` after successful spawn so the
/// main thread can call `child.kill()` on timeout. After the spawned thread
/// reads stdout/stderr and waits, it takes the child out of the Arc (consuming
/// it) so the main thread's `kill()` call (if it races) finds `None`.
fn run_apfel_in_thread(
    binary: &str,
    args: &[String],
    prompt: &str,
    child_handle: Arc<Mutex<Option<std::process::Child>>>,
) -> Result<(bool, Vec<u8>, Vec<u8>)> {
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            crate::error::LievoError::SummarizationFailed(format!("Failed to invoke apfel: {e}"))
        })?;

    // Store the child handle so the main thread can kill it on timeout.
    // We take stdin/stdout/stderr out of child first (they're borrowed from
    // child), then store the child in the Arc.
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    {
        let mut guard = child_handle.lock().expect("child_handle mutex poisoned");
        *guard = Some(child);
    }

    // Write prompt to stdin and close it.
    if let Some(mut stdin) = stdin {
        let _ = stdin.write_all(prompt.as_bytes());
        let _ = stdin.flush();
        // Dropping stdin closes the pipe; child receives EOF.
    }

    // Read stdout (blocks until child closes the write end or is killed).
    let mut stdout_buf = Vec::new();
    if let Some(mut stdout) = stdout {
        let _ = stdout.read_to_end(&mut stdout_buf);
    }

    // Read stderr.
    let mut stderr_buf = Vec::new();
    if let Some(mut stderr) = stderr {
        let _ = stderr.read_to_end(&mut stderr_buf);
    }

    // Wait for the child to exit. We take the child out of the Arc so the
    // main thread's kill() (if racing) finds None and does nothing.
    let mut child = {
        let mut guard = child_handle.lock().expect("child_handle mutex poisoned");
        guard.take().expect("child handle should be present")
    };

    let status = child.wait().map_err(|e| {
        crate::error::LievoError::SummarizationFailed(format!("Failed to wait on apfel: {e}"))
    })?;

    Ok((status.success(), stdout_buf, stderr_buf))
}

/// Parse the `#<n>: <summary>` lines from a batch response into a positional result vector.
/// Returns a Vec<Option<String>> where Some(summary) is at the position corresponding to the #n index.
/// This handles apfel's order-preserving behavior and gracefully tolerates malformed lines.
///
/// `pub(crate)` (not private) so the summarization pipeline can count parse
/// failures as a first-class outcome (issue #776): a batch that returns a line
/// but no `#<n>:` entry for a position is a parse failure, not a salvage miss,
/// and the pipeline must distinguish the two. The parser itself is unchanged —
/// the positional `#<n>:` contract and `parse_batch_response` behavior are preserved.
pub(crate) fn parse_batch_response(content: &str, num_snippets: usize) -> Vec<Option<String>> {
    let mut results = vec![None; num_snippets];

    for line in content.lines() {
        // Look for the pattern "#<n>:" where n is a number
        // Apfel may wrap output in markdown fences or include extra labeling
        // Find the "#" character and parse the index after it
        if let Some(hash_pos) = line.find('#') {
            let after_hash = &line[hash_pos + 1..];
            // Parse until the next ":"
            if let Some(colon_pos) = after_hash.find(':') {
                let index_str = &after_hash[..colon_pos].trim();
                if let Ok(index) = index_str.parse::<usize>() {
                    let summary = after_hash[colon_pos + 1..].trim().to_string();
                    if index < num_snippets {
                        results[index] = Some(summary);
                    }
                    // Silently skip out-of-range indices
                }
            }
        }
    }

    results
}

/// Pack snippets into batches respecting the char budget.
/// Returns a Vec of batches, where each batch is a Vec of (file, name, code) tuples.
/// A single snippet larger than the budget is still returned as its own batch
/// (do not drop it). This ensures every snippet gets a chance at summarization.
///
/// The packer accounts for the overhead that batch_summarize() adds:
/// - BATCH_USER_PROMPT (~79 chars)
/// - "\n\n" separator between prompt and body (2 chars)
/// - "\n" joins between entries (1 char per entry, N-1 for N entries)
///
/// `budget` is the per-backend input budget (chars) resolved for this run —
/// the same value the defensive guards and the retry guard compare against
/// (issue #792: the packer and the guards must never compute two different
/// budgets). The effective budget reserved for packed snippets is `budget`
/// minus this overhead.
pub fn pack_by_char_budget(
    snippets: &[(String, String, String)],
    budget: usize,
) -> Vec<Vec<(String, String, String)>> {
    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut current_size = 0usize;

    // Calculate fixed overhead: BATCH_USER_PROMPT + "\n\n" separator
    let prompt_overhead = BATCH_USER_PROMPT.len() + 2; // +2 for "\n\n"
    let effective_budget = budget.saturating_sub(prompt_overhead);

    for (file, name, code) in snippets {
        // Estimate input size with positional indexing:
        // Per-entry format: "--- Function: #" (16) + index_str (1-3 digits, estimate 2) + "\n" (1) + code + "\n---\n" (4)
        // = ~24 chars fixed overhead + code
        // Plus 1 char for the "\n" join separator between entries
        // (We account for this by adding 1 to the per-entry size; the first entry pays 0 extra,
        // subsequent ones pay 1 for their preceding newline.)
        let entry_size = 25 + code.len(); // 16 + 2 (index estimate) + 1 + 4 + 1 + 1 = 25 chars fixed overhead

        // If adding this snippet would exceed the effective budget and current batch is not empty, flush
        if current_size > 0 && current_size + entry_size > effective_budget {
            batches.push(current_batch);
            current_batch = Vec::new();
            current_size = 0;
        }

        // Add snippet to current batch
        current_batch.push((file.clone(), name.clone(), code.clone()));
        current_size += entry_size;
    }

    // Flush any remaining batch
    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    batches
}

/// Batch multiple code snippets into a single apfel call.
/// Returns a Vec<Option<String>> aligned to input order, where Some(summary) is at
/// the position corresponding to the input snippet's index, and None indicates missing result.
///
/// `transport` is `None` for the one-shot CLI fallback (issue #783).
pub fn batch_summarize(
    snippets: &[(String, String, String)],
    transport: Option<&BackendTransport>,
) -> Result<Vec<Option<String>>> {
    if snippets.is_empty() {
        return Ok(vec![]);
    }

    let batch_input = snippets
        .iter()
        .enumerate()
        .map(|(i, (_, _, code))| format!("--- Function: #{}\n{}\n---\n", i, code))
        .collect::<Vec<_>>()
        .join("\n");

    // Defensive check: ensure formatted input respects budget before invoking apfel
    let budget = resolve_input_char_budget(transport, None);
    let total_input = format!("{}\n\n{}", BATCH_USER_PROMPT, batch_input);
    if total_input.len() > budget {
        return Err(crate::error::LievoError::SummarizationFailed(format!(
            "batch_summarize: formatted input {len} {marker} {budget}",
            len = total_input.len(),
            marker = BUDGET_EXCEEDED_MARKER
        )));
    }

    let content = match transport {
        Some(transport) => {
            let backend = transport.backend;
            let model_name = transport.model_name.clone();
            crate::summarization::summarizer_backend::send_chat_completions_for(
                &transport.url,
                backend,
                model_name.as_deref(),
                BATCH_SYSTEM_PROMPT,
                &total_input,
            )?
        }
        None => {
            let (success, stdout, stderr) = run_apfel(
                "apfel",
                apfel_timeout_secs(),
                &["-o", "json", "-s", BATCH_SYSTEM_PROMPT],
                &total_input,
            )?;

            if !success {
                let stderr = String::from_utf8_lossy(&stderr);
                return Err(crate::error::LievoError::SummarizationFailed(format!(
                    "apfel batch failed with status: {stderr}"
                )));
            }

            let json_str = String::from_utf8_lossy(&stdout);
            let response: ApfelResponse = serde_json::from_str(&json_str).map_err(|e| {
                crate::error::LievoError::SummarizationFailed(format!(
                    "Failed to parse apfel batch JSON response: {e}"
                ))
            })?;
            response.content
        }
    };

    let results = parse_batch_response(&content, snippets.len());

    // Issue #776: a non-empty response body that yields zero parseable
    // `#<n>:` lines is a demux parse failure — the model honoured the request
    // but not the positional contract. Mark the batch so the pipeline can
    // count it as a first-class outcome (parse_failures) rather than letting
    // it degrade silently into the partial-batch salvage warning. A body that
    // has at least one parseable line is a normal (possibly partial) result,
    // not a parse failure.
    if results.iter().all(Option::is_none) {
        return Err(crate::error::LievoError::SummarizationFailed(format!(
            "batch_summarize: no parsable #<n>: lines in model output {}",
            crate::summarization::pipeline::PARSE_FAILURE_MARKER
        )));
    }

    Ok(results)
}

const ROLLUP_BATCH_SYSTEM_PROMPT: &str = "You are a concise code architect. You describe the purpose and role of a file, module, or subsystem in 1-2 sentences.";
const ROLLUP_BATCH_USER_PROMPT: &str = "Describe the purpose and role of each entry below. Output one line per entry: #<n>: <description>";

/// Batch-summarize rollup entities in a single apfel call (issue #771).
/// Reuses the same packing/demultiplexing algorithm as `batch_summarize` but
/// with a rollup-specific prompt pair. The "code" slot holds the pre-built
/// prompt. Caller must cap batch size at `ROLLUP_MAX_ENTITIES_PER_BATCH`.
///
/// `transport` is `None` for the one-shot CLI fallback (issue #783).
pub fn rollup_batch_summarize(
    entries: &[(String, String, String)],
    transport: Option<&BackendTransport>,
) -> Result<Vec<Option<String>>> {
    if entries.is_empty() {
        return Ok(vec![]);
    }

    let batch_input = entries
        .iter()
        .enumerate()
        .map(|(i, (_, _, prompt))| format!("--- Entry: #{}\n{}\n---\n", i, prompt))
        .collect::<Vec<_>>()
        .join("\n");
    let budget = resolve_input_char_budget(transport, None);
    let total_input = format!("{}\n\n{}", ROLLUP_BATCH_USER_PROMPT, batch_input);
    if total_input.len() > budget {
        return Err(crate::error::LievoError::SummarizationFailed(format!(
            "rollup_batch_summarize: formatted input {len} {marker} {budget}",
            len = total_input.len(),
            marker = BUDGET_EXCEEDED_MARKER
        )));
    }
    let content = match transport {
        Some(transport) => {
            let backend = transport.backend;
            let model_name = transport.model_name.clone();
            crate::summarization::summarizer_backend::send_chat_completions_for(
                &transport.url,
                backend,
                model_name.as_deref(),
                ROLLUP_BATCH_SYSTEM_PROMPT,
                &total_input,
            )?
        }
        None => {
            let (success, stdout, stderr) = run_apfel(
                "apfel",
                apfel_timeout_secs(),
                &["-o", "json", "-s", ROLLUP_BATCH_SYSTEM_PROMPT],
                &total_input,
            )?;
            if !success {
                let stderr = String::from_utf8_lossy(&stderr);
                return Err(crate::error::LievoError::SummarizationFailed(format!(
                    "apfel rollup batch failed with status: {stderr}"
                )));
            }
            let response: ApfelResponse = serde_json::from_str(&String::from_utf8_lossy(&stdout))
                .map_err(|e| {
                crate::error::LievoError::SummarizationFailed(format!(
                    "Failed to parse apfel rollup batch JSON response: {e}"
                ))
            })?;
            response.content
        }
    };
    Ok(parse_batch_response(&content, entries.len()))
}

#[cfg(test)]
#[path = "apfel_rollup_tests.rs"]
pub mod apfel_rollup_tests;

#[cfg(test)]
#[path = "apfel_oneshot_tests.rs"]
pub mod apfel_oneshot_tests;
