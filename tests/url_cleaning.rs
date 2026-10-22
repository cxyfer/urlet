//! 清理設定與各建立入口的整合測試，使用正式路由及隔離資料庫。

mod common;

use axum::http::StatusCode;
use common::{patch_json_as, post_form, post_form_as, put_json_as, TestApp};
use serde_json::{json, Value};
use urlet::services::settings::Settings;

const ORIGINAL: &str =
    "https://www.youtube.com/watch?v=ABC%2fdef&si=track&utm_source=share&t=90#chapter";
const CLEANED: &str = "https://www.youtube.com/watch?v=ABC%2fdef&t=90#chapter";
const SETTINGS_FORM: &str = "site_name=urlet&registration_mode=open&default_slug_length=7&max_urls_per_user=1000&default_redirect_type=302&click_retention_days=0";

async fn configure(app: &TestApp, admin: &str, values: Value) -> common::TestResponse {
    app.send(put_json_as("/api/v1/admin/settings", values, admin))
        .await
}

async fn stored_target(app: &TestApp, slug: &str) -> String {
    sqlx::query_scalar("SELECT target_url FROM urls WHERE slug = ?")
        .bind(slug)
        .fetch_one(&app.state.db)
        .await
        .expect("stored target")
}

async fn check_redirect(app: &TestApp, slug: &str, target: &str) {
    assert_eq!(stored_target(app, slug).await, target);
    let response = app.get(&format!("/{slug}")).await;
    assert_eq!(response.status, StatusCode::FOUND, "{}", response.body);
    assert_eq!(response.header("location").as_deref(), Some(target));
}

