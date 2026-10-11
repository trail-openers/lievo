// Repository identity: deriving a stable identity key from a repository's
// origin remote URL (issue #28, sub-issue 1 of epic #24).
//
// The identity key is `host/owner/repo`, fully lowercase, no
// credentials, no user, no `.git` suffix and no explicit standard port.
// Every spelling of the same remote (SSH, HTTPS, git://, ssh://, scp-like)
// normalizes to the same key. Unparseable or unknown-scheme URLs yield
// `None`, and the caller falls back to the canonical path.

use crate::config::RepoConfig;
use crate::identity_key::is_valid_identity_key;

/// Schemes we can key on. Any other scheme (ftp, telnet, ...) or a
/// missing/unknown shape yields `None` so the caller falls back to the
/// canonical path.
const SUPPORTED_SCHEMES: [&str; 3] = ["https", "ssh", "git"];

/// Default ports per scheme where an explicit port is redundant and dropped
/// from the key. Any other port is preserved as a distinguishing part.
const STANDARD_PORTS: [(&str, u16); 3] = [("https", 443), ("ssh", 22), ("git", 9418)];

/// Where a derived identity key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentitySource {
    /// The repo's `identity:` override in `.lievo/config.yaml`. The file is
    /// repo-controlled, so this source never drives a move.
    Override,
    /// The repository's normalized `origin` remote URL.
    Origin,
}

/// A derived identity key and the source it was derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub key: String,
    pub source: IdentitySource,
}

/// Whether `port` is the standard port for `scheme`, i.e. redundant in a key.
fn is_standard_port(scheme: &str, port: u16) -> bool {
    STANDARD_PORTS
        .iter()
        .any(|(s, p)| *s == scheme && *p == port)
}

/// Strip a trailing `.git` suffix (and any trailing slashes) from a path.
fn strip_repo_suffix(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    let stripped = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    // If `.git` was the entire last segment (e.g. `owner/.git`), stripping
    // it leaves a trailing slash (`owner/`); remove that too so the result
    // is a clean path without dangling separators.
    stripped.trim_end_matches('/')
}

/// Whether any `/`-separated segment of `path` is empty, `.` or `..`.
/// Such segments are path-traversal vectors and must never become part of
/// an identity key.
fn has_forbidden_segment(path: &str) -> bool {
    path.split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
}

/// Normalize a git origin remote URL into an identity key.
///
/// Returns `Some("host/owner/repo")` — fully lowercase, no `.git` suffix, no credentials,
/// no user, no explicit standard port — or `None` when the URL is unparseable, uses an
/// unsupported scheme, contains a control character, or lacks an owner/repo shape.
pub fn normalize_origin_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    // Control characters (ANSI escapes, embedded newlines, NUL, ...) must never
    // survive into an identity key: they would render raw in terminal output or
    // corrupt downstream parsing. Checked after trimming, so only a leading or
    // trailing whitespace run is removed and an embedded control char is still
    // rejected.
    if url.chars().any(|c| c.is_control()) {
        return None;
    }

    // Split host and path. For scheme URLs (`ssh://`, `https://`, `git://`)
    // the last `/` before the path separates host:port from path. For
    // scp-like URLs (`[user@]host:owner/repo`) the last `:` separates host
    // from path.
    let (scheme, host_port_raw, path) = if let Some((scheme, rest)) = url.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        let rest = rest.rsplit_once('@').map(|(_, r)| r).unwrap_or(rest);
        // Use the FIRST `/` after the host to split host:port from path.
        let (host_port, path) = rest.split_once('/')?;
        (scheme, host_port.to_string(), path.to_string())
    } else {
        // scp-like: [user@]host:owner/repo
        let (host_port, path) = url.rsplit_once(':')?;
        if path.is_empty() || path.starts_with('/') {
            return None;
        }
        let host_port = host_port
            .rsplit_once('@')
            .map(|(_, h)| h)
            .unwrap_or(host_port);
        ("ssh".to_string(), host_port.to_string(), path.to_string())
    };

    if !SUPPORTED_SCHEMES.contains(&scheme.as_str()) {
        return None;
    }

    let (host, port) = match host_port_raw.rsplit_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host_port_raw.as_str(), None),
    };

    if host.is_empty() || path.is_empty() || has_forbidden_segment(path.trim_end_matches('/')) {
        return None;
    }

    let mut key_host = host.to_ascii_lowercase();
    if let Some(port) = port {
        // A port that is present but out of range (e.g. `:65536`, `:abc`)
        // makes the URL unusable as a key — drop it silently only for
        // standard ports handled below, never for malformed ones.
        let port = port.parse::<u16>().ok()?;
        if !is_standard_port(&scheme, port) {
            key_host = format!("{key_host}:{port}");
        }
    }

    let repo_path = strip_repo_suffix(&path);
    if repo_path.is_empty() || !repo_path.contains('/') {
        return None;
    }

    // Length bound (issue #29): a key longer than `MAX_KEY_LEN` bytes is not
    // a real remote shape — reject it so an adversarial remote URL cannot
    // persist or print an unbounded identity key.
    let key = format!("{key_host}/{repo_path}").to_ascii_lowercase();
    if !is_valid_identity_key(&key) {
        return None;
    }

    Some(key)
}

/// Derive a repository's identity key and its source.
///
/// Order (matches the issue's acceptance criteria):
/// 1. `config.identity` override — checked BEFORE any git call, so a set
///    override wins even when the repo has no remote at all.
/// 2. The repository's `origin` remote URL, normalized. A bare repository
///    is not a valid identity source (its origin, if any, is not a checkout).
/// 3. `None` — the caller falls back to the canonical path.
///
/// An unreadable or missing origin remote does not error when the override
/// is set; it simply yields `None` otherwise.
pub fn derive_identity(repo_path: &std::path::Path, config: &RepoConfig) -> Option<Identity> {
    if let Some(override_key) = config.identity.as_deref()
        && !override_key.trim().is_empty()
    {
        let key = override_key.trim();
        // Defensive: a stored override with a traversal or empty segment
        // must never become an identity key, even though `validate()`
        // rejects such values at config load. Control characters are
        // rejected at the same boundary (they must not reach storage or
        // terminal output).
        // Length and control-character bounds (issue #29) are shared with
        // `normalize_origin_url` and `RepoConfig::validate`.
        if has_forbidden_segment(key) || !is_valid_identity_key(key) {
            return None;
        }
        return Some(Identity {
            key: key.to_ascii_lowercase(),
            source: IdentitySource::Override,
        });
    }

    let repo = git2::Repository::open(repo_path).ok()?;
    if repo.is_bare() {
        return None;
    }
    let url = repo.find_remote("origin").ok()?.url().ok()?.to_string();

    normalize_origin_url(&url).map(|key| Identity {
        key,
        source: IdentitySource::Origin,
    })
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
