//! Integration tests for multi-domain support.

mod common;

use axum::http::StatusCode;
use common::{delete_as, patch_json_as, post_form_with_host, TestApp};
use serde_json::{json, Value};

const ADMIN_PASSWORD: &str = "supersecret";

async fn register_admin(app: &TestApp) -> String {
    let (cookie, _) = app.register("admin@example.com", ADMIN_PASSWORD).await;
    cookie
}

async fn create_url(app: &TestApp, cookie: &str, payload: Value) -> Value {
    let response = app.post_json_as("/api/v1/urls", payload, cookie).await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "create failed: {}",
        response.body
    );
    response.json()
}

async fn domain_id(app: &TestApp, hostname: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM domains WHERE hostname = ? COLLATE NOCASE")
        .bind(hostname)
        .fetch_one(&app.state.db)
        .await
        .expect("domain id")
}

#[tokio::test]
async fn extra_domains_are_redirect_only() {
    let app = TestApp::spawn().await;
    app.add_domain("go.example.com", "active").await;
    let admin = register_admin(&app).await;

    let created = create_url(
        &app,
        &admin,
        json!({
            "target_url": "https://example.com/extra",
            "slug": "promo",
            "domain": "go.example.com",
        }),
    )
    .await;
    assert_eq!(created["domain"], "go.example.com");
    assert_eq!(created["short_url"], "http://go.example.com/promo");

    let redirect = app.get_with_host("/promo", "go.example.com").await;
    assert_eq!(redirect.status, StatusCode::FOUND);
    assert_eq!(
        redirect.header("location").as_deref(),
        Some("https://example.com/extra")
    );

    // The extra root is the small landing page, not the marketing page.
    let root = app.get_with_host("/", "go.example.com").await;
    assert_eq!(root.status, StatusCode::OK);
    assert!(root.body.contains("short link service"), "{}", root.body);
    assert!(root.body.contains("http://localhost:8080"));
    assert!(root.body.contains("noindex"));
    assert!(!root.body.contains("Create account"));

    // The canonical root keeps the marketing page.
    let home = app.get("/").await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(home.body.contains("Short links for teams"), "{}", home.body);

    // `/app` is redirected to the canonical host, preserving path and query.
    let app_redirect = app.get_with_host("/app?x=1", "go.example.com").await;
    assert_eq!(app_redirect.status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        app_redirect.header("location").as_deref(),
        Some("http://localhost:8080/app?x=1")
    );

    // State-changing /app requests are rejected, not redirected.
    let post_app = app
        .post_json_with_host("/app/login", json!({}), "go.example.com")
        .await;
    assert_eq!(post_app.status, StatusCode::NOT_FOUND);

    // The API is canonical-only on an extra domain.
    let api = app.get_with_host("/api/v1/domains", "go.example.com").await;
    assert_eq!(api.status, StatusCode::NOT_FOUND);
    assert_eq!(api.json()["error"]["code"], "not_found");

    // Health checks and assets stay host-agnostic.
    assert_eq!(
        app.get_with_host("/health", "go.example.com").await.status,
        StatusCode::OK
    );
    assert_eq!(
        app.get_with_host("/assets/app.css", "go.example.com")
            .await
            .status,
        StatusCode::OK
    );

    // Unknown hosts are invisible, and their 404 still gets the normal
    // security headers and request id from the outer middleware layers.
    let unknown = app.get_with_host("/promo", "unknown.example.com").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(
        unknown.header("x-content-type-options").as_deref(),
        Some("nosniff")
    );
    assert!(unknown.header("x-request-id").is_some());
}

