//! Theme catalogue, switching and permission integration tests.

mod common;

use axum::http::StatusCode;
use common::{get_as, post_form_as, put_json_as, TestApp};
use serde_json::json;

async fn admin(app: &TestApp) -> String {
    let (cookie, user) = app.register("admin@example.com", "supersecret").await;
    assert_eq!(user["user"]["role"], "admin");
    cookie
}

async fn member(app: &TestApp, email: &str) -> String {
    let (cookie, _) = app.register(email, "supersecret").await;
    cookie
}

#[tokio::test]
async fn the_theme_catalogue_is_seeded_and_listed_by_the_admin_api() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let response = app.get_as("/api/v1/admin/themes", &cookie).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let body = response.json();
    assert_eq!(body["switching"], "all");
    let themes = body["themes"].as_array().expect("themes");
    assert_eq!(themes.len(), 6);
    let names: Vec<&str> = themes
        .iter()
        .map(|theme| theme["key"].as_str().expect("key"))
        .collect();
    assert_eq!(
        names,
        vec!["verdant", "aurora", "paper", "signal", "gilded", "swiss"]
    );
    assert_eq!(themes[0]["default"], true);
    assert_eq!(themes[0]["enabled"], true);
    assert_eq!(themes[0]["css_url"], "/assets/app.css");
    assert!(themes
        .iter()
        .all(|theme| theme["css_url"].as_str().is_some()));
}

#[tokio::test]
async fn only_administrators_can_manage_themes() {
    let app = TestApp::spawn().await;
    admin(&app).await;
    let cookie = member(&app, "member@example.com").await;

    let response = app.get_as("/api/v1/admin/themes", &cookie).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.json()["error"]["code"], "forbidden");

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "default": "paper" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app.get_as("/app/admin/themes", &cookie).await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_admin_can_choose_the_enabled_set_and_default_theme() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({
                "enabled": ["paper", "swiss"],
                "default": "paper",
                "switching": "admin"
            }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let body = response.json();
    assert_eq!(body["switching"], "admin");
    let themes = body["themes"].as_array().expect("themes");
    let paper = themes
        .iter()
        .find(|theme| theme["key"] == "paper")
        .expect("paper");
    assert_eq!(paper["enabled"], true);
    assert_eq!(paper["default"], true);
    let verdant = themes
        .iter()
        .find(|theme| theme["key"] == "verdant")
        .expect("verdant");
    assert_eq!(verdant["enabled"], false);
    assert_eq!(verdant["default"], false);

    // The landing page switches to the new default immediately.
    let response = app.get("/").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("/assets/themes/paper"));
    assert!(response.body.contains("data-theme-key=\"paper\""));

    // The site policy is persisted and reflected in the admin page.
    let page = app.get_as("/app/admin/themes", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Paper Ledger"));
    assert!(page.body.contains("value=\"admin\" selected"));
}

