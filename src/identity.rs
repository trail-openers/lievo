// Repository identity: deriving a stable identity key from a repository's
// origin remote URL (issue #28, sub-issue 1 of epic #24).
//
// The identity key is `host/owner/repo` with a lowercase host, no
// credentials, no user, no `.git` suffix and no explicit standard port.
// Every spelling of the same remote (SSH, HTTPS, git://, ssh://, scp-like)
// normalizes to the same key. Unparseable or unknown-scheme URLs yield
// `None`, and the caller falls back to the canonical path.

use crate::config::RepoConfig;

/// Schemes we can key on. Any other scheme (ftp, telnet, ...) or a
/// missing/unknown shape yields `None` so the caller falls back to the
/// canonical path.
const SUPPORTED_SCHEMES: [&str; 3] = ["https", "ssh", "git"];

/// Default ports per scheme where an explicit port is redundant and dropped
/// from the key. Any other port is preserved as a distinguishing part.
const STANDARD_PORTS: [(&str, u16); 2] = [("https", 443), ("ssh", 22)];

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
/// Returns `Some("host/owner/repo")` — lowercase host, no `.git` suffix, no
/// credentials, no user, no explicit standard port — or `None` when the URL
/// is unparseable, uses an unsupported scheme, or lacks an owner/repo shape.
pub fn normalize_origin_url(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
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

    Some(format!("{key_host}/{repo_path}"))
}