#[tokio::test]
async fn settings_are_admin_only_atomic_and_survive_reload() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    let (member, _) = app.register("member@example.com", "supersecret").await;
    let defaults = app.get_as("/api/v1/admin/settings", &admin).await.json();
    assert_eq!(defaults["url_cleaning_enabled"], "false");
    assert_eq!(defaults["url_cleaning_rules"], "");
    assert_eq!(
        configure(&app, &member, json!({"url_cleaning_enabled":"true"}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(put_json_as(
            "/api/v1/admin/settings",
            json!({"url_cleaning_enabled":"true"}),
            ""
        ))
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );

    let rules = "# Custom site\nExample.COM utm_* ref";
    let saved = configure(
        &app,
        &admin,
        json!({"url_cleaning_enabled":"on", "url_cleaning_rules":rules}),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.body);
    assert_eq!(saved.json()["url_cleaning_enabled"], "true");
    assert_eq!(saved.json()["url_cleaning_rules"], rules);

    for invalid in [
        "* utm_*",
        "https://example.com ref",
        "example.com ref*bad",
        &" ".repeat(8193),
    ] {
        let response = configure(&app, &admin, json!({"url_cleaning_enabled":"false", "url_cleaning_rules":invalid, "site_name":"must not save"})).await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            response.body
        );
        assert_eq!(
            response.json()["error"]["details"]["field"],
            "url_cleaning_rules"
        );
        assert!(app.state.settings.url_cleaning_enabled());
        assert_eq!(app.state.settings.url_cleaning_rules(), rules);
        assert_eq!(app.state.settings.site_name(), "urlet");
    }
    assert_eq!(
        configure(&app, &admin, json!({"url_cleaning_enabled":"typo"}))
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let reloaded = Settings::load(&app.state.db, &app.state.config)
        .await
        .expect("reload");
    assert!(reloaded.url_cleaning_enabled());
    assert_eq!(reloaded.url_cleaning_rules(), rules);
    assert_eq!(reloaded.site_name(), "urlet");
    assert_eq!(
        reloaded
            .clean_target("https://a.example.com/?id=2&ref=x&utm_source=y".into())
            .unwrap(),
        "https://a.example.com/?id=2"
    );

    let audit = app
        .get_as("/api/v1/admin/audit?action=admin.settings_updated", &admin)
        .await
        .json();
    assert_eq!(
        audit["total"], 1,
        "rejected writes must not be audited as successful"
    );
}

#[tokio::test]
async fn creation_applies_policy_but_existing_links_and_target_edits_are_untouched() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    let (member, _) = app.register("member@example.com", "supersecret").await;
    let before = app
        .post_json_as(
            "/api/v1/urls",
            json!({"target_url":ORIGINAL,"slug":"before"}),
            &member,
        )
        .await;
    assert_eq!(before.status, StatusCode::CREATED);
    assert_eq!(before.json()["target_url"], ORIGINAL);
    assert_eq!(
        configure(
            &app,
            &admin,
            json!({"url_cleaning_enabled":"true", "url_cleaning_rules":"example.com ref"})
        )
        .await
        .status,
        StatusCode::OK
    );
    check_redirect(&app, "before", ORIGINAL).await;

    let created = app
        .post_json_as(
            "/api/v1/urls",
            json!({"target_url":ORIGINAL,"slug":"after"}),
            &member,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.json()["target_url"], CLEANED);
    check_redirect(&app, "after", CLEANED).await;
    let id = created.json()["id"].as_i64().unwrap();
    let edited = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({"target_url":ORIGINAL}),
            &member,
        ))
        .await;
    assert_eq!(edited.status, StatusCode::OK);
    assert_eq!(edited.json()["target_url"], ORIGINAL);
    check_redirect(&app, "after", ORIGINAL).await;

    for (slug, input, expected) in [
        (
            "custom",
            "https://sub.example.com/?id=123&ref=x",
            "https://sub.example.com/?id=123",
        ),
        (
            "unknown",
            "https://unknown.example/?utm_source=x",
            "https://unknown.example/?utm_source=x",
        ),
    ] {
        let result = app
            .post_json_as(
                "/api/v1/urls",
                json!({"target_url":input,"slug":slug}),
                &member,
            )
            .await;
        assert_eq!(result.status, StatusCode::CREATED);
        assert_eq!(result.json()["target_url"], expected);
        check_redirect(&app, slug, expected).await;
    }
    assert_eq!(
        configure(&app, &admin, json!({"url_cleaning_enabled":"false"}))
            .await
            .status,
        StatusCode::OK
    );
    let disabled = app
        .post_json_as(
            "/api/v1/urls",
            json!({"target_url":ORIGINAL,"slug":"disabled"}),
            &member,
        )
        .await;
    assert_eq!(disabled.status, StatusCode::CREATED);
    assert_eq!(disabled.json()["target_url"], ORIGINAL);

    assert_eq!(
        configure(&app, &admin, json!({"url_cleaning_enabled":"true"}))
            .await
            .status,
        StatusCode::OK
    );
    for invalid in [
        "javascript:alert(1)?utm_source=x",
        "https://youtube.com/\\evil?si=x",
    ] {
        let result = app
            .post_json_as("/api/v1/urls", json!({"target_url":invalid}), &member)
            .await;
        assert_eq!(result.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(result.json()["error"]["details"]["field"], "target_url");
    }
    let too_long = format!("https://youtube.com/?si={}", "x".repeat(2048));
    assert_eq!(
        app.post_json_as("/api/v1/urls", json!({"target_url":too_long}), &member)
            .await
            .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn public_api_and_both_browser_forms_store_clean_targets() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    assert_eq!(
        configure(
            &app,
            &admin,
            json!({"url_cleaning_enabled":"true", "public_create_access":"guest"})
        )
        .await
        .status,
        StatusCode::OK
    );
    let public = app
        .post_json(
            "/api/v1/public/urls",
            json!({"target_url":ORIGINAL,"slug":"public-api"}),
        )
        .await;
    assert_eq!(public.status, StatusCode::CREATED, "{}", public.body);
    assert_eq!(public.json()["target_url"], CLEANED);
    check_redirect(&app, "public-api", CLEANED).await;

    for (path, cookie, slug) in [
        ("/", None, "public-form"),
        ("/app/urls", Some(admin.as_str()), "dashboard-form"),
    ] {
        let body = serde_urlencoded::to_string([("target_url", ORIGINAL), ("slug", slug)]).unwrap();
        let response = app
            .send(post_form(
                path,
                &body,
                cookie,
                Some("http://localhost:8080"),
            ))
            .await;
        assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
        assert!(!response.header("location").unwrap().contains("err="));
        check_redirect(&app, slug, CLEANED).await;
    }

    let token = app
        .post_json_as("/api/v1/tokens", json!({"name":"cleanup test"}), &admin)
        .await;
    assert_eq!(token.status, StatusCode::CREATED, "{}", token.body);
    let bearer = token.json()["token"].as_str().unwrap().to_string();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/api/v1/urls")
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({"target_url":ORIGINAL,"slug":"token-api"}).to_string(),
        ))
        .unwrap();
    let response = app.send(request).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    assert_eq!(response.json()["target_url"], CLEANED);
    check_redirect(&app, "token-api", CLEANED).await;
}