#[tokio::test]
async fn a_member_may_switch_only_when_the_policy_allows_it() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    // `all` is the seeded policy, so a signed-in member can switch.
    let response = app
        .send(get_as("/app/theme/aurora?next=/app", &member_cookie))
        .await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location").as_deref(), Some("/app"));
    let theme_cookie = response.cookie_named("urlet_theme").expect("theme cookie");
    assert!(theme_cookie.contains("urlet_theme=aurora"));

    let combined = format!("{member_cookie}; {theme_cookie}");
    let page = app.send(get_as("/app", &combined)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("/assets/themes/aurora"));
    assert!(page.body.contains("data-theme-key=\"aurora\""));
    // Aurora is dark-only, so the light/dark toggle is not rendered.
    assert!(!page.body.contains("data-theme-toggle"));

    // Administrators switch the policy to admin-only.
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "switching": "admin" }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK);

    let response = app
        .send(get_as("/app/theme/paper?next=/app", &member_cookie))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn disabled_and_unknown_themes_are_rejected_or_fall_back() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["verdant"], "default": "verdant" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    // A stale cookie for a now-disabled theme renders the default instead.
    let page = app
        .send(get_as("/app", &format!("{cookie}; urlet_theme=aurora")))
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("/assets/app.css"));
    assert!(page.body.contains("data-theme-key=\"verdant\""));

    // The picker cannot set a disabled or unknown theme.
    let response = app
        .send(get_as("/app/theme/aurora?next=/app", &cookie))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let response = app
        .send(get_as("/app/theme/not-a-theme?next=/app", &cookie))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    // At least one theme and an enabled default are mandatory.
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": [], "default": "verdant" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["paper"], "default": "aurora" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn theme_assets_are_embedded_and_unknown_keys_are_404() {
    let app = TestApp::spawn().await;
    for key in ["aurora", "paper", "signal", "gilded", "swiss"] {
        let response = app.get(&format!("/assets/themes/{key}")).await;
        assert_eq!(response.status, StatusCode::OK, "{key}");
        assert_eq!(
            response.header("content-type").as_deref(),
            Some("text/css; charset=utf-8")
        );
        assert!(response.body.contains("body") || response.body.contains(":root"));
    }
    let response = app.get("/assets/themes/unknown").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let response = app.get("/assets/theme-ui.css").await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains(".theme-switcher"));
}

#[tokio::test]
async fn the_theme_picker_is_hidden_without_permission() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    // The admin sees the picker and can switch.
    let page = app.get_as("/app", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("data-theme-switcher"));
    assert!(page.body.contains("href=\"/app/theme/paper\""));

    // The signed-in landing page carries the same picker.
    let landing = app.get_as("/", &cookie).await;
    assert!(landing.body.contains("data-theme-switcher"));

    // A member sees it too while the policy is `all`.
    let page = app.get_as("/app", &member_cookie).await;
    assert!(page.body.contains("data-theme-switcher"));

    // Turn it off and the member's picker disappears, while the admin keeps it.
    app.send(put_json_as(
        "/api/v1/admin/themes",
        json!({ "switching": "admin" }),
        &cookie,
    ))
    .await;
    let page = app.get_as("/app", &member_cookie).await;
    assert!(!page.body.contains("data-theme-switcher"));
    let landing = app.get_as("/", &member_cookie).await;
    assert!(!landing.body.contains("data-theme-switcher"));
    let page = app.get_as("/app", &cookie).await;
    assert!(page.body.contains("data-theme-switcher"));
}

