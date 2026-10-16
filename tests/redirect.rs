//! Integration tests for the public redirect, password unlock and analytics.

mod common;

use axum::http::{header, StatusCode};
use common::{post_form, post_json_as, TestApp};
use serde_json::{json, Value};
use std::time::Duration;

async fn create_url(app: &TestApp, cookie: &str, payload: Value) -> Value {
    let response = app
        .send(post_json_as("/api/v1/urls", payload, cookie))
        .await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "create failed: {}",
        response.body
    );
    response.json()
}

/// Wait until the detached click task finished writing.
///
/// `urls.click_count` is updated last, so waiting for it means the click row and
/// the daily rollup are already committed.
async fn wait_for_clicks(app: &TestApp, url_id: i64, expected: i64) -> i64 {
    let mut seen = 0;
    for _ in 0..150 {
        seen = sqlx::query_scalar("SELECT click_count FROM urls WHERE id = ?")
            .bind(url_id)
            .fetch_one(&app.state.db)
            .await
            .expect("count clicks");
        if seen >= expected {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("expected {expected} clicks, saw {seen}");
}

#[tokio::test]
async fn redirects_with_the_configured_status() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com/landing", "slug": "go" }),
    )
    .await;

    let response = app.get("/go").await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(
        response.header("location").as_deref(),
        Some("https://example.com/landing")
    );
    assert_eq!(
        response.header("cache-control").as_deref(),
        Some("private, no-store")
    );
}

#[tokio::test]
async fn supports_permanent_redirects() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com/permanent", "slug": "perm", "redirect_type": 301 }),
    )
    .await;

    let response = app.get("/perm").await;
    assert_eq!(response.status, StatusCode::MOVED_PERMANENTLY);
    assert_eq!(
        response.header("cache-control").as_deref(),
        Some("private, max-age=300")
    );
}

#[tokio::test]
async fn slugs_are_case_insensitive_on_lookup() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "MiXeD" }),
    )
    .await;

    assert_eq!(app.get("/mixed").await.status, StatusCode::FOUND);
    assert_eq!(app.get("/MIXED").await.status, StatusCode::FOUND);
}

#[tokio::test]
async fn unknown_disabled_and_deleted_links_are_404() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let missing = app.get("/nothing-here").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.body.contains("Link not found"));

    let disabled = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "off" }),
    )
    .await;
    app.send(common::patch_json_as(
        &format!("/api/v1/urls/{}", disabled["id"].as_i64().expect("id")),
        json!({ "status": "disabled" }),
        &cookie,
    ))
    .await;
    assert_eq!(app.get("/off").await.status, StatusCode::NOT_FOUND);

    let deleted = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "gone" }),
    )
    .await;
    app.send(common::delete_as(
        &format!("/api/v1/urls/{}", deleted["id"].as_i64().expect("id")),
        &cookie,
    ))
    .await;
    assert_eq!(app.get("/gone").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn expired_and_exhausted_links_are_410() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let expiring = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "expiring", "expires_at": "2030-01-01" }),
    )
    .await;
    assert_eq!(app.get("/expiring").await.status, StatusCode::FOUND);
    sqlx::query("UPDATE urls SET expires_at = ? WHERE id = ?")
        .bind("2000-01-01T00:00:00.000Z")
        .bind(expiring["id"].as_i64().expect("id"))
        .execute(&app.state.db)
        .await
        .expect("expire");
    let expired = app.get("/expiring").await;
    assert_eq!(expired.status, StatusCode::GONE);
    assert!(expired.body.contains("expired"));

    let limited = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "limited", "max_clicks": 1 }),
    )
    .await;
    let id = limited["id"].as_i64().expect("id");
    assert_eq!(app.get("/limited").await.status, StatusCode::FOUND);
    wait_for_clicks(&app, id, 1).await;
    let reached = app.get("/limited").await;
    assert_eq!(reached.status, StatusCode::GONE);
    assert!(reached.body.contains("maximum number of clicks"));
}