#[tokio::test]
async fn the_same_slug_can_live_on_two_domains() {
    let app = TestApp::spawn().await;
    app.add_domain("go.example.com", "active").await;
    let admin = register_admin(&app).await;

    create_url(
        &app,
        &admin,
        json!({ "target_url": "https://example.com/a", "slug": "promo" }),
    )
    .await;
    create_url(
        &app,
        &admin,
        json!({
            "target_url": "https://example.com/b",
            "slug": "promo",
            "domain": "go.example.com",
        }),
    )
    .await;
    create_url(
        &app,
        &admin,
        json!({
            "target_url": "https://example.com/only-b",
            "slug": "extraonly",
            "domain": "go.example.com",
        }),
    )
    .await;

    let default = app.get("/promo").await;
    assert_eq!(default.status, StatusCode::FOUND);
    assert_eq!(
        default.header("location").as_deref(),
        Some("https://example.com/a")
    );

    let extra = app.get_with_host("/promo", "go.example.com").await;
    assert_eq!(extra.status, StatusCode::FOUND);
    assert_eq!(
        extra.header("location").as_deref(),
        Some("https://example.com/b")
    );

    // Public metadata defaults to the default domain and accepts an explicit one.
    let meta = app.get("/api/v1/public/urls/promo").await;
    assert_eq!(meta.status, StatusCode::OK);
    assert_eq!(meta.json()["domain"], "localhost:8080");
    let meta_extra = app
        .get("/api/v1/public/urls/promo?domain=go.example.com")
        .await;
    assert_eq!(meta_extra.status, StatusCode::OK);
    assert_eq!(meta_extra.json()["domain"], "go.example.com");

    // A slug that only exists on the extra domain is not visible by default.
    assert_eq!(
        app.get("/api/v1/public/urls/extraonly").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get("/api/v1/public/urls/extraonly?domain=go.example.com")
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn admin_domain_lifecycle_and_protections() {
    let app = TestApp::spawn().await;
    let admin = register_admin(&app).await;
    let canonical = domain_id(&app, "localhost:8080").await;

    // Add: pending.
    let created = app
        .post_json_as(
            "/api/v1/admin/domains",
            json!({ "hostname": "go.example.com" }),
            &admin,
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let domain = created.json();
    assert_eq!(domain["status"], "pending");
    assert_eq!(domain["source"], "ui");
    let go_id = domain["id"].as_i64().expect("id");

    // Invalid and duplicate hostnames are rejected.
    assert_eq!(
        app.post_json_as(
            "/api/v1/admin/domains",
            json!({ "hostname": "https://bad.example.com" }),
            &admin
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.post_json_as(
            "/api/v1/admin/domains",
            json!({ "hostname": "go.example.com" }),
            &admin
        )
        .await
        .status,
        StatusCode::CONFLICT
    );

    // Pending cannot be the default, and the status payload is validated.
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{go_id}"),
            json!({ "is_default": true }),
            &admin,
        ))
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{go_id}"),
            json!({ "status": "nonsense" }),
            &admin,
        ))
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // Activate, then use it.
    let activated = app
        .send(patch_json_as(
            &format!("/api/v1/admin/domains/{go_id}"),
            json!({ "status": "active" }),
            &admin,
        ))
        .await;
    assert_eq!(activated.status, StatusCode::OK, "{}", activated.body);
    assert_eq!(activated.json()["status"], "active");
    create_url(
        &app,
        &admin,
        json!({
            "target_url": "https://example.com/go",
            "slug": "promo",
            "domain": "go.example.com",
        }),
    )
    .await;

    // The canonical domain cannot be disabled or deleted.
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{canonical}"),
            json!({ "status": "disabled" }),
            &admin,
        ))
        .await
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/domains/{canonical}"),
            &admin,
        ))
        .await
        .status,
        StatusCode::CONFLICT
    );

    // The default domain cannot be disabled until another one replaces it.
    let defaulted = app
        .send(patch_json_as(
            &format!("/api/v1/admin/domains/{go_id}"),
            json!({ "is_default": true }),
            &admin,
        ))
        .await;
    assert_eq!(defaulted.status, StatusCode::OK, "{}", defaulted.body);
    assert_eq!(defaulted.json()["is_default"], true);
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{go_id}"),
            json!({ "status": "disabled" }),
            &admin,
        ))
        .await
        .status,
        StatusCode::CONFLICT
    );

    // Switch back, disable, and check that links stop resolving.
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{canonical}"),
            json!({ "is_default": true }),
            &admin,
        ))
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{go_id}"),
            json!({ "status": "disabled" }),
            &admin,
        ))
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        app.get_with_host("/promo", "go.example.com").await.status,
        StatusCode::NOT_FOUND
    );

    // Re-activate; the link works again.
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{go_id}"),
            json!({ "status": "active" }),
            &admin,
        ))
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        app.get_with_host("/promo", "go.example.com").await.status,
        StatusCode::FOUND
    );

    // A domain with links cannot be deleted.
    assert_eq!(
        app.send(delete_as(&format!("/api/v1/admin/domains/{go_id}"), &admin))
            .await
            .status,
        StatusCode::CONFLICT
    );

    // A zero-link UI domain can be deleted.
    let temp = app
        .post_json_as(
            "/api/v1/admin/domains",
            json!({ "hostname": "temp.example.com" }),
            &admin,
        )
        .await
        .json();
    let temp_id = temp["id"].as_i64().expect("temp id");
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/domains/{temp_id}"),
            &admin,
        ))
        .await
        .status,
        StatusCode::NO_CONTENT
    );

    // Unknown ids are 404.
    assert_eq!(
        app.send(patch_json_as(
            "/api/v1/admin/domains/999999",
            json!({ "status": "active" }),
            &admin,
        ))
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn environment_domains_are_active_and_insert_only() {
    let app = TestApp::spawn_with(|config| {
        config.extra_domains = vec!["env.example.com".to_string()];
        config.extra_domains_declared = true;
    })
    .await;
    let admin = register_admin(&app).await;

    assert_eq!(
        app.get_with_host("/", "env.example.com").await.status,
        StatusCode::OK
    );

    let list = app.get_as("/api/v1/admin/domains", &admin).await;
    assert_eq!(list.status, StatusCode::OK);
    let domains = list.json();
    let env = domains
        .as_array()
        .expect("array")
        .iter()
        .find(|domain| domain["hostname"] == "env.example.com")
        .expect("env domain");
    let page = app.get_as("/app/admin/domains", &admin).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("env.example.com"), "{}", page.body);
    assert!(page.body.contains("URLET_DOMAINS"), "{}", page.body);
    assert_eq!(env["source"], "env");
    assert_eq!(env["status"], "active");
    let env_id = env["id"].as_i64().expect("env id");

    // Environment domains can be disabled but not deleted from the UI.
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/domains/{env_id}"),
            &admin,
        ))
        .await
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{env_id}"),
            json!({ "status": "disabled" }),
            &admin,
        ))
        .await
        .status,
        StatusCode::OK
    );

    // A second bootstrap must not re-enable the disabled domain.
    urlet::server::bootstrap(&app.state)
        .await
        .expect("bootstrap again");
    let status: String = sqlx::query_scalar("SELECT status FROM domains WHERE id = ?")
        .bind(env_id)
        .fetch_one(&app.state.db)
        .await
        .expect("status");
    assert_eq!(status, "disabled");
    assert_eq!(
        app.get_with_host("/", "env.example.com").await.status,
        StatusCode::NOT_FOUND
    );

    // Removing it from URLET_DOMAINS detaches it from env management, after
    // which an administrator can delete the (zero-link) domain.
    let detached = urlet::db::domains::detach_env(&app.state.db, &[])
        .await
        .expect("detach env domains");
    assert_eq!(detached, 1);
    app.reload_domains().await;
    let source: String = sqlx::query_scalar("SELECT source FROM domains WHERE id = ?")
        .bind(env_id)
        .fetch_one(&app.state.db)
        .await
        .expect("source");
    assert_eq!(source, "ui");
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/domains/{env_id}"),
            &admin,
        ))
        .await
        .status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn changing_the_default_domain_changes_new_links() {
    let app = TestApp::spawn().await;
    let admin = register_admin(&app).await;

    let created = app
        .post_json_as(
            "/api/v1/admin/domains",
            json!({ "hostname": "go.example.com" }),
            &admin,
        )
        .await
        .json();
    let go_id = created["id"].as_i64().expect("id");
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{go_id}"),
            json!({ "status": "active" }),
            &admin,
        ))
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/admin/domains/{go_id}"),
            json!({ "is_default": true }),
            &admin,
        ))
        .await
        .status,
        StatusCode::OK
    );

    let link = create_url(
        &app,
        &admin,
        json!({ "target_url": "https://example.com/default", "slug": "defaulted" }),
    )
    .await;
    assert_eq!(link["domain"], "go.example.com");
    assert_eq!(link["short_url"], "http://go.example.com/defaulted");

    let choices = app.get_as("/api/v1/domains", &admin).await;
    assert_eq!(choices.status, StatusCode::OK);
    let choices = choices.json();
    assert_eq!(choices[0]["hostname"], "go.example.com");
    assert_eq!(choices[0]["is_default"], true);
}

