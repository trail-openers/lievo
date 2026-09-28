// Issue #855: source-root resolution tests for the JS/TS import resolver.
// These live in a separate file so `import_resolver.rs` stays under its
// 500-line source budget (the production code additions are small; the test
// surface is the bulk of the issue's test-surface requirement).

use crate::analysis::import_resolver::JsResolverContext;
use crate::analysis::relationships::RelationshipBuilder;
use crate::extraction::grouping::GroupingResult;
use crate::model::{CodeUnit, Entity, EntityTier, RelType};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn make_file(id: &str, path: &str) -> Entity {
    Entity {
        id: id.to_string(),
        project_id: "proj".to_string(),
        repo_id: None,
        tier: EntityTier::File,
        parent_id: None,
        name: Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string()),
        path: Some(path.to_string()),
        language: Some("JavaScript".to_string()),
        summary: None,
        summary_commit: None,
        metrics_json: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn write(repo: &Path, rel: &str, content: &str) {
    let full = repo.join(rel);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

#[test]
fn test_pnpm_workspace_members_resolved() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "pnpm-workspace.yaml",
        "packages:\n  - \"libs/*\"\n",
    );
    write(
        tmp.path(),
        "libs/util-pkg/package.json",
        r#"{"name": "util-pkg"}"#,
    );
    write(tmp.path(), "libs/util-pkg/core.ts", "export {};\n");
    let files = vec![make_file("f-core", "libs/util-pkg/core.ts")];
    let ctx = JsResolverContext::new(&files, tmp.path());
    let id = ctx
        .resolve("util-pkg/core", "src/main.ts")
        .expect("pnpm member must resolve");
    assert_eq!(id, "f-core");
}

// Issue #829: empty pnpm-workspace.yaml fails to parse, silently yields 0 members.
#[test]
fn test_pnpm_workspace_empty_file_yields_no_members_silently() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "pnpm-workspace.yaml", "");
    write(
        tmp.path(),
        "libs/alpha/package.json",
        r#"{"name": "alpha"}"#,
    );
    write(tmp.path(), "libs/alpha/util.js", "export {};\n");
    let files = vec![make_file("f-alpha-util", "libs/alpha/util.js")];
    let ctx = JsResolverContext::new(&files, tmp.path());
    assert_eq!(ctx.resolve("alpha/util", "src/main.ts"), None);
}

// Issue #829: comment-only pnpm-workspace.yaml is valid YAML, yields 0 members.
#[test]
fn test_pnpm_workspace_comment_only_yields_no_members_silently() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "pnpm-workspace.yaml", "# no packages listed\n");
    write(tmp.path(), "libs/beta/package.json", r#"{"name": "beta"}"#);
    write(tmp.path(), "libs/beta/util.js", "export {};\n");
    let files = vec![make_file("f-beta-util", "libs/beta/util.js")];
    let ctx = JsResolverContext::new(&files, tmp.path());
    assert_eq!(ctx.resolve("beta/util", "src/main.ts"), None);
}

const ANCHORED_YML: &str = r#"
default: &default
  source_path: app/javascript
  additional_paths: ['app/assets']
development:
  <<: *default
  compile: true
production:
  <<: *default
"#;

#[test]
fn test_shakapacker_only_bare_and_relative_both_resolve() {
    // Shakapacker-only fixture (no webpacker.yml): a file importing
    // "shop/components/Widget" and another importing it relatively BOTH
    // produce an imports edge to app/javascript/shop/components/Widget/index.js.
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
    write(
        tmp.path(),
        "app/javascript/shop/components/Widget/index.js",
        "export {};\n",
    );
    let files = vec![make_file(
        "f-widget",
        "app/javascript/shop/components/Widget/index.js",
    )];
    let ctx = JsResolverContext::new(&files, tmp.path());

    // Bare specifier through the source root.
    let id = ctx
        .resolve("shop/components/Widget", "app/javascript/src/main.js")
        .expect("bare specifier must resolve through source root");
    assert_eq!(id, "f-widget");

    // Relative specifier from the same root (independent of source-root logic).
    // "./components/Widget" from "app/javascript/shop/src/main.js" would
    // resolve to "app/javascript/shop/src/components/Widget" (wrong path).
    // Use a correct relative path: "../components/Widget" from
    // "app/javascript/shop/pages/main.js" → "app/javascript/shop/components/Widget".
    let id = ctx
        .resolve("../components/Widget", "app/javascript/shop/pages/main.js")
        .expect("relative specifier must resolve");
    assert_eq!(id, "f-widget");
}

