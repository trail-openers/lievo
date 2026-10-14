# Contributing to Lievo

Thanks for your interest in contributing to Lievo!

## Contributor License Agreement (CLA)

Before your first pull request is merged, you must sign the [Contributor License Agreement](CLA.md). The CLA explains the two forms (individual and company/entity) and what each grants.

**Why it exists:** Lievo is licensed AGPL-3.0. To stay able to offer additional terms — for example, a commercial licence for enterprises that cannot or do not want to comply with the AGPL — the maintainers need a grant that covers every contribution, including the right to relicense. The CLA preserves your copyright; it grants Janni Turunen and Trail Openers Oy a perpetual, irrevocable licence to use, modify, and relicense your contribution under any terms (including commercial ones), plus a patent grant. See [CLA.md](CLA.md) for the full text.

Signing is automated: when you open a pull request, the CLA Assistant bot checks your signature and tells you how to sign (either via the GitHub UI flow or by commenting on the PR). Your signature record — your GitHub username and the time you signed — is kept in a private repository for data-protection reasons.

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
- **ci**: CI configuration

Format: `feat: brief description (#123)`, or `feat: brief description` with a `Closes #123` trailer in the body.

**Do not put the issue number in the scope** (`feat(#123):` is wrong). release-plz and git-cliff build the changelog from commit subjects, and numeric scopes get mangled in the generated `CHANGELOG.md`. Reference the issue at the end of the subject or in the body.

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

Releases are automated with [release-plz](https://release-plz.dev) and built by cargo-dist. Never bump the version in `Cargo.toml` or edit `CHANGELOG.md` by hand.

1. **Merge feature PRs to `main`** with Conventional Commit subjects (see above). Every push to `main` runs the `Release-plz` workflow (`.github/workflows/release-plz.yml`).
2. **release-plz opens or updates a release PR** (`release-pr` job). It bumps `Cargo.toml` and `Cargo.lock` from the commits since the last tag (`release-plz.toml`: `feat` bumps minor, `fix` bumps patch while the crate is 0.x) and prepends the generated section to `CHANGELOG.md` (git-cliff, `cliff.toml`).
3. **Merge the release PR.** The `release` job publishes the crate to crates.io and pushes the `v<version>` tag (no GitHub Release from release-plz; `git_release_enable = false`).
4. **cargo-dist builds the GitHub Release.** The `v*` tag triggers `.github/workflows/release.yml`, which builds the binaries and installers for all five targets and creates the GitHub Release.

Rules:

- Do not hand-edit `CHANGELOG.md` or the version. release-plz regenerates both from `main` on every push.
- Do not leave the release PR open while other PRs merge. Each merge to `main` rewrites the release PR, so review and merge its final state promptly.
- The first release-plz release is `0.2.0`, derived from the existing `v0.1.0` tag. release-plz must not re-tag or re-publish `0.1.0`; check that the release PR proposes `0.2.0` before merging it.
- release-plz does not update the `## lievo vX.Y.Z` heading in `THIRD_PARTY_NOTICES.md`. Update it on the release PR branch before merging (see "Regenerating third-party notices").

### One-time setup (operator)

These steps are done once by a maintainer with admin rights on the repository. They are not automated.

1. **crates.io token**: create an API token with the `publish-new` and `publish-update` scopes, and store it as the repository secret `CARGO_REGISTRY_TOKEN`. This is token-based publishing, not trusted publishing (OIDC).
2. **GitHub token for release-plz**: create a fine-grained token with `Contents: read and write` and `Pull requests: read and write` on this repository, and store it as the repository secret `RELEASE_PLZ_TOKEN`. The default `GITHUB_TOKEN` cannot be used: tags and PRs it creates do not trigger other workflows, so the `v*` tag would never start `release.yml`. (Alternatively, point the workflow at the existing `PERSONAL_ACCESS_TOKEN` secret.)
3. **Allow Actions to open PRs**: in Settings → Actions → General → Workflow permissions, enable "Allow GitHub Actions to create and approve pull requests".

### Regenerating third-party notices

`THIRD_PARTY_NOTICES.md` lists every dependency's licence and the notice texts the AGPL requires you to carry. It is generated on demand (the tool is not a crate dependency):

```bash
# requires: cargo install cargo-about --features cli
cargo about generate THIRD_PARTY_NOTICES.md.hbs -o THIRD_PARTY_NOTICES.md
```

`cargo-about` reads licence metadata from crates.io; where a crate's metadata is missing but the crate ships a licence file (e.g. `model2vec` is MIT), the notices file carries a hand-recorded entry — keep those entries current when the dependency set changes. The notices file must be committed and shipped in the release archives.

The file also carries two hand-maintained sections that `cargo-about` does not generate:

- **option-ext source pointer** — identifies the MPL-2.0 covered source (crate name, version, crates.io page, repository URL) so the file-level copyleft obligation is locatable.
- **Bundled native components** — the licences of C/C++ sources that `-sys` crates bundle under their own (non-crate) licence: libgit2 (GPL-2.0 with linking exception, via `libgit2-sys`), Oniguruma (2-clause BSD-style, via `onig_sys`), zlib (via `libz-sys`), SQLite (public domain, via `libsqlite3-sys`), and one-line Apache-2.0 notes for `usearch` and `numkong` (no NOTICE file ships). Each entry names the `-sys` crate and version that ships the bundled code.

**When a `-sys` crate is upgraded** (libgit2-sys, onig_sys, libz-sys, libsqlite3-sys, or any new one), re-verify the bundled-native section against the new registry source (`~/.cargo/registry/src/<registry>/<crate>-<version>/...`): the bundled code's licence may have changed, and the `-sys` crate's own crates.io metadata (which `cargo-about` reads) does not reflect the bundled code's licence. Also confirm the OpenSSL-headers statement in that section still holds for the new libgit2-sys version (it is only compiled when `git2`'s `https` feature is enabled on Linux, which lievo does not enable).

The `## lievo vX.Y.Z` heading in the file carries the current version. Bump it on the release PR branch before merging the release PR.

### Smoke test after a release

Once the GitHub Release is published, smoke test the published shell installer on a clean macOS and a clean Linux machine:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/trail-openers/lievo/releases/latest/download/lievo-installer.sh | sh
lievo --version
# Verify MCP responds to an initialize request:
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke-test","version":"1.0.0"}}}' | timeout 10 lievo mcp 2>/dev/null | head -1
```

Both should print `lievo` followed by the released version (e.g. `lievo 0.2.0`) and a JSON-RPC response with `serverInfo`.

### Runner allocation per target

All five build targets run on GitHub-hosted runners (see `dist plan` for the current allocation). The exact runner per target is set by cargo-dist in `.github/workflows/release.yml`, which is regenerated with `dist generate` (cargo-dist version pinned in `dist-workspace.toml`) — never hand-edit it.

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
