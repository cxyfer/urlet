//! Integration tests for splash pages and the per-group redirect-mode policy.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{patch_json_as, post_form_as, put_json_as, TestApp};
use serde_json::{json, Value};

/// Register the first account (administrator) and return its cookie.
async fn admin(app: &TestApp) -> String {
    let (cookie, _) = app.register("admin@example.com", "supersecret").await;
    cookie
}

/// Register a normal account and return `(cookie, id)`.
async fn member(app: &TestApp, email: &str) -> (String, i64) {
    let (cookie, user) = app.register(email, "supersecret").await;
    (cookie, user["user"]["id"].as_i64().expect("id"))
}

async fn create_group(app: &TestApp, cookie: &str, body: Value) -> Value {
    let response = app.post_json_as("/api/v1/admin/groups", body, cookie).await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "group create failed: {}",
        response.body
    );
    response.json()
}

async fn assign(app: &TestApp, admin_cookie: &str, user_id: i64, group_id: i64) {
    let response = app
        .send(patch_json_as(
            &format!("/api/v1/admin/users/{user_id}"),
            json!({ "group_id": group_id }),
            admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
}

async fn create_url(app: &TestApp, cookie: &str, payload: Value) -> Value {
    let response = app
        .send(common::post_json_as("/api/v1/urls", payload, cookie))
        .await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "create failed: {}",
        response.body
    );
    response.json()
}

/// Wait until the detached click writer updated the counter.
async fn wait_for_clicks(app: &TestApp, url_id: i64, expected: i64) {
    let mut seen = 0;
    for _ in 0..150 {
        seen = sqlx::query_scalar("SELECT click_count FROM urls WHERE id = ?")
            .bind(url_id)
            .fetch_one(&app.state.db)
            .await
            .expect("count clicks");
        if seen >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("expected {expected} clicks, saw {seen}");
}

#[tokio::test]
async fn splash_pages_render_custom_copy_and_count_one_click() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://example.com/promo?a=1&b=2",
            "slug": "sale",
            "redirect_mode": "splash",
            "splash_delay_seconds": 7,
            "splash_title": "Summer sale",
            "splash_message": "First line\nSecond line"
        }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");
    assert_eq!(url["redirect_mode"], "splash");
    assert_eq!(url["splash_delay_seconds"], 7);
    assert_eq!(url["splash_title"], "Summer sale");
    assert_eq!(url["splash_message"], "First line\nSecond line");

    let page = app.get("/sale").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert_eq!(
        page.header("content-type").as_deref(),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(page.header("cache-control").as_deref(), Some("no-store"));
    assert!(
        page.header("location").is_none(),
        "the splash page must not redirect immediately"
    );
    assert!(page.body.contains("Summer sale"));
    assert!(page.body.contains("First line"));
    assert!(page.body.contains("Second line"));
    assert!(page.body.contains("Redirecting in 7 seconds"));
    assert!(
        page.body
            .contains("content=\"7;url=https://example.com/promo"),
        "the meta refresh must carry the delay and target: {}",
        page.body
    );
    assert!(page.body.contains("example.com"));
    assert!(page.body.contains("<a class=\"btn btn-primary\""));

    // The splash view is the click: no second count happens on the target,
    // which is external anyway.
    wait_for_clicks(&app, id, 1).await;
}

#[tokio::test]
async fn splash_copy_is_html_escaped() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://example.com/escaped",
            "slug": "escaped",
            "redirect_mode": "splash",
            "splash_title": "<script>alert(1)</script>",
            "splash_message": "<b>not bold</b>"
        }),
    )
    .await;

    let page = app.get("/escaped").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!page.body.contains("<script>alert(1)</script>"));
    assert!(page.body.contains("&lt;b&gt;not bold&lt;/b&gt;"));
}

