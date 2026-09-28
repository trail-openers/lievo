# Contributor Guidelines for lievo

This document establishes quality standards, workflow expectations, and project-specific constraints for all development work on lievo. For the contributor onboarding workflow (CLA, releases, smoke tests), see [CONTRIBUTING.md](CONTRIBUTING.md).

---

## 1. Minimalist Engineering Philosophy

**CORE PRINCIPLE: Every line of code is a liability.**

Before creating anything, ask these questions:

1. **Is this explicitly required** by the GitHub issue?
2. **Can existing code/tools** solve this instead?
3. **What's the SIMPLEST** way to meet the requirement?
4. **Will removing this break** core functionality?
5. **Am I building for hypothetical** future needs?

**If you cannot justify the necessity, DO NOT CREATE IT.**

### Design Principles

- **One Purpose Per Component**: Each function/module should do one thing well
- **No Speculative Features**: Don't build for "future needs" — solve today's problem
- **Prefer Existing**: Reuse stdlib and existing code before creating new abstractions
- **Challenge Everything**: Question every new addition

### Forbidden Patterns

- ❌ "Nice to have" features not in the issue
- ❌ Premature abstractions
- ❌ Utility functions that appear once
- ❌ Defensive coding for unlikely scenarios
- ❌ Over-engineered solutions to simple problems

---

## 2. Pre-Push Quality Gates

**CI is for VERIFICATION, not DISCOVERY.**

All checks MUST pass locally before `git push`. Never push to "see if CI catches anything." Fix issues locally first.

### Required Checks

Run all checks together before pushing:

```bash
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

Breaking it down:

```bash
# Code formatting check (no changes, just verify)
cargo fmt --check

# Linting with all warnings as errors
cargo clippy -- -D warnings

# All tests must pass
cargo test
```

### Workflow Before Push

1. Make changes locally
2. Run `cargo test` — all tests pass ✓
3. Run `cargo clippy -- -D warnings` — zero warnings ✓
4. Run `cargo fmt --check` — properly formatted ✓
5. `git push` (now safe)

### If Any Check Fails

- ❌ Fix the issue locally
- ❌ Re-run the failing check
- ❌ Verify all checks pass before pushing

Do NOT:
- ❌ Use `--no-verify` to skip checks
- ❌ Commit formatting fixes as separate commits
- ❌ Suppress warnings with `#[allow(...)]`
- ❌ Push with failing tests

---

## 3. Testing Standards

### Test-Driven Development Encouraged

- Write tests first when possible
- Tests document expected behavior
- All tests must pass before commit

### Testing Requirements

- All tests pass: `cargo test`
- New functionality should have corresponding tests
- Edge cases should be tested
- Error paths should be tested where applicable

### Test Organization

- Unit tests: In same file, `#[cfg(test)]` module
- Integration tests: In `tests/` directory
- Use descriptive test names: `test_function_with_scenario_returns_expected_result`

### Before Pushing

```bash
cargo test  # Zero test failures
```

---

## 4. Code Style & Conventions

### Rust Idioms

- Follow Rust naming conventions: `snake_case` for functions/variables, `CamelCase` for types
- Use idiomatic Rust patterns (pattern matching, Result/Option)
- Prefer explicit types where clarity helps, omit where inference is clear
- Use `?` operator for error propagation

### Code Formatting

All code must pass `cargo fmt`:

```bash
cargo fmt          # Auto-format all code
cargo fmt --check  # Verify format will pass CI
```

---

## 5. File Size Limits

### Counting Method

Physical `wc -l` lines — blanks and comments included. No `#[cfg(test)]` exclusion arithmetic.

### Budgets

| Category | Budget |
|---|---|
| Source files (`.rs` under `src/`) | 500 lines |
| Dedicated test files (`*_tests.rs`, `*_tests_fixtures.rs`, files under `tests/`, files inside a `*_tests` directory) | 800 lines |
| Config files (`Cargo.toml`, `Cargo.lock`, `.github/workflows/*.yml`, `scripts/*.rs`) | 200 lines |

Rationale: test files are linear suites, not multi-responsibility modules — they get a larger budget rather than full exemption. Config files are short by nature; a 200-line cap keeps them scannable.

### Grandfathering

Files already over their budget at the branch point are recorded in a checked-in baseline manifest (`scripts/file_size_baseline.txt`, `path=lines` per line). A grandfathered file may grow at most 10% above its baseline (integer ceiling) in any single PR. The CI gate fails when:

- (a) a non-grandfathered file exceeds its category budget, or
- (b) a grandfathered file exceeds `baseline × 1.1`

A grandfathered file that shrinks to its budget or below no longer breaches — remove its entry from the manifest in the same PR (follow-up chore).

### CI Gate

