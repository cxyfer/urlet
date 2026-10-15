//! Integration tests for short link management.

mod common;

use axum::http::StatusCode;
use common::{delete_as, patch_json_as, post_json_as, TestApp};
use serde_json::{json, Value};

async fn create_url(app: &TestApp, cookie: &str, payload: Value) -> common::TestResponse {
    app.send(post_json_as("/api/v1/urls", payload, cookie)).await
}

#[tokio::test]
async fn creates_a_link_with_a_generated_slug() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let response = create_url(&app, &cookie, json!({ "target_url": "https://example.com/landing" })).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    let body = response.json();
    assert_eq!(body["slug"].as_str().expect("slug").len(), 7);
    assert_eq!(body["state"], "active");
    assert_eq!(body["is_custom_slug"], false);
    assert_eq!(body["target_url"], "https://example.com/landing");
    assert_eq!(body["short_url"], format!("http://localhost:8080/{}", body["slug"].as_str().expect("slug")));
    assert_eq!(body["click_count"], 0);
    assert_eq!(body["has_password"], false);
}

#[tokio::test]
async fn creates_a_link_with_a_custom_slug_and_metadata() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let response = create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://example.com/docs",
            "slug": "Docs-2026",
            "title": "Documentation",
            "notes": "shared with the team",
            "tags": ["work", "docs"],
            "password": "linkpass",
            "max_clicks": 10,
            "redirect_type": 301,
        }),
    )
    .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    let body = response.json();
    assert_eq!(body["slug"], "Docs-2026");
    assert_eq!(body["is_custom_slug"], true);
    assert_eq!(body["has_password"], true);
    assert_eq!(body["max_clicks"], 10);
    assert_eq!(body["redirect_type"], 301);
    assert_eq!(body["tags"], json!(["docs", "work"]));
    assert!(body["password"].is_null(), "the plain password is never returned");
}

#[tokio::test]
async fn rejects_duplicate_and_reserved_slugs() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let first = create_url(&app, &cookie, json!({ "target_url": "https://a.example", "slug": "promo" })).await;
    assert_eq!(first.status, StatusCode::CREATED);

    // Case-insensitive uniqueness.
    let duplicate = create_url(&app, &cookie, json!({ "target_url": "https://b.example", "slug": "PROMO" })).await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    assert_eq!(duplicate.json()["error"]["code"], "conflict");

    let reserved = create_url(&app, &cookie, json!({ "target_url": "https://c.example", "slug": "api" })).await;
    assert_eq!(reserved.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reserved.json()["error"]["details"]["field"], "slug");

    let invalid = create_url(&app, &cookie, json!({ "target_url": "https://d.example", "slug": "bad slug" })).await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn rejects_dangerous_and_malformed_targets() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    for target in [
        "javascript:alert(1)",
        "data:text/html;base64,PHNjcmlwdD4=",
        "file:///etc/passwd",
        "not a url at all",
        "",
    ] {
        let response = create_url(&app, &cookie, json!({ "target_url": target })).await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{target} must be rejected (got {})",
            response.body
        );
    }
}

#[tokio::test]
async fn normalises_target_urls() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let response = create_url(&app, &cookie, json!({ "target_url": "Example.COM/Path" })).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    assert_eq!(response.json()["target_url"], "https://example.com/Path");
}

#[tokio::test]
async fn validates_expiry_and_click_limits() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let past = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "expires_at": "2020-01-01T00:00:00Z" }),
    )
    .await;
    assert_eq!(past.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(past.json()["error"]["details"]["field"], "expires_at");

    let garbage = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "expires_at": "next tuesday" }),
    )
    .await;
    assert_eq!(garbage.status, StatusCode::UNPROCESSABLE_ENTITY);

    let zero_clicks = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "max_clicks": 0 }),
    )
    .await;
    assert_eq!(zero_clicks.status, StatusCode::UNPROCESSABLE_ENTITY);

    let future = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "expires_at": "2030-06-01" }),
    )
    .await;
    assert_eq!(future.status, StatusCode::CREATED, "{}", future.body);
    assert_eq!(future.json()["expires_at"], "2030-06-01T00:00:00.000Z");
}

