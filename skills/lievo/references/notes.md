# lievo install, summarizer backends, configuration, data

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

Check with `lievo --version`. Installing needs network access (GitHub and crates.io). The embedding model is fetched from Hugging Face on the first refresh that builds the vector index.

## Configuration

### Environment variables

| Variable | Meaning |
|---|---|
| `LIEVO_PROJECT_DIR=/path/to/repo` | Pins the repo. Wins over the working directory. |
| `LIEVO_NO_REFRESH=1` | Disables automatic background indexing. |
| `LIEVO_MCP_TOOLS=a,b` | Extra MCP tools to list. Default lists only `lievo_explore`. |
| `LIEVO_DB=/path/to/db` | Moves the SQLite database off `~/.lievo/`. |
| `LIEVO_APFEL_ENDPOINT=url` | Summarizer endpoint. Overrides `apfel_endpoint` in config. |
| `LIEVO_SUMMARIZER_TOKEN=token` | Bearer token sent to the summarizer endpoint. Keep it out of config files and version control. |

### `<repo>/.lievo/config.yaml`

Per-repo settings. It is optional and lives in the checkout. **It is untrusted for repos you don't control** (see the security notes below).

| Key | Meaning |
|---|---|
| `summarize` (bool) | Unset: automatic. With the `apfel` backend, summaries are on when `apfel` is on PATH. With `llama-server` or `generic`, they are on when `apfel_endpoint` is set. `true` forces on, `false` disables. |
| `apfel_endpoint` (string) | Base URL of the summarizer, e.g. `http://127.0.0.1:8080`. Must be an absolute `http(s)` URL. |
| `summarizer_backend` (string) | `apfel` (default), `llama-server`, or `generic`. Any other value is a config error. |
| `summarizer_model` (string) | The `model` field sent in requests. Ignored by `apfel`. Must match what the server reports. |
| `summarizer_input_char_budget` (int) | Per-call input budget in characters. Default 24,000; clamped to 2,000–64,000. Applies to `llama-server` and `generic` only. |
| `identity` (string) | Overrides the identity key derived from the `origin` remote. Used for project naming and moved-checkout matching. |
| `exclude`, `languages`, `preserve_function_entities`, `subsystems`, `module_depth` | Analysis overrides. |

### Security

- **Repo-supplied config can point summaries at a host you did not choose.** A `.lievo/config.yaml` in an untrusted repo can set `apfel_endpoint` or `summarizer_backend`. A `refresh` that summarizes then sends source code to that endpoint, and sends `LIEVO_SUMMARIZER_TOKEN` as a bearer token if the variable is set. Before refreshing a repo you don't control, read its `.lievo/config.yaml`, or refresh with `--no-summarize`, which contacts no summarizer.
- **Use `https` for non-loopback endpoints.** lievo accepts plain `http` URLs, and it warns when a summary run sends source to a non-loopback host. Loopback endpoints do not trigger the warning.
- **`identity:` in repo config is untrusted too.** It sets the project's identity key, which affects project naming and whether a moved checkout is re-attached to an existing project. See [data-storage.md#known-limitations](https://github.com/trail-openers/lievo/blob/main/docs/data-storage.md#known-limitations).

## Summarizer backends

Summarization is optional. lievo never downloads, pins, or bundles a model. The model is whatever the configured server serves.

| Backend | Use it for | Health probe |
|---|---|---|
| `apfel` (default) | On-device summaries on Apple Silicon, through the `apfel` binary on PATH, or an apfel server at `apfel_endpoint`. | `GET /health` |
| `llama-server` | A llama.cpp server at `apfel_endpoint`. | `GET /health` |
| `generic` | Any OpenAI-compatible server at `apfel_endpoint`. | `GET /v1/models`, then `GET /health` |