#[tokio::test]
async fn host_names_are_normalised() {
    let app = TestApp::spawn().await;
    app.add_domain("go.example.com", "active").await;
    let admin = register_admin(&app).await;
    create_url(
        &app,
        &admin,
        json!({
            "target_url": "https://example.com/normalised",
            "slug": "promo",
            "domain": "go.example.com",
        }),
    )
    .await;

    for host in [
        "GO.EXAMPLE.COM",
        "go.example.com.",
        "go.example.com:80",
        "go.example.com:443",
    ] {
        assert_eq!(
            app.get_with_host("/promo", host).await.status,
            StatusCode::FOUND,
            "{host}"
        );
    }
    for host in [
        "go.example.com:8080",
        "unknown.example.com",
        "https://go.example.com",
    ] {
        assert_eq!(
            app.get_with_host("/promo", host).await.status,
            StatusCode::NOT_FOUND,
            "{host}"
        );
    }
}

#[tokio::test]
async fn unlock_cookies_are_bound_to_the_domain() {
    let app = TestApp::spawn().await;
    app.add_domain("go.example.com", "active").await;
    let admin = register_admin(&app).await;

    create_url(
        &app,
        &admin,
        json!({
            "target_url": "https://example.com/a",
            "slug": "promo",
            "password": "linkpass",
        }),
    )
    .await;
    create_url(
        &app,
        &admin,
        json!({
            "target_url": "https://example.com/b",
            "slug": "promo",
            "password": "linkpass",
            "domain": "go.example.com",
        }),
    )
    .await;

    let unlocked = app
        .post_json("/promo/unlock", json!({ "password": "linkpass" }))
        .await;
    assert_eq!(unlocked.status, StatusCode::OK);
    let default_name = urlet::services::unlock::cookie_name("localhost:8080", "promo");
    let extra_name = urlet::services::unlock::cookie_name("go.example.com", "promo");
    let cookie = unlocked.cookie_named(&default_name).expect("cookie");
    assert_eq!(
        app.get_as("/promo", &cookie).await.status,
        StatusCode::FOUND
    );

    // The cookie name alone does not unlock the other domain.
    let forged = cookie.replace(&default_name, &extra_name);
    let extra_page = app
        .send(common::get_with_host_as(
            "/promo",
            "go.example.com",
            &forged,
        ))
        .await;
    assert_eq!(extra_page.status, StatusCode::OK);
    assert!(extra_page.body.contains("Protected link"));

    // Unlocking on the extra domain issues a domain-bound cookie.
    let extra_unlock = app
        .post_json_with_host(
            "/promo/unlock",
            json!({ "password": "linkpass" }),
            "go.example.com",
        )
        .await;
    assert_eq!(extra_unlock.status, StatusCode::OK);
    let extra_cookie = extra_unlock
        .cookie_named(&extra_name)
        .expect("extra unlock cookie");
    let followed = app
        .send(common::get_with_host_as(
            "/promo",
            "go.example.com",
            &extra_cookie,
        ))
        .await;
    assert_eq!(followed.status, StatusCode::FOUND);
    assert_eq!(
        followed.header("location").as_deref(),
        Some("https://example.com/b")
    );

    // The HTML unlock form also works on the extra domain: the gate lets it
    // through and CSRF sees a matching Host/Origin pair.
    let form = app
        .send(post_form_with_host(
            "/promo/unlock",
            "password=linkpass",
            None,
            Some("http://go.example.com"),
            "go.example.com",
        ))
        .await;
    assert_eq!(form.status, StatusCode::SEE_OTHER);
    assert_eq!(form.header("location").as_deref(), Some("/promo"));
}

