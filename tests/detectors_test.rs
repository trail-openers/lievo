use lievo::extraction::detectors::{
    detect_cargo_workspace, detect_npm_workspace, detect_python_packages, detect_subsystems,
};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_detect_cargo_workspace() {
    let temp = TempDir::new().unwrap();
    let cargo_toml = temp.path().join("Cargo.toml");
    fs::write(
        &cargo_toml,
        r#"
[workspace]
members = [
    "crates/conductor",
    "crates/agents",
    "tools"
]
"#,
    )
    .unwrap();

    let subsystems = detect_cargo_workspace(temp.path());
    assert!(subsystems.is_some());

    let map = subsystems.unwrap();
    assert_eq!(map.get("crates/conductor"), Some(&"conductor".to_string()));
    assert_eq!(map.get("crates/agents"), Some(&"agents".to_string()));
    assert_eq!(map.get("tools"), Some(&"tools".to_string()));
    assert_eq!(map.get("."), Some(&"root".to_string()));
}

#[test]
fn test_detect_cargo_workspace_glob_ignored_for_phase_a() {
    let temp = TempDir::new().unwrap();
    let cargo_toml = temp.path().join("Cargo.toml");
    fs::write(
        &cargo_toml,
        r#"
[workspace]
members = [
    "crates/*",
]
"#,
    )
    .unwrap();

    // Phase A: ignore globs, should return None since no simple paths
    let subsystems = detect_cargo_workspace(temp.path());
    assert!(subsystems.is_some());
    let map = subsystems.unwrap();
    // Should only have root, no glob-expanded members
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("."), Some(&"root".to_string()));
}

#[test]
fn test_detect_npm_workspace_array() {
    let temp = TempDir::new().unwrap();
    let package_json = temp.path().join("package.json");
    fs::write(
        &package_json,
        r#"{
  "name": "monorepo",
  "workspaces": ["packages/ui", "packages/api", "tools"]
}"#,
    )
    .unwrap();

    let subsystems = detect_npm_workspace(temp.path());
    assert!(subsystems.is_some());

    let map = subsystems.unwrap();
    assert_eq!(map.get("packages/ui"), Some(&"ui".to_string()));
    assert_eq!(map.get("packages/api"), Some(&"api".to_string()));
    assert_eq!(map.get("tools"), Some(&"tools".to_string()));
}

#[test]
fn test_detect_npm_workspace_object() {
    let temp = TempDir::new().unwrap();
    let package_json = temp.path().join("package.json");
    fs::write(
        &package_json,
        r#"{
  "name": "monorepo",
  "workspaces": {
    "packages": ["packages/ui", "packages/api"]
  }
}"#,
    )
    .unwrap();

    let subsystems = detect_npm_workspace(temp.path());
    assert!(subsystems.is_some());

    let map = subsystems.unwrap();
    assert_eq!(map.get("packages/ui"), Some(&"ui".to_string()));
    assert_eq!(map.get("packages/api"), Some(&"api".to_string()));
}

#[test]
fn test_detect_npm_workspace_glob_expansion() {
    let temp = TempDir::new().unwrap();

    // Create packages/foo and packages/bar directories
    fs::create_dir_all(temp.path().join("packages/foo")).unwrap();
    fs::create_dir_all(temp.path().join("packages/bar")).unwrap();

    let package_json = temp.path().join("package.json");
    fs::write(
        &package_json,
        r#"{
  "name": "monorepo",
  "workspaces": ["packages/*"]
}"#,
    )
    .unwrap();

    let subsystems = detect_npm_workspace(temp.path());
    assert!(subsystems.is_some());

    let map = subsystems.unwrap();
    assert_eq!(map.get("packages/bar"), Some(&"bar".to_string()));
    assert_eq!(map.get("packages/foo"), Some(&"foo".to_string()));
    assert_eq!(map.get("."), Some(&"root".to_string()));
}