#[test]
fn test_webpacker_yml_bare_and_relative_both_resolve() {
    // Same as above but with config/webpacker.yml instead of shakapacker.yml.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "config/webpacker.yml",
        "default:\n  source_path: app/javascript\n",
    );
    write(
        tmp.path(),
        "app/javascript/shop/components/Widget/index.js",
        "export {};\n",
    );
    let files = vec![make_file(
        "f-widget",
        "app/javascript/shop/components/Widget/index.js",
    )];
    let ctx = JsResolverContext::new(&files, tmp.path());
    let id = ctx
        .resolve("shop/components/Widget", "app/javascript/src/main.js")
        .expect("webpacker.yml source_path must resolve");
    assert_eq!(id, "f-widget");
}

#[test]
fn test_jsconfig_base_url_bare_resolves() {
    // jsconfig.json compilerOptions.baseUrl (no tsconfig present) as the
    // source root.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "jsconfig.json",
        r#"{"compilerOptions": {"baseUrl": "app/javascript"}}"#,
    );
    write(
        tmp.path(),
        "app/javascript/shop/components/Widget/index.js",
        "export {};\n",
    );
    let files = vec![make_file(
        "f-widget",
        "app/javascript/shop/components/Widget/index.js",
    )];
    let ctx = JsResolverContext::new(&files, tmp.path());
    let id = ctx
        .resolve("shop/components/Widget", "app/javascript/src/main.js")
        .expect("jsconfig baseUrl must resolve");
    assert_eq!(id, "f-widget");
}

#[test]
fn test_tsconfig_without_base_url_does_not_block_shakapacker() {
    // The www-repo shape: tsconfig.json exists with no baseUrl/paths; only
    // shakapacker.yml carries the source root. Resolution must work.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "tsconfig.json",
        r#"{"compilerOptions": {"experimentalDecorators": true, "allowJs": true}}"#,
    );
    write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
    write(
        tmp.path(),
        "app/javascript/shop/components/Widget/index.js",
        "export {};\n",
    );
    let files = vec![make_file(
        "f-widget",
        "app/javascript/shop/components/Widget/index.js",
    )];
    let ctx = JsResolverContext::new(&files, tmp.path());
    let id = ctx
        .resolve("shop/components/Widget", "app/javascript/src/main.js")
        .expect("tsconfig without baseUrl must not block shakapacker root");
    assert_eq!(id, "f-widget");
}

#[test]
fn test_two_roots_target_only_in_second_resolves_deterministically() {
    // Two roots (source_path + additional_paths); the target exists only in
    // the second (additional_paths) root. Must resolve through the second
    // root — order matters, and it must be deterministic (no HashMap).
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "config/shakapacker.yml",
        "default:\n  source_path: app/javascript\n  additional_paths: ['lib/frontend']\n",
    );
    write(
        tmp.path(),
        "lib/frontend/deep/Widget/index.js",
        "export {};\n",
    );
    let files = vec![make_file("f-deep", "lib/frontend/deep/Widget/index.js")];
    let ctx = JsResolverContext::new(&files, tmp.path());
    let id = ctx
        .resolve("deep/Widget", "src/main.js")
        .expect("target in second root must resolve deterministically");
    assert_eq!(id, "f-deep");
}

#[test]
fn test_bare_dependency_root_package_json_not_resolved_locally() {
    // A bare specifier declared as a dependency in the ROOT package.json,
    // whose name matches a local directory under a source root, must NOT be
    // resolved locally.
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
    write(
        tmp.path(),
        "package.json",
        r#"{"name": "root", "dependencies": {"react": "^18.0.0"}}"#,
    );
    write(tmp.path(), "app/javascript/react/index.js", "export {};\n");
    let files = vec![make_file("f-react", "app/javascript/react/index.js")];
    let ctx = JsResolverContext::new(&files, tmp.path());
    assert_eq!(
        ctx.resolve("react", "src/main.js"),
        None,
        "declared dependency must not resolve locally even if a same-named dir exists"
    );
}

