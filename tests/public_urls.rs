//! Integration tests for the public shortener (the form on the landing page and
//! `POST /api/v1/public/urls`).
//!
//! The feature is off by default, so every test starts by opening it to the
//! level it exercises. The reserved Guest account is what an anonymous visitor's
//! links belong to, and it must stay invisible to people.

mod common;

use axum::http::StatusCode;
use common::{post_form, TestApp};
use serde_json::{json, Value};

/// Same origin as the test host, so the CSRF middleware lets a form through.
const ORIGIN: &str = "http://localhost:8080";

/// Update one stored setting (the cache follows the write).
async fn set(app: &TestApp, key: &str, value: &str) {
    app.state
        .settings
        .update(&app.state.db, key, value, None)
        .await
        .expect("update setting");
}

async fn open_to_guests(app: &TestApp) {
    set(app, "public_create_access", "guest").await;
}

/// Register the first account, which the instance promotes to administrator.
async fn register_admin(app: &TestApp) -> String {
    let (cookie, _) = app.register("root@example.com", "supersecret").await;
    cookie
}

/// Register an ordinary member (an administrator has to exist first).
async fn register_member(app: &TestApp) -> String {
    let (cookie, _) = app.register("member@example.com", "supersecret").await;
    cookie
}

/// Id of the reserved account that owns anonymous links.
async fn guest_id(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT id FROM users WHERE is_system = 1")
        .fetch_one(&app.state.db)
        .await
        .expect("the reserved Guest account exists")
}

/// Number of links owned by the Guest account.
async fn guest_links(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM urls WHERE user_id = ?")
        .bind(guest_id(app).await)
        .fetch_one(&app.state.db)
        .await
        .expect("count guest links")
}

/// Submit the landing page form.
async fn submit(app: &TestApp, body: &str) -> common::TestResponse {
    app.send(post_form("/", body, None, Some(ORIGIN))).await
}

/// Slug of the link a successful form submission redirected to.
fn created_slug(response: &common::TestResponse) -> String {
    assert_eq!(
        response.status,
        StatusCode::SEE_OTHER,
        "form submission failed: {}",
        response.body
    );
    let location = response.header("location").expect("a Location header");
    assert!(
        location.starts_with("/?shortened="),
        "unexpected redirect: {location}"
    );
    location
        .trim_start_matches("/?shortened=")
        .split('&')
        .next()
        .expect("slug")
        .to_string()
}

fn api_body(target: &str) -> Value {
    json!({ "target_url": target })
}

#[tokio::test]
async fn the_form_follows_the_access_level() {
    let app = TestApp::spawn().await;
    let form = "id=\"public_target_url\"";

    // Default (`user`): anonymous visitors do not see the form, signed-in
    // accounts do.
    assert!(!app.get("/").await.body.contains(form));
    register_admin(&app).await;
    let member = register_member(&app).await;
    assert!(app.get_as("/", &member).await.body.contains(form));

    // `guest`: the form appears for everybody.
    open_to_guests(&app).await;
    assert!(app.get("/").await.body.contains(form));
    assert!(app.get_as("/", &member).await.body.contains(form));

    // `admin`: a member loses it again, an administrator keeps it.
    set(&app, "public_create_access", "admin").await;
    assert!(!app.get_as("/", &member).await.body.contains(form));
    assert!(!app.get("/").await.body.contains(form));
}

#[tokio::test]
async fn an_anonymous_visitor_creates_a_link_owned_by_the_guest_account() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    let registered = urlet::db::users::count(&app.state.db)
        .await
        .expect("count users");

    let response = submit(
        &app,
        "target_url=https%3A%2F%2Fexample.com%2Fpromo&title=Promo",
    )
    .await;
    let slug = created_slug(&response);

    let (owner, target): (i64, String) =
        sqlx::query_as("SELECT user_id, target_url FROM urls WHERE slug = ?")
            .bind(&slug)
            .fetch_one(&app.state.db)
            .await
            .expect("the link was stored");
    assert_eq!(owner, guest_id(&app).await, "owned by the Guest account");
    assert_eq!(target, "https://example.com/promo");
    // The reserved account is not a registration.
    assert_eq!(
        urlet::db::users::count(&app.state.db)
            .await
            .expect("count users"),
        registered
    );

    // The landing page reads the link back and prints its short URL.
    let page = app
        .get(response.header("location").expect("location").as_str())
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains(&format!("http://localhost:8080/{slug}")),
        "the short URL is shown: {}",
        page.body
    );
    // Nothing is taken from the query string verbatim: an unknown slug shows no
    // result box.
    let missing = app.get("/?shortened=doesnotexist&domain=localhost").await;
    assert!(!missing.body.contains("id=\"public-short-url\""));
}

