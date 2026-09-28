# Quickstart

Install lievo, register it once with your agent, and ask a question. Every step below is copy-paste, except the `/path/to/repo` placeholders, where you substitute your repo's absolute path.

## Install

macOS and Linux (no Rust toolchain required):

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/trail-openers/lievo/releases/latest/download/lievo-installer.sh | sh
```

Windows (best-effort):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/trail-openers/lievo/releases/latest/download/lievo-installer.ps1 | iex"
```

The installers place `lievo` on your `PATH`. Check the install with `lievo --version`. To uninstall the binary, delete the `lievo` executable from wherever the installer put it (the installers print the exact path at the end of their run).

Falling back to Rust, or building from source:

```bash
cargo install --git https://github.com/trail-openers/lievo.git
```

## Connect your agent

Register lievo once, at user level — one line per agent, and it works in every git repository (Cursor is the exception; see below). `lievo mcp` resolves the repository it is launched in, registers it on first use, and indexes it automatically in the background.

Lievo works alongside your agent's built-in search and read tools; it does not replace them.

### Claude Code

```bash
claude mcp add lievo --scope user -- lievo mcp
```

Claude Code launches the server in your project directory and sets `CLAUDE_PROJECT_DIR`, which lievo reads — no further configuration. Remove with `claude mcp remove lievo`.

### Codex CLI

```bash
codex mcp add lievo -- lievo mcp
```

Writes `[mcp_servers.lievo]` to `~/.codex/config.toml`. Codex launches the server from the directory you run Codex in, so start Codex from inside your repo. To pin one repo instead, register with `codex mcp add lievo --env LIEVO_PROJECT_DIR=/path/to/repo -- lievo mcp`; `LIEVO_PROJECT_DIR` wins over everything else. Remove with `codex mcp remove lievo`.

### VS Code (agent mode)

```bash
code --add-mcp '{"name":"lievo","command":"lievo","args":["mcp"]}'
```

That adds lievo to your VS Code user profile, available in every workspace. The server's working directory defaults to the workspace folder, which is what lievo resolves from. Inside VS Code, use `MCP: List Servers` to start and manage it, or open the user `mcp.json` via `MCP: Open User Configuration` and edit. If your agent runs outside a workspace (or you want one fixed repo), add `"cwd": "/path/to/repo"` to the server entry.

### Cursor

Cursor is the one client that needs the repo named explicitly. It does not start MCP servers in the open workspace folder, and it has no `cwd` setting or workspace variable to pass it. So register lievo per repo, in that repo's `.cursor/mcp.json`, with the repo's absolute path:

```json
{
  "mcpServers": {
    "lievo": {
      "command": "lievo",
      "args": ["mcp"],
      "env": { "LIEVO_PROJECT_DIR": "/path/to/repo" }
    }
  }
}
```

Restart Cursor, or enable the server from **Customize**. To check the path is right, run `lievo doctor /path/to/repo`.

### pi

pi has no built-in MCP support; use the [pi-mcp-adapter](https://www.npmjs.com/package/pi-mcp-adapter) package. Add an `mcpServers` entry to the shared user-global config `~/.config/mcp/mcp.json` (the project-local `.mcp.json` works too, but the user file is what makes one registration cover every repo):

```json
{
  "mcpServers": {
    "lievo": { "command": "lievo", "args": ["mcp"] }
  }
}
```

pi launches the server from the directory you start pi in, so start pi from inside your repo. If that is not possible for you, add `"cwd": "/path/to/repo"` to the entry — it pins the working directory for every session.

## First use

Open your repo in your agent and ask a structural question — "where is X implemented?", "what depends on this file?".

- On first use, lievo resolves and registers the repo automatically and starts the core structural index in the background. The first index takes seconds for most repos (longer for very large ones); while it builds, `lievo_explore` answers "indexing in progress" — the agent keeps using its built-in tools and retrying shortly.
- After that, indexing stays incremental and automatic on every agent start, and it builds only the core structural index.
- Summaries and the semantic (vector) index come from a manual `lievo refresh`; the semantic index powers `lievo query entities --semantic` and the `search_entities` tool — not `lievo_explore`.
- `LIEVO_NO_REFRESH=1` disables the automatic indexing; run `lievo refresh` manually instead.

## Where data lives

- `~/.lievo/` — database (`lievo.db`) and per-repo indices; delete the directory to reset lievo completely.
- `LIEVO_DB=/path/to/db` — point the database somewhere else. When it is set, the database lives there rather than in `~/.lievo/`, so delete that file too to reset lievo. `lievo doctor` shows the path actually in use.

## Troubleshooting

When something is off, run lievo from the repository and ask it to check:

```bash
lievo doctor            # human report
lievo doctor --format json
```

`lievo doctor` is read-only. It reports whether lievo will work in that directory — the resolved repo and its source, registration, index state, and where the data lives — and each problem line names the single command that fixes it. It creates and deletes a temporary file in the data directory to test that the directory is writable, but it does not change your index or your registrations. It also diagnoses two states it can only name, not fix: a database that cannot be opened (corrupt or locked), and a repo with no commits yet. It exits 0 when lievo will work here and 1 when it will not.

Two things worth checking when the agent reads the wrong repo:

- If `LIEVO_PROJECT_DIR` is set, it wins over the client's working directory — unset it or point it at the right repo.
- If `lievo mcp <project>` fails with `project has multiple repos; MCP server requires a single-repo project`, that project has more than one registered repository. Reduce it to one (e.g. `lievo admin delete-repo <name> <project>`), or register a fresh single-repo project with `lievo admin add-repo <path>` and use that project name.

To report a bug, open an issue on the [bug report template](https://github.com/trail-openers/lievo/issues/new?template=bug_report.yml) and paste the full `lievo doctor` output (the template has a field for it).

## Uninstall

- Claude Code: `claude mcp remove lievo`
- Codex CLI: `codex mcp remove lievo`
- VS Code: `MCP: List Servers`, select lievo, uninstall
- Cursor: delete the `"lievo"` entry from each repo's `.cursor/mcp.json`
- pi: delete the `"lievo"` entry from `~/.config/mcp/mcp.json`

Then remove the `lievo` binary (delete it from wherever the installer placed it) and, optionally, its data:

```bash
rm -rf ~/.lievo           # optional: delete all lievo data
```