#[test]
fn test_detect_npm_workspace_mixed_patterns() {
    let temp = TempDir::new().unwrap();

    // Create packages/alpha directory for glob match
    fs::create_dir_all(temp.path().join("packages/alpha")).unwrap();
    // Create apps/web as a direct path (no glob)
    fs::create_dir_all(temp.path().join("apps/web")).unwrap();

    let package_json = temp.path().join("package.json");
    fs::write(
        &package_json,
        r#"{
  "name": "monorepo",
  "workspaces": ["apps/web", "packages/*"]
}"#,
    )
    .unwrap();

    let subsystems = detect_npm_workspace(temp.path());
    assert!(subsystems.is_some());

    let map = subsystems.unwrap();
    // Direct path
    assert_eq!(map.get("apps/web"), Some(&"web".to_string()));
    // Glob-expanded
    assert_eq!(map.get("packages/alpha"), Some(&"alpha".to_string()));
}

#[test]
fn test_detect_npm_workspace_object_form_with_glob() {
    let temp = TempDir::new().unwrap();

    fs::create_dir_all(temp.path().join("packages/core")).unwrap();
    fs::create_dir_all(temp.path().join("packages/utils")).unwrap();

    let package_json = temp.path().join("package.json");
    fs::write(
        &package_json,
        r#"{
  "name": "monorepo",
  "workspaces": {
    "packages": ["packages/*"]
  }
}"#,
    )
    .unwrap();

    let subsystems = detect_npm_workspace(temp.path());
    assert!(subsystems.is_some());

    let map = subsystems.unwrap();
    assert_eq!(map.get("packages/core"), Some(&"core".to_string()));
    assert_eq!(map.get("packages/utils"), Some(&"utils".to_string()));
}

#[test]
fn test_detect_npm_workspace_glob_empty_directory() {
    let temp = TempDir::new().unwrap();

    // Create the parent dir but leave it empty
    fs::create_dir_all(temp.path().join("packages")).unwrap();

    let package_json = temp.path().join("package.json");
    fs::write(
        &package_json,
        r#"{
  "name": "monorepo",
  "workspaces": ["packages/*"]
}"#,
    )
    .unwrap();

    let subsystems = detect_npm_workspace(temp.path());
    // Still returns Some because we have root; glob just adds nothing
    assert!(subsystems.is_some());
    let map = subsystems.unwrap();
    // Only root present, no glob-expanded members
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("."), Some(&"root".to_string()));
}

#[test]
fn test_detect_npm_workspace_glob_ignores_files() {
    let temp = TempDir::new().unwrap();

    // Create a directory and a file inside packages/
    fs::create_dir_all(temp.path().join("packages/valid-pkg")).unwrap();
    fs::create_dir_all(temp.path().join("packages")).unwrap();
    fs::write(temp.path().join("packages/README.md"), "# readme").unwrap();

    let package_json = temp.path().join("package.json");
    fs::write(
        &package_json,
        r#"{
  "name": "monorepo",
  "workspaces": ["packages/*"]
}"#,
    )
    .unwrap();

    let subsystems = detect_npm_workspace(temp.path()).unwrap();
    // File should not appear as subsystem
    assert!(!subsystems.contains_key("packages/README.md"));
    // Directory should appear
    assert_eq!(
        subsystems.get("packages/valid-pkg"),
        Some(&"valid-pkg".to_string())
    );
}

#[test]
fn test_detect_python_packages() {
    let temp = TempDir::new().unwrap();
    let auth_dir = temp.path().join("auth");
    let api_dir = temp.path().join("api");
    let vendor_dir = temp.path().join("vendor");

    fs::create_dir_all(&auth_dir).unwrap();
    fs::create_dir_all(&api_dir).unwrap();
    fs::create_dir_all(&vendor_dir).unwrap();

    fs::write(auth_dir.join("__init__.py"), "").unwrap();
    fs::write(api_dir.join("__init__.py"), "").unwrap();
    // vendor has no __init__.py

    let subsystems = detect_python_packages(temp.path());
    assert!(subsystems.is_some());

    let (map, _depth) = subsystems.unwrap();
    // Keys are relative paths (not absolute)
    assert_eq!(map.get("auth"), Some(&"auth".to_string()));
    assert_eq!(map.get("api"), Some(&"api".to_string()));
    assert_eq!(map.get("vendor"), None);
}