#[tokio::test]
async fn a_visitor_cannot_create_when_only_users_may() {
    let app = TestApp::spawn().await;
    let response = submit(&app, "target_url=https%3A%2F%2Fexample.com%2Fdenied").await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/?err=denied"),
        "the visitor is told the form is closed"
    );
    assert_eq!(guest_links(&app).await, 0);

    let api = app
        .post_json(
            "/api/v1/public/urls",
            api_body("https://example.com/denied"),
        )
        .await;
    assert_eq!(api.status, StatusCode::FORBIDDEN, "{}", api.body);
}

#[tokio::test]
async fn admin_only_access_refuses_a_signed_in_member() {
    let app = TestApp::spawn().await;
    register_admin(&app).await;
    set(&app, "public_create_access", "admin").await;
    let member = register_member(&app).await;

    let response = app
        .send(post_form(
            "/",
            "target_url=https%3A%2F%2Fexample.com%2Fdenied",
            Some(&member),
            Some(ORIGIN),
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(response.header("location").as_deref(), Some("/?err=denied"));

    let api = app
        .post_json_as(
            "/api/v1/public/urls",
            api_body("https://example.com/denied"),
            &member,
        )
        .await;
    assert_eq!(api.status, StatusCode::FORBIDDEN, "{}", api.body);
}

#[tokio::test]
async fn an_administrator_may_create_when_access_is_admin_only() {
    let app = TestApp::spawn().await;
    let admin = register_admin(&app).await;
    set(&app, "public_create_access", "admin").await;

    let response = app
        .send(post_form(
            "/",
            "target_url=https%3A%2F%2Fexample.com%2Froot",
            Some(&admin),
            Some(ORIGIN),
        ))
        .await;
    created_slug(&response);
}

#[tokio::test]
async fn disabled_fields_are_refused_server_side() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    // Custom slugs and titles are on by default; tags are not.
    set(&app, "public_allow_slug", "false").await;

    let response = submit(&app, "target_url=https%3A%2F%2Fexample.com%2Fx&slug=promo").await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(response.header("location").as_deref(), Some("/?err=denied"));
    assert_eq!(guest_links(&app).await, 0);

    // The API refuses it the same way, so hiding the input is not the defence.
    let api = app
        .post_json(
            "/api/v1/public/urls",
            json!({ "target_url": "https://example.com/x", "tags": ["promo"] }),
        )
        .await;
    assert_eq!(api.status, StatusCode::FORBIDDEN, "{}", api.body);

    // The minimal submission still works.
    let ok = submit(&app, "target_url=https%3A%2F%2Fexample.com%2Fminimal").await;
    created_slug(&ok);
}

#[tokio::test]
async fn enabled_fields_are_stored() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    set(&app, "public_allow_tags", "true").await;
    set(&app, "public_allow_expiry", "true").await;

    let expires = urlet::services::time::iso_in_hours(48);
    let body = format!(
        "target_url=https%3A%2F%2Fexample.com%2Ffull&slug=public-full&tags=promo%2Cnews\
         &expires_at={}",
        urlencode(&expires)
    );
    let slug = created_slug(&submit(&app, &body).await);

    let (custom, expires_at): (i64, Option<String>) =
        sqlx::query_as("SELECT is_custom_slug, expires_at FROM urls WHERE slug = ?")
            .bind(&slug)
            .fetch_one(&app.state.db)
            .await
            .expect("stored link");
    assert_eq!(custom, 1);
    assert_eq!(expires_at.as_deref(), Some(expires.as_str()));

    let tags: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM url_tags ut JOIN tags t ON t.id = ut.tag_id
          WHERE ut.url_id = (SELECT id FROM urls WHERE slug = ?)",
    )
    .bind(&slug)
    .fetch_one(&app.state.db)
    .await
    .expect("tags");
    assert_eq!(tags, 2);
}

