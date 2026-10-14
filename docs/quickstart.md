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

## Project names

Each repository is registered under a project name. lievo picks it for you when you register a repo, and you can set it explicitly.

Automatic names (no explicit name) use the first of these that is free:

- the checkout's directory name (usually the repo name);
- `<owner>/<repo>`, from the remote identity, when the directory name is already a project name;
- `<owner>/<repo>-2`, `<owner>/<repo>-3`, … when that is taken too.

A repo with no remote identity (no origin lievo can normalise and no `identity:` key) is named from its directory, with `-2`, `-3`, … suffixes when that is taken.

A second live checkout of a remote that is already registered gets its own project named `<repo>@<dir>` (for example `lievo@feature-x`), even when its directory name is free, so the two checkouts do not share a project. A later live checkout with the same directory name joins that `<repo>@<dir>` project (if that name is taken by another identity, the next free suffix is used).

Explicit names (`lievo admin add-repo <path> <project>`) are used as given:

- the name is free → a new project;
- Refused if the named project holds any repo with a different remote identity, which covers mixed projects (only when the new checkout has an identity); pick another name or register the path under its own project;
- the name is held by the same remote, or by a repo with no stored identity → the checkout joins that project as an additional repo.

The name is used as given (surrounding whitespace is trimmed).

Without an argument, `lievo mcp` registers the repository it is started in under a derived name. With `<project>`, it serves that already-registered project and registers nothing.

To set the remote identity lievo uses for a repo yourself, add an `identity:` key to `<repo>/.lievo/config.yaml` in the form `host/owner/repo`:

```yaml
identity: github.com/trail-openers/lievo
```

The `identity:` key is read from the checkout itself, so a committed config in a repository you don't control can make it join an existing project under that identity. It never moves an existing registration, but treat repo-supplied `identity:` values as untrusted. See [Known limitations](data-storage.md#known-limitations).

When a checkout has moved, lievo does one of the following:

- automatically named, identity from the origin remote, old path gone → the existing registration's path is updated and its project, index and status are kept (no full reindex);
- an explicit project name, or an `identity:` override → no move (the explicit-name rules above apply, or a fresh project is created);
- the move can't proceed (e.g. the index is in use by another process) → a notice is printed, the original registration is left unchanged, and the path is registered as a new project;
- the state is unclear (unknown database row state, or index files left split or leftover at the new path) → an error with recovery steps, nothing indexed. Follow the printed recovery steps, then re-run.

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
