use lievo::extraction::framework::detect_framework_static;
use std::fs;
use tempfile::TempDir;

// ── Basic lookup table tests ──────────────────────────────────────────────

#[test]
fn test_detect_framework_static_empty_dir_returns_none() {
    let tmp = TempDir::new().unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap();
    assert!(profile.is_none());
}

#[test]
fn test_detect_framework_static_rails_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("Gemfile"),
        "gem 'rails', '~> 7.0'\ngem 'pg'\n",
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Rails");
    assert_eq!(profile.module_depth, 2);
    assert!(profile.key_files.contains(&"config/routes.rb".to_string()));
    assert!(
        profile
            .subsystems
            .iter()
            .any(|(p, n)| p == "app" && n == "app")
    );
    assert!(
        profile
            .subsystems
            .iter()
            .any(|(p, n)| p == "." && n == "infrastructure")
    );
}

#[test]
fn test_detect_framework_static_django_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("requirements.txt"),
        "django==4.2\npsycopg2-binary\n",
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Django");
    assert_eq!(profile.module_depth, 2);
    assert!(profile.key_files.contains(&"manage.py".to_string()));
}

#[test]
fn test_detect_framework_static_fastapi_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("requirements.txt"),
        "fastapi==0.104\nuvicorn\n",
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "FastAPI");
    assert_eq!(profile.module_depth, 2);
}

#[test]
fn test_detect_framework_static_nextjs_lookup() {
    let tmp = TempDir::new().unwrap();
    // "next" is an exact dep name in package.json — must match exactly, not as substring
    fs::write(
        tmp.path().join("package.json"),
        r#"{"dependencies":{"next":"^14.0","react":"^18.0"}}"#,
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Next.js");
    assert_eq!(profile.module_depth, 2);
    assert!(profile.key_files.contains(&"package.json".to_string()));
}

#[test]
fn test_detect_framework_static_nextjs_no_false_positive_from_next_auth() {
    let tmp = TempDir::new().unwrap();
    // "next-auth" must NOT match the "next" lookup key (exact match required for JS)
    fs::write(
        tmp.path().join("package.json"),
        r#"{"dependencies":{"next-auth":"^4.0","react":"^18.0"}}"#,
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap();
    assert!(
        profile.is_none() || profile.unwrap().name != "Next.js",
        "next-auth must not match the Next.js lookup entry"
    );
}

#[test]
fn test_detect_framework_static_nestjs_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("package.json"),
        r#"{"dependencies":{"@nestjs/core":"^10.0","@nestjs/common":"^10.0"}}"#,
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "NestJS");
    assert_eq!(profile.module_depth, 3);
    assert!(profile.key_files.contains(&"nest-cli.json".to_string()));
    assert!(
        profile
            .subsystems
            .iter()
            .any(|(p, n)| p == "src" && n == "app")
    );
}

#[test]
fn test_detect_framework_static_nuxtjs_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("package.json"),
        r#"{"dependencies":{"nuxt":"^3.0","vue":"^3.0"}}"#,
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Nuxt.js");
    assert_eq!(profile.module_depth, 2);
    assert!(profile.key_files.contains(&"nuxt.config.ts".to_string()));
}

#[test]
fn test_detect_framework_static_laravel_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("composer.json"),
        r#"{"require":{"laravel/framework":"^10.0","php":">=8.1"}}"#,
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Laravel");
    assert_eq!(profile.module_depth, 2);
    assert!(profile.key_files.contains(&"composer.json".to_string()));
    // Verify subsystem filtering — Laravel should have app, routes, database, tests
    assert!(
        profile
            .subsystems
            .iter()
            .any(|(p, n)| p == "app" && n == "app")
    );
    assert!(
        profile
            .subsystems
            .iter()
            .any(|(p, n)| p == "routes" && n == "routing")
    );
    assert!(
        profile
            .subsystems
            .iter()
            .any(|(p, n)| p == "database" && n == "database")
    );
    assert!(
        profile
            .subsystems
            .iter()
            .any(|(p, n)| p == "tests" && n == "testing")
    );
}