#[tokio::test]
async fn the_per_address_budget_is_enforced() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    set(&app, "public_rate_limit_per_ip", "1").await;
    // The shared budget must not be what stops the second request.
    set(&app, "public_rate_limit_global", "0").await;

    created_slug(&submit(&app, "target_url=https%3A%2F%2Fexample.com%2Fone").await);
    let second = submit(&app, "target_url=https%3A%2F%2Fexample.com%2Ftwo").await;
    assert_eq!(
        second.header("location").as_deref(),
        Some("/?err=rate_limited"),
        "the second creation from one address is refused"
    );
    assert_eq!(guest_links(&app).await, 1);
}

#[tokio::test]
async fn the_shared_visitor_budget_is_enforced() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    // No per-address limit at all: only the shared bucket can stop the flood.
    set(&app, "public_rate_limit_per_ip", "0").await;
    set(&app, "public_rate_limit_global", "1").await;

    created_slug(&submit(&app, "target_url=https%3A%2F%2Fexample.com%2Fone").await);
    let second = submit(&app, "target_url=https%3A%2F%2Fexample.com%2Ftwo").await;
    assert_eq!(
        second.header("location").as_deref(),
        Some("/?err=rate_limited")
    );

    // The API draws from the same shared budget.
    let api = app
        .post_json("/api/v1/public/urls", api_body("https://example.com/three"))
        .await;
    assert_eq!(api.status, StatusCode::TOO_MANY_REQUESTS, "{}", api.body);
    assert_eq!(guest_links(&app).await, 1);
}

#[tokio::test]
async fn a_zero_budget_disables_the_limit() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    set(&app, "public_rate_limit_per_ip", "0").await;
    set(&app, "public_rate_limit_global", "0").await;

    for index in 0..3 {
        let body = format!("target_url=https%3A%2F%2Fexample.com%2F{index}");
        created_slug(&submit(&app, &body).await);
    }
    assert_eq!(guest_links(&app).await, 3);
}

#[tokio::test]
async fn the_public_api_answers_json() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;

    let response = app
        .post_json(
            "/api/v1/public/urls",
            json!({ "target_url": "https://example.com/api", "slug": "from-api" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    let body = response.json();
    assert_eq!(body["slug"], "from-api");
    assert_eq!(body["domain"], "localhost:8080");
    assert_eq!(body["short_url"], "http://localhost:8080/from-api");
    assert_eq!(body["target_url"], "https://example.com/api");
    assert!(body["created_at"].is_string());
    // The identifier of the link is not handed out: the caller has no account to
    // manage it with.
    assert!(body.get("id").is_none());

    // The slug is taken now, and a collision is reported as a conflict.
    let conflict = app
        .post_json(
            "/api/v1/public/urls",
            json!({ "target_url": "https://example.com/other", "slug": "from-api" }),
        )
        .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT, "{}", conflict.body);
}

#[tokio::test]
async fn a_signed_in_member_owns_what_the_public_form_creates() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("member@example.com", "supersecret").await;

    let response = app
        .send(post_form(
            "/",
            "target_url=https%3A%2F%2Fexample.com%2Fmine",
            Some(&cookie),
            Some(ORIGIN),
        ))
        .await;
    let slug = created_slug(&response);

    let owner_email: String = sqlx::query_scalar(
        "SELECT u.email FROM urls JOIN users u ON u.id = urls.user_id WHERE urls.slug = ?",
    )
    .bind(&slug)
    .fetch_one(&app.state.db)
    .await
    .expect("owner");
    assert_eq!(owner_email, "member@example.com");
    assert_eq!(guest_links(&app).await, 0);
}

