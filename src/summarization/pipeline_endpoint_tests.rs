// Moved from pipeline_tests.rs (at its grandfathered size cap, issue #869)
// to allow adding the crate-wide env lock around the LIEVO_APFEL_ENDPOINT
// set_var/remove_var calls.

use crate::config::RepoConfig;
use crate::summarization::pipeline::remote_endpoint_warning;

/// Remote-endpoint notice (PR #787 review, security): a usable endpoint on
/// a non-loopback host means source code leaves the machine; the warning
/// names the host. Loopback endpoints stay silent (the normal local setup
/// must not be nagged), and userinfo must never be echoed.
///
/// Every `set_var`/`remove_var` is under the crate-wide env lock (issue #869).
#[test]
fn test_remote_endpoint_warning_names_remote_host_and_stays_silent_on_loopback() {
    let _env_guard = crate::test_env_support::env_lock();

    let remote = RepoConfig {
        apfel_endpoint: Some("https://llama.example.com:8080".to_string()),
        ..Default::default()
    };
    let warning = remote_endpoint_warning(&remote);
    assert_eq!(
        warning.as_deref(),
        Some(
            "warning: summarizing via remote endpoint llama.example.com:8080; source code will be sent to this host"
        ),
        "remote host must be named in the warning"
    );

    // Loopback endpoints: silent. `effective_apfel_endpoint` reads the
    // LIEVO_APFEL_ENDPOINT env var with priority over the config field,
    // so the var must be clear while these configs are in scope (other
    // tests in this binary set the var via their own env guards).
    let prior_endpoint = std::env::var("LIEVO_APFEL_ENDPOINT");
    unsafe { std::env::remove_var("LIEVO_APFEL_ENDPOINT") };
    for host in ["127.0.0.1", "localhost", "::1"] {
        let cfg = RepoConfig {
            apfel_endpoint: Some(format!("http://{host}:8080")),
            ..Default::default()
        };
        let endpoint = cfg.effective_apfel_endpoint().unwrap_or_default();
        let got = remote_endpoint_warning(&cfg);
        assert!(
            got.is_none(),
            "loopback endpoint {host} must stay silent (effective={endpoint:?}, got={got:?})"
        );
    }
    if let Ok(v) = prior_endpoint {
        unsafe { std::env::set_var("LIEVO_APFEL_ENDPOINT", v) };
    }

    // No endpoint: silent.
    assert!(remote_endpoint_warning(&RepoConfig::default()).is_none());

    // Userinfo is stripped before printing — credentials never echoed.
    let userinfo = RepoConfig {
        apfel_endpoint: Some("http://user:pass@example.com:9000".to_string()),
        ..Default::default()
    };
    let userinfo_warning = remote_endpoint_warning(&userinfo).expect("warning present");
    assert!(
        userinfo_warning.contains("example.com:9000"),
        "host must still be named: {userinfo_warning}!"
    );
    assert!(
        !userinfo_warning.contains("user:pass"),
        "userinfo must never be echoed: {userinfo_warning}!"
    );
}
