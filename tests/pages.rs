//! Integration tests for the server rendered pages.

mod common;

use axum::http::StatusCode;
use common::{post_form, post_form_as, TestApp};
use serde_json::json;

async fn login(app: &TestApp, email: &str, password: &str) -> String {
    let response = app
        .send(post_form(
            "/app/login",
            &format!("email={email}&password={password}"),
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(
        response.status,
        StatusCode::SEE_OTHER,
        "login failed: {}",
        response.body
    );
    response.session_cookie().expect("session cookie")
}

#[tokio::test]
async fn home_redirects_to_the_app() {
    let app = TestApp::spawn().await;
    let response = app.get("/").await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(response.header("location").as_deref(), Some("/app"));
}

#[tokio::test]
async fn dashboard_requires_a_session() {
    let app = TestApp::spawn().await;
    let response = app.get("/app").await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(response.header("location").as_deref(), Some("/app/login"));

    let response = app.get("/app/urls/1").await;
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/login?next=%2Fapp%2Furls%2F1")
    );
}

#[tokio::test]
async fn login_page_renders_and_redirects_authenticated_visitors() {
    let app = TestApp::spawn().await;
    let page = app.get("/app/login").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Sign in"));
    assert!(page.body.contains("/assets/app.css"));
    assert!(page.body.contains("name=\"password\""));

    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let redirected = app.get_as("/app/login", &cookie).await;
    assert_eq!(redirected.status, StatusCode::SEE_OTHER);
    assert_eq!(redirected.header("location").as_deref(), Some("/app"));
}

#[tokio::test]
async fn registration_page_honours_the_registration_mode() {
    let app = TestApp::spawn().await;
    let open = app.get("/app/register").await;
    assert_eq!(open.status, StatusCode::OK);
    assert!(open.body.contains("Create account"));
    assert!(!open.body.contains("Invite code"));

    app.state
        .settings
        .update(&app.state.db, "registration_mode", "invite", None)
        .await
        .expect("switch to invite");
    let invite = app.get("/app/register").await;
    assert!(invite.body.contains("Invite code"));

    app.state
        .settings
        .update(&app.state.db, "registration_mode", "closed", None)
        .await
        .expect("close registration");
    let closed = app.get("/app/register").await;
    assert!(closed.body.contains("Registration is currently closed"));
    assert!(!closed.body.contains("name=\"password\""));
}

#[tokio::test]
async fn form_login_and_logout_round_trip() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;

    let cookie = login(&app, "user@example.com", "supersecret").await;
    let dashboard = app.get_as("/app", &cookie).await;
    assert_eq!(dashboard.status, StatusCode::OK);
    assert!(dashboard.body.contains("Your links"));
    assert!(dashboard.body.contains("New short link"));

    let logout = app.send(post_form_as("/app/logout", "", &cookie)).await;
    assert_eq!(logout.status, StatusCode::SEE_OTHER);
    assert_eq!(
        logout.header("location").as_deref(),
        Some("/app/login?ok=logout")
    );
    assert_eq!(
        app.get_as("/app", &cookie).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn wrong_credentials_render_an_error() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;

    let response = app
        .send(post_form(
            "/app/login",
            "email=user@example.com&password=wrong",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("Invalid email or password"));
}

#[tokio::test]
async fn registration_form_creates_and_signs_in() {
    let app = TestApp::spawn().await;
    let response = app
        .send(post_form(
            "/app/register",
            "email=new@example.com&password=supersecret&display_name=New+User",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let cookie = response.session_cookie().expect("session cookie");

    let dashboard = app.get_as("/app", &cookie).await;
    assert_eq!(dashboard.status, StatusCode::OK);
    assert!(dashboard.body.contains("New User"));
}

#[tokio::test]
async fn short_link_forms_create_update_and_delete() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let created = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Flanding&slug=promo\
             &title=Launch&tags=campaign%2C+docs&expires_at=2030-01-01+12%3A00&max_clicks=25",
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::SEE_OTHER, "{}", created.body);
    let location = created.header("location").expect("location");
    assert!(location.starts_with("/app/urls/"), "{location}");
    assert!(location.ends_with("?ok=created"));

    let detail = app.get_as(&location, &cookie).await;
    assert_eq!(detail.status, StatusCode::OK);
    assert!(detail.body.contains("promo"));
    assert!(detail.body.contains("example.com/landing"));
    assert!(detail.body.contains("Launch"));
    assert!(detail.body.contains("campaign"));
    assert!(detail.body.contains("25"), "click limit is rendered");
    assert!(detail.body.contains("2030-01-01"), "expiry is rendered");

    let id: i64 = location
        .trim_start_matches("/app/urls/")
        .split('?')
        .next()
        .expect("id")
        .parse()
        .expect("numeric id");

    // Update: rename, add a password and clear the click limit.
    let updated = app
        .send(post_form_as(
            &format!("/app/urls/{id}"),
            "target_url=https%3A%2F%2Fexample.com%2Fupdated&slug=promo&title=Renamed\
             &tags=campaign&password=linkpass&clear_max_clicks=on&status=active&redirect_type=301",
            &cookie,
        ))
        .await;
    assert_eq!(updated.status, StatusCode::SEE_OTHER, "{}", updated.body);
    assert_eq!(
        updated.header("location").as_deref(),
        Some(format!("/app/urls/{id}?ok=updated").as_str())
    );

    let detail = app.get_as(&format!("/app/urls/{id}"), &cookie).await;
    assert!(detail.body.contains("Renamed"));
    assert!(detail.body.contains("example.com/updated"));
    assert!(detail.body.contains("currently password protected"));

    // A cross-site form submission must be refused.
    let blocked = app
        .send(post_form(
            &format!("/app/urls/{id}/delete"),
            "",
            Some(&cookie),
            Some("https://evil.example"),
        ))
        .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);

    let deleted = app
        .send(post_form_as(&format!("/app/urls/{id}/delete"), "", &cookie))
        .await;
    assert_eq!(deleted.status, StatusCode::SEE_OTHER);
    assert_eq!(
        deleted.header("location").as_deref(),
        Some("/app?ok=deleted")
    );
    let list = app.get_as("/app", &cookie).await;
    assert!(list.body.contains("No links match this filter") || !list.body.contains("promo"));
}

#[tokio::test]
async fn create_form_reports_errors_with_a_flash_code() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    // Validation failures map to specific flash codes so the banner can explain
    // the rule that was broken.
    let invalid_target = app
        .send(post_form_as(
            "/app/urls",
            "target_url=javascript%3Aalert%281%29",
            &cookie,
        ))
        .await;
    assert_eq!(
        invalid_target.header("location").as_deref(),
        Some("/app?err=target")
    );

    let reserved = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fa.example&slug=api",
            &cookie,
        ))
        .await;
    assert_eq!(
        reserved.header("location").as_deref(),
        Some("/app?err=slug")
    );

    let past_expiry = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fa.example&expires_at=2000-01-01",
            &cookie,
        ))
        .await;
    assert_eq!(
        past_expiry.header("location").as_deref(),
        Some("/app?err=expiry")
    );

    let short_password = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fa.example&password=ab",
            &cookie,
        ))
        .await;
    assert_eq!(
        short_password.header("location").as_deref(),
        Some("/app?err=password_rules")
    );

    let first = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fa.example&slug=taken",
            &cookie,
        ))
        .await;
    assert_eq!(first.status, StatusCode::SEE_OTHER);
    let duplicate = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fb.example&slug=taken",
            &cookie,
        ))
        .await;
    assert_eq!(
        duplicate.header("location").as_deref(),
        Some("/app?err=conflict")
    );

    let flash = app.get_as("/app?err=conflict", &cookie).await;
    assert!(flash.body.contains("already taken"));
}

