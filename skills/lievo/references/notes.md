# lievo install, summarizer, data, benchmarks

## Install
macOS / Linux (no Rust toolchain needed):
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/trail-openers/lievo/releases/latest/download/lievo-installer.sh | sh
```
Windows (best-effort):
```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/trail-openers/lievo/releases/latest/download/lievo-installer.ps1 | iex"
```
From source: `cargo install --git https://github.com/trail-openers/lievo.git`
Check with `lievo --version`. Install needs network (GitHub + crates.io); the embedding model is fetched from Hugging Face on first use.

## Summarizer backends
Summarization is optional. Backends: `apfel` (default), `llama-server` (llama.cpp), `generic` (any OpenAI-compatible endpoint). Lievo never downloads or pins a model — the model is whatever the server serves.

Config lives in `<repo>/.lievo/config.yaml` (per repo); env vars override where noted.

| Key / env | Meaning |
|---|---|
| `summarize` (bool) | Unset = auto (on if a backend is configured; for `apfel`, on if `apfel` is on PATH). `false` disables entirely. |
| `apfel_endpoint` (string) / `LIEVO_APFEL_ENDPOINT` | Base URL, e.g. `http://127.0.0.1:8080`. Must be absolute http(s). Local `http` is fine but prompts carry source code. |
| `summarizer_backend` (string) | `apfel` (default) / `llama-server` / `generic`. Unknown = config error. |
| `summarizer_model` (string) | The `model` field sent in requests. Ignored by `apfel`; id must match what the server reports (llama-server: full `-hf` spec; Ollama: short name; generic: whatever `/v1/models` reports). |
| `summarizer_input_char_budget` (int) | Per-call input budget in chars (default 24,000, clamped 2,000–64,000). Applies to `llama-server`/`generic` only. |
| `LIEVO_SUMMARIZER_TOKEN` (env) | Bearer token. Deliberately env, not config — config is checked in, tokens are not. Never commit. |

Health probe: lievo probes `GET /v1/models` then `GET /health` (order depends on backend) and accepts the standard model-list shape or a `{status: ...}` body. A 200 with the wrong shape degrades to the no-summarizer path — never a hard error.

### apfel on this host (Apple Silicon) — the working summarizer
apfel (v1.9.x) uses Apple Foundation Models on-device; it is on PATH at `/opt/homebrew/bin/apfel` and **works directly** with no remote server or model download. It honors lievo's `#<n>:` batch contract and produces correct one-line summaries, but is **slow** (~1.5–2s/function; ~480 functions ≈ 15 min). A plain `lievo refresh` auto-enables it. For quick structural work use `--no-summarize`; for summaries, run a plain refresh in the background and/or `lievo summarize <project>` later.

### Backends that do NOT work for lievo on this host
- **`gemma-4-26b-a4b`** — served locally by `tool-server gemma4moe` on `http://127.0.0.1:8099/v1` (start with `tool-server gemma4moe --sleep-idle-seconds 90`; stop with `llm-stop`), or via `halo` at `http://halo:8080/v1`. Fast (~2s) and good at single-item queries, but it **loops to empty output on any multi-item batch prompt** — with or without the `#<n>` position tokens — so lievo's `parse_batch_response` (one `#<n>:` line per input) yields nothing and summaries are silently skipped (the documented `#<n>:` parse-failure mode: a format issue, not connectivity). Verified 2026-10-14.
- **`halo`** (`http://halo:8080/v1`): same gemma-4 model, and intermittently unreachable from this Mac (`No route to host`).

The contract lievo requires (from `src/summarization/apfel.rs`): system prompt + user body `Summarize each function below. Output one line per function: #<n>: <summary>`, items as `--- Function: #0\n<code>\n---`. A backend must emit one `#<n>:` line per entry; plain-prose or empty output = parse failure. A degraded run (backend unreachable or contract-ignoring) never fails: indexing succeeds, summaries are skipped, and an unreachable backend logs a warning naming it.

### llama-server (cross-platform default for remote)
```bash
brew install llama.cpp
llama-server -hf unsloth/Qwen3-4B-GGUF:Q4_K_M -c 32768 -ngl all --port 8080
# .lievo/config.yaml
apfel_endpoint: http://127.0.0.1:8080
summarizer_backend: llama-server
```
### Ollama (easy alternative)
OpenAI-compatible at `http://localhost:11434/v1`. Two gotchas: default context is 4096 tokens (set `OLLAMA_CONTEXT_LENGTH=8192 ollama serve`) and default port 11434 collides with apfel's default (move one of them).
```yaml
apfel_endpoint: http://127.0.0.1:11434
summarizer_backend: generic
summarizer_model: qwen3:4b
```
Licensing: prefer Apache-2.0 weights; **do not use Qwen2.5-Coder-3B** (its 3B is non-commercial Qwen Research License). The batch prompt expects one `#<n>:` line per input item — a model that ignores this produces unparseable output (a format issue, not a connectivity one).

### Verifying before indexing
```bash
curl -s http://127.0.0.1:8080/health        # llama-server / apfel: {"status":"ok"}
curl -s http://127.0.0.1:11434/v1/models    # generic / Ollama: {"object":"list","data":[...]}
```
Then a one-shot `lievo refresh <project>` prints a verbatim degradation warning if the backend is unreachable (indexing still succeeds; only summaries are skipped).

## Data paths & reset
| Path | Purpose |
|---|---|
| `~/.lievo/lievo.db` | SQLite DB (projects, repos, entities, relationships, summaries, insights). |
| `~/.lievo/indices/{repo-hash}/ts-index/` | Per-repo tree-sitter + vector index (FNV-1a hash of repo path). |
| `~/.cache/huggingface/hub/` (or `HF_HUB_CACHE` / `HF_HOME/hub`) | Embedding model weights (~16 MB, `minishlab/potion-code-16M-v2`), shared with other HF tools. |
| `<repo>/.lievo/config.yaml` | Optional per-repo config. |
| `~/.lievo/debug-import-resolution.log` | Optional import-resolution debug log (1 MB cap). |

- `LIEVO_DB=/path/to/db` moves the DB off `~/.lievo/` (then delete that file, not the dir, to reset).
- Reset everything: `rm -rf ~/.lievo` (rebuilds from source on next refresh).
- Reset only the vector index: `rm -rf ~/.lievo/indices/` (DB stays intact; model re-downloads if its HF cache was wiped).
- `lievo doctor` shows the DB path actually in use.

## Benchmarks (context on when lievo helps)
Measured on an 11,000-file commercial monorepo (lievo parses ~20% — the JS portion; Ruby/React invisible). 435 runs, headless Claude Sonnet, set-F1 of returned file paths vs hand-derived gold. Caveats: single repo, single model, JS-only, pre-release build, raw data not published — ballpark, not peer-reviewed.

| Arm | Multi-turn F1 | Change-impact F1 | Bug localization |
|---|---|---|---|
| baseline (built-ins only) | 0.550 | 0.951 | 0.371 |
| **built-ins + lievo (shipping config)** | **0.657** (+0.106, significant) | 0.926 (tied) | 0.331 (tied) |
| lievo only | 0.781 | 0.765 (worse than built-ins) | — |

Headline: with lievo **alongside** normal search, long multi-step work improves (more precise, fewer irrelevant files, ~same recall) and it never measurably worsens anything tested. Change-impact and bug-localization show **no measurable difference** — so for those, lievo is a neutral-to-helpful complement, not a magic bullet. `lievo_only` (replacing built-ins) is not a recommendation: it wins multi-turn F1 but loses change-impact F1.