#[tokio::test]
async fn bot_protection_guards_the_public_form() {
    // A provider, a site key and a secret are configured through the
    // environment overrides; the public flow defaults to protected. The
    // verification endpoint is never reached because a missing token is
    // rejected before the provider call.
    let app = TestApp::spawn_with(|config| {
        config.captcha_provider_override = Some("turnstile".to_string());
        config.captcha_site_key_override = Some("site-key".to_string());
        config.captcha_secret_key_override = Some("secret-key".to_string());
        config.captcha_verify_url = Some("http://127.0.0.1:1/siteverify".to_string());
    })
    .await;
    open_to_guests(&app).await;

    // The landing page renders the widget.
    let page = app.get("/").await;
    assert!(page.body.contains("cf-turnstile"), "widget rendered");
    assert!(page.body.contains("data-sitekey=\"site-key\""));

    let response = submit(&app, "target_url=https%3A%2F%2Fexample.com%2Fbot").await;
    assert_eq!(
        response.header("location").as_deref(),
        Some("/?err=captcha"),
        "a missing token is refused"
    );
    assert_eq!(guest_links(&app).await, 0);

    let api = app
        .post_json("/api/v1/public/urls", api_body("https://example.com/bot"))
        .await;
    assert_eq!(api.status, StatusCode::BAD_REQUEST, "{}", api.body);
}

#[tokio::test]
async fn a_signed_in_session_follows_the_create_switch_not_the_public_one() {
    let app = TestApp::spawn_with(|config| {
        config.captcha_provider_override = Some("turnstile".to_string());
        config.captcha_site_key_override = Some("site-key".to_string());
        config.captcha_secret_key_override = Some("secret-key".to_string());
        // `captcha_on_create` stays off, so a session is not challenged.
        config.captcha_on_create_override = Some(false);
        config.captcha_verify_url = Some("http://127.0.0.1:1/siteverify".to_string());
    })
    .await;
    let (cookie, _) = app.register("member@example.com", "supersecret").await;

    // The public switch is on, but it only guards anonymous visitors.
    assert!(app.state.settings.public_captcha());
    let response = app
        .send(post_form(
            "/",
            "target_url=https%3A%2F%2Fexample.com%2Fsession",
            Some(&cookie),
            Some(ORIGIN),
        ))
        .await;
    created_slug(&response);
}

#[tokio::test]
async fn guest_links_are_labelled_and_the_account_is_protected() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    created_slug(&submit(&app, "target_url=https%3A%2F%2Fexample.com%2Fguestlink").await);

    let (cookie, _) = app.register("root@example.com", "supersecret").await;

    // The user list never shows the reserved account.
    let users = app.get_as("/app/admin/users", &cookie).await;
    assert_eq!(users.status, StatusCode::OK);
    assert!(!users.body.contains("guest@urlet.invalid"));
    assert!(!users.body.contains(">Guest<"));

    // The link list labels its owner as the Guest account.
    let urls = app.get_as("/app/admin/urls", &cookie).await;
    assert_eq!(urls.status, StatusCode::OK);
    assert!(
        urls.body.contains(">Guest</a>"),
        "the owner column names the Guest account"
    );
    assert!(!urls.body.contains("guest@urlet.invalid"));

    // The reserved account cannot be edited or deleted, not even by name.
    let id = guest_id(&app).await;
    let patch = app
        .send(common::patch_json_as(
            &format!("/api/v1/admin/users/{id}"),
            json!({ "role": "admin" }),
            &cookie,
        ))
        .await;
    assert_eq!(patch.status, StatusCode::FORBIDDEN, "{}", patch.body);
    let delete = app
        .send(common::delete_as(
            &format!("/api/v1/admin/users/{id}"),
            &cookie,
        ))
        .await;
    assert_eq!(delete.status, StatusCode::FORBIDDEN, "{}", delete.body);

    // Neither can a password reset: it would replace the unparsable sentinel
    // that keeps the account from ever verifying a password.
    let reset = app
        .send(common::post_json_as(
            &format!("/api/v1/admin/users/{id}/password"),
            json!({ "new_password": "chosen-by-the-admin" }),
            &cookie,
        ))
        .await;
    assert_eq!(reset.status, StatusCode::FORBIDDEN, "{}", reset.body);
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE is_system = 1")
        .fetch_one(&app.state.db)
        .await
        .expect("guest hash");
    assert_eq!(hash, "!", "the sentinel hash is untouched");

    // And it still cannot sign in, whatever password is tried. The refusal comes
    // from the suspension, so it holds even though the hash cannot parse.
    let login = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "guest@urlet.invalid", "password": "supersecret" }),
        )
        .await;
    assert_eq!(login.status, StatusCode::UNAUTHORIZED, "{}", login.body);
}