#[tokio::test]
async fn settings_form_handles_invalid_rules_clearing_and_old_pages() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    let body =
        format!("{SETTINGS_FORM}&url_cleaning_enabled=on&url_cleaning_rules=example.com+ref");
    let saved = app
        .send(post_form_as("/app/admin/settings", &body, &admin))
        .await;
    assert_eq!(
        saved.header("location").as_deref(),
        Some("/app/admin/settings?ok=saved")
    );
    assert!(app.state.settings.url_cleaning_enabled());
    assert_eq!(app.state.settings.url_cleaning_rules(), "example.com ref");

    let old_page = app
        .send(post_form_as("/app/admin/settings", SETTINGS_FORM, &admin))
        .await;
    assert_eq!(old_page.status, StatusCode::SEE_OTHER);
    assert!(
        app.state.settings.url_cleaning_enabled(),
        "an older form must not reset cleanup"
    );
    let invalid = serde_urlencoded::to_string([
        ("url_cleaning_rules", "example.com <script>"),
        ("url_cleaning_enabled", "on"),
    ])
    .unwrap();
    let response = app
        .send(post_form_as(
            "/app/admin/settings",
            &format!("{SETTINGS_FORM}&{invalid}"),
            &format!("{admin}; urlet_lang=zh-Hant"),
        ))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.body.contains("規則未儲存"));
    assert!(response.body.contains("aria-invalid=\"true\""));
    assert!(response.body.contains("example.com &lt;script&gt;"));
    assert!(!response.body.contains("example.com <script>"));
    assert_eq!(app.state.settings.url_cleaning_rules(), "example.com ref");

    let cleared = app
        .send(post_form_as(
            "/app/admin/settings",
            &format!("{SETTINGS_FORM}&url_cleaning_rules="),
            &admin,
        ))
        .await;
    assert_eq!(cleared.status, StatusCode::SEE_OTHER);
    assert!(!app.state.settings.url_cleaning_enabled());
    assert_eq!(app.state.settings.url_cleaning_rules(), "");
    let reloaded = Settings::load(&app.state.db, &app.state.config)
        .await
        .unwrap();
    assert!(!reloaded.url_cleaning_enabled());
    assert_eq!(reloaded.url_cleaning_rules(), "");
}

#[tokio::test]
async fn cleanup_controls_render_in_every_theme_and_language() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    for theme in urlet::services::theme::ALL {
        for (locale, heading) in [
            ("en", "Tracking parameter cleanup"),
            ("zh-Hant", "追蹤參數清理"),
            ("zh-Hans", "追踪参数清理"),
        ] {
            let cookie = format!("{admin}; urlet_theme={}; urlet_lang={locale}", theme.key());
            let page = app.get_as("/app/admin/settings", &cookie).await;
            assert_eq!(page.status, StatusCode::OK, "{} {locale}", theme.key());
            assert!(page.body.contains(heading));
            assert!(page.body.contains("name=\"url_cleaning_enabled\""));
            assert!(page.body.contains("name=\"url_cleaning_rules\""));
            assert!(page.body.contains("youtube.com utm_* si feature"));
        }
    }
}

#[tokio::test]
async fn startup_seeds_missing_cleanup_keys_without_overwriting_existing_settings() {
    let app = TestApp::spawn().await;
    app.state
        .settings
        .update(&app.state.db, "site_name", "Existing site", None)
        .await
        .unwrap();
    sqlx::query("DELETE FROM settings WHERE key IN ('url_cleaning_enabled', 'url_cleaning_rules')")
        .execute(&app.state.db)
        .await
        .unwrap();
    let reloaded = Settings::load(&app.state.db, &app.state.config)
        .await
        .unwrap();
    assert_eq!(reloaded.site_name(), "Existing site");
    assert!(!reloaded.url_cleaning_enabled());
    assert_eq!(reloaded.url_cleaning_rules(), "");
    assert_eq!(reloaded.clean_target(ORIGINAL.into()).unwrap(), ORIGINAL);
}
