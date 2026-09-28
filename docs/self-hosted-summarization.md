# Self-Hosted LLM Summarization

Summarization is **optional**. Lievo indexes and retrieves your code fully
without it. A summarizer only produces tier-1 summaries (per-module prose).
If no backend is configured, or the configured backend is absent or
unreachable, indexing and retrieval still succeed — the run simply produces
no tier-1 summaries. Nothing blocks, and nothing fails.

```
$ lievo refresh myproj
  [myrepo] done: 12 entities, 11 relationships in 4ms
warning: llama-server backend at 127.0.0.1:8080 is unavailable (server not
running; it must be started separately); degrading to the no-summarizer path
— no summaries will be produced
```

The warning above is verbatim what lievo emits when a configured backend
cannot be reached. The run continues; indexing and retrieval still succeed —
only tier-1 summaries are skipped.

## First run

To go from a configured backend to your first set of summaries, lievo needs
a project, a registered repository, and a refresh:

```sh
lievo admin create-project myproj
lievo admin add-repo /path/to/your/repo myproj
lievo refresh myproj
```

- `create-project <name>` creates the project.
- `add-repo <path> [project]` registers a local git repository and adds it
  to the project (the project name defaults to the directory name when
  omitted).
- `refresh [project]` indexes the repository and — if summarization is
  enabled (see below) — runs the summarizer against the configured backend.