#[tokio::test]
async fn links_are_isolated_between_users() {
    let app = TestApp::spawn().await;
    let (alice, _) = app.register("alice@example.com", "supersecret").await;
    let (bob, _) = app.register("bob@example.com", "supersecret").await;

    let created = create_url(&app, &alice, json!({ "target_url": "https://alice.example", "slug": "alice-link" })).await;
    let id = created.json()["id"].as_i64().expect("id");

    // Bob sees nothing.
    let bob_list = app.get_as("/api/v1/urls", &bob).await;
    assert_eq!(bob_list.json()["total"], 0);
    assert_eq!(app.get_as(&format!("/api/v1/urls/{id}"), &bob).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        app.send(patch_json_as(&format!("/api/v1/urls/{id}"), json!({ "title": "hijack" }), &bob)).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(app.send(delete_as(&format!("/api/v1/urls/{id}"), &bob)).await.status, StatusCode::NOT_FOUND);

    // Alice still owns it and nothing changed.
    let alice_view = app.get_as(&format!("/api/v1/urls/{id}"), &alice).await;
    assert_eq!(alice_view.status, StatusCode::OK);
    assert!(alice_view.json()["title"].is_null());
}

#[tokio::test]
async fn lists_with_search_filters_and_pagination() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let fixtures: [(&str, &str, &str, Option<&str>); 3] = [
        ("alpha", "https://alpha.example/one", "work", None),
        ("beta", "https://beta.example/two", "personal", None),
        ("gamma", "https://gamma.example/three", "work", None),
    ];
    for (slug, target, tag, status) in fixtures {
        let mut payload = json!({ "target_url": target, "slug": slug, "tags": [tag] });
        if let Some(status) = status {
            payload["status"] = json!(status);
        }
        let response = create_url(&app, &cookie, payload).await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    }

    let all = app.get_as("/api/v1/urls", &cookie).await.json();
    assert_eq!(all["total"], 3);
    assert_eq!(all["per_page"], 20);
    assert_eq!(all["total_pages"], 1);

    let search = app.get_as("/api/v1/urls?q=beta", &cookie).await.json();
    assert_eq!(search["total"], 1);
    assert_eq!(search["items"][0]["slug"], "beta");

    let by_tag = app.get_as("/api/v1/urls?tag=work", &cookie).await.json();
    assert_eq!(by_tag["total"], 2);

    let sorted = app.get_as("/api/v1/urls?sort=slug", &cookie).await.json();
    assert_eq!(sorted["items"][0]["slug"], "alpha");

    let paged = app.get_as("/api/v1/urls?per_page=2&page=2", &cookie).await.json();
    assert_eq!(paged["items"].as_array().expect("items").len(), 1);
    assert_eq!(paged["total_pages"], 2);

    // A wildcard in the search term must be treated literally.
    let literal = app.get_as("/api/v1/urls?q=%25", &cookie).await.json();
    assert_eq!(literal["total"], 0);
}

#[tokio::test]
async fn patches_fields_and_clears_optional_ones() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let created = create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://example.com/one",
            "title": "Original",
            "password": "linkpass",
            "max_clicks": 5,
            "tags": ["a"]
        }),
    )
    .await
    .json();
    let id = created["id"].as_i64().expect("id");

    let updated = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({
                "target_url": "https://example.com/two",
                "title": "Renamed",
                "password": null,
                "max_clicks": null,
                "tags": ["b", "c"]
            }),
            &cookie,
        ))
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    let body = updated.json();
    assert_eq!(body["target_url"], "https://example.com/two");
    assert_eq!(body["title"], "Renamed");
    assert_eq!(body["has_password"], false);
    assert!(body["max_clicks"].is_null());
    assert_eq!(body["tags"], json!(["b", "c"]));

    // Absent fields are untouched: the slug is unchanged.
    assert_eq!(body["slug"], created["slug"]);
}