#[test]
fn test_detect_framework_static_symfony_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("composer.json"),
        r#"{"require":{"symfony/framework-bundle":"^6.0","php":">=8.1"}}"#,
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Symfony");
    assert_eq!(profile.module_depth, 2);
    assert!(
        profile
            .key_files
            .contains(&"config/services.yaml".to_string())
    );
    assert!(
        profile
            .subsystems
            .iter()
            .any(|(p, n)| p == "src" && n == "app")
    );
}

#[test]
fn test_detect_framework_static_spring_boot_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("pom.xml"),
        "<project>\
         <dependencies>\
           <dependency><artifactId>spring-boot-starter-web</artifactId></dependency>\
         </dependencies>\
         </project>",
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Spring Boot");
    assert_eq!(profile.module_depth, 2);
}

#[test]
fn test_detect_framework_static_gin_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("go.mod"),
        "module example.com/app\n\nrequire (\n    github.com/gin-gonic/gin v1.9.0\n)\n",
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Gin");
    assert_eq!(profile.module_depth, 1);
}

#[test]
fn test_detect_framework_static_echo_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("go.mod"),
        "module example.com/app\n\nrequire (\n    github.com/labstack/echo v4.11.0\n)\n",
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Echo");
    assert_eq!(profile.module_depth, 1);
}

#[test]
fn test_detect_framework_static_phoenix_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("mix.exs"),
        "defp deps do\n  [{:phoenix, \"~> 1.7\"}, {:ecto, \"~> 3.0\"}]\nend\n",
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Phoenix");
    assert_eq!(profile.module_depth, 2);
}

#[test]
fn test_detect_framework_static_actix_web_lookup() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"myapp\"\nversion = \"0.1.0\"\n\n[dependencies]\nactix-web = \"4\"\n",
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Actix Web");
    assert_eq!(profile.module_depth, 1);
    assert!(profile.key_files.contains(&"Cargo.toml".to_string()));
}

// ── Django app detection ───────────────────────────────────────────────────

fn make_django_app(dir: &std::path::Path, name: &str) {
    let app = dir.join(name);
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("models.py"), "# models\n").unwrap();
    fs::write(app.join("views.py"), "# views\n").unwrap();
}

fn make_django_app_with_apps_py(dir: &std::path::Path, name: &str) {
    let app = dir.join(name);
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("models.py"), "# models\n").unwrap();
    fs::write(app.join("apps.py"), "# apps\n").unwrap();
}

fn write_django_requirements(dir: &std::path::Path) {
    fs::write(dir.join("requirements.txt"), "django==4.2\npsycopg2\n").unwrap();
}

#[test]
fn test_django_three_apps_detected_as_subsystems() {
    let tmp = TempDir::new().unwrap();
    write_django_requirements(tmp.path());
    make_django_app(tmp.path(), "users");
    make_django_app(tmp.path(), "orders");
    make_django_app(tmp.path(), "payments");

    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Django");
    assert_eq!(profile.subsystems.len(), 3);

    let paths: Vec<&str> = profile.subsystems.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"users"));
    assert!(paths.contains(&"orders"));
    assert!(paths.contains(&"payments"));
}

#[test]
fn test_django_nested_apps_detected() {
    let tmp = TempDir::new().unwrap();
    write_django_requirements(tmp.path());
    make_django_app(&tmp.path().join("apps"), "users");
    make_django_app(&tmp.path().join("apps"), "orders");

    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Django");
    assert_eq!(profile.subsystems.len(), 2);

    let paths: Vec<&str> = profile.subsystems.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"apps/users"));
    assert!(paths.contains(&"apps/orders"));

    // Display names are the leaf directory name
    let names: Vec<&str> = profile.subsystems.iter().map(|(_, n)| n.as_str()).collect();
    assert!(names.contains(&"users"));
    assert!(names.contains(&"orders"));
}

#[test]
fn test_django_no_apps_keeps_empty_subsystems() {
    let tmp = TempDir::new().unwrap();
    write_django_requirements(tmp.path());
    // Only manage.py — no apps
    fs::write(tmp.path().join("manage.py"), "#!/usr/bin/env python\n").unwrap();

    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Django");
    assert!(profile.subsystems.is_empty());
}