#[test]
fn test_detect_python_packages_path_uses_relative_path() {
    // Python detector stores relative paths as keys (not absolute)
    let temp = TempDir::new().unwrap();
    let myapp_dir = temp.path().join("myapp");
    fs::create_dir_all(&myapp_dir).unwrap();
    fs::write(myapp_dir.join("__init__.py"), "").unwrap();

    let (map, _depth) = detect_python_packages(temp.path()).unwrap();
    // Key should be relative "myapp", not an absolute path
    assert!(map.contains_key("myapp"));
    assert_eq!(map.get("myapp"), Some(&"myapp".to_string()));
}

// Security: path traversal must not escape repo root
#[test]
fn test_expand_workspace_glob_rejects_path_traversal() {
    let dir = TempDir::new().unwrap();
    let repo_path = dir.path();
    fs::write(
        repo_path.join("package.json"),
        r#"{"workspaces": ["../../*"]}"#,
    )
    .unwrap();
    let result = detect_npm_workspace(repo_path);
    // Should return None or only root — NOT enumerate parent directories
    match result {
        None => {} // acceptable
        Some(map) => {
            assert!(map.len() <= 1, "path traversal should not add subsystems");
        }
    }
}

// Security: bare `*` pattern must be rejected to avoid enumerating entire repo root
#[test]
fn test_expand_workspace_glob_bare_star_rejected() {
    let dir = TempDir::new().unwrap();
    let repo_path = dir.path();
    fs::create_dir(repo_path.join("subdir")).unwrap();
    fs::write(repo_path.join("package.json"), r#"{"workspaces": ["*"]}"#).unwrap();
    let result = detect_npm_workspace(repo_path);
    // Bare * should be rejected — return None (only root if any)
    match result {
        None => {}
        Some(map) => assert!(!map.contains_key("subdir")),
    }
}

// Issue #391 Bug B: src-layout detection
#[test]
fn test_detect_python_packages_scans_src_layout() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create src-layout packages
    let src_dir = root.join("src");
    let myapp_dir = src_dir.join("myapp");
    fs::create_dir_all(&myapp_dir).unwrap();
    fs::write(myapp_dir.join("__init__.py"), "").unwrap();

    let result = detect_python_packages(root);
    assert!(result.is_some(), "should detect src-layout packages");
    let (subsystems, _depth) = result.unwrap();
    assert!(
        subsystems.contains_key("src/myapp"),
        "src/myapp should be detected"
    );
}

#[test]
fn test_detect_python_packages_combines_root_and_src_layout() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create root-level package
    let root_pkg = root.join("mylib");
    fs::create_dir_all(&root_pkg).unwrap();
    fs::write(root_pkg.join("__init__.py"), "").unwrap();

    // Create src-layout package
    let src_dir = root.join("src");
    let src_pkg = src_dir.join("myapp");
    fs::create_dir_all(&src_pkg).unwrap();
    fs::write(src_pkg.join("__init__.py"), "").unwrap();

    let result = detect_python_packages(root);
    assert!(result.is_some());
    let (subsystems, _depth) = result.unwrap();
    assert!(
        subsystems.contains_key("mylib"),
        "root-level package should be detected"
    );
    assert!(
        subsystems.contains_key("src/myapp"),
        "src-layout package should be detected"
    );
}

#[test]
fn test_detect_python_packages_src_without_init_py_skipped() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create src/ directory with a subdirectory that doesn't have __init__.py
    let src_dir = root.join("src");
    let subdir = src_dir.join("mydir");
    fs::create_dir_all(&subdir).unwrap();
    // No __init__.py created

    let result = detect_python_packages(root);
    assert!(
        result.is_none(),
        "should not detect packages without __init__.py"
    );
}

#[test]
fn test_detect_python_packages_src_skips_venv_with_init_py() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create src/venv/ with __init__.py — must be skipped
    let src_venv = root.join("src").join("venv");
    fs::create_dir_all(&src_venv).unwrap();
    fs::write(src_venv.join("__init__.py"), "").unwrap();

    // Create a real package
    let src_pkg = root.join("src").join("myapp");
    fs::create_dir_all(&src_pkg).unwrap();
    fs::write(src_pkg.join("__init__.py"), "").unwrap();

    let result = detect_python_packages(root);
    assert!(result.is_some());
    let (subsystems, _depth) = result.unwrap();
    assert!(
        !subsystems.contains_key("src/venv"),
        "src/venv should be skipped even with __init__.py"
    );
    assert!(
        subsystems.contains_key("src/myapp"),
        "src/myapp should be detected"
    );
}

