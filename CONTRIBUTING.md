# Contributing to Lievo

Thanks for your interest in contributing to Lievo!

## Contributor License Agreement (CLA)

Before your first pull request is merged, you must sign the [Contributor License Agreement](CLA.md). The CLA explains the two forms (individual and company/entity) and what each grants.

**Why it exists:** Lievo is licensed AGPL-3.0. To stay able to offer additional terms — for example, a commercial licence for enterprises that cannot or do not want to comply with the AGPL — the maintainers need a grant that covers every contribution, including the right to relicense. The CLA preserves your copyright; it grants Janni Turunen and Trail Openers Oy a perpetual, irrevocable licence to use, modify, and relicense your contribution under any terms (including commercial ones), plus a patent grant. See [CLA.md](CLA.md) for the full text.

Signing is automated: when you open a pull request, the CLA Assistant bot checks your signature and tells you how to sign (either via the GitHub UI flow or by commenting on the PR). Signed contributions are recorded in the repository.

> Note: the CLA text is subject to legal review before the first outside contribution is merged.

## Development Workflow

1. Read the spec: `docs/spec.md` (especially the appendices)
2. Check the GitHub issue for requirements
3. Create a feature branch: `feature/issue-N-brief-description`
4. Implement with TDD approach
5. Run quality gates locally
6. Commit with conventional commit format
7. Push and create a PR

## Quality Gates

Before committing, all three gates must pass:

```bash
# Format check (must pass)
cargo fmt --check

# Lint with all warnings as errors (must pass)
cargo clippy -- -D warnings

# Run all tests (must pass)
cargo test
```

No exceptions. If a gate fails, fix it before committing.

## Conventional Commits

Use conventional commit format:

- **feat**: New features or capabilities
- **fix**: Bug fixes
- **docs**: Documentation changes only
- **refactor**: Code restructuring without behavior change
- **test**: Test additions or modifications
- **chore**: Maintenance tasks, dependency updates

Format: `feat(#123): brief description`

Always include the issue number after the commit type.

## Branch Naming

- Branches: `feature/issue-NUMBER-brief-description`
- Example: `feature/issue-21-error-handling`

## Code Style

Lievo follows the Rust systems philosophy (see AGENTS.md):

- **Std-first**: Prefer standard library over external crates
- **Zero-cost abstractions**: No runtime overhead for abstractions
- **Memory safety**: Safe Rust first, unsafe only when profiled and necessary
- **Minimal dependencies**: Every crate is a liability
- **&str > String**: Use string slices when ownership isn't needed

## Testing

- **Unit tests**: Tests for individual modules
- **Integration tests**: End-to-end tests against tree-sitter extraction indexes
- **Fixture tests**: Test codebases in `tests/fixtures/`
- **Coverage**: Target 80%+ for new code

## Documentation

- Public API: Use `///` doc comments
- Complex algorithms: Add inline comments explaining the approach
- CLI commands: Include examples in help text

## Issue Splitting

When splitting work from `docs/spec.md` into issues:

- Each issue should be 1-3 days of focused work
- If an issue is too big, split it further
- Mark dependencies explicitly: `Depends on: #NNN`, `Blocks: #NNN`
- Define the interface contract in each issue

## When in Doubt

1. Read the spec: `docs/spec.md` (especially the appendices)
2. Check the GitHub issue for the specific requirement
3. Keep it simple — prefer straightforward solutions over clever ones

## Releasing

lievo is released as prebuilt binaries via cargo-dist. A release is triggered by a semver tag (e.g. `v0.1.0`) pushed by the operator; cargo-dist only publishes a tag whose version matches the package version in `Cargo.toml`, so the version must be bumped first.