#[test]
fn test_django_dir_without_models_py_not_detected_as_app() {
    let tmp = TempDir::new().unwrap();
    write_django_requirements(tmp.path());

    // views.py only — no models.py
    let no_model = tmp.path().join("noapp");
    fs::create_dir_all(&no_model).unwrap();
    fs::write(no_model.join("views.py"), "# views\n").unwrap();

    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Django");
    assert!(profile.subsystems.is_empty());
}

#[test]
fn test_django_app_with_apps_py_detected() {
    let tmp = TempDir::new().unwrap();
    write_django_requirements(tmp.path());
    make_django_app_with_apps_py(tmp.path(), "catalog");

    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Django");
    assert_eq!(profile.subsystems.len(), 1);
    assert_eq!(profile.subsystems[0].0, "catalog");
}

// ── Go standard layout detection ──────────────────────────────────────────

fn write_go_mod(dir: &std::path::Path, framework_dep: Option<&str>) {
    let mut content = "module example.com/app\n\ngo 1.21\n".to_string();
    if let Some(dep) = framework_dep {
        content.push_str(&format!("\nrequire (\n    {} v1.0.0\n)\n", dep));
    }
    fs::write(dir.join("go.mod"), content).unwrap();
}

#[test]
fn test_go_gin_with_standard_layout_detected() {
    let tmp = TempDir::new().unwrap();
    write_go_mod(tmp.path(), Some("github.com/gin-gonic/gin"));
    fs::create_dir_all(tmp.path().join("cmd")).unwrap();
    fs::create_dir_all(tmp.path().join("internal")).unwrap();
    fs::create_dir_all(tmp.path().join("pkg")).unwrap();

    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Gin");
    // cmd, internal, pkg + infrastructure dot entry
    assert_eq!(profile.subsystems.len(), 4);

    let paths: Vec<&str> = profile.subsystems.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"cmd"));
    assert!(paths.contains(&"internal"));
    assert!(paths.contains(&"pkg"));
    assert!(paths.contains(&"."));
}

#[test]
fn test_go_gin_with_only_cmd_dir() {
    let tmp = TempDir::new().unwrap();
    write_go_mod(tmp.path(), Some("github.com/gin-gonic/gin"));
    fs::create_dir_all(tmp.path().join("cmd")).unwrap();

    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Gin");
    // cmd + infrastructure dot entry
    assert_eq!(profile.subsystems.len(), 2);

    let paths: Vec<&str> = profile.subsystems.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"cmd"));
    assert!(paths.contains(&"."));
}

#[test]
fn test_go_without_framework_but_with_standard_layout_returns_go_profile() {
    let tmp = TempDir::new().unwrap();
    // Plain go.mod with no framework dep
    write_go_mod(tmp.path(), None);
    fs::create_dir_all(tmp.path().join("cmd")).unwrap();
    fs::create_dir_all(tmp.path().join("internal")).unwrap();

    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Go");
    assert_eq!(profile.module_depth, 1);
    assert!(profile.key_files.contains(&"go.mod".to_string()));

    let paths: Vec<&str> = profile.subsystems.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"cmd"));
    assert!(paths.contains(&"internal"));
    assert!(paths.contains(&"."));
}

#[test]
fn test_go_without_standard_layout_returns_none() {
    let tmp = TempDir::new().unwrap();
    // Plain go.mod, no cmd/internal/pkg
    write_go_mod(tmp.path(), None);

    let profile = detect_framework_static(tmp.path()).unwrap();
    assert!(profile.is_none());
}

// ── No-match / edge cases ──────────────────────────────────────────────────

#[test]
fn test_detect_framework_static_no_match_returns_none() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("Gemfile"), "gem 'sinatra'\ngem 'pg'\n").unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap();
    assert!(profile.is_none(), "sinatra should not match any framework");
}

#[test]
fn test_detect_framework_static_case_insensitive() {
    let tmp = TempDir::new().unwrap();
    // Django with mixed case version specifier
    fs::write(
        tmp.path().join("requirements.txt"),
        "Django==4.2\nPsycopg2-binary\n",
    )
    .unwrap();
    let profile = detect_framework_static(tmp.path()).unwrap().unwrap();
    assert_eq!(profile.name, "Django");
}