#[test]
fn test_detect_python_packages_src_multiple_packages_detected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create multiple src-layout packages
    for pkg_name in &["core", "utils", "api"] {
        let pkg_dir = root.join("src").join(pkg_name);
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("__init__.py"), "").unwrap();
    }

    let result = detect_python_packages(root);
    assert!(result.is_some());
    let (subsystems, _depth) = result.unwrap();
    assert!(subsystems.contains_key("src/core"));
    assert!(subsystems.contains_key("src/utils"));
    assert!(subsystems.contains_key("src/api"));
}

// Issue #395: Verify that "." is NOT added to subsystem map when named packages are detected
// This prevents module explosion — "." would match all files and with module_depth=2 create thousands of modules
#[test]
fn test_detect_python_packages_no_root_subsystem_when_packages_found() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create a root-level package
    let mypkg_dir = root.join("mypkg");
    fs::create_dir_all(&mypkg_dir).unwrap();
    fs::write(mypkg_dir.join("__init__.py"), "").unwrap();

    let result = detect_python_packages(root);
    assert!(result.is_some());
    let (subsystems, _depth) = result.unwrap();

    // "." should NOT be in the subsystem map when we have named packages
    assert!(
        !subsystems.contains_key("."),
        "root \".\" subsystem should not be added when named packages are detected",
    );
    // But the named packages should be present
    assert!(subsystems.contains_key("mypkg"));
}

// Fix 1: Test verifying Django/Gin/Echo profiles return even with no conventional dirs
#[test]
fn test_dynamic_scanning_frameworks_return_profile_without_dirs() {
    // Django: has_dynamic_subsystem_detection=true, should return profile even with empty subsystems
    {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(root.join("manage.py"), "#!/usr/bin/env python\n").unwrap();
        fs::write(root.join("requirements.txt"), "django\n").unwrap();
        // No apps/ directory with models.py + views.py

        let (_subsystems, _depth, profile) = detect_subsystems(root).unwrap();
        assert!(
            profile.is_some(),
            "Django profile should be returned even with no matching dirs (has_dynamic_subsystem_detection=true)"
        );
        assert_eq!(profile.unwrap().name, "Django");
    }

    // Gin: has_dynamic_subsystem_detection=true, should return profile even with no cmd/internal/pkg
    {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("go.mod"),
            "module test\nrequire github.com/gin-gonic/gin v1.0.0\n",
        )
        .unwrap();
        // No cmd/, internal/, or pkg/ directories

        let (_subsystems, _depth, profile) = detect_subsystems(root).unwrap();
        assert!(
            profile.is_some(),
            "Gin profile should be returned even with no matching dirs (has_dynamic_subsystem_detection=true)"
        );
        assert_eq!(profile.unwrap().name, "Gin");
    }

    // Echo: has_dynamic_subsystem_detection=true, should return profile even with no cmd/internal/pkg
    {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("go.mod"),
            "module test\nrequire github.com/labstack/echo v1.0.0\n",
        )
        .unwrap();
        // No cmd/, internal/, or pkg/ directories

        let (_subsystems, _depth, profile) = detect_subsystems(root).unwrap();
        assert!(
            profile.is_some(),
            "Echo profile should be returned even with no matching dirs (has_dynamic_subsystem_detection=true)"
        );
        assert_eq!(profile.unwrap().name, "Echo");
    }
}