If the backend is not running and summarization is enabled (either
explicitly or auto-enabled per the [config table](#config-reference)
below), `refresh` succeeds with the degradation warning shown above. With
the default `apfel` backend and `apfel` not on PATH, summarization is
silently skipped — no warning, no summaries — and `refresh` still
succeeds.

## Choosing a backend

This page documents two setup paths:

| Backend | Role | Platforms |
|---|---|---|
| **llama-server** (llama.cpp) | Recommended default | macOS, Linux, Windows |
| **Ollama** | Easy alternative | macOS, Linux, Windows |

Any other OpenAI-compatible server works through the [generic
backend](#generic-openai-compatible-servers) section below. lievo does not
document per-server setup pages for vLLM, TGI, LM Studio, or
mlx_lm.server.

Lievo never bundles, downloads, or pins a model. The model is whatever your
server was started with — see [Model selection](#model-selection).

## Recommended: llama-server

[llama-server](https://github.com/ggml-org/llama.cpp) (llama.cpp's
server) is the only surveyed option that is open source and runs on all
three platforms: macOS (Metal), Linux (CPU/CUDA/ROCm/Vulkan), and Windows
(Vulkan via winget, CUDA via conda-forge or a source build).

It serves the OpenAI-compatible endpoints lievo needs, defaults to
`127.0.0.1:8080` (no port collision), and builds its model download in:
`-hf user/repo:QUANT` fetches the GGUF on first run — no separate
huggingface-cli step.

### macOS

Install with Homebrew ([llama.cpp formula](https://formulae.brew.sh/formula/llama.cpp)):

```sh
brew install llama.cpp
```

Serve a model (the `-hf` flag downloads it automatically on first run):

```sh
llama-server -hf unsloth/Qwen3-4B-GGUF:Q4_K_M -c 32768 -ngl all --port 8080
```

- `-c 32768` — 32k-token context window (lievo batches fit well inside
  it; the per-call budget can be adjusted with
  [`summarizer_input_char_budget`](#config-reference) if you run a
  smaller context).
- `-ngl all` — offload all layers to the GPU (Metal on Apple Silicon).

### Linux

Same formula via Homebrew, or your distro's package, or conda-forge:

```sh
brew install llama.cpp
# or: conda install -c conda-forge llama.cpp
```

Serve:

```sh
llama-server -hf unsloth/Qwen3-4B-GGUF:Q4_K_M -c 32768 -ngl all --port 8080
```

On NVIDIA GPUs, `-ngl all` uses CUDA. On AMD GPUs, use a ROCm or Vulkan
build. CPU-only works too — see [Hardware](#hardware).

### Windows

Install the llama.cpp package via winget (Vulkan backend; [package
reference](https://github.com/ggml-org/llama.cpp)):

```powershell
winget install ggml.llamacpp
```

Serve (Vulkan is used automatically where available):

```sh
llama-server -hf unsloth/Qwen3-4B-GGUF:Q4_K_M -c 32768 -ngl all --port 8080
```

For CUDA on Windows, use the [conda-forge
llama.cpp](https://anaconda.org/conda-forge/llama.cpp) package instead, or
build from source with CUDA enabled.

### Binding to the network

llama-server binds `127.0.0.1` by default — lievo talks to it over
localhost, which is all it needs. `--host 0.0.0.0` exposes the server on
your network; do this only deliberately, since the served model and its
browser UI become reachable by every machine on the LAN.

llama-server also accepts an `Authorization: Bearer` header when started
with `--api-key`. Lievo sends its token (set via
`LIEVO_SUMMARIZER_TOKEN`) on both the health probe and the chat request, so
a `--api-key`-secured server works out of the box.

## Easy alternative: Ollama

[Ollama](https://ollama.com) has the simplest install and the simplest
model fetch, and exposes an OpenAI-compatible API at
`http://localhost:11434/v1`.

> **⚠ Two gotchas before you use Ollama with lievo:**
>
> 1. **Context window.** Ollama defaults its context window to **4096
>    tokens** on machines with less than 24 GiB of VRAM. Lievo sends
>    roughly 2000-token batches and rollups of up to 40 entities, so a 4k
>    context risks silent truncation. Set a larger value before starting:
>    `OLLAMA_CONTEXT_LENGTH=8192 ollama serve`.
> 2. **Port collision.** Ollama's default port, **11434**, is the same
>    default apfel uses. If you ever also run apfel, move Ollama to a
>    different port: `OLLAMA_HOST=127.0.0.1:11435 ollama serve`, and point
>    lievo's `apfel_endpoint` at `http://127.0.0.1:11435`.

### Install

```sh
# macOS (Homebrew)
brew install ollama

# macOS / Linux (official script)
curl -fsSL https://ollama.com/install.sh | sh

# Windows (official script)
irm https://ollama.com/install.ps1 | iex
```

### Fetch and serve

```sh
ollama pull qwen3:4b
OLLAMA_CONTEXT_LENGTH=8192 ollama serve
```

### Configure lievo

Ollama's OpenAI-compatible base URL is `http://127.0.0.1:11434` (or your
moved port), and it presents as a generic OpenAI-compatible endpoint:

```yaml
# .lievo/config.yaml
apfel_endpoint: http://127.0.0.1:11434
summarizer_backend: generic
```

Lievo probes `GET http://127.0.0.1:11434/v1/models` and accepts the
OpenAI-compatible response:

```json
{ "object": "list", "data": [ { "id": "qwen3:4b", "object": "model", ... } ] }
```

(Ollama's `/v1/models` endpoint returns this standard list shape — not the
native `{"models":[...]}` body Ollama uses at its non-OpenAI
`/api/tags` route.)

## Generic OpenAI-compatible servers

Any server that speaks the OpenAI-compatible API works as a lievo backend
without lievo documenting it. Lievo only needs two things:

1. **A health probe it can validate.** Lievo probes the paths
   `GET /v1/models` **then** `GET /health` (in that order) against your
   `apfel_endpoint`. The first path that returns a validating body wins.
   A body validates if it is one of:
   - the standard model-list shape — `object` is `"list"` and `data` is an
     array (it may be **empty**; a server reporting no loaded models is
     still a live endpoint), or
   - a liveness body with a top-level `status` field that is a string or
     boolean (e.g. `{"status":"ok"}`).
   Any other 200 body (including the native Ollama `{"models":[...]}`
   shape) means "no server of this backend here" — lievo degrades to the
   no-summarizer path; it is never a hard error.
2. **`POST /v1/chat/completions`** — standard OpenAI chat completions.

Configure with `summarizer_backend: generic`:

```yaml
# .lievo/config.yaml
apfel_endpoint: http://127.0.0.1:11434
summarizer_backend: generic
# optional: the `model` field sent in the request body. When unset, lievo
# omits the field entirely (the server serves whatever it was started with).
summarizer_model: qwen3:4b
```

One line on concurrency: lievo's current request path is serial, so a
single server instance is sufficient.

## Config reference

All settings live in `.lievo/config.yaml` (per repository). Environment
variables override where noted.

| Key / env var | Type | Meaning |
|---|---|---|
| `summarize` | bool | Enables summarization. Unset = auto: on if a backend is configured (for `apfel`, on if `apfel` is on PATH); `false` disables it entirely. |
| `apfel_endpoint` | string | Base URL of the backend server, e.g. `http://127.0.0.1:8080`. Must be an absolute http(s) URL. lievo does not verify TLS certificates and sends prompts over plain `http` when the endpoint is local — use a trusted/local endpoint, since prompts contain source code and the bearer token travels in the same connection. |
| `LIEVO_APFEL_ENDPOINT` | env | Overrides `apfel_endpoint` when set to a non-empty value. |
| `summarizer_backend` | string | `apfel` (default), `llama-server`, or `generic`. Case-insensitive; an unknown name is a config error. |
| `summarizer_model` | string | The `model` field sent in chat requests. Ignored by `apfel`; optional for `llama-server`; omitted entirely for `generic` when unset. When set, the value must match the id your server reports (see note below). |
| `summarizer_input_char_budget` | int | Override of the per-call summarizer input budget, in characters. Applies only to `llama-server` and `generic`; `apfel` ignores it (its 8,000 reflects a real model constraint). Default 24,000; clamped to 2,000–64,000 rather than rejected, so an absurd value cannot silently produce failing requests. You are the one who chose `-c`, so you know your context window: run a small-context model? LOWER it. Large window? You may raise it. |
| `LIEVO_SUMMARIZER_TOKEN` | env | Bearer token sent as `Authorization: Bearer <token>` on both the health probe and chat requests. Deliberately an env var, not a config field — config is checked in, tokens are not. Empty = unset. |

```yaml
# A complete llama-server configuration
apfel_endpoint: http://127.0.0.1:8080
summarizer_backend: llama-server
```

**`summarizer_model` id form.** When you set `summarizer_model`, the value
must match the id your server reports — the two backends report it
differently:

- **llama-server**: the full `-hf` spec, path-like, exactly as given on the
  command line — e.g. `unsloth/Qwen3-4B-GGUF:Q4_K_M` (as in the serve
  commands above; this is what `GET /v1/models` reports as `id`).
- **Ollama**: the short model name — e.g. `qwen3:4b`.
- **generic**: whatever id your server reports — check `GET /v1/models` on
  your server to confirm.

**Lievo never downloads, bundles, or pins a model.** It has no model of its
own and no model-fetching step; the model is whatever the server you point
it at serves. (Continue's docs show users routinely assuming a config entry
triggers a download — it does not here.)

## Verifying before you index

Run these **before** `lievo refresh <project>` so a misconfiguration surfaces
as an explicit warning instead of an unexplained "no summaries" run.

### 1. Is the server up and correctly shaped?

For **llama-server** (default port 8080):

```sh
curl -s http://127.0.0.1:8080/health
# {"status":"ok"}
```

For **Ollama** / **generic** (default port 11434):

```sh
curl -s http://127.0.0.1:11434/v1/models
# {"object":"list","data":[...]}
```

This is exactly what lievo probes: `/health` for `llama-server` and
`apfel`; `/v1/models` then `/health` for `generic`. An `llama-server` that
is still loading a model returns HTTP 503 — wait until it returns
`{"status":"ok"}`.

If you secured the server with an API key, include the header:

```sh
curl -s -H "Authorization: Bearer $LIEVO_SUMMARIZER_TOKEN" \
  http://127.0.0.1:8080/health
```

As a non-technical check, llama-server also serves a browser UI at its base
URL (`http://127.0.0.1:8080`) — if you can open the page, the server is up.

### 2. Does lievo see it?

A one-shot `lievo refresh <project>` shows the degradation warning verbatim
if the backend is unreachable (no summaries, indexing still proceeds), or
produces summaries if it is. The message names the backend, the
`host:port` (never your token), and the reason.

## Hardware

Peer tools publish **RAM bands** rather than token rates; this section
follows that convention. Every throughput figure below carries its hardware
and its source class, and is provided as a rough guide, not a spec.

**Minimum viable: 16 GB of unified system RAM. 32 GB is comfortable.**

Model download sizes (all fit a 16 GB Mac alongside an IDE and browser):

| Model | Quant | Size |
|---|---|---|
| Qwen3-4B | Q4_K_M | ~2.5 GB |
| granite-4.2-3b | Q4_K_M | ~2.2 GB |
| Ternary-Bonsai-1.7B | Q2_0 | ~0.5 GB |

Throughput, with source class:

| Hardware | Model | Rate | Source class |
|---|---|---|---|
| Apple M4 Max (Metal) | Qwen3-4B Q4_K_M | 151–163 tok/s | vendor-adjacent measurement |
| Apple M1 (16 GB) | Qwen3-4B Q4_K_M | ~15–30 tok/s | **estimate** (scaling from the M4 figure; no direct M1/M2 16 GB measurement exists) |
| Apple M1 | Qwen3-0.6B 4-bit | 54 tok/s | independently measured |
| NVIDIA RTX 4060 (8 GB) | Qwen3-4B Q4_K_M | ~70–84 tok/s | third-party estimate |
| Modern 8-core x86, CPU-only | 3–4B Q4 model | ~10–25 tok/s | independent community measurement |

### Measured on lievo's actual prompts

The following table is a different and stronger class of evidence than the
throughput table above: it is a single-sample local observation (not a
benchmark) taken while running lievo's own batch prompts — entity-throughput
rather than token rate, since lievo batches multiple entities per chat call
(function-tier and file-tier rollup calls included). The numbers below come
from the end-to-end verification run that added this row.

| Hardware | Model | Rate | Source class |
|---|---|---|---|
| Apple M5 (24 GB unified memory) | Qwen3-4B Q4_K_M, 32k context, `-ngl all` | ~2.1 s per entity (50 entities in ~103 s wall-clock) | **measured on lievo's actual prompts** — single sample from the end-to-end run, not a benchmark |

CPU-only is usable but slower. Treat single-digit tok/s figures from very
old or very small machines as not representative of a modern CPU.

## Model selection

Lievo does not pick or ship a model — model selection is an open question
in its own right (tracked separately). The names above (Qwen3-4B,
granite-4.2-3b, Ternary-Bonsai-1.7B) are **examples known to work with
lievo's batch format**, not a recommendation of one over another.

One licensing note: **do not use Qwen2.5-Coder-3B** — its 3B weight is
licensed under the non-commercial Qwen Research License (only its 1.5B is
Apache-2.0). Prefer Apache-2.0-licensed weights for anything you point at.

Lievo's batch prompt asks the model to answer with one line per input item,
prefixed by its position — e.g. `#0: <summary>`, `#1: <summary>`. Models
that ignore this numbered-line contract will produce output lievo cannot
parse (see [Troubleshooting](#troubleshooting)).

## Troubleshooting

All of these degrade to "no summaries" (indexing and retrieval are never
blocked). The tell-tale is the warning naming your backend and endpoint.

**Port 11434 collision — Ollama vs apfel.** Both default to 11434. If a
service you did not intend answers on that port, lievo probes it, the
response body does not match the configured backend's shape, and lievo
degrades to unavailable. Move Ollama to a different port
(`OLLAMA_HOST=127.0.0.1:11435 ollama serve`) and point `apfel_endpoint`
there — or move the other service.

**A configured backend not running.** The warning says
`server not running; it must be started separately` (for `llama-server` /
`generic`, which lievo never starts for you). Start the server, confirm the
[verification step](#verifying-before-you-index), and re-run.

**A wrong service answering on the configured port.** The warning says the
backend is `unavailable` with the same "degrading to the no-summarizer
path" phrasing, but the reason is the health check failing — a 200 response
whose body is not the expected shape (e.g. the native Ollama
`{"models":[...]}` body on a port configured for another backend). Check
what is actually listening: `curl -s <endpoint>/v1/models` or
`curl -s <endpoint>/health`, and make sure the `summarizer_backend` you set
matches the server that is there.

**Ollama's 4k default context.** Summaries come back truncated or empty on
long inputs while everything else looks healthy. This is the 4096-token
default. Set `OLLAMA_CONTEXT_LENGTH=8192` (or higher) and restart
`ollama serve`.

**`#<n>:` parse failures.** The model answered in plain prose instead of
one `#<n>:` line per input, so nothing parsed. This is a model-format
issue, not a connectivity issue. It is more common with very small models
that ignore the batch instruction. Try a larger or more instruction-following
model, or a model known to honor the numbered-line contract.

**`warning: N function entities not found in storage — summaries
skipped. Run 'lievo refresh --force <project>' to re-index.`** This can
appear even on a fresh index — it is an internal extraction/lookup mismatch
inside lievo, not a server failure and not a sign that the run failed. The
run completed; the affected entities simply have no tier-1 summary. Run
`lievo refresh --force <project>` to re-index, and file an issue if the
warning persists on a fresh project.

**`WARN rollup_to_tier: skipping entity with no child summaries`** (with
`entity_id=...`, `tier=File`, `child_count=0` fields). This fires for any
file that contains no summarized child entities — files with no functions,
for example. It is a `WARN`-level log line, not an error: the run succeeded
and the file simply has nothing to roll up into a file-tier summary.