#[tokio::test]
async fn patch_rejects_empty_and_invalid_bodies() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let created = create_url(&app, &cookie, json!({ "target_url": "https://example.com" })).await.json();
    let id = created["id"].as_i64().expect("id");

    let empty = app.send(patch_json_as(&format!("/api/v1/urls/{id}"), json!({}), &cookie)).await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);

    let bad_status = app
        .send(patch_json_as(&format!("/api/v1/urls/{id}"), json!({ "status": "weird" }), &cookie))
        .await;
    assert_eq!(bad_status.status, StatusCode::UNPROCESSABLE_ENTITY);

    let bad_target = app
        .send(patch_json_as(&format!("/api/v1/urls/{id}"), json!({ "target_url": "javascript:1" }), &cookie))
        .await;
    assert_eq!(bad_target.status, StatusCode::UNPROCESSABLE_ENTITY);

    let missing = app.send(patch_json_as("/api/v1/urls/99999", json!({ "title": "x" }), &cookie)).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleted_links_disappear_but_keep_their_slug() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let created = create_url(&app, &cookie, json!({ "target_url": "https://example.com", "slug": "keeper" })).await.json();
    let id = created["id"].as_i64().expect("id");

    let deleted = app.send(delete_as(&format!("/api/v1/urls/{id}"), &cookie)).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(app.get_as(&format!("/api/v1/urls/{id}"), &cookie).await.status, StatusCode::NOT_FOUND);
    assert_eq!(app.get_as("/api/v1/urls", &cookie).await.json()["total"], 0);

    // The slug is still reserved so old links cannot be hijacked.
    let reuse = create_url(&app, &cookie, json!({ "target_url": "https://evil.example", "slug": "keeper" })).await;
    assert_eq!(reuse.status, StatusCode::CONFLICT);

    assert_eq!(app.send(delete_as(&format!("/api/v1/urls/{id}"), &cookie)).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn enforces_the_per_user_quota() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    app.state
        .settings
        .update(&app.state.db, "max_urls_per_user", "1", None)
        .await
        .expect("update quota");

    assert_eq!(create_url(&app, &cookie, json!({ "target_url": "https://one.example" })).await.status, StatusCode::CREATED);
    let blocked = create_url(&app, &cookie, json!({ "target_url": "https://two.example" })).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert!(blocked.json()["error"]["message"].as_str().expect("message").contains("limit"));
}

#[tokio::test]
async fn link_state_reflects_expiry_limit_and_status() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let expiring = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "expiring", "expires_at": "2030-01-01" }),
    )
    .await
    .json();
    assert_eq!(expiring["state"], "active");
    assert!(expiring["expires_in"].as_str().expect("human").starts_with("in "));

    // Force the expiry into the past and re-read the state.
    sqlx::query("UPDATE urls SET expires_at = ? WHERE id = ?")
        .bind("2000-01-01T00:00:00.000Z")
        .bind(expiring["id"].as_i64().expect("id"))
        .execute(&app.state.db)
        .await
        .expect("expire");
    let expired = app.get_as(&format!("/api/v1/urls/{}", expiring["id"].as_i64().expect("id")), &cookie).await;
    assert_eq!(expired.json()["state"], "expired");

    let limited = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "limited", "max_clicks": 2 }),
    )
    .await
    .json();
    sqlx::query("UPDATE urls SET click_count = 2 WHERE id = ?")
        .bind(limited["id"].as_i64().expect("id"))
        .execute(&app.state.db)
        .await
        .expect("fill clicks");
    let reached = app.get_as(&format!("/api/v1/urls/{}", limited["id"].as_i64().expect("id")), &cookie).await;
    assert_eq!(reached.json()["state"], "limit_reached");

    let disabled = create_url(&app, &cookie, json!({ "target_url": "https://example.com", "slug": "disabled" })).await.json();
    let id = disabled["id"].as_i64().expect("id");
    let response = app.send(patch_json_as(&format!("/api/v1/urls/{id}"), json!({ "status": "disabled" }), &cookie)).await;
    assert_eq!(response.json()["state"], "disabled");
}

#[tokio::test]
async fn tags_can_be_listed_and_deleted() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    create_url(&app, &cookie, json!({ "target_url": "https://example.com", "tags": ["work", "urgent"] })).await;

    let list = app.get_as("/api/v1/tags", &cookie).await;
    assert_eq!(list.status, StatusCode::OK);
    let items = list.json()["items"].clone();
    assert_eq!(items.as_array().expect("items").len(), 2);
    let urgent = items
        .as_array()
        .expect("items")
        .iter()
        .find(|tag| tag["name"] == "urgent")
        .expect("urgent tag");
    assert_eq!(urgent["url_count"], 1);

    let tag_id = urgent["id"].as_i64().expect("id");
    let deleted = app.send(delete_as(&format!("/api/v1/tags/{tag_id}"), &cookie)).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(app.get_as("/api/v1/tags", &cookie).await.json()["items"].as_array().expect("items").len(), 1);

    assert_eq!(app.send(delete_as(&format!("/api/v1/tags/{tag_id}"), &cookie)).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_api_routes_return_json_404() {
    let app = TestApp::spawn().await;
    let response = app.get("/api/v1/nope").await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.json()["error"]["code"], "not_found");
}
