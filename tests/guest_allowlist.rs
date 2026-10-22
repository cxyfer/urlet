//! 訪客白名單在完整 middleware、公開建立與管理設定流程中的行為。

mod common;

use axum::http::StatusCode;
use common::{post_form, post_form_as, post_json, put_json_as, with_bearer, TestApp};
use serde_json::json;
use urlet::services::{guest_allowlist, settings::Settings};

const FORM: &str = "site_name=urlet&registration_mode=open&default_slug_length=7&max_urls_per_user=1000&default_redirect_type=302&click_retention_days=0";

async fn set(app: &TestApp, key: &str, value: &str) {
    app.state
        .settings
        .update(&app.state.db, key, value, None)
        .await
        .unwrap();
}

async fn open(app: &TestApp) {
    set(app, "public_create_access", "guest").await;
    set(app, "public_rate_limit_per_ip", "0").await;
}

async fn create(app: &TestApp, target: &str) -> common::TestResponse {
    app.post_json("/api/v1/public/urls", json!({"target_url": target}))
        .await
}

#[tokio::test]
async fn defaults_restrict_both_anonymous_endpoints_and_explain_recovery() {
    let app = TestApp::spawn().await;
    open(&app).await;
    assert!(app.state.settings.guest_allowlist_enabled());

    let refused = create(&app, "https://unlisted.example/article").await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert_eq!(refused.json()["error"]["code"], "forbidden");
    assert_eq!(refused.json()["error"]["message"], guest_allowlist::BLOCKED);
    let form = app
        .send(post_form(
            "/",
            "target_url=https://unlisted.example/article",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(
        form.header("location").as_deref(),
        Some("/?err=guest_allowlist")
    );
    let page = app.get("/?err=guest_allowlist").await;
    assert!(page
        .body
        .contains("This destination is not allowed for guests"));
    assert!(page.body.contains("View allowed destinations"));
    assert!(page.body.contains("*.youtube.com"));
    assert!(page.body.contains("<code>*.wikipedia.org</code>"));

    let accepted = create(&app, "www.youtube.com/watch?v=123").await;
    assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.body);
    assert_eq!(
        accepted.json()["target_url"],
        "https://www.youtube.com/watch?v=123"
    );
    for target in [
        "https://en.wikipedia.org/w/index.php",
        "https://stackoverflow.com/users/1",
        "https://www.cna.com.tw/",
        "https://gitlab.com/gitlab-org/gitlab",
        "https://developer.mozilla.org/en-US/docs/Web",
        "https://open.spotify.com/track/example",
        "https://www.reuters.com/world/",
    ] {
        let response = create(&app, target).await;
        assert_eq!(
            response.status,
            StatusCode::CREATED,
            "{target}: {}",
            response.body
        );
    }
    for target in ["https://user.github.io/", "https://user.gitlab.io/"] {
        assert_eq!(
            create(&app, target).await.status,
            StatusCode::FORBIDDEN,
            "{target}"
        );
    }
    let form = app
        .send(post_form(
            "/",
            "target_url=https://www.ptt.cc/bbs/Rust/index.html",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert!(form.header("location").unwrap().starts_with("/?shortened="));
}

#[tokio::test]
async fn forged_hosts_and_paths_do_not_spend_the_shared_guest_budget() {
    let app = TestApp::spawn().await;
    open(&app).await;
    set(&app, "guest_allowlist", "example.com/articles/").await;
    set(&app, "public_rate_limit_global", "1").await;
    for target in [
        "https://example.com.evil.test/articles/a",
        "https://sub.example.com/articles/a",
        "https://example.com@evil.test/articles/a",
        "https://evil.test@example.com/articles/a",
        "https://example.com:8443/articles/a",
        "https://example.com/redirect?next=/articles/a",
        "https://example.com/articles/%2e%2e/redirect",
        "https://example.com/articles/%252e%252e/redirect",
        "https://example.com/articles/a%2fb",
    ] {
        let response = create(&app, target).await;
        assert_eq!(
            response.status,
            StatusCode::FORBIDDEN,
            "{target}: {}",
            response.body
        );
    }
    assert_eq!(
        create(&app, "https://example.com/articles/one")
            .await
            .status,
        StatusCode::CREATED
    );
    assert_eq!(
        create(&app, "https://example.com/articles/two")
            .await
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM urls")
        .fetch_one(&app.state.db)
        .await
        .unwrap();
    assert_eq!(count, 1, "被拒絕的請求不得建立資料列");
}

#[tokio::test]
async fn disallowed_guests_are_rejected_before_captcha() {
    let app = TestApp::spawn_with(|config| {
        config.captcha_provider_override = Some("turnstile".into());
        config.captcha_site_key_override = Some("site-key".into());
        config.captcha_secret_key_override = Some("secret-key".into());
        config.captcha_verify_url = Some("http://127.0.0.1:1/siteverify".into());
    })
    .await;
    open(&app).await;
    assert_eq!(
        create(&app, "https://unlisted.example").await.status,
        StatusCode::FORBIDDEN
    );
    // 允許的網址才進入驗證碼流程；缺 token 時回傳 400。
    assert_eq!(
        create(&app, "https://github.com/cxyfer/urlet").await.status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn admin_edits_apply_immediately_and_invalid_batches_are_atomic() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    open(&app).await;
    let updated = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({
                "guest_allowlist": " Example.COM/articles/ \r\nexample.com/articles/\n",
                "guest_allowlist_enabled": "on"
            }),
            &admin,
        ))
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    assert_eq!(updated.json()["guest_allowlist"], "example.com/articles/");
    assert_eq!(updated.json()["guest_allowlist_enabled"], "true");
    assert_eq!(
        create(&app, "https://example.com/articles/one")
            .await
            .status,
        StatusCode::CREATED
    );
    assert_eq!(
        create(&app, "https://github.com/cxyfer/urlet").await.status,
        StatusCode::FORBIDDEN
    );
    for bad in [
        json!({"guest_allowlist":"foo.*.example.com", "site_name":"Bad"}),
        json!({"guest_allowlist_enabled":"typo", "site_name":"Bad"}),
    ] {
        let response = app
            .send(put_json_as("/api/v1/admin/settings", bad, &admin))
            .await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            response.body
        );
        assert_eq!(app.state.settings.site_name(), "urlet");
        assert!(app.state.settings.guest_allowlist_enabled());
        assert_eq!(
            app.state.settings.guest_allowlist(),
            "example.com/articles/"
        );
    }
    let saved = app
        .send(post_form_as(
            "/app/admin/settings",
            &format!("{FORM}&guest_allowlist_enabled=on&guest_allowlist=other.example%2Fnews%2F"),
            &admin,
        ))
        .await;
    assert_eq!(
        saved.header("location").as_deref(),
        Some("/app/admin/settings?ok=saved")
    );
    assert_eq!(
        create(&app, "https://example.com/articles/two")
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        create(&app, "https://other.example/news/one").await.status,
        StatusCode::CREATED
    );
    let page = app.get_as("/app/admin/settings", &admin).await;
    assert!(page.body.contains("other.example/news/"));

    let invalid = app
        .send(post_form_as(
            "/app/admin/settings",
            &format!("{FORM}&guest_allowlist_enabled=on&guest_allowlist=https%3A%2F%2Fbad.example"),
            &admin,
        ))
        .await;
    assert_eq!(
        invalid.header("location").as_deref(),
        Some("/app/admin/settings?err=guest_allowlist_rules")
    );
    assert_eq!(app.state.settings.guest_allowlist(), "other.example/news/");
}

#[tokio::test]
async fn wildcard_edits_roundtrip_through_admin_api_and_form_and_enforce_subdomains_only() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    open(&app).await;
    let updated = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({"guest_allowlist":" *.Wikipedia.ORG.\n*.wikipedia.org\n"}),
            &admin,
        ))
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    assert_eq!(updated.json()["guest_allowlist"], "*.wikipedia.org");
    for target in [
        "https://zh.wikipedia.org/wiki/Rust",
        "https://de.m.wikipedia.org/wiki/Rust",
    ] {
        assert_eq!(create(&app, target).await.status, StatusCode::CREATED);
    }
    for target in [
        "https://wikipedia.org/wiki/Rust",
        "https://evilwikipedia.org/wiki/Rust",
        "https://zh.wikipedia.org.evil.test/wiki/Rust",
    ] {
        assert_eq!(create(&app, target).await.status, StatusCode::FORBIDDEN);
    }

    let saved = app
        .send(post_form_as(
            "/app/admin/settings",
            &format!("{FORM}&guest_allowlist_enabled=on&guest_allowlist=%2A.wikipedia.org%2Fwiki%2F%0Awikipedia.org"),
            &admin,
        ))
        .await;
    assert_eq!(
        saved.header("location").as_deref(),
        Some("/app/admin/settings?ok=saved")
    );
    let rules = "*.wikipedia.org/wiki/\nwikipedia.org";
    assert_eq!(app.state.settings.guest_allowlist(), rules);
    let reloaded = Settings::load(&app.state.db, &app.state.config)
        .await
        .unwrap();
    assert_eq!(reloaded.guest_allowlist(), rules);
    let page = app.get_as("/app/admin/settings", &admin).await;
    assert!(page.body.contains("*.wikipedia.org/wiki/"));
    let page = app.get("/").await;
    assert!(page.body.contains("<code>*.wikipedia.org/wiki/</code>"));
    assert_eq!(
        create(&app, "https://wikipedia.org/").await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        create(&app, "https://fr.m.wikipedia.org/wiki/Rust")
            .await
            .status,
        StatusCode::CREATED
    );
    assert_eq!(
        create(&app, "https://fr.m.wikipedia.org/w/index.php")
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let form = app
        .send(post_form(
            "/",
            "target_url=https://fr.m.wikipedia.org/wiki/Rust",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert!(form.header("location").unwrap().starts_with("/?shortened="));
    let form = app
        .send(post_form(
            "/",
            "target_url=https://fr.m.wikipedia.org/w/index.php",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(
        form.header("location").as_deref(),
        Some("/?err=guest_allowlist")
    );
}

#[tokio::test]
async fn empty_lists_persist_and_old_forms_cannot_disable_the_guard() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    open(&app).await;
    let cleared = app
        .send(post_form_as(
            "/app/admin/settings",
            &format!("{FORM}&guest_allowlist_enabled=on&guest_allowlist="),
            &admin,
        ))
        .await;
    assert_eq!(
        cleared.header("location").as_deref(),
        Some("/app/admin/settings?ok=saved")
    );
    let old = app
        .send(post_form_as("/app/admin/settings", FORM, &admin))
        .await;
    assert_eq!(
        old.header("location").as_deref(),
        Some("/app/admin/settings?ok=saved")
    );
    assert!(app.state.settings.guest_allowlist_enabled());
    assert_eq!(
        create(&app, "https://github.com/cxyfer/urlet").await.status,
        StatusCode::FORBIDDEN
    );
    assert!(app
        .get("/")
        .await
        .body
        .contains("No destinations are currently allowed"));
    let reloaded = Settings::load(&app.state.db, &app.state.config)
        .await
        .unwrap();
    assert!(reloaded.guest_allowlist_enabled());
    assert_eq!(reloaded.guest_allowlist(), "");

    // 未勾選但有 textarea 的新表單，才是明確關閉白名單。
    app.send(post_form_as(
        "/app/admin/settings",
        &format!("{FORM}&guest_allowlist="),
        &admin,
    ))
    .await;
    assert!(!app.state.settings.guest_allowlist_enabled());
    assert_eq!(
        create(&app, "https://unlisted.example/now-allowed")
            .await
            .status,
        StatusCode::CREATED
    );
    assert!(!app
        .get("/")
        .await
        .body
        .contains("View allowed destinations"));
    let reloaded = Settings::load(&app.state.db, &app.state.config)
        .await
        .unwrap();
    assert!(!reloaded.guest_allowlist_enabled());
    assert_eq!(reloaded.guest_allowlist(), "");
}

#[tokio::test]
async fn signed_in_members_and_tokens_keep_their_existing_permissions() {
    let app = TestApp::spawn().await;
    app.register("admin@example.com", "supersecret").await;
    let (member, _) = app.register("member@example.com", "supersecret").await;
    open(&app).await;
    set(&app, "guest_allowlist", "").await;
    for endpoint in ["/api/v1/public/urls", "/api/v1/urls"] {
        let response = app
            .post_json_as(
                endpoint,
                json!({"target_url":"https://unlisted.example/member"}),
                &member,
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    }
    for endpoint in ["/", "/app/urls"] {
        let response = app
            .send(post_form(
                endpoint,
                "target_url=https://unlisted.example/form",
                Some(&member),
                Some("http://localhost:8080"),
            ))
            .await;
        assert_eq!(response.status, StatusCode::SEE_OTHER);
        assert!(!response.header("location").unwrap().contains("err="));
    }
    let issued = app
        .post_json_as("/api/v1/tokens", json!({"name":"allowlist-test"}), &member)
        .await;
    assert_eq!(issued.status, StatusCode::CREATED, "{}", issued.body);
    let token = issued.json()["token"].as_str().unwrap().to_string();
    for endpoint in ["/api/v1/public/urls", "/api/v1/urls"] {
        let response = app
            .send(with_bearer(
                post_json(
                    endpoint,
                    json!({"target_url":"https://unlisted.example/token"}),
                ),
                &token,
            ))
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    }
    assert!(!app
        .get_as("/", &member)
        .await
        .body
        .contains("View allowed destinations"));
    let refused = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({"guest_allowlist_enabled":"false"}),
            &member,
        ))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let refused = app
        .send(post_form_as(
            "/app/admin/settings",
            &format!("{FORM}&guest_allowlist="),
            &member,
        ))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    assert!(app.state.settings.guest_allowlist_enabled());
}

#[tokio::test]
async fn every_theme_displays_the_controls_and_guest_rules() {
    let app = TestApp::spawn().await;
    let (admin, _) = app.register("admin@example.com", "supersecret").await;
    open(&app).await;
    for theme in ["verdant", "aurora", "paper", "signal", "gilded", "swiss"] {
        let page = app.get_as("/", &format!("urlet_theme={theme}")).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("View allowed destinations"), "{theme}");
        let page = app
            .get_as(
                "/app/admin/settings",
                &format!("{admin}; urlet_theme={theme}"),
            )
            .await;
        assert_eq!(page.status, StatusCode::OK);
        for field in ["guest_allowlist", "guest_allowlist_enabled"] {
            assert!(
                page.body.contains(&format!("id=\"{field}\"")),
                "{theme}: {field}"
            );
        }
    }
}
