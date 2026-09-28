# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | ✅ Current release |

## Reporting a Vulnerability

If you discover a security vulnerability in lievo, please report it responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

Use GitHub's **private vulnerability reporting** feature: go to the [Security Advisories page](https://github.com/trail-openers/lievo/security/advisories/new) and create a new advisory.

You should receive an acknowledgement within 48 hours. We will work with you to understand the issue and coordinate a fix before any public disclosure.

## Scope

Lievo is a local code analysis tool. Its primary security surface includes:

- **SQLite database** (`~/.lievo/lievo.db`) — contains extracted code metadata. Protected by filesystem permissions.
- **MCP server** (`lievo mcp`) — communicates over stdio with a single connected client. Does not listen on network ports.
- **Semantic search index** (`~/.lievo/indices/`) — contains vector embeddings of code. Protected by filesystem permissions.
- **Dependencies** — lievo depends on tree-sitter parsers, model2vec, usearch, and other Rust crates. Vulnerabilities in dependencies are tracked via Dependabot, a `cargo audit` CI gate, and a committed `.cargo/audit.toml` allowlist.

## Security Considerations

- Lievo reads source code from repositories you explicitly register. It does not access files outside registered repositories.
- The MCP server is designed for local use over stdio. It does not implement authentication or encryption — these are provided by the MCP client transport.
- Analysis results are stored locally and are not transmitted to external services.