// Fix 3: Test for Bug A's actual fix — verifies FastAPI fallthrough works
#[test]
fn test_fastapi_with_no_conventional_dirs_falls_through_to_src_layout() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Make it look like a FastAPI project (requirements.txt with fastapi)
    fs::write(root.join("requirements.txt"), "fastapi\nuvicorn\n").unwrap();

    // No conventional FastAPI dirs (no routers/, models/, etc.)
    // But has src-layout package
    let src_pkg = root.join("src").join("myapp");
    fs::create_dir_all(&src_pkg).unwrap();
    fs::write(src_pkg.join("__init__.py"), "").unwrap();

    let (subsystems, _depth, profile) = detect_subsystems(root).unwrap();

    // Should have fallen through to detect_python_packages
    assert!(
        subsystems.contains_key("src/myapp"),
        "src/myapp should be detected via fallthrough from FastAPI profile with no matching dirs"
    );
    // Profile may still be Some(FastAPI) or None — both are acceptable
    // Key invariant: we get the python package
    let _ = profile; // Suppress unused variable warning
}

#[test]
fn test_django_with_no_apps_does_not_fall_through() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Django project (manage.py is the key signal)
    fs::write(root.join("manage.py"), "#!/usr/bin/env python\n").unwrap();
    fs::write(root.join("requirements.txt"), "django\n").unwrap();

    // No apps/ directory (nothing for Django to scan)
    // But has a Python package at root
    let myapp_dir = root.join("myapp");
    fs::create_dir_all(&myapp_dir).unwrap();
    fs::write(myapp_dir.join("__init__.py"), "").unwrap();

    let (_subsystems, _depth, profile) = detect_subsystems(root).unwrap();

    // Django profile should still be returned (has_dynamic_subsystem_detection=true means no fallthrough)
    assert!(
        profile.is_some(),
        "Django profile should be returned even with empty subsystems"
    );
    assert_eq!(profile.unwrap().name, "Django");
}

// ── JS/TS subsystem detection (#689) ─────────────────────────────────────

#[test]
fn test_detect_npm_workspace_www_shaped_supplements_unmatched_js_ts_dirs() {
    // www-shaped: workspaces: ["ssr"] + bulk JS/TS source in unmatched
    // top-level dirs. detect_npm_workspace must supplement its map so the
    // unmatched dirs don't collapse into ".".
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Workspace member
    fs::create_dir_all(root.join("ssr/src")).unwrap();
    fs::write(root.join("ssr/src/index.js"), "// ssr\n").unwrap();

    // Unmatched top-level JS/TS dirs
    for (dir, file) in [
        ("app/javascript", "main.js"),
        ("cypress", "spec.ts"),
        ("lib", "util.tsx"),
    ] {
        let path = root.join(dir);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(file), "// stub\n").unwrap();
    }

    fs::write(
        root.join("package.json"),
        r#"{"name": "www", "workspaces": ["ssr"]}"#,
    )
    .unwrap();

    let map = detect_npm_workspace(root).unwrap();
    assert_eq!(map.get("."), Some(&"root".to_string()));
    assert_eq!(map.get("ssr"), Some(&"ssr".to_string()));
    assert_eq!(map.get("app"), Some(&"app".to_string()));
    assert_eq!(map.get("cypress"), Some(&"cypress".to_string()));
    assert_eq!(map.get("lib"), Some(&"lib".to_string()));
    assert!(map.len() >= 5);
}

#[test]
fn test_detect_npm_workspace_subsystem_map_via_detect_subsystems_www_shaped() {
    // End-to-end through detect_subsystems: www-shaped fixture must yield
    // a map with ssr + each unmatched top-level JS/TS dir.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("ssr/src")).unwrap();
    fs::write(root.join("ssr/src/index.js"), "// ssr\n").unwrap();
    for (dir, file) in [
        ("app/javascript", "main.js"),
        ("cypress", "spec.ts"),
        ("lib", "util.tsx"),
    ] {
        let path = root.join(dir);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(file), "// stub\n").unwrap();
    }

    fs::write(
        root.join("package.json"),
        r#"{"name": "www", "workspaces": ["ssr"]}"#,
    )
    .unwrap();

    let (map, depth, profile) = detect_subsystems(root).unwrap();
    assert_eq!(map.len(), 5); // . + ssr + app + cypress + lib
    assert_eq!(map.get("ssr"), Some(&"ssr".to_string()));
    assert_eq!(map.get("app"), Some(&"app".to_string()));
    assert_eq!(map.get("cypress"), Some(&"cypress".to_string()));
    assert_eq!(map.get("lib"), Some(&"lib".to_string()));
    assert_eq!(depth, 1);
    assert!(profile.is_none());
}