#[tokio::test]
async fn the_theme_form_updates_the_catalogue() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    let body = "theme_switching=admin\
                &enabled_verdant=on\
                &enabled_gilded=on\
                &default_theme=gilded";
    let response = app
        .send(post_form_as("/app/admin/themes", body, &cookie))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/admin/themes?ok=saved")
    );

    let page = app.get_as("/app/admin/themes", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("/assets/themes/gilded"));
    assert!(page.body.contains("selected"));

    // A form with everything unchecked is rejected without changing anything.
    let response = app
        .send(post_form_as(
            "/app/admin/themes",
            "theme_switching=all&default_theme=verdant",
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert!(response
        .header("location")
        .as_deref()
        .unwrap_or_default()
        .contains("err=theme"));

    let api = app.get_as("/api/v1/admin/themes", &cookie).await;
    let body = api.json();
    assert_eq!(body["switching"], "admin");
    let gilded = body["themes"]
        .as_array()
        .expect("themes")
        .iter()
        .find(|theme| theme["key"] == "gilded")
        .expect("gilded");
    assert_eq!(gilded["default"], true);
}

#[tokio::test]
async fn every_theme_renders_every_page() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    // One ordinary link for the detail page and one password-protected link
    // for the public password page.
    let created = app
        .post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com/ordinary" }),
            &cookie,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let ordinary_id = created.json()["id"].as_i64().expect("id");

    let protected = app
        .post_json_as(
            "/api/v1/urls",
            json!({
                "target_url": "https://example.com/protected",
                "password": "secret123"
            }),
            &cookie,
        )
        .await;
    assert_eq!(protected.status, StatusCode::CREATED, "{}", protected.body);
    let protected_slug = protected.json()["slug"].as_str().expect("slug").to_string();

    let pages = [
        "/",
        "/app",
        "/app/account",
        "/app/invites",
        "/app/admin",
        "/app/admin/users",
        "/app/admin/groups",
        "/app/admin/urls",
        "/app/admin/invites",
        "/app/admin/domains",
        "/app/admin/themes",
        "/app/admin/settings",
        "/app/admin/audit",
        "/app/urls/{id}",
    ];

    for key in ["verdant", "aurora", "paper", "signal", "gilded", "swiss"] {
        let css = if key == "verdant" {
            "/assets/app.css".to_string()
        } else {
            format!("/assets/themes/{key}")
        };
        let cookie_with_theme = format!("{cookie}; urlet_theme={key}");
        for template in pages {
            let uri = template.replace("{id}", &ordinary_id.to_string());
            let response = app.send(get_as(&uri, &cookie_with_theme)).await;
            assert_eq!(
                response.status,
                StatusCode::OK,
                "{key} {uri} failed: {}",
                response.body
            );
            assert!(
                response.body.contains(&css),
                "{key} {uri} did not load {css}"
            );
            assert!(
                response.body.contains(&format!("data-theme-key=\"{key}\"")),
                "{key} {uri} did not mark the theme"
            );
        }

        // The public password page and the generic 404 page also honour the
        // cookie, because both are rendered outside the signed-in shell.
        let password = app
            .send(get_as(&format!("/{protected_slug}"), &cookie_with_theme))
            .await;
        assert_eq!(password.status, StatusCode::OK, "{key} password page");
        assert!(password.body.contains(&css));
        assert!(password.body.contains(&format!("data-theme-key=\"{key}\"")));

        let missing = app
            .send(get_as("/definitely-not-a-slug", &cookie_with_theme))
            .await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND, "{key} 404");
        assert!(missing.body.contains(&css));
    }
}

#[tokio::test]
async fn every_theme_renders_the_public_pages() {
    let app = TestApp::spawn().await;
    for key in ["verdant", "aurora", "paper", "signal", "gilded", "swiss"] {
        let css = if key == "verdant" {
            "/assets/app.css".to_string()
        } else {
            format!("/assets/themes/{key}")
        };
        let cookie = format!("urlet_theme={key}");
        for uri in ["/", "/app/login", "/app/register"] {
            let response = app.send(get_as(uri, &cookie)).await;
            assert_eq!(
                response.status,
                StatusCode::OK,
                "{key} {uri} failed: {}",
                response.body
            );
            assert!(response.body.contains(&css), "{key} {uri} missing {css}");
            assert!(response.body.contains(&format!("data-theme-key=\"{key}\"")));
        }
        // The public shell still carries exactly one language switcher and the
        // sign-in form even when a non-default theme is active.
        let login = app.send(get_as("/app/login", &cookie)).await;
        assert_eq!(login.body.matches("data-lang-switcher").count(), 1);
        assert!(login.body.contains("name=\"password\""));
    }
}

#[tokio::test]
async fn every_theme_renders_the_extra_domain_landing() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    app.add_domain("extra.example.com", "active").await;

    for key in ["verdant", "aurora", "paper", "signal", "gilded", "swiss"] {
        let css = if key == "verdant" {
            "/assets/app.css".to_string()
        } else {
            format!("/assets/themes/{key}")
        };
        let cookie_with_theme = format!("{cookie}; urlet_theme={key}");
        let response = app
            .send(common::get_with_host_as(
                "/",
                "extra.example.com",
                &cookie_with_theme,
            ))
            .await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "{key} short landing: {}",
            response.body
        );
        assert!(response.body.contains(&css), "{key} missing {css}");
        assert!(response.body.contains(&format!("data-theme-key=\"{key}\"")));
        assert!(response.body.contains("noindex"));
    }
}