#[tokio::test]
async fn non_admins_cannot_manage_domains() {
    let app = TestApp::spawn().await;
    let _admin = register_admin(&app).await;
    let (user, _) = app.register("user@example.com", ADMIN_PASSWORD).await;

    assert_eq!(
        app.get_as("/api/v1/admin/domains", &user).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.post_json_as(
            "/api/v1/admin/domains",
            json!({ "hostname": "nope.example.com" }),
            &user
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn admin_domains_page_renders() {
    let app = TestApp::spawn().await;
    let admin = register_admin(&app).await;
    assert_eq!(
        app.post_json_as(
            "/api/v1/admin/domains",
            json!({ "hostname": "go.example.com" }),
            &admin,
        )
        .await
        .status,
        StatusCode::CREATED
    );

    let page = app.get_as("/app/admin/domains", &admin).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("go.example.com"), "{}", page.body);
    assert!(page.body.contains("Add domain"), "{}", page.body);
    assert!(page.body.contains("localhost:8080"), "{}", page.body);

    let (user, _) = app.register("normal@example.com", ADMIN_PASSWORD).await;
    assert_eq!(
        app.get_as("/app/admin/domains", &user).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn links_require_an_active_domain_and_move_conflicts_return_409() {
    let app = TestApp::spawn().await;
    let admin = register_admin(&app).await;
    app.add_domain("a.example.com", "active").await;
    app.add_domain("b.example.com", "active").await;
    app.add_domain("pending.example.com", "pending").await;
    app.add_domain("disabled.example.com", "disabled").await;

    for hostname in [
        "pending.example.com",
        "disabled.example.com",
        "unknown.example.com",
    ] {
        assert_eq!(
            app.post_json_as(
                "/api/v1/urls",
                json!({
                    "target_url": "https://example.com",
                    "slug": "blocked",
                    "domain": hostname,
                }),
                &admin,
            )
            .await
            .status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{hostname}"
        );
    }

    let first = create_url(
        &app,
        &admin,
        json!({
            "target_url": "https://example.com/a",
            "slug": "promo",
            "domain": "a.example.com",
        }),
    )
    .await;
    create_url(
        &app,
        &admin,
        json!({
            "target_url": "https://example.com/b",
            "slug": "promo",
            "domain": "b.example.com",
        }),
    )
    .await;

    // Moving to another domain must not collide with an existing slug.
    let conflict = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{}", first["id"]),
            json!({ "domain": "b.example.com" }),
            &admin,
        ))
        .await;
    assert_eq!(conflict.status, StatusCode::CONFLICT, "{}", conflict.body);

    // Changing the slug and moving in one request is allowed.
    let moved = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{}", first["id"]),
            json!({ "domain": "b.example.com", "slug": "moved" }),
            &admin,
        ))
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    assert_eq!(moved.json()["domain"], "b.example.com");
    assert_eq!(moved.json()["slug"], "moved");
    assert_eq!(
        app.get_with_host("/moved", "b.example.com").await.status,
        StatusCode::FOUND
    );
}