#[tokio::test]
async fn a_rejected_bot_check_does_not_consume_the_shared_budget() {
    // A provider is configured, so the anonymous flow is protected. Every
    // attempt below fails the check (no token); none of them may spend the
    // budget the whole visitor population draws from.
    let app = TestApp::spawn_with(|config| {
        config.captcha_provider_override = Some("turnstile".to_string());
        config.captcha_site_key_override = Some("site-key".to_string());
        config.captcha_secret_key_override = Some("secret-key".to_string());
        config.captcha_verify_url = Some("http://127.0.0.1:1/siteverify".to_string());
    })
    .await;
    open_to_guests(&app).await;
    set(&app, "public_rate_limit_per_ip", "0").await;
    set(&app, "public_rate_limit_global", "1").await;

    for attempt in 0..3 {
        let response = submit(&app, "target_url=https%3A%2F%2Fexample.com%2Fbot").await;
        assert_eq!(
            response.header("location").as_deref(),
            Some("/?err=captcha"),
            "attempt {attempt} must fail the bot check, not the budget"
        );
    }
    assert_eq!(guest_links(&app).await, 0);
}

#[tokio::test]
async fn the_guest_budget_is_its_own_setting() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    set(&app, "public_rate_limit_per_ip", "0").await;
    set(&app, "public_rate_limit_global", "0").await;
    // The two budgets are independent: one link per account, two for visitors.
    set(&app, "max_urls_per_user", "1").await;
    set(&app, "public_max_urls", "2").await;

    for index in 0..2 {
        let body = format!("target_url=https%3A%2F%2Fexample.com%2Fguest-{index}");
        created_slug(&submit(&app, &body).await);
    }
    let third = submit(&app, "target_url=https%3A%2F%2Fexample.com%2Fguest-2").await;
    assert_eq!(
        third.header("location").as_deref(),
        Some("/?err=quota"),
        "the visitor budget, not the per-user quota, is what fills up"
    );

    // A real account still follows `max_urls_per_user`.
    let member = register_member(&app).await;
    let first = app
        .post_json_as(
            "/api/v1/urls",
            api_body("https://example.com/mine"),
            &member,
        )
        .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    let second = app
        .post_json_as(
            "/api/v1/urls",
            api_body("https://example.com/mine-2"),
            &member,
        )
        .await;
    assert_eq!(second.status, StatusCode::FORBIDDEN, "{}", second.body);
}

#[tokio::test]
async fn a_malformed_number_is_refused_instead_of_dropped() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    set(&app, "public_allow_max_clicks", "true").await;

    // The click cap is enabled and the value is nonsense: the link must not be
    // created without the cap the visitor asked for.
    let response = submit(
        &app,
        "target_url=https%3A%2F%2Fexample.com%2Ftypo&max_clicks=abc",
    )
    .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/?err=invalid")
    );
    assert_eq!(guest_links(&app).await, 0);

    // The same value must not slip past a *disabled* field either.
    set(&app, "public_allow_max_clicks", "false").await;
    let response = submit(
        &app,
        "target_url=https%3A%2F%2Fexample.com%2Ftypo&max_clicks=abc",
    )
    .await;
    assert_eq!(
        response.header("location").as_deref(),
        Some("/?err=invalid")
    );
    assert_eq!(guest_links(&app).await, 0);

    // A blank field is simply "not submitted".
    created_slug(
        &submit(
            &app,
            "target_url=https%3A%2F%2Fexample.com%2Fblank&max_clicks=",
        )
        .await,
    );

    // The countdown field is held to the same rule.
    set(&app, "public_allow_redirect", "true").await;
    let response = submit(
        &app,
        "target_url=https%3A%2F%2Fexample.com%2Fsplash&splash_delay_seconds=soon",
    )
    .await;
    assert_eq!(
        response.header("location").as_deref(),
        Some("/?err=invalid")
    );
    assert_eq!(guest_links(&app).await, 1);
}