A release candidate: PR bumping `Cargo.toml` (and `Cargo.lock`) to `X.Y.Z-rc.N`, and the `## lievo vX.Y.Z` heading in `THIRD_PARTY_NOTICES.md` (or regenerate the file, see below) → merge → tag `vX.Y.Z-rc.N` (cargo-dist builds a GitHub pre-release). The final release: PR bumping to `X.Y.Z` and setting the date on the `## [X.Y.Z]` heading in `CHANGELOG.md` → merge → tag `vX.Y.Z`. Note that `releases/latest/download/` installer URLs in the docs only start working once the first non-prerelease is out.

### Regenerating third-party notices

`THIRD_PARTY_NOTICES.md` lists every dependency's licence and the notice texts the AGPL requires you to carry. It is generated on demand (the tool is not a crate dependency):

```bash
# requires: cargo install cargo-about --features cli
cargo about generate THIRD_PARTY_NOTICES.md.hbs -o THIRD_PARTY_NOTICES.md
```

`cargo-about` reads licence metadata from crates.io; where a crate's metadata is missing but the crate ships a licence file (e.g. `model2vec` is MIT), the notices file carries a hand-recorded entry — keep those entries current when the dependency set changes. The notices file must be committed and shipped in the release archives.

### Dry run (pre-release)

1. Push the pre-release tag (after the version bump above merged): `git tag v0.1.0-rc.1 && git push origin v0.1.0-rc.1`
2. Watch the GitHub Actions run — the `plan` job validates the config, then all build jobs run in parallel.
3. Verify the GitHub pre-release contains: archives for all 5 targets, `sha256.sum`, `lievo-installer.sh`, `lievo-installer.ps1`.
4. Smoke test the published shell installer on a clean macOS and a clean Linux machine (see below).

### Smoke test (copy-paste commands)

On a clean macOS (Apple Silicon or Intel):
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/trail-openers/lievo/releases/download/v0.1.0-rc.1/lievo-installer.sh | sh
lievo --version
# Verify MCP responds to an initialize request:
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-test","version":"1.0.0"}}}' | timeout 10 lievo mcp 2>/dev/null | head -1
```

On a clean Linux (x86_64 or ARM):
```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/trail-openers/lievo/releases/download/v0.1.0-rc.1/lievo-installer.sh | sh
lievo --version
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-test","version":"1.0.0"}}}' | timeout 10 lievo mcp 2>/dev/null | head -1
```

Both should print `lievo` followed by the version in `Cargo.toml` (`lievo 0.1.0-rc.1` for this dry run) and a JSON-RPC response with `serverInfo`.

### Cutting the real release

1. Confirm the dry run succeeded and the smoke test passed.
2. **Bump to the final version**: PR bumping `Cargo.toml` (and `Cargo.lock`) from `0.1.0-rc.1` to `0.1.0`, and the `## lievo` heading in `THIRD_PARTY_NOTICES.md` → merge.
3. **Set the changelog date**: in `CHANGELOG.md`, replace the `- Unreleased` placeholder on the `## [0.1.0]` heading with today's date in `YYYY-MM-DD` form, leaving the rest of the entry untouched. Commit this in a normal PR and merge it — do **not** do this in the release commit itself.
4. From the merge commit on `main`, push the final tag: `git tag v0.1.0 && git push origin v0.1.0`. The tag must point at the merge commit that carries the dated changelog, so the GitHub Release body renders the correct version section.
5. The release workflow creates a GitHub release with all artifacts.

### Runner allocation per target

All five build targets run on GitHub-hosted runners (see `dist plan` for the current allocation). The exact runner per target is set by cargo-dist in `.github/workflows/release.yml`, which is regenerated with `dist generate` — never hand-edit it.

| Target | Runner (GitHub-hosted) |
|---|---|
| aarch64-apple-darwin | macOS runner |
| x86_64-apple-darwin | macOS runner (Intel) |
| x86_64-unknown-linux-gnu | Ubuntu runner |
| aarch64-unknown-linux-gnu | Ubuntu ARM runner |
| x86_64-pc-windows-msvc | Windows runner |

As a public repository, GitHub-hosted minutes are free within the plan; there is no custom/self-hosted runner component.

## Questions?

Open an issue or start a discussion on GitHub.