#[tokio::test]
async fn splash_links_can_be_edited_and_switched_back_to_direct() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://example.com/back",
            "slug": "back",
            "redirect_mode": "splash",
            "splash_title": "Before",
            "splash_message": "Copy"
        }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    // An empty/blank mode is treated like an omitted field (the create path
    // does the same), so it must not turn into a validation error.
    let blank = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({ "redirect_mode": "   ", "title": "Kept" }),
            &cookie,
        ))
        .await;
    assert_eq!(blank.status, StatusCode::OK, "{}", blank.body);
    assert_eq!(blank.json()["redirect_mode"], "splash");
    assert_eq!(blank.json()["title"], "Kept");

    let patched = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({
                "redirect_mode": "direct",
                "splash_delay_seconds": 9,
                "splash_title": null,
                "splash_message": null
            }),
            &cookie,
        ))
        .await;
    assert_eq!(patched.status, StatusCode::OK, "{}", patched.body);
    let patched = patched.json();
    assert_eq!(patched["redirect_mode"], "direct");
    assert_eq!(patched["splash_delay_seconds"], 9);
    assert_eq!(patched["splash_title"], Value::Null);
    assert_eq!(patched["splash_message"], Value::Null);

    let response = app.get("/back").await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(
        response.header("location").as_deref(),
        Some("https://example.com/back")
    );
}

#[tokio::test]
async fn the_web_edit_form_can_clear_the_splash_copy() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://example.com/clear",
            "slug": "clear-copy",
            "redirect_mode": "splash",
            "splash_title": "Old heading",
            "splash_message": "Old message"
        }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    let response = app
        .send(post_form_as(
            &format!("/app/urls/{id}"),
            "target_url=https%3A%2F%2Fexample.com%2Fclear&redirect_mode=splash             &splash_delay_seconds=6&splash_title=&splash_message=",
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response.header("location").as_deref(),
        Some(format!("/app/urls/{id}?ok=updated").as_str())
    );

    let stored = app
        .get_as(&format!("/api/v1/urls/{id}"), &cookie)
        .await
        .json();
    assert_eq!(stored["splash_title"], Value::Null);
    assert_eq!(stored["splash_message"], Value::Null);
    assert_eq!(stored["splash_delay_seconds"], 6);

    let page = app.get("/clear-copy").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("Redirecting in 6 seconds"));
    assert!(!page.body.contains("Old heading"));
    assert!(!page.body.contains("Old message"));
}

#[tokio::test]
async fn an_unknown_stored_mode_falls_back_to_direct_and_the_form_preserves_it() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://example.com/legacy-mode",
            "slug": "legacy-mode",
            "redirect_mode": "splash",
            "splash_title": "Legacy heading"
        }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    // The CHECK constraint prevents this in normal operation; bypass it on one
    // connection to model a database written by an older/future build.
    let mut connection = app.state.db.acquire().await.expect("connection");
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .expect("relax checks");
    sqlx::query("UPDATE urls SET redirect_mode = 'legacy' WHERE id = ?")
        .bind(id)
        .execute(&mut *connection)
        .await
        .expect("legacy mode");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *connection)
        .await
        .expect("restore checks");
    drop(connection);

    // The redirect path treats the unknown value as direct.
    let response = app.get("/legacy-mode").await;
    assert_eq!(
        response.status,
        StatusCode::FOUND,
        "an unknown mode must fall back to direct: {}",
        response.body
    );

    // The edit form exposes the raw value and does not preselect direct.
    let page = app.get_as(&format!("/app/urls/{id}"), &cookie).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("Unrecognised mode"));
    assert!(page.body.contains("(legacy)"));

    // A browser submits no redirect_mode when the disabled placeholder is
    // selected, so an unrelated edit keeps the stored value.
    let saved = app
        .send(post_form_as(
            &format!("/app/urls/{id}"),
            "target_url=https%3A%2F%2Fexample.com%2Flegacy-mode",
            &cookie,
        ))
        .await;
    assert_eq!(saved.status, StatusCode::SEE_OTHER, "{}", saved.body);
    let stored: String = sqlx::query_scalar("SELECT redirect_mode FROM urls WHERE id = ?")
        .bind(id)
        .fetch_one(&app.state.db)
        .await
        .expect("stored mode");
    assert_eq!(stored, "legacy");
}