#[tokio::test]
async fn dashboard_filters_and_flash_banners() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    app.send(post_form_as(
        "/app/urls",
        "target_url=https%3A%2F%2Falpha.example&slug=alpha&tags=work",
        &cookie,
    ))
    .await;

    let filtered = app
        .get_as("/app?q=alpha&tag=work&sort=clicks", &cookie)
        .await;
    assert_eq!(filtered.status, StatusCode::OK);
    assert!(filtered.body.contains("value=\"alpha\""));

    let empty = app.get_as("/app?q=nomatch", &cookie).await;
    assert!(empty.body.contains("No links match this filter"));

    // Unknown flash codes are ignored rather than rendered.
    let unknown = app.get_as("/app?ok=%3Cscript%3E", &cookie).await;
    assert!(!unknown.body.contains("<script>alert"));
}

#[tokio::test]
async fn account_page_lists_sessions_and_issues_tokens() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let page = app.get_as("/app/account", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("API tokens"));
    assert!(page.body.contains("this device"));

    let created = app
        .send(post_form_as(
            "/app/account/tokens",
            "name=ci+pipeline&expires_at=2030-01-01",
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::OK);
    assert!(created.body.contains("Copy this token now"));
    assert!(created.body.contains("uw_"));

    // The plain token only exists in that response; the list shows a prefix.
    let page = app.get_as("/app/account", &cookie).await;
    assert!(page.body.contains("ci pipeline"));
    assert!(!page.body.contains("Copy this token now"));
}