#[tokio::test]
async fn the_result_box_is_hidden_while_the_form_is_closed() {
    let app = TestApp::spawn().await;
    register_admin(&app).await;
    let member = register_member(&app).await;
    let created = app
        .post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com/private", "slug": "private-one", "password": "linkpass" }),
            &member,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);

    // With the public shortener closed, `/?shortened=` must not confirm that a
    // slug exists — it would be a free existence oracle.
    let probe = "/?shortened=private-one&domain=localhost:8080";
    let page = app.get(probe).await;
    assert!(
        !page.body.contains("id=\"public-short-url\""),
        "the result box stays hidden"
    );

    // Once the form is open, the same request shows the short URL.
    open_to_guests(&app).await;
    let page = app.get(probe).await;
    assert!(page.body.contains("id=\"public-short-url\""));
    assert!(page.body.contains("http://localhost:8080/private-one"));
}

#[tokio::test]
async fn the_form_hides_fields_the_group_forbids() {
    let app = TestApp::spawn().await;
    register_admin(&app).await;
    let member = register_member(&app).await;

    // The public switches allow custom slugs; the member's group does not.
    let now = urlet::services::time::now_iso();
    sqlx::query(
        "INSERT INTO user_groups (name, allow_custom_slug, is_default, created_at, updated_at)
         VALUES ('NoSlug', 0, 0, ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&app.state.db)
    .await
    .expect("group");
    let group: i64 = sqlx::query_scalar("SELECT id FROM user_groups WHERE name = 'NoSlug'")
        .fetch_one(&app.state.db)
        .await
        .expect("group id");
    sqlx::query("UPDATE users SET group_id = ? WHERE email_normalized = 'member@example.com'")
        .bind(group)
        .execute(&app.state.db)
        .await
        .expect("assign group");

    assert!(app.state.settings.public_fields().slug, "the switch is on");
    let page = app.get_as("/", &member).await;
    assert!(
        !page.body.contains("id=\"public_slug\""),
        "the group's rule wins over the public switch"
    );
    // A visitor (the ungrouped Guest) still gets the field once the form is open
    // to them.
    open_to_guests(&app).await;
    assert!(app.get("/").await.body.contains("id=\"public_slug\""));
}

#[tokio::test]
async fn the_first_registration_is_still_an_administrator() {
    // The reserved Guest account exists from the first migration, so the
    // "first user becomes an administrator" rule must ignore it.
    let app = TestApp::spawn().await;
    let (_, user) = app.register("first@example.com", "supersecret").await;
    assert_eq!(user["user"]["role"], "admin", "{}", user);
}

#[tokio::test]
async fn the_landing_page_keeps_working_for_extra_domains() {
    let app = TestApp::spawn().await;
    open_to_guests(&app).await;
    app.add_domain("go.example.com", "active").await;

    // An extra domain stays redirect-only: no form, and the form endpoint does
    // not exist there.
    let page = app.get_with_host("/", "go.example.com").await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(!page.body.contains("id=\"public_target_url\""));

    let response = app
        .send(common::post_form_with_host(
            "/",
            "target_url=https%3A%2F%2Fexample.com%2Fextra",
            None,
            Some("http://go.example.com"),
            "go.example.com",
        ))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(guest_links(&app).await, 0);
}

#[tokio::test]
async fn admin_only_access_also_closes_the_dashboard_and_the_api() {
    let app = TestApp::spawn().await;
    let admin = register_admin(&app).await;
    set(&app, "public_create_access", "admin").await;
    let member = register_member(&app).await;

    // The dashboard explains itself instead of offering a form that would fail.
    let page = app.get_as("/app", &member).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(!page.body.contains("action=\"/app/urls\""));
    assert!(
        page.body
            .contains("An administrator has reserved link creation"),
        "the notice is shown"
    );

    // A hand-written submission is refused, and so is the JSON API.
    let form = app
        .send(post_form(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Fdash",
            Some(&member),
            Some(ORIGIN),
        ))
        .await;
    assert_eq!(form.status, StatusCode::SEE_OTHER);
    assert_eq!(form.header("location").as_deref(), Some("/app?err=denied"));

    let api = app
        .post_json_as("/api/v1/urls", api_body("https://example.com/api"), &member)
        .await;
    assert_eq!(api.status, StatusCode::FORBIDDEN, "{}", api.body);

    // The administrator keeps every path.
    let created = app
        .post_json_as(
            "/api/v1/urls",
            api_body("https://example.com/admin-link"),
            &admin,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
}

/// Percent encode a value for a form body.
fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
