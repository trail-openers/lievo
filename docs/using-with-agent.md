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

Get the skill from the repository, not from a lievo checkout, so the steps work anywhere. Clone the repo to a temporary directory. This uses `main`. Until the skill is merged to `main`, add `--branch feature/issue-42-add-agent-skill`.

```bash
git clone --depth 1 https://github.com/trail-openers/lievo /tmp/lievo-src
```

Then copy `skills/lievo` into your agent's skill location.

**Claude Code** (user-wide):
```bash
mkdir -p ~/.claude/skills && cp -R /tmp/lievo-src/skills/lievo ~/.claude/skills/
```
Claude Code loads the skill when a task matches its name and description. For a project-local copy, use `.claude/skills/` in the repo root instead.

**pi** (pi scans `~/.agents/skills/`, per pi's `docs/skills.md`):
```bash
mkdir -p ~/.agents/skills && cp -R /tmp/lievo-src/skills/lievo ~/.agents/skills/
```

**Hermes Agent**:
```bash
mkdir -p ~/.hermes/skills && cp -R /tmp/lievo-src/skills/lievo ~/.hermes/skills/
```
See [Hermes Agent](#hermes-agent) for Hermes-specific notes.

### Agents without a verified skill loader

For Codex CLI, Cursor, VS Code, or any agent whose skill loading is not verified here, copy the skill to a stable path and reference that path from the agent's instructions file. Do not reference `skills/lievo/` relative to your project, because that directory does not exist in your project.

```bash
mkdir -p ~/.local/share/lievo && cp -R /tmp/lievo-src/skills/lievo ~/.local/share/
```

Then add a line to your instructions file, such as `AGENTS.md`:

```markdown
For structural code questions (what depends on X, what breaks if Y changes), follow ~/.local/share/lievo/SKILL.md.
```

Re-run the clone and copy to update the skill.

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

- **Skill:** install it as shown above. The skill is written in agent-neutral terms ("shell", "file search", "file read"). In Hermes those correspond to the `terminal`, `search_files`, and `read_file` tools (the names the skill's earlier Hermes version used; not re-verified here).
- **Frontmatter:** the shipped `SKILL.md` has no Hermes-only metadata (`metadata.hermes`, `related_skills`, `platforms`). Hermes does not need it to load the skill. If you want Hermes tags or related skills, add them to your installed copy.
- **MCP:** register `lievo mcp` with Hermes following its MCP documentation. That registration is not verified here.

## Verify

From inside a registered repo:

```bash
lievo doctor          # expect: registered: true, index: current
lievo admin status    # expect: entity counts and a "Summary coverage:" line
```

Then ask the agent a structural question, such as "what depends on `src/storage/sqlite.rs`?", and confirm it calls lievo rather than only searching files.

**Reading the right repository.** If the agent reads the wrong repo, check `LIEVO_PROJECT_DIR` (it wins over the working directory). `lievo doctor` names the fix for each case.

For measured evidence on when lievo helps, see [benchmarks.md](benchmarks.md).