#[tokio::test]
async fn profile_and_password_forms_work() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let profile = app
        .send(post_form_as(
            "/app/account",
            "display_name=Ada+Lovelace",
            &cookie,
        ))
        .await;
    assert_eq!(
        profile.header("location").as_deref(),
        Some("/app/account?ok=profile")
    );
    let page = app.get_as("/app/account", &cookie).await;
    assert!(page.body.contains("Ada Lovelace"));

    let wrong = app
        .send(post_form_as(
            "/app/account/password",
            "current_password=nope&new_password=brandnewpass",
            &cookie,
        ))
        .await;
    assert_eq!(
        wrong.header("location").as_deref(),
        Some("/app/account?err=password")
    );

    let changed = app
        .send(post_form_as(
            "/app/account/password",
            "current_password=supersecret&new_password=brandnewpass",
            &cookie,
        ))
        .await;
    assert_eq!(
        changed.header("location").as_deref(),
        Some("/app/account?ok=password")
    );
    assert!(app
        .send(post_form(
            "/app/login",
            "email=user@example.com&password=brandnewpass",
            None,
            Some("http://localhost:8080"),
        ))
        .await
        .session_cookie()
        .is_some());
}

#[tokio::test]
async fn revoking_the_current_session_signs_out() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let sessions = app.get_as("/api/v1/auth/sessions", &cookie).await.json();
    let current = sessions["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|session| session["current"] == true)
        .expect("current session")["id"]
        .as_i64()
        .expect("id");

    let response = app
        .send(post_form_as(
            &format!("/app/account/sessions/{current}/delete"),
            "",
            &cookie,
        ))
        .await;
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/login?ok=logout")
    );
    assert_eq!(
        app.get_as("/app", &cookie).await.status,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn tags_are_rendered_in_list_views() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    for (slug, tags) in [("first", "campaign,docs"), ("second", "campaign")] {
        let response = app
            .send(post_form_as(
                "/app/urls",
                &format!(
                    "target_url=https%3A%2F%2F{slug}.example&slug={slug}&tags={}",
                    tags.replace(',', "%2C")
                ),
                &cookie,
            ))
            .await;
        assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    }

    // The dashboard and the JSON list both resolve tags for many links at once.
    let dashboard = app.get_as("/app", &cookie).await;
    assert!(
        dashboard.body.contains("campaign"),
        "dashboard shows the tag"
    );
    assert!(dashboard.body.contains("docs"), "dashboard shows every tag");

    let list = app.get_as("/api/v1/urls", &cookie).await.json();
    let first = list["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["slug"] == "first")
        .expect("first link");
    assert_eq!(first["tags"], serde_json::json!(["campaign", "docs"]));
    let second = list["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["slug"] == "second")
        .expect("second link");
    assert_eq!(second["tags"], serde_json::json!(["campaign"]));

    // Tag filtering still works and reports usage counts. (The "top links"
    // card is intentionally not filtered, so assert through the JSON API.)
    let filtered = app.get_as("/api/v1/urls?tag=docs", &cookie).await.json();
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["items"][0]["slug"], "first");

    let tags = app.get_as("/api/v1/tags", &cookie).await.json();
    let campaign = tags["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|tag| tag["name"] == "campaign")
        .expect("campaign tag");
    assert_eq!(campaign["url_count"], 2);
}

#[tokio::test]
async fn json_api_still_works_alongside_the_pages() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let created = app
        .send(common::post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com" }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED);

    let tokens = app.get_as("/api/v1/tokens", &cookie).await;
    assert_eq!(tokens.status, StatusCode::OK);
    assert_eq!(tokens.json()["items"].as_array().expect("items").len(), 0);
}