#[test]
fn test_detect_npm_workspace_below_threshold_not_supplemented() {
    // Only 1 unmatched top-level JS/TS dir (< max(3, 25%)) — pure
    // monorepo behavior is preserved: no supplement.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("packages/a")).unwrap();
    fs::write(root.join("packages/a/index.js"), "// a\n").unwrap();
    fs::create_dir_all(root.join("packages/b")).unwrap();
    fs::write(root.join("packages/b/index.js"), "// b\n").unwrap();

    fs::create_dir_all(root.join("app/javascript")).unwrap();
    fs::write(root.join("app/javascript/main.js"), "// app\n").unwrap();

    fs::write(
        root.join("package.json"),
        r#"{"name": "monorepo", "workspaces": ["packages/a", "packages/b"]}"#,
    )
    .unwrap();

    let map = detect_npm_workspace(root).unwrap();
    // . + 2 members, no supplement
    assert_eq!(map.len(), 3);
    assert!(!map.contains_key("app"));
}

#[test]
fn test_detect_js_ts_subsystems_plain_repo() {
    // Plain JS/TS repo: package.json, no workspaces, top-level src/ + lib/
    // with JS/TS files.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("src/components")).unwrap();
    fs::write(root.join("src/components/Button.tsx"), "// button\n").unwrap();
    fs::create_dir_all(root.join("lib")).unwrap();
    fs::write(root.join("lib/util.js"), "// util\n").unwrap();

    fs::write(
        root.join("package.json"),
        r#"{"name": "plain", "dependencies": {"react": "18.0.0"}}"#,
    )
    .unwrap();

    let (map, depth, _profile) = detect_subsystems(root).unwrap();
    assert_eq!(map.get("src"), Some(&"src".to_string()));
    assert_eq!(map.get("lib"), Some(&"lib".to_string()));
    assert_eq!(map.get("."), Some(&"root".to_string()));
    // Depth 2 so src/ subdirs (components, hooks) become distinct modules.
    assert_eq!(depth, 2);
}

