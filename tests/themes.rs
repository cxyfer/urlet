//! Theme catalogue, switching and permission integration tests.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
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

    // A partial update that only changes the policy must keep the catalogue
    // that the previous request committed.
    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "switching": "all" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let body = response.json();
    assert_eq!(body["switching"], "all");
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
    for theme in urlet::services::theme::ALL {
        let key = theme.key();
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
async fn embedded_asset_urls_are_versioned() {
    let app = TestApp::spawn().await;
    let version = urlet::VERSION;

    // The default theme carries the build version on every CSS/JS URL, so a
    // browser cannot keep a previous release's stylesheet for an hour.
    let page = app.get("/").await;
    for asset in [
        "/assets/theme.js",
        "/assets/app.css",
        "/assets/app.js",
        "/assets/theme-ui.css",
    ] {
        let expected = format!("{asset}?v={version}");
        assert!(page.body.contains(&expected), "missing {expected}");
    }

    // Non-default themes version their own stylesheet URL too.
    let cookie = admin(&app).await;
    let themed = app
        .get_as("/app", &format!("{cookie}; urlet_theme=aurora"))
        .await;
    let expected = format!("/assets/themes/aurora?v={version}");
    assert!(themed.body.contains(&expected), "missing {expected}");
}

#[tokio::test]
async fn the_theme_picker_menu_paints_its_own_text_and_background() {
    let app = TestApp::spawn().await;
    let response = app.get("/assets/theme-ui.css").await;
    assert_eq!(response.status, StatusCode::OK);
    let rule = response
        .body
        .split(".theme-menu {")
        .nth(1)
        .and_then(|rest| rest.split('}').next())
        .expect("the shared picker CSS must define .theme-menu");
    // The menu floats over content and must not inherit the colour of the bar
    // that opened it: on inverted bars (Swiss, Signal, Gilded) that colour is
    // the same as the menu surface, so both sides of the pair are explicit.
    assert!(rule.contains("background:"), "{rule}");
    assert!(rule.contains("color:"), "{rule}");
    assert!(!rule.contains("color: inherit"), "{rule}");
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
    assert_eq!(
        page.body.matches("aria-current=\"true\"").count(),
        1,
        "the picker must mark exactly one current entry"
    );

    // The signed-in landing page carries the same picker.
    let landing = app.get_as("/", &cookie).await;
    assert!(landing.body.contains("data-theme-switcher"));

    // A member sees it too while the policy is `all`.
    let page = app.get_as("/app", &member_cookie).await;
    assert!(page.body.contains("data-theme-switcher"));

    // Turn it off: the member's picker disappears, and an existing cookie is
    // ignored at render time rather than only being hidden in the UI.
    let member_theme = format!("{member_cookie}; urlet_theme=aurora");
    let before = app.get_as("/app", &member_theme).await;
    assert!(before.body.contains("/assets/themes/aurora"));
    app.send(put_json_as(
        "/api/v1/admin/themes",
        json!({ "switching": "admin" }),
        &cookie,
    ))
    .await;
    let page = app.get_as("/app", &member_theme).await;
    assert!(!page.body.contains("data-theme-switcher"));
    assert!(page.body.contains("/assets/app.css"));
    assert!(page.body.contains("data-theme-key=\"verdant\""));
    let landing = app.get_as("/", &member_theme).await;
    assert!(!landing.body.contains("data-theme-switcher"));
    assert!(landing.body.contains("/assets/app.css"));
    // Anonymous cookies are suppressed too under `admin`.
    let anonymous = app.get_as("/", "urlet_theme=paper").await;
    assert!(anonymous.body.contains("/assets/app.css"));
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

    for theme in urlet::services::theme::ALL {
        let key = theme.key();
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
            if uri == "/app" {
                assert!(
                    response.body.contains("Your links"),
                    "{key} dashboard content is empty"
                );
            }
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
    for theme in urlet::services::theme::ALL {
        let key = theme.key();
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

    for theme in urlet::services::theme::ALL {
        let key = theme.key();
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

#[tokio::test]
async fn catalogue_and_policy_roll_back_together() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;

    // Remove the destination table of the second write. The theme transaction
    // must fail as a whole: the enabled/default rows written before the settings
    // insert have to roll back, not be left behind under a 500 response.
    sqlx::query("DROP TABLE settings")
        .execute(&app.state.db)
        .await
        .expect("drop settings");

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({
                "enabled": ["paper"],
                "default": "paper",
                "switching": "admin"
            }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);

    let enabled: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM themes WHERE is_enabled = 1")
        .fetch_one(&app.state.db)
        .await
        .expect("enabled count");
    let default: String = sqlx::query_scalar("SELECT key FROM themes WHERE is_default = 1")
        .fetch_one(&app.state.db)
        .await
        .expect("default key");
    assert_eq!(enabled, 6, "the enabled set must roll back with the policy");
    assert_eq!(
        default, "verdant",
        "the default must roll back with the policy"
    );
}

#[tokio::test]
async fn administrators_can_preview_a_disabled_theme_without_enabling_it() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "enabled": ["verdant"], "default": "verdant" }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    // A member cannot use the preview cookie.
    let response = app
        .send(get_as(
            "/app/theme/aurora?preview=1&next=/app",
            &member_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    let response = app
        .send(get_as(
            "/app/theme/aurora?preview=1&next=/app",
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FOUND);
    let preview = response
        .cookie_named("urlet_theme_preview")
        .expect("preview cookie");
    let raw = response
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("urlet_theme_preview="))
        .expect("raw preview cookie");
    assert!(raw.contains("HttpOnly"));
    assert!(raw.contains("SameSite=Lax"));
    assert!(raw.contains("Path=/"));
    assert!(raw.contains("Max-Age=3600"));
    let combined = format!("{admin_cookie}; {preview}");
    let page = app.get_as("/app", &combined).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("/assets/themes/aurora"));
    assert!(page.body.contains("data-theme-key=\"aurora\""));

    let admin_page = app.get_as("/app/admin/themes", &combined).await;
    assert!(admin_page.body.contains("Stop preview"));
    assert!(admin_page.body.contains("preview=1"));

    let stop = app
        .send(get_as("/app/theme/preview/stop?next=/app", &combined))
        .await;
    assert_eq!(stop.status, StatusCode::FOUND);
    let cleared = stop
        .cookie_named("urlet_theme_preview")
        .expect("clear cookie");
    assert!(cleared.starts_with("urlet_theme_preview="));
    let page = app
        .get_as("/app", &format!("{admin_cookie}; {cleared}"))
        .await;
    assert!(page.body.contains("/assets/app.css"));
}

#[tokio::test]
async fn the_site_default_clears_the_explicit_choice() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let switched = app
        .send(get_as("/app/theme/paper?next=/app", &cookie))
        .await;
    let theme_cookie = switched.cookie_named("urlet_theme").expect("theme");
    let combined = format!("{cookie}; {theme_cookie}");
    assert!(app
        .get_as("/app", &combined)
        .await
        .body
        .contains("/assets/themes/paper"));

    let response = app
        .send(get_as("/app/theme/default?next=/app", &combined))
        .await;
    assert_eq!(response.status, StatusCode::FOUND);
    let cleared = response.cookie_named("urlet_theme").expect("clear cookie");
    assert!(cleared.starts_with("urlet_theme="));
    let page = app.get_as("/app", &format!("{cookie}; {cleared}")).await;
    assert!(page.body.contains("/assets/app.css"));
}

#[tokio::test]
async fn logging_out_clears_the_theme_cookies() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let response = app.send(post_form_as("/app/logout", "", &cookie)).await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    let cookies: Vec<String> = response
        .headers
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_string))
        .collect();
    assert!(cookies
        .iter()
        .any(|value| value.starts_with("urlet_session=;")));
    assert!(cookies
        .iter()
        .any(|value| value.starts_with("urlet_theme=;")));
    assert!(cookies
        .iter()
        .any(|value| value.starts_with("urlet_theme_preview=;")));
}

