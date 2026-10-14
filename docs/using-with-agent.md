# Using lievo with your agent

You've installed lievo and registered it with an agent. This page finishes the job: it makes the agent *use* lievo well, not just reach for it. It covers the four agents lievo supports — **Claude Code, pi, Hermes Agent, and Codex CLI** — and the one file that ties it together, the **lievo skill**.

## The two parts

An agent needs two things to exploit lievo's map:

1. **The MCP server** (`lievo mcp`) — gives the agent lievo's *tools* (`lievo_explore` by default). This is how the agent talks to lievo.
2. **The lievo skill** (`skills/lievo/`) — teaches the agent *how* to use those tools: the query loop, the JSON output contracts, the summaries vs no-summaries distinction, and the pitfalls.

Register the server once, import the skill once, and every repo the agent works in benefits. The MCP server is the interface; the skill is the training. Install both.

> **What the skill contains** — a query loop (`doctor` → `subsystems` → `entities` → `entity`/`deps`/`dependents` → `impact`), the full CLI + MCP command map, the verified JSON output contracts, the summaries distinction, the `LIEVO_*` environment variables, and the pitfalls. The skill's `SKILL.md` is the source of truth for all of it; this page only covers *installing* it per agent, so the two never drift.

## Register the MCP server

`lievo mcp` resolves the repository it is launched in, registers it on first use, and indexes in the background. One command per agent; remove by reversing it.

### Claude Code

```bash
claude mcp add lievo --scope user -- lievo mcp     # add
claude mcp remove lievo                              # remove
```

Claude Code sets `CLAUDE_PROJECT_DIR`, which lievo reads — no further configuration.

### Codex CLI

```bash
codex mcp add lievo -- lievo mcp                     # add
codex mcp remove lievo                                # remove
```

Writes `[mcp_servers.lievo]` to `~/.codex/config.toml`. Start Codex from inside the repo. To pin one repo instead of following the working directory:

```bash
codex mcp add lievo --env LIEVO_PROJECT_DIR=/path/to/repo -- lievo mcp
```

### pi

pi has no native MCP; it talks to lievo through the [pi-mcp-adapter](https://www.npmjs.com/package/pi-mcp-adapter) package. Add an `mcpServers` entry to the shared user-global config so one registration covers every repo:

```jsonc
// ~/.config/mcp/mcp.json
{
  "mcpServers": {
    "lievo": { "command": "lievo", "args": ["mcp"] }
  }
}
```

Start pi from inside the repo, or pin the working directory by adding `"cwd": "/path/to/repo"` to the entry.

### Cursor (for completeness)

Cursor does not start MCP servers in the workspace folder and has no `cwd` variable, so the repo path must be named explicitly, per repo, in `.cursor/mcp.json` with `LIEVO_PROJECT_DIR`. See [quickstart.md](quickstart.md#cursor).

## Install the lievo skill

The skill lives at `skills/lievo/` in this repository. Copy the directory into the agent's skill location.

### Claude Code

```bash
cp -r skills/lievo ~/.claude/skills/        # user-wide
# or, project-local (committed with the repo):
cp -r skills/lievo .claude/skills/
```

Claude Code loads it automatically when the task matches the skill's name and description.

### Hermes Agent

```bash
cp -r skills/lievo ~/.hermes/skills/
```

It is then available to any session via the skill loader.

### pi

pi has no first-class skill loader. Two options, in order of preference:

1. **Include the skill in the agent's instructions.** Reference `skills/lievo/SKILL.md` from a project instruction file so the agent reads it at task start.
2. **Rely on the MCP tools alone.** `lievo_explore`'s own tool description carries the core usage guidance (batch files into one call, the `scope`/`files`/`bundle` modes, the continuation contract). The skill makes the agent *reliably* pick the right query and parse the JSON; without it the agent may still reach for grep first.

### Codex CLI

Codex has no first-class skill loader either. As with pi, include `skills/lievo/SKILL.md` in `AGENTS.md` or another project instructions file, or rely on the MCP tools. The skill's command map is exactly what `AGENTS.md` would otherwise have to repeat.

## Verify

From inside a registered repo:

```bash
lievo doctor          # expect: registered: true, index: current
lievo admin status    # expect: entity counts + a "Summary coverage:" line
```

Then ask the agent a structural question — *"what depends on `src/storage/sqlite.rs`?"* — and confirm it calls lievo rather than only grepping.

**Reading the right repository.** If the agent reads the wrong repo, check `LIEVO_PROJECT_DIR` (it wins over the working directory) and that the project is single-repo; `lievo doctor` names the fix for each case.