#[tokio::test]
async fn an_out_of_range_stored_delay_is_clamped_when_rendering() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://example.com/clamped",
            "slug": "clamped-delay",
            "redirect_mode": "splash"
        }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    let mut connection = app.state.db.acquire().await.expect("connection");
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .expect("relax checks");
    sqlx::query("UPDATE urls SET splash_delay_seconds = 999 WHERE id = ?")
        .bind(id)
        .execute(&mut *connection)
        .await
        .expect("out-of-range delay");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *connection)
        .await
        .expect("restore checks");
    drop(connection);

    let page = app.get("/clamped-delay").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("Redirecting in 60 seconds"));
    assert!(page
        .body
        .contains("content=\"60;url=https://example.com/clamped\""));
}

#[tokio::test]
async fn splash_fields_are_validated() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    for (payload, field) in [
        (
            json!({ "target_url": "https://example.com", "slug": "bad-mode", "redirect_mode": "interstitial" }),
            "redirect_mode",
        ),
        (
            json!({ "target_url": "https://example.com", "slug": "bad-delay", "splash_delay_seconds": 0 }),
            "splash_delay_seconds",
        ),
        (
            json!({ "target_url": "https://example.com", "slug": "bad-delay-2", "splash_delay_seconds": 61 }),
            "splash_delay_seconds",
        ),
        (
            json!({ "target_url": "https://example.com", "slug": "long-title", "splash_title": "x".repeat(121) }),
            "splash_title",
        ),
        (
            json!({ "target_url": "https://example.com", "slug": "long-message", "splash_message": "x".repeat(501) }),
            "splash_message",
        ),
    ] {
        let response = app
            .send(common::post_json_as("/api/v1/urls", payload, &cookie))
            .await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            response.body
        );
        assert_eq!(response.json()["error"]["details"]["field"], field);
    }

    let url = create_url(
        &app,
        &cookie,
        json!({ "target_url": "https://example.com", "slug": "valid-splash" }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");
    let response = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({ "redirect_mode": "nope" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn a_password_page_is_the_only_interstitial() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://secret.example/private",
            "slug": "vault-splash",
            "password": "linkpass",
            "redirect_mode": "splash",
            "splash_title": "Should not appear yet"
        }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    let locked = app.get("/vault-splash").await;
    assert_eq!(locked.status, StatusCode::OK);
    assert!(locked.body.contains("Protected link"));
    assert!(!locked.body.contains("Should not appear yet"));
    assert!(!locked.body.contains("secret.example"));

    let unlocked = app
        .send(common::post_form(
            "/vault-splash/unlock",
            "password=linkpass",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(unlocked.status, StatusCode::SEE_OTHER);
    let unlock_cookie = unlocked
        .cookie_named(&urlet::services::unlock::cookie_name(
            "localhost:8080",
            "vault-splash",
        ))
        .expect("unlock cookie");

    let followed = app.get_as("/vault-splash", &unlock_cookie).await;
    assert_eq!(
        followed.status,
        StatusCode::FOUND,
        "an unlocked password link must skip the splash page: {}",
        followed.body
    );
    assert_eq!(
        followed.header("location").as_deref(),
        Some("https://secret.example/private")
    );
    wait_for_clicks(&app, id, 1).await;
}

#[tokio::test]
async fn public_metadata_exposes_the_mode_but_not_the_target() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    create_url(
        &app,
        &cookie,
        json!({
            "target_url": "https://private.example/board",
            "slug": "meta-splash",
            "redirect_mode": "splash"
        }),
    )
    .await;

    let response = app.get("/api/v1/public/urls/meta-splash").await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json()["redirect_mode"], "splash");
    assert!(!response.body.contains("private.example"));
}

#[tokio::test]
async fn a_direct_only_group_blocks_splash_creation_but_keeps_existing_links() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "direct@example.com").await;

    // Create a splash link while the account is still in the choice group, so
    // the policy below only affects new links.
    let legacy = create_url(
        &app,
        &member_cookie,
        json!({
            "target_url": "https://example.com/legacy",
            "slug": "legacy-splash",
            "redirect_mode": "splash"
        }),
    )
    .await;
    let legacy_id = legacy["id"].as_i64().expect("id");

    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Direct only", "redirect_mode_policy": "direct" }),
    )
    .await;
    assert_eq!(group["redirect_mode_policy"], "direct");
    assign(
        &app,
        &admin_cookie,
        member_id,
        group["id"].as_i64().expect("id"),
    )
    .await;

    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["limits"]["redirect_mode_policy"], "direct");

    // Omitted mode takes the policy default.
    let created = create_url(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/new", "slug": "direct-new" }),
    )
    .await;
    assert_eq!(created["redirect_mode"], "direct");

    let refused = app
        .send(common::post_json_as(
            "/api/v1/urls",
            json!({
                "target_url": "https://example.com/blocked",
                "slug": "blocked-splash",
                "redirect_mode": "splash"
            }),
            &member_cookie,
        ))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);
    assert_eq!(refused.json()["error"]["code"], "forbidden");
    assert!(refused.json()["error"]["message"]
        .as_str()
        .expect("message")
        .contains("direct redirects"));

    // The existing splash link still renders and can be edited without
    // switching modes; switching to the allowed direct mode is accepted.
    let page = app.get("/legacy-splash").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("on your way"));

    let edited = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{legacy_id}"),
            json!({ "title": "Edited" }),
            &member_cookie,
        ))
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", edited.body);
    assert_eq!(edited.json()["redirect_mode"], "splash");

    let switched = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{legacy_id}"),
            json!({ "redirect_mode": "direct" }),
            &member_cookie,
        ))
        .await;
    assert_eq!(switched.status, StatusCode::OK, "{}", switched.body);
    assert_eq!(switched.json()["redirect_mode"], "direct");

    let refused = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{legacy_id}"),
            json!({ "redirect_mode": "splash" }),
            &member_cookie,
        ))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);

    // A crafted browser form gets the same refusal and a localised flash code.
    let refused_form = app
        .send(post_form_as(
            &format!("/app/urls/{legacy_id}"),
            "target_url=https%3A%2F%2Fexample.com%2Flegacy&redirect_mode=splash",
            &member_cookie,
        ))
        .await;
    assert_eq!(refused_form.status, StatusCode::SEE_OTHER);
    assert_eq!(
        refused_form.header("location").as_deref(),
        Some(format!("/app/urls/{legacy_id}?err=mode_denied").as_str())
    );

    // The browser form hides the blocked option.
    let dashboard = app.get_as("/app", &member_cookie).await;
    assert_eq!(dashboard.status, StatusCode::OK, "{}", dashboard.body);
    assert!(dashboard.body.contains("value=\"direct\""));
    assert!(
        !dashboard.body.contains("value=\"splash\""),
        "a direct-only group must not offer the splash option"
    );

    // The browser form surfaces the refusal as a specific flash message.
    let form = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Fno&slug=form-no&redirect_mode=splash",
            &member_cookie,
        ))
        .await;
    assert_eq!(form.status, StatusCode::SEE_OTHER);
    assert_eq!(
        form.header("location").as_deref(),
        Some("/app?err=mode_denied")
    );
}