#[tokio::test]
async fn the_theme_admin_form_is_admin_and_csrf_protected() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    // A member cannot reach or POST the page even with a same-origin form.
    let response = app
        .send(post_form_as(
            "/app/admin/themes",
            "theme_switching=all&default_theme=verdant&enabled_verdant=on",
            &member_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);

    // A cookie-authenticated POST without an Origin/Referer is rejected by the
    // shared CSRF middleware before the handler runs.
    let response = app
        .send(common::post_form(
            "/app/admin/themes",
            "theme_switching=all&default_theme=verdant&enabled_verdant=on",
            Some(&admin_cookie),
            None,
        ))
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_theme_policy_can_be_changed_through_the_settings_api() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let member_cookie = member(&app, "member@example.com").await;

    let response = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({ "theme_switching": "admin" }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    let catalogue = app.get_as("/api/v1/admin/themes", &admin_cookie).await;
    assert_eq!(catalogue.json()["switching"], "admin");
    let page = app.get_as("/app", &member_cookie).await;
    assert!(!page.body.contains("data-theme-switcher"));
}

#[tokio::test]
async fn theme_updates_are_audited() {
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

    let audit = app.get_as("/api/v1/admin/audit", &cookie).await;
    assert_eq!(audit.status, StatusCode::OK);
    assert!(
        audit.body.contains("admin.themes_updated"),
        "theme update missing from audit: {}",
        audit.body
    );
}

#[tokio::test]
async fn ported_themes_keep_the_optional_invite_field() {
    let app = TestApp::spawn().await;
    for key in ["aurora", "paper", "signal", "gilded", "swiss"] {
        let page = app
            .get_as("/app/register", &format!("urlet_theme={key}"))
            .await;
        assert_eq!(page.status, StatusCode::OK, "{key}");
        assert!(
            page.body.contains("name=\"invite_code\""),
            "{key} register form lost the invite field"
        );
        assert!(
            page.body
                .contains("Optional: if you were given a group invite code"),
            "{key} register form lost the optional invite hint"
        );
    }
}

#[tokio::test]
async fn a_stale_enabled_row_does_not_break_a_partial_update() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let now = urlet::services::time::now_iso();
    sqlx::query(
        "INSERT INTO themes (key, is_enabled, is_default, sort_order, created_at, updated_at)
         VALUES ('ghost', 1, 0, 999, ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&app.state.db)
    .await
    .expect("insert ghost");
    app.state
        .themes
        .reload(&app.state.db)
        .await
        .expect("reload themes");

    let response = app
        .send(put_json_as(
            "/api/v1/admin/themes",
            json!({ "switching": "admin" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let body = response.json();
    assert_eq!(body["switching"], "admin");
    assert!(body["themes"]
        .as_array()
        .expect("themes")
        .iter()
        .all(|theme| theme["key"] != "ghost"));
    let enabled: i64 = sqlx::query_scalar("SELECT is_enabled FROM themes WHERE key = 'ghost'")
        .fetch_one(&app.state.db)
        .await
        .expect("ghost row");
    assert_eq!(enabled, 0, "the unknown row must be disabled by the update");
}

#[tokio::test]
async fn the_theme_cookie_is_secure_when_configured() {
    let app = TestApp::spawn_with(|config| config.secure_cookies = true).await;
    let (cookie, _) = app.register("admin@example.com", "supersecret").await;
    let response = app
        .send(get_as("/app/theme/paper?next=/app", &cookie))
        .await;
    assert_eq!(response.status, StatusCode::FOUND);
    let raw = response
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("urlet_theme="))
        .expect("theme cookie");
    assert!(raw.contains("Secure"));
    assert!(raw.contains("HttpOnly"));
    assert!(raw.contains("SameSite=Lax"));
}

#[tokio::test]
async fn the_referer_keeps_theme_switch_on_the_same_site() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("admin@example.com", "supersecret").await;
    let request = Request::builder()
        .method("GET")
        .uri("/app/theme/paper")
        .header(header::HOST, "localhost:8080")
        .header(header::COOKIE, &cookie)
        .header(header::REFERER, "http://localhost:8080/app/account")
        .body(Body::empty())
        .expect("request");
    let response = app.send(request).await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location").as_deref(), Some("/app/account"));

    // An external referer is ignored rather than becoming an open redirect.
    let request = Request::builder()
        .method("GET")
        .uri("/app/theme/paper")
        .header(header::HOST, "localhost:8080")
        .header(header::COOKIE, &cookie)
        .header(header::REFERER, "https://evil.example/app")
        .body(Body::empty())
        .expect("request");
    let response = app.send(request).await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location").as_deref(), Some("/app"));
}

#[tokio::test]
async fn concurrent_partial_updates_do_not_lose_the_catalogue() {
    let app = TestApp::spawn().await;
    let enabled = vec!["paper".to_string()];

    let catalogue = urlet::services::theme::update_selection(
        &app.state.db,
        &app.state.themes,
        &app.state.settings,
        Some(&enabled),
        Some("paper"),
        None,
        None,
    );
    let policy = urlet::services::theme::update_selection(
        &app.state.db,
        &app.state.themes,
        &app.state.settings,
        None,
        None,
        Some(urlet::services::settings::ThemeSwitching::AdminOnly),
        None,
    );
    let (catalogue, policy) = tokio::join!(catalogue, policy);
    catalogue.expect("catalogue update");
    policy.expect("policy update");

    let paper: (i64, i64) =
        sqlx::query_as("SELECT is_enabled, is_default FROM themes WHERE key = 'paper'")
            .fetch_one(&app.state.db)
            .await
            .expect("paper row");
    let switching: String =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'theme_switching'")
            .fetch_one(&app.state.db)
            .await
            .expect("policy");
    assert_eq!(
        paper,
        (1, 1),
        "the policy update must keep paper enabled/default"
    );
    assert_eq!(switching, "admin");
    assert_eq!(
        app.state.settings.theme_switching(),
        urlet::services::settings::ThemeSwitching::AdminOnly
    );
    assert!(app
        .state
        .themes
        .is_enabled(urlet::services::theme::ThemeKey::Paper));
}

#[tokio::test]
async fn every_theme_renders_the_splash_page() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let created = app
        .send(common::post_json_as(
            "/api/v1/urls",
            json!({
                "target_url": "https://example.com/themed",
                "slug": "themed-splash",
                "redirect_mode": "splash",
                "splash_title": "Theme splash"
            }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);

    for key in ["verdant", "aurora", "paper", "signal", "gilded", "swiss"] {
        let switched = app
            .send(get_as(&format!("/app/theme/{key}?next=/app"), &cookie))
            .await;
        assert_eq!(
            switched.status,
            StatusCode::FOUND,
            "{key}: {}",
            switched.body
        );
        let theme_cookie = switched.cookie_named("urlet_theme").expect("theme cookie");
        let both = format!("{cookie}; {theme_cookie}");
        let page = app.get_as("/themed-splash", &both).await;
        assert_eq!(page.status, StatusCode::OK, "{key}: {}", page.body);
        assert!(page.body.contains("Theme splash"), "{key} lost the copy");
        assert!(
            page.body.contains(&format!("data-theme-key=\"{key}\"")),
            "{key} did not render its own shell"
        );
        assert!(
            page.body.contains("Continue now"),
            "{key} did not render the splash body"
        );
    }
}
