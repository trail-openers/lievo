# CLI Architecture

Lievo's CLI is built for agent-first interaction while supporting human readability. The architecture follows the principle that explicit override precedence and deterministic behavior are more important than implicit detection.

## Command Organization

Admin commands (project/repo lifecycle, status, info) live under `lievo admin`. Daily workflow commands (query, mcp, refresh) are top-level. This keeps the top-level surface minimal and reduces cognitive load.

## Format Selection Precedence

The CLI uses a two-layer format selection strategy:

1. **Explicit override (`--format` flag)** — Always wins, regardless of environment
2. **Default fallback** — `OutputFormat::Human` when no explicit override is provided

**Why this design**: Explicit args take precedence over terminal detection. This ensures scripts and agents can control output deterministically without fighting TTY auto-detection heuristics.

Example precedence:
```bash
lievo query entities --project myapp "auth"              # Human (default)
lievo query entities --project myapp --format json "auth" # JSON (explicit wins)
```

## TTY vs Non-TTY Behavior

**Current implementation**: Lievo does not perform TTY detection for format selection. The output format is always determined by the `--format` flag or the default (`OutputFormat::Human`), not by pipe presence.

**Why this design**: TTY detection creates unpredictable behavior in scripts and nested command environments. A command should behave identically whether run interactively or piped to `jq`. Format control belongs in the user's shell command, not in the CLI's heuristics.

**Forward compatibility**: Future error output enhancements (#399) will introduce structured stderr for machine-readable error handling, but will maintain the same stdout format contract. TTY detection will not be added for format selection — this is a deliberate design choice for deterministic behavior.

## Exit Code Contract

The CLI follows a strict exit code contract:

- **`0`** — Success, data written to stdout
- **Non-zero** — Failure, error message to stderr

All errors return non-zero exit codes, including:
- Database errors
- Entity/project not found
- Parse errors
- Configuration errors

**Principle**: Exit codes are binary (success/failure). Error type and context go to stderr, not to exit codes.

## Error Handling Pattern

Errors are surfaced at the CLI boundary via the `LievoError` enum. The main function in `lievo.rs` catches `Result` errors and formats them for stderr:

```rust
if let Err(e) = result {
    eprintln!("Error: {e}");
    process::exit(1);
}
```

**Why `eprintln!`**: Separates diagnostic information from data stream. Tools like `jq` receive only valid JSON or human-readable output on stdout.

## Command Routing

The CLI uses `clap`'s derive API with `Parser` and `Subcommand` traits:

```rust
#[derive(Parser)]
struct Cli {
    #[arg(long, global = true)]
    format: Option<OutputFormat>,

    #[command(subcommand)]
    command: Commands,
}
```

The `global = true` flag on `format` allows it to be used with any subcommand without repetition.

## Query Pattern

Query commands share a common pattern:

1. Accept `format: OutputFormat` parameter
2. Call storage layer to fetch data
3. Route to formatter (`format_*_human` or `format_*_json`)
4. Print formatted string to stdout

All formatting is pure functions returning `String`. No I/O occurs in the formatting layer. This makes testing straightforward and keeps separation of concerns.

See [CLI Output Contracts](cli-output-contracts.md) for details on the format contract.