#[test]
fn test_bare_dependency_workspace_member_package_json_not_resolved_locally() {
    // A bare specifier declared as a dependency in a WORKSPACE MEMBER's
    // package.json (not the root) whose name matches a local directory under
    // a source root must also NOT be resolved locally.
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "package.json",
        r#"{"name": "root", "workspaces": ["ssr"]}"#,
    );
    write(
        tmp.path(),
        "ssr/package.json",
        r#"{"name": "ssr", "dependencies": {"shop": "1.0.0"}}"#,
    );
    write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
    write(tmp.path(), "app/javascript/shop/index.js", "export {};\n");
    let files = vec![make_file("f-shop", "app/javascript/shop/index.js")];
    let ctx = JsResolverContext::new(&files, tmp.path());
    assert_eq!(
        ctx.resolve("shop", "src/main.js"),
        None,
        "workspace-member dependency must not resolve locally"
    );
}

#[test]
fn test_unresolved_bare_under_root_counts_internal() {
    // An unresolved bare specifier whose first segment is an existing
    // directory under a detected root (not a declared dependency) must count
    // Internal; a bare npm package must count External.
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
    write(tmp.path(), "app/javascript/shop/hooks.js", "export {};\n");
    let files = vec![make_file("f-hooks", "app/javascript/shop/hooks.js")];
    let ctx = JsResolverContext::new(&files, tmp.path());

    // "shop/missing" — shop is a real dir under the root, but no file resolves.
    assert!(ctx.resolve("shop/missing", "src/main.js").is_none());
    assert!(
        ctx.matches_source_root("shop/missing"),
        "shop/missing must match source root (internal classification)"
    );
    // "lodash" — not under any root, not a declared dep → external.
    assert!(ctx.resolve("lodash", "src/main.js").is_none());
    assert!(
        !ctx.matches_source_root("lodash"),
        "lodash must not match source root (external classification)"
    );
}

#[test]
fn test_matches_source_root_returns_false_for_relative_and_absolute() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
    write(tmp.path(), "app/javascript/shop/x.js", "export {};\n");
    let files = vec![make_file("f-x", "app/javascript/shop/x.js")];
    let ctx = JsResolverContext::new(&files, tmp.path());
    assert!(!ctx.matches_source_root("./shop/x"));
    assert!(!ctx.matches_source_root("/shop/x"));
}

#[test]
fn test_matches_source_root_false_for_declared_dependency() {
    // A declared dependency that also names a dir under the root must NOT
    // match (guard #4).
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
    write(
        tmp.path(),
        "package.json",
        r#"{"name": "root", "dependencies": {"utils": "1.0.0"}}"#,
    );
    write(tmp.path(), "app/javascript/utils/index.js", "export {};\n");
    let files = vec![make_file("f-utils", "app/javascript/utils/index.js")];
    let ctx = JsResolverContext::new(&files, tmp.path());
    assert!(
        !ctx.matches_source_root("utils"),
        "declared dependency must not match source root"
    );
}

#[test]
fn test_e2e_regression_shape_target_alias_importer_hop0_relative_importer_hop1() {
    // End-to-end shape of the regression case:
    //   target (Widget/index.js)
    //   <- aliasImporter (imports "shop/components/Widget")  [hop 0]
    //   <- relativeImporter (imports "./components/Widget" from a sibling dir) [hop 1]
    //
    // Both must produce an imports edge to the target. This pins the
    // resolution logic that feeds blast_radius and get_impact.
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
    write(
        tmp.path(),
        "app/javascript/shop/components/Widget/index.js",
        "export {};\n",
    );
    write(
        tmp.path(),
        "app/javascript/shop/pages/Forum/components/ForumCore/index.js",
        "import Widget from \"shop/components/Widget\";\n",
    );
    write(
        tmp.path(),
        "app/javascript/shop/pages/Forum/components/ForumWithLoading/index.js",
        "import ForumCore from \"../ForumCore\";\n",
    );
    let files = vec![
        make_file("f-widget", "app/javascript/shop/components/Widget/index.js"),
        make_file(
            "f-forumcore",
            "app/javascript/shop/pages/Forum/components/ForumCore/index.js",
        ),
        make_file(
            "f-forumloading",
            "app/javascript/shop/pages/Forum/components/ForumWithLoading/index.js",
        ),
    ];
    let ctx = JsResolverContext::new(&files, tmp.path());

    // aliasImporter (ForumCore) imports "shop/components/Widget" → Widget.
    let id = ctx
        .resolve(
            "shop/components/Widget",
            "app/javascript/shop/pages/Forum/components/ForumCore/index.js",
        )
        .expect("alias import must resolve");
    assert_eq!(id, "f-widget");

    // relativeImporter (ForumWithLoading) imports "../ForumCore" → ForumCore.
    let id = ctx
        .resolve(
            "../ForumCore",
            "app/javascript/shop/pages/Forum/components/ForumWithLoading/index.js",
        )
        .expect("relative import must resolve");
    assert_eq!(id, "f-forumcore");
}