`scripts/check_file_size.rs` (registered as the `check_file_size` example in `Cargo.toml`, mirroring `check_dead_files.rs`) scans every `.rs` file under `src/` (including `src/bin/lievo/`), `Cargo.toml`, `Cargo.lock`, `.github/workflows/*.yml`, and `scripts/*.rs`. It enforces the budgets above plus the grandfathering cap. It runs in the CI check job after the orphan gate:

```bash
cargo run --example check_file_size   # exit 0 = compliant; exit 1 = breach
```

Self-tests: `rustc --edition 2021 --test scripts/check_file_size.rs`.

### Refactoring Triggers

When a file exceeds its budget OR has 3+ distinct responsibilities:
1. Identify separate concerns
2. Extract into new module/file
3. Keep interfaces clean
4. Test both old and new module

### Rationale

- Smaller files are easier to understand
- Focused modules have clearer responsibility
- Easier to test in isolation
- Reduces cognitive load during maintenance

---

## 6. Git Workflow

### Conventional Commits

All commits use [Conventional Commits](https://www.conventionalcommits.org/) format:

```
feat(#123): brief description
fix(#45): brief description
docs(#78): brief description
```

**Commit Types**:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `refactor`: Code restructuring (no behavior change)
- `test`: Test additions/modifications
- `perf`: Performance improvements
- `chore`: Maintenance tasks

**Example**:
```
feat(#12): add json output format support

Users can now export analysis results as JSON.
Adds --json flag to all query commands.

Closes #12
```

### Branch Naming Convention

```
feature/issue-{NUMBER}-brief-description
```

Examples:
- `feature/issue-12-add-json-output`
- `feature/issue-45-fix-relationship-detection`
- `feature/issue-78-optimize-analysis-performance`

### Push & PR Workflow

1. Create feature branch from `main`
2. Make changes with conventional commits
3. All quality gates pass locally (see Pre-Push Quality Gates section)
4. Push to your fork
5. Open a PR against `trail-openers/lievo`
6. CI verifies all checks pass
7. PR is squash-merged to main

### Branch Protection Rules

- ❌ NO direct commits to `main`
- ❌ NO force pushes to `main`
- ✅ All work on feature branches
- ✅ PRs required for all changes

---

## 7. Documentation Policy

### The 200-PR Test

Before creating documentation, ask: "Will this be true in 200 PRs?"

- **YES** (principle that endures) → Document the principle (WHY)
- **NO** (implementation detail) → Skip or use code comments (WHAT/HOW)

For durable principles specific to CLI design, see [CLI Architecture](docs/cli-architecture.md) and [CLI Output Contracts](docs/cli-output-contracts.md).

### What to Document

- ✅ Project goals and vision (README)
- ✅ Core concepts and terminology (docs/)
- ✅ Non-obvious algorithms (code comments)
- ✅ API contract and usage examples (docs/)
- ✅ Configuration options (docs/)
- ✅ Security considerations (SECURITY.md)
- ✅ Architectural decisions (docs/)

### Forbidden Documentation

- ❌ Issue drafts or implementation summaries
- ❌ Fix notes or scratch files
- ❌ TODO items (create GitHub issues instead)
- ❌ Implementation plans (IMPLEMENTATION_PLAN.md)
- ❌ Design documents (DESIGN.md)
- ❌ Research notes (RESEARCH.md)
- ❌ Temporary work artifacts

**Why**: Documenting implementation details creates future maintenance burden. If it changes frequently, it shouldn't be documented.

### Documentation Location

- **Project-level**: README.md, CONTRIBUTING.md, SECURITY.md (root directory)
- **Concept docs**: docs/ directory with .md files
- **Code comments**: Non-obvious logic, algorithm explanation, edge cases
- **Issue tracking**: Decisions and learnings go in GitHub issues, not documentation files

### File Naming

- All .md documentation files must use lowercase filenames (e.g., cli-architecture.md, not CLI_Architecture.md)
- Conventional root files (README.md, AGENTS.md, CHANGELOG.md, CONTRIBUTING.md, SECURITY.md) and .github/ templates are exempt

---

## 8. Architecture & Module Structure

### Architecture

- **Library + CLI + MCP server**: Public API in `src/lib.rs`, CLI in `src/bin/lievo/`, and MCP server in `src/mcp/`.
- **Std-First Philosophy**: Prefer stdlib over external crates when possible.
- **Minimal Dependencies**: Question every new dependency. Is stdlib sufficient?

### Module Structure

- **model/**: Project, Repository, Entity, Relationship, Insight types
- **storage/**: SQLite schema, Storage trait, queries
- **extraction/**: tree-sitter parsing, grouping heuristics, metrics
- **analysis/**: Relationships, incremental updates, insights
- **query/**: Entity queries, dependency analysis, impact reports
- **mcp/**: MCP server and tool definitions

### Async & Concurrency

- **ALLOWED for I/O**: `async`/`await` and `tokio` are allowed for I/O-bound operations such as MCP transport and external integrations
- **REQUIRED synchronous**: Core analysis logic (entity extraction, relationship building, metrics computation) must stay synchronous
- **Rationale**: Analysis is CPU-bound and benefits from deterministic execution. Async is only used where I/O latency matters

### Dependencies

- **Parsing**: tree-sitter with Rust/Python/JavaScript/Go parsers (in-process, no subprocesses)
- **Embeddings**: model2vec + usearch for on-device semantic search
- **Storage**: rusqlite with bundled SQLite
- **Optional**: apfel (external binary) for on-device summarization
- **Std-first approach**: Prefer Rust stdlib over external crates
- **Minimal dependencies**: Every new crate requires justification
- **Review before adding**: Ask "Is stdlib insufficient?" before importing new crates

### Performance vs Simplicity

- Optimize for simplicity first
- Profile before optimizing
- Don't trade clarity for marginal performance gains

---

## 9. Commands Reference

| Task | Command |
|------|---------|
| Build project | `cargo build` |
| Run tests | `cargo test` |
| Format check | `cargo fmt --check` |
| Format code | `cargo fmt` |
| Lint code | `cargo clippy -- -D warnings` |
| All quality gates | `cargo fmt --check && cargo clippy -- -D warnings && cargo test` |
| Release build | `cargo build --release` |
| Run locally | `cargo run --bin lievo -- <args>` |
| Clean build artifacts | `cargo clean` |
| Check (no compile) | `cargo check` |

### Pre-Push Checklist

```bash
# 1. Verify tests pass
cargo test

# 2. Verify linting passes
cargo clippy -- -D warnings

# 3. Verify formatting passes
cargo fmt --check

# 4. Verify all together
cargo fmt --check && cargo clippy -- -D warnings && cargo test

# 5. Push when all green
git push
```

---

## 10. Issue-Driven Development

### Before Starting Work

1. **GitHub issue exists** for the work
2. **Issue clearly describes** the requirement
3. **Your approach matches** the issue scope exactly
4. **No scope expansion** without updating the issue

### During Development

- Keep changes focused on the issue
- If you discover additional work, create a separate issue
- Discuss in issue before implementing surprises

### Scope Control Protocol

- **READ**: `gh issue view #123` for complete requirements
- **VALIDATE**: All work matches issue content exactly
- **REFUSE**: Any work not explicitly listed in issue
- **EXPAND**: Update issue before adding scope

### In Commit Messages

Always reference the issue:

```
feat(#123): add json output support

Closes #123
```

---

## 11. Code Review & Quality

### Pre-Commit Verification

Before EVERY commit, verify:
- [ ] Tests pass locally: `cargo test`
- [ ] Linting passes: `cargo clippy -- -D warnings`
- [ ] Formatting passes: `cargo fmt --check`
- [ ] Issue requirements are met

### Forbidden Practices

- ❌ `#[allow(clippy::...)]` — Fix the actual issue
- ❌ Suppressing warnings — Fix the code
- ❌ `TODO` comments — Create a GitHub issue instead
- ❌ Incomplete error handling — Handle all error cases
- ❌ Unused code — Delete it

### Code Comments

- Write comments for WHY, not WHAT
- Code should be clear enough that WHAT is obvious
- Comments should explain non-obvious algorithms or design decisions
- Update comments when code changes

---

## 12. Performance & Optimization

### Philosophy

- **Optimize for simplicity first**
- Profile before optimizing
- Don't trade clarity for marginal gains

### When to Optimize

- After profiling shows the bottleneck
- When it measurably impacts user experience
- Not for theoretical future improvements

### When NOT to Optimize

- ❌ Premature optimization
- ❌ Sacrificing readability for "efficiency"
- ❌ Over-engineering for edge cases

---

## Summary

**lievo Standards at a Glance:**

1. **Every line of code is a liability** — justify necessity before creation
2. **Quality gates are mandatory** — all checks pass locally before push
3. **Conventional commits** — feat/fix/docs format with issue numbers
4. **Library + CLI + MCP server, async for I/O only** — lib.rs public API + lievo.rs CLI + src/mcp/ MCP server, async/tokio for MCP transport and external integrations, synchronous core analysis
5. **500-line source file limit** — refactor when files get too large
6. **Issue-driven work** — all changes match GitHub issue requirements
7. **Documentation principle** — the 200-PR test determines what gets documented
8. **Std-first with parsing stack** — prefer stdlib, tree-sitter + model2vec + usearch for extraction

**Golden Rule**: Question every addition. Simplest solution wins.

For contributor onboarding, CLA signing, and release procedure, see [CONTRIBUTING.md](CONTRIBUTING.md).