#[tokio::test]
async fn a_splash_only_group_blocks_direct_links() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "splash@example.com").await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Splash only", "redirect_mode_policy": "splash" }),
    )
    .await;
    assign(
        &app,
        &admin_cookie,
        member_id,
        group["id"].as_i64().expect("id"),
    )
    .await;

    let created = create_url(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/always", "slug": "always-splash" }),
    )
    .await;
    assert_eq!(created["redirect_mode"], "splash");

    let refused = app
        .send(common::post_json_as(
            "/api/v1/urls",
            json!({
                "target_url": "https://example.com/direct",
                "slug": "not-direct",
                "redirect_mode": "direct"
            }),
            &member_cookie,
        ))
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN, "{}", refused.body);

    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["limits"]["redirect_mode_policy"], "splash");

    let dashboard = app.get_as("/app", &member_cookie).await;
    assert_eq!(dashboard.status, StatusCode::OK, "{}", dashboard.body);
    assert!(dashboard.body.contains("value=\"splash\""));
    assert!(
        !dashboard.body.contains("value=\"direct\""),
        "a splash-only group must not offer direct redirects"
    );
}

#[tokio::test]
async fn the_web_form_creates_a_splash_link_and_the_group_form_sets_the_policy() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "form@example.com").await;

    // The browser form can create a splash link with custom copy.
    let created = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Fform&slug=form-splash\
             &redirect_mode=splash&splash_delay_seconds=9\
             &splash_title=Launch+day&splash_message=Welcome+aboard",
            &member_cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::SEE_OTHER, "{}", created.body);
    let location = created.header("location").expect("location");
    assert!(location.starts_with("/app/urls/"), "{location}");
    let path = location.split('?').next().expect("path");
    let id: i64 = path
        .rsplit('/')
        .next()
        .expect("id")
        .parse()
        .expect("numeric id");

    let page = app.get("/form-splash").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("Launch day"));
    assert!(page.body.contains("Welcome aboard"));
    assert!(page.body.contains("Redirecting in 9 seconds"));

    let detail = app.get_as(&format!("/app/urls/{id}"), &member_cookie).await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.body);
    assert!(detail.body.contains("Launch day"));
    assert!(detail.body.contains("value=\"9\""));

    // The admin group form stores the policy too.
    let group = app
        .send(post_form_as(
            "/app/admin/groups",
            "name=Form+policy&redirect_mode_policy=splash&allow_custom_slug=on&allow_password=on\
             &allow_expiry=on&allow_max_clicks=on&allow_tags=on&allow_api_tokens=on\
             &allow_domain_selection=on",
            &admin_cookie,
        ))
        .await;
    assert_eq!(group.status, StatusCode::SEE_OTHER, "{}", group.body);
    let list = app
        .get_as("/api/v1/admin/groups", &admin_cookie)
        .await
        .json();
    let stored = list["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["name"] == "Form policy")
        .expect("group")
        .clone();
    assert_eq!(stored["redirect_mode_policy"], "splash");

    // The redesigned admin form renders the policy as a radio group and marks
    // the stored choice; the create form defaults to "choice".
    let group_page = app.get_as("/app/admin/groups", &admin_cookie).await;
    assert_eq!(group_page.status, StatusCode::OK, "{}", group_page.body);
    assert!(group_page.body.contains("name=\"redirect_mode_policy\""));
    assert!(group_page.body.contains("value=\"splash\" checked"));
    assert!(group_page.body.contains("value=\"choice\" checked"));

    // The group policy endpoint is validated.
    let invalid = app
        .send(put_json_as(
            &format!(
                "/api/v1/admin/groups/{}",
                stored["id"].as_i64().expect("id")
            ),
            json!({ "name": "Form policy", "redirect_mode_policy": "sometimes" }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        invalid.json()["error"]["details"]["field"],
        "redirect_mode_policy"
    );

    // ...and assigning a member to a splash-only group applies on the next
    // request, because limits are resolved per request.
    let group_id = stored["id"].as_i64().expect("id");
    assign(&app, &admin_cookie, member_id, group_id).await;
    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["limits"]["redirect_mode_policy"], "splash");

    // A partial/legacy form post that omits the radios keeps the stored policy...
    let omitted = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            "name=Form+policy&domain_policy=all",
            &admin_cookie,
        ))
        .await;
    assert_eq!(omitted.status, StatusCode::SEE_OTHER, "{}", omitted.body);
    let list = app
        .get_as("/api/v1/admin/groups", &admin_cookie)
        .await
        .json();
    let stored = list["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["id"] == group_id)
        .expect("group")
        .clone();
    assert_eq!(stored["redirect_mode_policy"], "splash");

    // ...while an explicit choice still updates it.
    let changed = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            "name=Form+policy&domain_policy=all&redirect_mode_policy=direct",
            &admin_cookie,
        ))
        .await;
    assert_eq!(changed.status, StatusCode::SEE_OTHER, "{}", changed.body);
    let list = app
        .get_as("/api/v1/admin/groups", &admin_cookie)
        .await
        .json();
    let stored = list["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["id"] == group_id)
        .expect("group")
        .clone();
    assert_eq!(stored["redirect_mode_policy"], "direct");

    // The group audit trail records both the new and the previous policy.
    let audit = app
        .get_as(
            "/api/v1/admin/audit?action=admin.group_updated",
            &admin_cookie,
        )
        .await
        .json();
    let entry = audit["items"]
        .as_array()
        .expect("audit items")
        .iter()
        .find(|item| item["metadata"]["previous_redirect_mode_policy"] == "splash")
        .expect("policy change is audited")
        .clone();
    assert_eq!(entry["metadata"]["redirect_mode_policy"], "direct");
}