#[tokio::test]
async fn password_protected_links_never_leak_the_target() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://secret.example/private", "slug": "vault", "password": "linkpass" }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    let page = app.get("/vault").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Protected link"));
    assert!(
        !page.body.contains("secret.example"),
        "the target must not be rendered"
    );
    assert!(page.header("cache-control").is_some());

    // No click is recorded for a page view alone.
    tokio::time::sleep(Duration::from_millis(60)).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clicks WHERE url_id = ?")
        .bind(id)
        .fetch_one(&app.state.db)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn unlock_flow_with_form_and_json() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://secret.example/private", "slug": "vault2", "password": "linkpass" }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    // Wrong password: form submission re-renders the page with an error.
    let wrong = app
        .send(post_form(
            "/vault2/unlock",
            "password=nope",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert!(wrong.body.contains("Incorrect password"));
    assert!(!wrong.body.contains("secret.example"));

    // Wrong password via JSON.
    let wrong_json = app
        .send(common::post_json(
            "/vault2/unlock",
            json!({ "password": "nope" }),
        ))
        .await;
    assert_eq!(wrong_json.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_json.json()["error"]["code"], "unauthorized");

    // Correct password via JSON returns the redirect target and a cookie.
    let ok_json = app
        .send(common::post_json(
            "/vault2/unlock",
            json!({ "password": "linkpass" }),
        ))
        .await;
    assert_eq!(ok_json.status, StatusCode::OK, "{}", ok_json.body);
    assert_eq!(ok_json.json()["redirect"], "/vault2");
    let unlock_cookie = ok_json
        .cookie_named("uwurls_unlock_vault2")
        .expect("unlock cookie");
    assert!(ok_json
        .header("set-cookie")
        .expect("set-cookie")
        .contains("HttpOnly"));

    // With the cookie the link redirects.
    let followed = app.get_as("/vault2", &unlock_cookie).await;
    assert_eq!(followed.status, StatusCode::FOUND);
    assert_eq!(
        followed.header("location").as_deref(),
        Some("https://secret.example/private")
    );
    wait_for_clicks(&app, id, 1).await;

    // Form flow returns a 303 to the short link.
    let ok_form = app
        .send(post_form(
            "/vault2/unlock",
            "password=linkpass",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(ok_form.status, StatusCode::SEE_OTHER);
    assert_eq!(ok_form.header("location").as_deref(), Some("/vault2"));
    assert!(ok_form.cookie_named("uwurls_unlock_vault2").is_some());
}

#[tokio::test]
async fn unlock_cookies_are_scoped_to_one_link() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    for slug in ["first", "second"] {
        create_url(
            &app,
            &cookie,
            json!({ "target_url": "https://example.com", "slug": slug, "password": "linkpass" }),
        )
        .await;
    }

    let unlocked = app
        .send(common::post_json(
            "/first/unlock",
            json!({ "password": "linkpass" }),
        ))
        .await;
    let unlock_cookie = unlocked
        .cookie_named("uwurls_unlock_first")
        .expect("cookie");

    assert_eq!(
        app.get_as("/first", &unlock_cookie).await.status,
        StatusCode::FOUND
    );
    // The same cookie value under another name must not unlock the second link.
    let forged = unlock_cookie.replace("uwurls_unlock_first", "uwurls_unlock_second");
    let second = app.get_as("/second", &forged).await;
    assert_eq!(second.status, StatusCode::OK);
    assert!(second.body.contains("Protected link"));
}

#[tokio::test]
async fn unlock_attempts_are_rate_limited() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "brute", "password": "linkpass" }),
    )
    .await;

    let mut limited = false;
    for _ in 0..15 {
        let response = app
            .send(common::post_json(
                "/brute/unlock",
                json!({ "password": "wrong" }),
            ))
            .await;
        if response.status == StatusCode::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
    }
    assert!(limited, "unlock must be rate limited");
}

#[tokio::test]
async fn clicks_are_recorded_with_analytics_dimensions() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "tracked" }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    let mut builder = axum::http::Request::builder()
        .method("GET")
        .uri("/tracked")
        .header(header::HOST, "localhost:8080")
        .header(
            header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36",
        )
        .header(header::REFERER, "https://news.example.com/story");
    builder = builder.header(header::COOKIE, "unrelated=1");
    let response = app
        .send(builder.body(axum::body::Body::empty()).expect("request"))
        .await;
    assert_eq!(response.status, StatusCode::FOUND);

    wait_for_clicks(&app, id, 1).await;

    let row: (String, String, String, Option<String>, i64, i64) = sqlx::query_as(
        "SELECT device_type, browser, os, referrer_host, is_bot, is_unique FROM clicks WHERE url_id = ?",
    )
    .bind(id)
    .fetch_one(&app.state.db)
    .await
    .expect("click row");
    assert_eq!(row.0, "Desktop");
    assert_eq!(row.1, "Chrome");
    assert_eq!(row.2, "Windows");
    assert_eq!(row.3.as_deref(), Some("news.example.com"));
    assert_eq!(row.4, 0);
    assert_eq!(row.5, 1, "first click from a visitor is unique");

    // Counters and rollups are updated too.
    let (clicks, unique, last): (i64, i64, Option<String>) = sqlx::query_as(
        "SELECT click_count, unique_click_count, last_clicked_at FROM urls WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&app.state.db)
    .await
    .expect("url counters");
    assert_eq!((clicks, unique), (1, 1));
    assert!(last.is_some());

    let rollup: i64 = sqlx::query_scalar("SELECT clicks FROM url_daily_stats WHERE url_id = ?")
        .bind(id)
        .fetch_one(&app.state.db)
        .await
        .expect("rollup");
    assert_eq!(rollup, 1);
}

