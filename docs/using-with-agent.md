# Using lievo with your agent

An agent gets the most from lievo with two parts:

1. **The MCP server** (`lievo mcp`) provides lievo's tools. Registration commands per client are in [quickstart.md](quickstart.md#connect-your-agent). They are the single source for registration; this page does not repeat them.
2. **The lievo skill** (`skills/lievo/`) teaches the agent how to use those tools. It covers the query loop, the CLI and MCP command maps, the JSON output contracts, the summaries distinction, and the pitfalls. Its `SKILL.md` is the source of truth; this page covers installing it.

Register the server once and install the skill once. Every repo the agent works in then benefits.

| Agent | MCP registration | Skill location |
|---|---|---|
| Claude Code | [quickstart](quickstart.md#claude-code) | `~/.claude/skills/lievo/` (user) or `.claude/skills/lievo/` (project) |
| pi | [quickstart](quickstart.md#pi) | `~/.agents/skills/lievo/` |
| Codex CLI | [quickstart](quickstart.md#codex-cli) | No verified skill loader; see [the fallback](#agents-without-a-verified-skill-loader) |
| Hermes Agent | Per Hermes's MCP documentation (not verified here) | `~/.hermes/skills/lievo/` |
| Cursor, VS Code | [quickstart](quickstart.md#cursor) / [quickstart](quickstart.md#vs-code-agent-mode) | MCP only; no verified skill loader |

## Install the skill

Install from the repository, not from a lievo checkout, so the steps work anywhere. The command clones into a fresh temporary directory, copies `skills/lievo` to your agent's destination, and deletes the clone. Each step runs only if the previous one succeeded. Replace `<dest>` with the destination from the list below:

```bash
src=$(mktemp -d) && git clone --depth 1 https://github.com/trail-openers/lievo "$src" && mkdir -p <dest> && cp -R "$src/skills/lievo/." <dest>/ && rm -rf "$src"
```

Destinations:

- **Claude Code:** `~/.claude/skills/lievo` (user-wide), or `.claude/skills/lievo` in the repo root (project-local). Claude Code loads the skill when a task matches its name and description.
- **pi:** `~/.agents/skills/lievo` (pi scans `~/.agents/skills/`, per pi's `docs/skills.md`).
- **Hermes Agent:** `~/.hermes/skills/lievo`.
- **Agents without a verified skill loader:** `~/.local/share/lievo`, see [the fallback](#agents-without-a-verified-skill-loader).

To update, re-run the same command. To pin a fixed version, add `--branch vX.Y.Z` to the `git clone` once a release includes the skill.

### Agents without a verified skill loader

For Codex CLI, Cursor, VS Code, or any agent whose skill loading is not verified here, install to `~/.local/share/lievo` with the command above, then reference that path from the agent's instructions file. Do not reference `skills/lievo/` relative to your project, because that directory does not exist in your project.

Add a line to your instructions file, such as `AGENTS.md`:

```markdown
For structural code questions (what depends on X, what breaks if Y changes), follow ~/.local/share/lievo/SKILL.md.
```

## Per-agent notes

### Claude Code

Claude Code sets `CLAUDE_PROJECT_DIR`, which lievo reads, so the MCP server needs no extra configuration.

### pi

- **MCP:** pi ships built-in MCP support. [quickstart](quickstart.md#pi) has the registration command, and pi's own `docs/mcp.md` describes the config. Pi names lievo's tools `mcp__lievo__<tool>`. By default pi reaches MCP tools through codemode, not as directly declared tools.
- **Third-party alternative:** [pi-mcp-adapter](https://www.npmjs.com/package/pi-mcp-adapter) is a separate npm package with its own config format. Its docs are the source of truth for that config. Use one of the two options, not both.
- **Skill:** `~/.agents/skills/lievo/` (see above).

### Codex CLI

Start Codex from inside the repo, or pin the repo with `LIEVO_PROJECT_DIR` as described in [quickstart](quickstart.md#codex-cli). Codex has no skill loader verified here, so use the [fallback](#agents-without-a-verified-skill-loader).

### Hermes Agent

For Hermes's skill and MCP setup, see Hermes Agent's own documentation. The skill destination is listed above.

## Automatic indexing and repo config

`lievo mcp` starts a background index when a repo is new or its HEAD has moved. That background indexing never summarizes, so it never contacts a repo-configured endpoint. Only an explicit run can: `lievo refresh` without `--no-summarize`, or `lievo summarize`. Set `LIEVO_NO_REFRESH=1` to disable automatic indexing entirely.

A repo's `.lievo/config.yaml` is inside the checkout, so treat it as untrusted for repos you don't control. See [the security notes](../skills/lievo/references/notes.md#configuration).

## Verify

From inside a registered repo:

```bash
lievo doctor          # expect: registered: true, index: current
lievo admin status    # expect: entity counts and a "Summary coverage:" line
```

Then ask the agent a structural question, such as "what depends on `src/storage/sqlite.rs`?", and confirm it calls lievo rather than only searching files.

**Reading the right repository.** If the agent reads the wrong repo, check `LIEVO_PROJECT_DIR` (it wins over the working directory). `lievo doctor` names the fix for each case.

For measured evidence on when lievo helps, see [benchmarks.md](benchmarks.md).