#[test]
fn test_detect_js_ts_subsystems_skips_generated_dirs() {
    // A repo whose only "JS/TS source" lives under generated/hidden dirs must
    // not gain phantom subsystems.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("node_modules/vendor")).unwrap();
    fs::write(root.join("node_modules/vendor/lib.js"), "// vendor\n").unwrap();
    fs::create_dir_all(root.join(".next")).unwrap();
    fs::write(root.join(".next/chunk.js"), "// next build\n").unwrap();
    fs::create_dir_all(root.join("dist")).unwrap();
    fs::write(root.join("dist/bundle.js"), "// bundle\n").unwrap();

    fs::write(root.join("package.json"), r#"{"name": "empty"}"#).unwrap();

    // JS/TS detector: no eligible dirs
    // JS/TS detection is exercised through the detect_subsystems chain (no
    // crate-level re-export of the detector itself): the repo has only
    // generated/hidden JS/TS dirs, so no JS/TS subsystems appear in the map.
    let (map, _depth, profile) = detect_subsystems(root).unwrap();
    assert!(!map.contains_key("node_modules"));
    assert!(!map.contains_key(".next"));
    assert!(!map.contains_key("dist"));
    assert!(profile.is_none());
}

#[test]
fn test_detect_subsystems_js_ts_detector_runs_after_framework_table() {
    // The JS/TS top-level scan must run AFTER the framework table, so that
    // framework profiles (Laravel, Rails, NestJS, etc.) are not pre-empted.
    //
    // This test uses a NestJS fixture: package.json with @nestjs/core dep
    // (no workspaces) + src/ with JS/TS files. The NestJS profile defines
    // subsystems [src, test]; the JS/TS scan would produce a generic map
    // without a profile. The ordering contract is: framework table first,
    // then JS/TS scan as a fallback for repos with no framework match.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.ts"), "// ts\n").unwrap();
    fs::create_dir_all(root.join("test")).unwrap();
    fs::write(root.join("test/app.spec.ts"), "// spec\n").unwrap();

    fs::write(
        root.join("package.json"),
        r#"{"name": "nestjs-app", "dependencies": {"@nestjs/core": "^10.0.0"}}"#,
    )
    .unwrap();

    let (map, _depth, profile) = detect_subsystems(root).unwrap();

    // NestJS profile must be attached — the framework table ran before the
    // JS/TS scan and its framework-defined subsystems were kept.
    let profile_name = profile.as_ref().map(|p| p.name.as_str());
    assert_eq!(
        profile_name,
        Some("NestJS"),
        "JS/TS scan must run after the framework table; profile: {profile_name:?}, map: {map:?}"
    );
    // The framework-defined src subsystem is present, not a generic scan result.
    assert_eq!(map.get("src"), Some(&"app".to_string()));
    // test/ from the framework profile, not the generic scan.
    assert!(map.contains_key("test"), "map: {map:?}");
}

#[test]
fn test_detect_subsystems_nextjs_style_src_subdirs_are_modules() {
    // Next.js-style repo: "next" dep, src/components/ + src/hooks/. The
    // Next.js framework profile contributes no usable subsystems, so the
    // fall-through branch hands off to the JS/TS scan (which runs after the
    // framework table) and maps src/; with module depth 2, src subdirs become
    // distinct modules downstream.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    fs::create_dir_all(root.join("src/components")).unwrap();
    fs::write(root.join("src/components/Button.tsx"), "// button\n").unwrap();
    fs::create_dir_all(root.join("src/hooks")).unwrap();
    fs::write(root.join("src/hooks/useThing.ts"), "// hook\n").unwrap();

    fs::write(
        root.join("package.json"),
        r#"{"name": "app", "dependencies": {"next": "14.0.0"}}"#,
    )
    .unwrap();

    let (map, depth, _profile) = detect_subsystems(root).unwrap();
    assert_eq!(map.get("src"), Some(&"src".to_string()));
    assert_eq!(depth, 2);
}

#[test]
fn test_detect_subsystems_mixed_rails_and_js_yields_multi_subsystem_map() {
    // Mixed Rails+JS repo (issue #689 edge case): Gemfile + package.json
    // with workspaces, plus JS/TS top-level dirs. The npm detector (or its
    // JS/TS supplement) must win over the Rails framework profile and the
    // resulting map must be multi-subsystem — not a degenerate "."-only
    // collapse and not the Rails table clobbering the JS/TS dirs.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Rails side
    fs::write(root.join("Gemfile"), "gem 'rails', '~> 7.0'\n").unwrap();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("config/application.rb"), "# app\n").unwrap();

    // JS side: workspace member ssr + unmatched top-level JS/TS dirs
    fs::create_dir_all(root.join("ssr/src")).unwrap();
    fs::write(root.join("ssr/src/index.js"), "// ssr\n").unwrap();
    for (dir, file) in [
        ("app/javascript", "main.js"),
        ("cypress", "spec.ts"),
        ("lib", "util.tsx"),
    ] {
        let path = root.join(dir);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join(file), "// stub\n").unwrap();
    }

    fs::write(
        root.join("package.json"),
        r#"{"name": "mixed", "workspaces": ["ssr"]}"#,
    )
    .unwrap();

    let (map, _depth, profile) = detect_subsystems(root).unwrap();

    // Multi-subsystem: the www-style shape (root + ssr + each unmatched JS/TS
    // dir), not the Rails framework table ("app"/"spec"/"test"/".").
    assert_eq!(map.get("."), Some(&"root".to_string()));
    assert_eq!(map.get("ssr"), Some(&"ssr".to_string()));
    assert_eq!(map.get("app"), Some(&"app".to_string()));
    assert_eq!(map.get("cypress"), Some(&"cypress".to_string()));
    assert_eq!(map.get("lib"), Some(&"lib".to_string()));
    assert!(map.len() >= 5, "map: {map:?}");

    // The JS/TS/npm path took precedence — no Rails profile attached.
    assert!(
        profile.is_none(),
        "expected no Rails profile, got: {profile:?}"
    );
}