#[tokio::test]
async fn bot_traffic_is_flagged_and_can_be_disabled() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "bots" }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    let request = axum::http::Request::builder()
        .method("GET")
        .uri("/bots")
        .header(header::HOST, "localhost:8080")
        .header(header::USER_AGENT, "curl/8.4.0")
        .body(axum::body::Body::empty())
        .expect("request");
    assert_eq!(app.send(request).await.status, StatusCode::FOUND);
    wait_for_clicks(&app, id, 1).await;

    let is_bot: i64 = sqlx::query_scalar("SELECT is_bot FROM clicks WHERE url_id = ?")
        .bind(id)
        .fetch_one(&app.state.db)
        .await
        .expect("click");
    assert_eq!(is_bot, 1);

    // Turning analytics off stops recording entirely.
    app.state
        .settings
        .update(&app.state.db, "analytics_enabled", "false", None)
        .await
        .expect("disable analytics");
    assert_eq!(app.get("/bots").await.status, StatusCode::FOUND);
    tokio::time::sleep(Duration::from_millis(80)).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clicks WHERE url_id = ?")
        .bind(id)
        .fetch_one(&app.state.db)
        .await
        .expect("count");
    assert_eq!(count, 1, "no new click row while analytics is disabled");
}

#[tokio::test]
async fn statistics_endpoint_reports_series_and_breakdowns() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "stats" }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    for agent in [
        "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Version/17.0 Mobile/15E148 Safari/604.1",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Firefox/121.0",
    ] {
        let request = axum::http::Request::builder()
            .method("GET")
            .uri("/stats")
            .header(header::HOST, "localhost:8080")
            .header(header::USER_AGENT, agent)
            .header(header::REFERER, "https://ref.example.com/page")
            .body(axum::body::Body::empty())
            .expect("request");
        assert_eq!(app.send(request).await.status, StatusCode::FOUND);
    }
    wait_for_clicks(&app, id, 2).await;

    let response = app
        .get_as(&format!("/api/v1/urls/{id}/stats?days=7"), &cookie)
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let stats = response.json();
    assert_eq!(stats["summary"]["total_clicks"], 2);
    assert_eq!(stats["summary"]["human_clicks"], 2);
    assert_eq!(stats["daily"].as_array().expect("daily").len(), 7);
    assert_eq!(stats["daily"][6]["clicks"], 2);
    assert_eq!(stats["referrers"][0]["label"], "ref.example.com");
    assert_eq!(stats["referrers"][0]["count"], 2);
    let devices: Vec<String> = stats["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .map(|entry| entry["label"].as_str().expect("label").to_string())
        .collect();
    assert!(devices.contains(&"Mobile".to_string()));
    assert!(devices.contains(&"Desktop".to_string()));

    let clicks = app
        .get_as(&format!("/api/v1/urls/{id}/clicks"), &cookie)
        .await
        .json();
    assert_eq!(clicks["total"], 2);
    assert_eq!(clicks["items"].as_array().expect("items").len(), 2);
    assert!(
        clicks["items"][0].get("ip_hash").is_none(),
        "hashes are never exposed"
    );

    // Another user cannot read the statistics.
    let (other, _) = app.register("other@example.com", "supersecret").await;
    assert_eq!(
        app.get_as(&format!("/api/v1/urls/{id}/stats"), &other)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn public_metadata_never_exposes_the_target() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://secret.example.com/board", "slug": "meta", "title": "Board deck", "password": "linkpass" }),
    )
    .await;

    let response = app.get("/api/v1/public/urls/meta").await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert_eq!(body["slug"], "meta");
    assert_eq!(body["title"], "Board deck");
    assert_eq!(body["has_password"], true);
    assert_eq!(body["state"], "active");
    assert!(
        !response.body.contains("secret.example.com"),
        "the target must not leak"
    );

    assert_eq!(
        app.get("/api/v1/public/urls/unknown").await.status,
        StatusCode::NOT_FOUND
    );

    // A disabled link disappears from the public endpoint too.
    let created = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "hidden" }),
    )
    .await;
    app.send(common::patch_json_as(
        &format!("/api/v1/urls/{}", created["id"].as_i64().expect("id")),
        json!({ "status": "disabled" }),
        &cookie,
    ))
    .await;
    assert_eq!(
        app.get("/api/v1/public/urls/hidden").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn reserved_paths_still_work_next_to_the_slug_route() {
    let app = TestApp::spawn().await;
    assert_eq!(app.get("/health").await.status, StatusCode::OK);
    assert_eq!(app.get("/assets/app.css").await.status, StatusCode::OK);
    assert_eq!(app.get("/assets/app.js").await.status, StatusCode::OK);
    assert_eq!(app.get("/favicon.ico").await.status, StatusCode::OK);
}