/// Derive a repository's identity key.
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
pub fn derive_identity(repo_path: &std::path::Path, config: &RepoConfig) -> Option<String> {
    if let Some(override_key) = config.identity.as_deref()
        && !override_key.trim().is_empty()
    {
        let key = override_key.trim();
        // Defensive: a stored override with a traversal or empty segment
        // must never become an identity key, even though `validate()`
        // rejects such values at config load.
        if has_forbidden_segment(key) {
            return None;
        }
        return Some(key.to_ascii_lowercase());
    }

    let repo = git2::Repository::open(repo_path).ok()?;
    if repo.is_bare() {
        return None;
    }
    let url = repo.find_remote("origin").ok()?.url().ok()?.to_string();

    normalize_origin_url(&url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    /// Table-driven cases: every URL form in the issue's edge-case list must
    /// collapse to the same key `github.com/owner/repo`.
    #[test]
    fn test_normalize_origin_url_all_spellings_collapse_to_one_key() {
        let expected = "github.com/owner/repo";

        let cases = [
            // scp-like SSH
            "git@github.com:owner/repo",
            "git@github.com:owner/repo.git",
            // ssh:// with explicit standard port and user
            "ssh://git@github.com:22/owner/repo",
            "ssh://git@github.com/owner/repo",
            "ssh://git@github.com/owner/repo.git",
            // HTTPS with and without credentials
            "https://github.com/owner/repo",
            "https://github.com/owner/repo.git",
            "https://github.com/owner/repo/",
            "https://user:token@github.com/owner/repo",
            "https://user@github.com/owner/repo.git",
            // git://
            "git://github.com/owner/repo",
            "git://github.com/owner/repo.git",
            // uppercase host must lowercase
            "https://GitHub.com/owner/repo",
            "git@GitHub.com:owner/repo",
            // trailing slash variants
            "ssh://git@github.com/owner/repo/",
        ];

        for url in &cases {
            let key =
                normalize_origin_url(url).unwrap_or_else(|| panic!("failed to normalize: {url}"));
            assert_eq!(
                key, expected,
                "URL {url} should normalize to {expected}, got {key}"
            );
        }
    }

    #[test]
    fn test_normalize_origin_url_strips_credentials_and_token_never_leaks() {
        let url = "https://user:supersecrettoken123@github.com/owner/repo";
        let key = normalize_origin_url(url).unwrap();
        assert_eq!(key, "github.com/owner/repo");
        assert!(
            !key.contains("supersecrettoken123"),
            "token must not appear in the key: {key}"
        );
        assert!(
            !key.contains("user"),
            "user must not appear in the key: {key}"
        );
        assert!(!key.contains('@'), "no '@' in key: {key}");
    }

    #[test]
    fn test_normalize_origin_url_explicit_standard_port_same_as_plain() {
        let a = normalize_origin_url("ssh://git@github.com:22/owner/repo").unwrap();
        let b = normalize_origin_url("git@github.com:owner/repo").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, "github.com/owner/repo");
    }

    #[test]
    fn test_normalize_origin_url_nonstandard_port_is_preserved() {
        let key = normalize_origin_url("ssh://git@github.com:2222/owner/repo").unwrap();
        assert_eq!(key, "github.com:2222/owner/repo");
        let key = normalize_origin_url("https://github.com:8443/owner/repo").unwrap();
        assert_eq!(key, "github.com:8443/owner/repo");
    }

    #[test]
    fn test_normalize_origin_url_unparseable_returns_none() {
        for url in [
            "",
            "   ",
            "not-a-url",
            "https://",
            "http://github.com",
            "git@github.com",
            "ssh://",
            "ftp://github.com/owner/repo",
            "telnet://github.com/owner/repo",
        ] {
            assert!(
                normalize_origin_url(url).is_none(),
                "{url:?} must normalize to None"
            );
        }
    }

    #[test]
    fn test_normalize_origin_url_requires_owner_and_repo() {
        // Host only, no path → None
        assert_eq!(normalize_origin_url("https://github.com/"), None);
        // A single-segment path is not owner/repo → None
        assert_eq!(normalize_origin_url("https://github.com/repo"), None);
        assert_eq!(normalize_origin_url("git@github.com:repo"), None);
    }

    #[test]
    fn test_normalize_origin_url_rejects_dot_segments_and_invalid_ports() {
        // Path traversal / dot segments are never valid identity keys.
        for url in [
            "git@github.com:../../etc/passwd",
            "https://github.com/../evil/repo",
            "https://github.com/owner/.git",
            "ssh://github.com/owner/./repo",
            "git@github.com:owner//repo",
        ] {
            assert!(
                normalize_origin_url(url).is_none(),
                "{url:?} must normalize to None"
            );
        }
        // A port that is present but does not parse as u16 is a malformed
        // URL: the function returns None rather than silently dropping it.
        for url in [
            "ssh://github.com:65536/owner/repo",
            "https://github.com:abc/owner/repo",
        ] {
            assert!(
                normalize_origin_url(url).is_none(),
                "{url:?} must normalize to None"
            );
        }
    }

    // --- derive_identity ---

    /// Build a tempdir git repo with an origin remote set to `url`.
    fn make_repo_with_remote(dir: &Path, url: &str) -> git2::Repository {
        let repo = git2::Repository::init(dir).unwrap();
        if !url.is_empty() {
            repo.config()
                .unwrap()
                .set_str("remote.origin.url", url)
                .unwrap();
        }
        repo
    }

    fn default_config_with_identity(identity: &str) -> RepoConfig {
        RepoConfig {
            identity: Some(identity.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn test_derive_identity_override_wins_without_any_git_repo() {
        // Override is checked before any git call: even a non-repo directory
        // with an override yields the override key.
        let tmp = TempDir::new().unwrap();
        let config = default_config_with_identity("Example.com/Some/Override");
        let identity = derive_identity(tmp.path(), &config);
        assert_eq!(identity.as_deref(), Some("example.com/some/override"));
    }

    #[test]
    fn test_derive_identity_override_wins_in_repo_without_remote() {
        // Repo exists but has no origin remote; override still wins (no error).
        let tmp = TempDir::new().unwrap();
        let _ = git2::Repository::init(tmp.path()).unwrap();
        let config = default_config_with_identity("example.com/other/repo");
        let identity = derive_identity(tmp.path(), &config);
        assert_eq!(identity.as_deref(), Some("example.com/other/repo"));
    }

    #[test]
    fn test_derive_identity_override_wins_over_live_remote() {
        let tmp = TempDir::new().unwrap();
        let _ = make_repo_with_remote(tmp.path(), "https://github.com/real/repo");
        let config = default_config_with_identity("example.com/override/repo");
        let identity = derive_identity(tmp.path(), &config);
        assert_eq!(identity.as_deref(), Some("example.com/override/repo"));
    }

    #[test]
    fn test_derive_identity_override_with_dot_segment_returns_none() {
        // Defensive: an override that snuck past validation (e.g. written
        // directly) with a traversal segment must not become a key.
        let tmp = TempDir::new().unwrap();
        let config = default_config_with_identity("github.com/../evil/repo");
        let identity = derive_identity(tmp.path(), &config);
        assert!(identity.is_none());
    }

    #[test]
    fn test_derive_identity_from_origin_remote() {
        let tmp = TempDir::new().unwrap();
        let _ = make_repo_with_remote(tmp.path(), "git@github.com:owner/repo");
        let config = RepoConfig::default();
        let identity = derive_identity(tmp.path(), &config);
        assert_eq!(identity.as_deref(), Some("github.com/owner/repo"));
    }

    #[test]
    fn test_derive_identity_repo_without_origin_returns_none() {
        let tmp = TempDir::new().unwrap();
        let _ = git2::Repository::init(tmp.path()).unwrap();
        let config = RepoConfig::default();
        let identity = derive_identity(tmp.path(), &config);
        assert!(identity.is_none());
    }

    #[test]
    fn test_derive_identity_bare_repo_returns_none() {
        let tmp = TempDir::new().unwrap();
        let _ = git2::Repository::init_bare(tmp.path()).unwrap();
        let config = RepoConfig::default();
        let identity = derive_identity(tmp.path(), &config);
        assert!(identity.is_none());
    }

    #[test]
    fn test_derive_identity_non_repo_returns_none() {
        let tmp = TempDir::new().unwrap();
        let config = RepoConfig::default();
        let identity = derive_identity(tmp.path(), &config);
        assert!(identity.is_none());
    }
}