When `apfel_endpoint` is set, all three send chat requests to `POST {apfel_endpoint}/v1/chat/completions` (apfel without an endpoint uses the apfel CLI on PATH). A probe that returns a model-list shape or a `{status: ...}` body is accepted. A 200 with the wrong shape falls back to the no-summarizer path. It is never a hard error.

### What a backend must support

lievo's batch prompt asks for one line per function, and the backend must follow that format:

- **Batch contract.** The system prompt and user body ask for `#<n>: <summary>`, one line per input item, from items formatted as `--- Function: #<n>`. The backend must emit one `#<n>:` line per item. Plain prose or empty output is a parse failure.
- **Multi-item behavior.** A model that answers a single item well but returns empty output for the multi-item prompt yields no summaries. The run does not fail. Summaries are skipped. Test with a multi-item prompt, not a single-item one.
- **Input budget.** The backend must accept the per-call input budget (`summarizer_input_char_budget`, or the backend default).
- **Reachability.** The health probe must succeed, and `POST /v1/chat/completions` must work.
- **Authentication.** If the server requires a bearer token, set `LIEVO_SUMMARIZER_TOKEN`.

An unreachable backend, or one that fails the health probe, prints a warning that names the backend and endpoint. Indexing still succeeds, and summaries are skipped.

### Check a backend before relying on it

1. Probe it: `curl -s http://127.0.0.1:8080/health` (llama-server or apfel), or `curl -s http://127.0.0.1:11434/v1/models` (generic).
2. Refresh one project with summaries enabled: `lievo refresh <project>`. Read the output for warnings.
3. Check coverage: `lievo admin status <project>`. The `Summary coverage:` lines should show a non-zero done count. `0/N` means the backend produced no usable summaries.

### Examples

llama-server:
```bash
brew install llama.cpp
llama-server -hf <owner>/<model>-GGUF:<quant> -c 32768 -ngl all --port 8080
```
```yaml
# .lievo/config.yaml in the repo you are indexing
apfel_endpoint: http://127.0.0.1:8080
summarizer_backend: llama-server
```

Ollama (generic):
```yaml
apfel_endpoint: http://127.0.0.1:11434
summarizer_backend: generic
summarizer_model: <model name from /v1/models>
```
Ollama's default context is 4,096 tokens. Start it with a larger value, e.g. `OLLAMA_CONTEXT_LENGTH=8192 ollama serve`. Its default port (11434) is also a common apfel default, so move one of them if both run on one machine.

Choose a model whose license allows your use. The model is not part of lievo, so its license is yours to check.

## Install the skill and the MCP server

See [docs/using-with-agent.md](https://github.com/trail-openers/lievo/blob/main/docs/using-with-agent.md) for installing this skill into each agent, and [docs/quickstart.md](https://github.com/trail-openers/lievo/blob/main/docs/quickstart.md#connect-your-agent) for MCP registration.

## Data paths and reset

| Path | Purpose |
|---|---|
| `~/.lievo/lievo.db` | SQLite database: projects, repos, entities, relationships, summaries, insights. |
| `~/.lievo/indices/<repo-hash>/` | Per-repo tree-sitter and vector index. |
| `~/.cache/huggingface/hub/` (or `HF_HUB_CACHE` / `HF_HOME/hub`) | Embedding model weights (downloaded on first semantic use), shared with other Hugging Face tools. |
| `<repo>/.lievo/config.yaml` | Optional per-repo config. |

`lievo doctor` shows the database path actually in use.

### Reset (ask the user first; never run these autonomously)

These commands delete data. An agent must ask the user before running any of them, and must not run them as part of a task.

- Delete the database only: remove the file named by `LIEVO_DB`, or `~/.lievo/lievo.db`. Removing the directory is not needed.
- Delete only the vector index: `rm -rf ~/.lievo/indices/`. The database stays. The model re-downloads if its Hugging Face cache was wiped.
- Delete everything: `rm -rf ~/.lievo`. The next refresh rebuilds from source.
- Remove one repo's data: `lievo admin delete-repo <name> <project>`.