fn make_js_code_unit(file: &str, imports: Vec<&str>) -> CodeUnit {
    CodeUnit {
        name: "fn".to_string(),
        qualified_name: format!("{}::fn", file.replace('/', "::")),
        unit_type: "function".to_string(),
        file: file.to_string(),
        line: 1,
        end_line: 10,
        language: "JavaScript".to_string(),
        signature: None,
        code: None,
        docstring: None,
        parent_class: None,
        complexity: 1,
        has_branches: false,
        has_loops: false,
        has_error_handling: false,
        calls: vec![],
        imports: imports.into_iter().map(str::to_string).collect(),
    }
}

#[test]
fn test_e2e_relationship_builder_shakapacker_alias_and_relative_both_emit_edges() {
    // End-to-end: two files both importing Widget (one via bare source-root
    // specifier, one via relative path) produce imports edges to the same
    // target. This is the regression shape: target <- aliasImporter (hop 0)
    // <- relativeImporter (hop 1).
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "config/shakapacker.yml", ANCHORED_YML);
    write(
        tmp.path(),
        "app/javascript/shop/components/Widget/index.js",
        "export {};\n",
    );
    write(
        tmp.path(),
        "app/javascript/shop/pages/Forum/components/ForumCore/index.js",
        "import Widget from \"shop/components/Widget\";\n",
    );
    write(
        tmp.path(),
        "app/javascript/shop/pages/Forum/components/ForumWithLoading/index.js",
        "import ForumCore from \"../ForumCore\";\n",
    );

    let widget = make_file("f-widget", "app/javascript/shop/components/Widget/index.js");
    let forumcore = make_file(
        "f-forumcore",
        "app/javascript/shop/pages/Forum/components/ForumCore/index.js",
    );
    let forumloading = make_file(
        "f-forumloading",
        "app/javascript/shop/pages/Forum/components/ForumWithLoading/index.js",
    );
    let grouping = GroupingResult {
        subsystems: vec![],
        modules: vec![],
        files: vec![widget.clone(), forumcore.clone(), forumloading.clone()],
        preserve_function_entities: false,
    };

    let units = vec![
        make_js_code_unit(
            "app/javascript/shop/pages/Forum/components/ForumCore/index.js",
            vec!["shop/components/Widget"],
        ),
        make_js_code_unit(
            "app/javascript/shop/pages/Forum/components/ForumWithLoading/index.js",
            vec!["../ForumCore"],
        ),
    ];

    let (rels, _unresolved) =
        RelationshipBuilder::build(&units, &grouping, "proj", "repo", tmp.path()).unwrap();

    let imports: Vec<_> = rels
        .iter()
        .filter(|r| r.rel_type == RelType::Imports)
        .collect();

    // ForumCore → Widget (bare source-root specifier).
    let fc_widget = imports
        .iter()
        .find(|r| r.source_id == "f-forumcore" && r.target_id == "f-widget")
        .expect("ForumCore must have an imports edge to Widget via bare specifier");
    assert_eq!(fc_widget.provenance, crate::model::EdgeProvenance::Resolved);

    // ForumWithLoading → ForumCore (relative specifier).
    let fl_fc = imports
        .iter()
        .find(|r| r.source_id == "f-forumloading" && r.target_id == "f-forumcore")
        .expect("ForumWithLoading must have an imports edge to ForumCore via relative path");
    assert_eq!(fl_fc.provenance, crate::model::EdgeProvenance::Resolved);
}
