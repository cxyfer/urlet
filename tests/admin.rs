//! Integration tests for the admin API and admin pages.

mod common;

use axum::http::StatusCode;
use common::{
    delete_as, patch_json_as, post_form_as, post_json, post_json_as, put_json_as, TestApp,
};
use serde_json::{json, Value};

/// Register the first account (administrator) and return `(cookie, id)`.
async fn admin(app: &TestApp) -> (String, i64) {
    let (cookie, user) = app.register("admin@example.com", "supersecret").await;
    (cookie, user["user"]["id"].as_i64().expect("id"))
}

/// Register a normal account and return `(cookie, id)`.
async fn member(app: &TestApp, email: &str) -> (String, i64) {
    let (cookie, user) = app.register(email, "supersecret").await;
    (cookie, user["user"]["id"].as_i64().expect("id"))
}

#[tokio::test]
async fn non_admins_are_rejected() {
    let app = TestApp::spawn().await;
    admin(&app).await;
    let (member_cookie, _) = member(&app, "member@example.com").await;

    for uri in [
        "/api/v1/admin/users",
        "/api/v1/admin/urls",
        "/api/v1/admin/invites",
        "/api/v1/admin/themes",
        "/api/v1/admin/settings",
        "/api/v1/admin/audit",
        "/api/v1/admin/stats",
    ] {
        let response = app.get_as(uri, &member_cookie).await;
        assert_eq!(
            response.status,
            StatusCode::FORBIDDEN,
            "{uri} must be admin only"
        );
        assert_eq!(response.json()["error"]["code"], "forbidden");
    }

    assert_eq!(
        app.get("/api/v1/admin/users").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.get_as("/app/admin", &member_cookie).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.get_as("/app/admin/users", &member_cookie).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn admin_pages_render_for_administrators() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    member(&app, "member@example.com").await;

    for (uri, needle) in [
        ("/app/admin", "System overview"),
        ("/app/admin/users", "member@example.com"),
        ("/app/admin/groups", "User groups"),
        ("/app/admin/urls", "All links"),
        ("/app/admin/invites", "Invite codes"),
        ("/app/admin/themes", "Theme catalogue"),
        ("/app/admin/settings", "Instance settings"),
        ("/app/admin/audit", "Audit log"),
    ] {
        let response = app.get_as(uri, &cookie).await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "{uri} failed: {}",
            response.body
        );
        assert!(response.body.contains(needle), "{uri} is missing {needle}");
    }
}

#[tokio::test]
async fn user_listing_supports_filters_and_link_counts() {
    let app = TestApp::spawn().await;
    let (cookie, admin_id) = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "member@example.com").await;
    app.send(post_json_as(
        "/api/v1/urls",
        json!({ "target_url": "https://example.com" }),
        &member_cookie,
    ))
    .await;

    let all = app.get_as("/api/v1/admin/users", &cookie).await.json();
    assert_eq!(all["total"], 2);

    let admins = all["items"]
        .as_array()
        .expect("items")
        .iter()
        .filter(|item| item["role"] == "admin")
        .count();
    assert_eq!(admins, 1);

    let member_row = all["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["id"] == member_id)
        .expect("member row");
    assert_eq!(member_row["link_count"], 1);

    let search = app
        .get_as("/api/v1/admin/users?q=member", &cookie)
        .await
        .json();
    assert_eq!(search["total"], 1);
    assert_eq!(search["items"][0]["id"], member_id);

    let by_role = app
        .get_as("/api/v1/admin/users?role=admin", &cookie)
        .await
        .json();
    assert_eq!(by_role["total"], 1);
    assert_eq!(by_role["items"][0]["id"], admin_id);
}

#[tokio::test]
async fn suspending_a_user_kills_their_sessions() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "member@example.com").await;

    assert_eq!(
        app.get_as("/api/v1/auth/me", &member_cookie).await.status,
        StatusCode::OK
    );

    let response = app
        .send(patch_json_as(
            &format!("/api/v1/admin/users/{member_id}"),
            json!({ "status": "suspended" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json()["status"], "suspended");

    assert_eq!(
        app.get_as("/api/v1/auth/me", &member_cookie).await.status,
        StatusCode::UNAUTHORIZED
    );
    let login = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "member@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(login.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn administrators_cannot_lock_themselves_out() {
    let app = TestApp::spawn().await;
    let (cookie, admin_id) = admin(&app).await;

    let demote = app
        .send(patch_json_as(
            &format!("/api/v1/admin/users/{admin_id}"),
            json!({ "role": "user" }),
            &cookie,
        ))
        .await;
    assert_eq!(demote.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(demote.json()["error"]["message"]
        .as_str()
        .expect("message")
        .contains("your own"));

    let suspend = app
        .send(patch_json_as(
            &format!("/api/v1/admin/users/{admin_id}"),
            json!({ "status": "suspended" }),
            &cookie,
        ))
        .await;
    assert_eq!(suspend.status, StatusCode::UNPROCESSABLE_ENTITY);

    let delete = app
        .send(delete_as(
            &format!("/api/v1/admin/users/{admin_id}"),
            &cookie,
        ))
        .await;
    assert_eq!(delete.status, StatusCode::UNPROCESSABLE_ENTITY);

    // Still an administrator.
    assert_eq!(
        app.get_as("/api/v1/admin/users", &cookie).await.status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn password_reset_revokes_sessions_and_changes_the_password() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "member@example.com").await;

    let response = app
        .send(post_json_as(
            &format!("/api/v1/admin/users/{member_id}/password"),
            json!({ "new_password": "brandnewpass" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", response.body);
    assert_eq!(
        app.get_as("/api/v1/auth/me", &member_cookie).await.status,
        StatusCode::UNAUTHORIZED
    );

    let login = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "member@example.com", "password": "brandnewpass" }),
        )
        .await;
    assert_eq!(login.status, StatusCode::OK);
}

#[tokio::test]
async fn deleting_a_user_removes_their_links() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "member@example.com").await;
    let created = app
        .send(post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com", "slug": "gone" }),
            &member_cookie,
        ))
        .await
        .json();
    let url_id = created["id"].as_i64().expect("id");

    let response = app
        .send(delete_as(
            &format!("/api/v1/admin/users/{member_id}"),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    assert_eq!(app.get("/gone").await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        app.get_as(&format!("/api/v1/urls/{url_id}"), &cookie)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.get_as("/api/v1/admin/users", &cookie).await.json()["total"],
        1
    );
}

#[tokio::test]
async fn admin_can_list_and_moderate_links() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "member@example.com").await;
    let created = app
        .send(post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com/thing", "slug": "thing" }),
            &member_cookie,
        ))
        .await
        .json();
    let url_id = created["id"].as_i64().expect("id");

    let listing = app.get_as("/api/v1/admin/urls", &cookie).await.json();
    assert_eq!(listing["total"], 1);
    assert_eq!(listing["items"][0]["slug"], "thing");

    let by_owner = app
        .get_as(&format!("/api/v1/admin/urls?user_id={member_id}"), &cookie)
        .await
        .json();
    assert_eq!(by_owner["total"], 1);
    let other = app
        .get_as("/api/v1/admin/urls?user_id=99999", &cookie)
        .await
        .json();
    assert_eq!(other["total"], 0);

    let disabled = app
        .send(patch_json_as(
            &format!("/api/v1/admin/urls/{url_id}"),
            json!({ "status": "disabled" }),
            &cookie,
        ))
        .await;
    assert_eq!(disabled.status, StatusCode::OK, "{}", disabled.body);
    assert_eq!(disabled.json()["state"], "disabled");
    assert_eq!(app.get("/thing").await.status, StatusCode::NOT_FOUND);

    let deleted = app
        .send(delete_as(&format!("/api/v1/admin/urls/{url_id}"), &cookie))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.get_as("/api/v1/admin/urls", &cookie).await.json()["total"],
        0
    );
}

#[tokio::test]
async fn invites_drive_invite_only_registration() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    app.state
        .settings
        .update(&app.state.db, "registration_mode", "invite", None)
        .await
        .expect("invite mode");

    let created = app
        .send(post_json_as(
            "/api/v1/admin/invites",
            json!({ "note": "design team", "max_uses": 2, "expires_at": "2030-01-01" }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.header("cache-control").as_deref(), Some("no-store"));
    let body = created.json();
    let code = body["code"].as_str().expect("code").to_string();
    assert_eq!(code.len(), 10);
    let link = body["link"].as_str().expect("link").to_string();
    assert_eq!(
        link,
        format!("http://localhost:8080/app/register?invite={code}")
    );
    let invite_id = body["invite"]["id"].as_i64().expect("id");
    assert_eq!(body["invite"]["remaining"], 2);
    assert_eq!(body["invite"]["usable"], true);
    assert_eq!(body["invite"]["code"], code);
    assert_eq!(body["invite"]["link"], link);

    // The code and link stay available in later listings.
    let listing = app.get_as("/api/v1/admin/invites", &cookie).await;
    assert_eq!(listing.header("cache-control").as_deref(), Some("no-store"));
    let listing = listing.json();
    assert_eq!(listing["items"].as_array().expect("items").len(), 1);
    assert_eq!(listing["items"][0]["code"], code.as_str());
    assert_eq!(listing["items"][0]["link"], link.as_str());

    // Registering with the code consumes one use.
    let registered = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "first@example.com", "password": "supersecret", "invite_code": code }),
        )
        .await;
    assert_eq!(
        registered.status,
        StatusCode::CREATED,
        "{}",
        registered.body
    );
    let listing = app.get_as("/api/v1/admin/invites", &cookie).await.json();
    assert_eq!(listing["items"][0]["used_count"], 1);
    assert_eq!(listing["items"][0]["remaining"], 1);

    // A revoked code cannot be used.
    let revoked = app
        .send(delete_as(
            &format!("/api/v1/admin/invites/{invite_id}"),
            &cookie,
        ))
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT);
    let listing = app.get_as("/api/v1/admin/invites", &cookie).await.json();
    assert_eq!(listing["items"][0]["code"], code.as_str());
    assert!(
        listing["items"][0]["link"].is_null(),
        "a revoked code must not expose a link: {}",
        listing["items"][0]
    );
    let blocked = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "second@example.com", "password": "supersecret", "invite_code": code }),
        )
        .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/invites/{invite_id}"),
            &cookie
        ))
        .await
        .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn invite_form_creates_a_code_and_keeps_it_visible() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;

    let response = app
        .send(post_form_as(
            "/app/admin/invites",
            "note=team&max_uses=3&expires_at=2030-01-01",
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(
        response.header("cache-control").as_deref(),
        Some("no-store")
    );
    assert!(response.body.contains("Invite code created."));

    // The stored copy is encrypted, but the page can show the plaintext and
    // its registration link again.
    let (sealed, code_hash): (String, String) =
        sqlx::query_as("SELECT code_sealed, code_hash FROM invite_codes WHERE note = 'team'")
            .fetch_one(&app.state.db)
            .await
            .expect("sealed code");
    let code =
        urlet::services::invite_code::open(app.state.config.secret.as_slice(), &code_hash, &sealed)
            .expect("decrypt");
    assert!(!sealed.contains(&code), "plaintext must not be stored");
    let link = urlet::services::invites::registration_link(&app.state.config.base_url, &code);
    assert!(response.body.contains(&code), "{}", response.body);
    assert!(response.body.contains(&link), "{}", response.body);

    let page = app.get_as("/app/admin/invites", &cookie).await;
    assert_eq!(page.header("cache-control").as_deref(), Some("no-store"));
    assert!(page.body.contains("team"));
    assert!(page.body.contains(&code), "{}", page.body);
    assert!(page.body.contains(&link), "{}", page.body);
    assert!(page.body.contains("Copy link"), "{}", page.body);
    assert!(
        !page.body.contains("only once"),
        "the page must not claim the code is one-time"
    );
}

#[tokio::test]
async fn legacy_invites_fall_back_to_the_prefix() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;

    sqlx::query(
        "INSERT INTO invite_codes
             (code_hash, code_prefix, note, max_uses, used_count, status, created_at)
         VALUES ('legacy-hash', 'OLD0', 'legacy', 1, 0, 'active', '2026-01-01T00:00:00.000Z')",
    )
    .execute(&app.state.db)
    .await
    .expect("legacy invite");

    let page = app.get_as("/app/admin/invites", &cookie).await;
    assert_eq!(page.header("cache-control").as_deref(), Some("no-store"));
    assert!(page.body.contains("OLD0…"), "{}", page.body);
    assert!(
        page.body.contains("Codes issued before this version"),
        "{}",
        page.body
    );

    let listing = app.get_as("/api/v1/admin/invites", &cookie).await;
    assert_eq!(listing.header("cache-control").as_deref(), Some("no-store"));
    let listing = listing.json();
    assert!(listing["items"][0]["code"].is_null());
    assert!(listing["items"][0]["link"].is_null());
}

#[tokio::test]
async fn sealed_codes_cannot_be_swapped_between_rows() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;

    for note in ["first", "second"] {
        let created = app
            .post_json_as(
                "/api/v1/admin/invites",
                json!({ "note": note, "max_uses": 1 }),
                &cookie,
            )
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    }

    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, code_sealed FROM invite_codes ORDER BY id")
            .fetch_all(&app.state.db)
            .await
            .expect("sealed rows");
    assert_eq!(rows.len(), 2);
    sqlx::query("UPDATE invite_codes SET code_sealed = ? WHERE id = ?")
        .bind(&rows[1].1)
        .bind(rows[0].0)
        .execute(&app.state.db)
        .await
        .expect("swap first");
    sqlx::query("UPDATE invite_codes SET code_sealed = ? WHERE id = ?")
        .bind(&rows[0].1)
        .bind(rows[1].0)
        .execute(&app.state.db)
        .await
        .expect("swap second");

    // The AAD carries each row's code_hash, so neither swapped value opens.
    let listing = app.get_as("/api/v1/admin/invites", &cookie).await.json();
    for item in listing["items"].as_array().expect("items") {
        assert!(item["code"].is_null(), "{item}");
        assert!(item["link"].is_null(), "{item}");
    }
}

#[tokio::test]
async fn settings_can_be_read_and_updated() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;

    let current = app.get_as("/api/v1/admin/settings", &cookie).await.json();
    assert_eq!(current["registration_mode"], "open");
    assert_eq!(current["site_name"], "urlet");

    let updated = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({ "site_name": "Short Links", "max_urls_per_user": "5", "analytics_enabled": "false" }),
            &cookie,
        ))
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    assert_eq!(updated.json()["site_name"], "Short Links");

    // The new values take effect immediately.
    assert_eq!(app.state.settings.site_name(), "Short Links");
    assert_eq!(app.state.settings.max_urls_per_user(), 5);
    assert!(!app.state.settings.analytics_enabled());

    let login_page = app.get("/app/login").await;
    assert!(login_page.body.contains("Short Links"));

    // Unknown keys and invalid values are rejected.
    let unknown = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({ "nope": "1" }),
            &cookie,
        ))
        .await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);

    let invalid = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({ "default_slug_length": "2" }),
            &cookie,
        ))
        .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn settings_form_persists_values() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;

    let response = app
        .send(post_form_as(
            "/app/admin/settings",
            "site_name=Team+Links&registration_mode=invite&default_slug_length=9\
             &max_urls_per_user=250&max_api_tokens_per_user=7&default_redirect_type=301&analytics_enabled=on&click_retention_days=30",
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/app/admin/settings?ok=saved")
    );

    assert_eq!(app.state.settings.site_name(), "Team Links");
    assert_eq!(app.state.settings.default_slug_length(), 9);
    assert_eq!(app.state.settings.max_api_tokens_per_user(), 7);
    assert_eq!(app.state.settings.default_redirect_type(), 301);
    assert_eq!(app.state.settings.click_retention_days(), 30);

    // Analytics off means a new setting is honoured by the settings page.
    let page = app.get_as("/app/admin/settings", &cookie).await;
    assert!(page.body.contains("Team Links"));
    assert!(page.body.contains("invite — invite code required"));
}

#[tokio::test]
async fn audit_log_records_actions_and_filters() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "member@example.com").await;
    app.send(post_json_as(
        "/api/v1/urls",
        json!({ "target_url": "https://example.com", "slug": "audited" }),
        &member_cookie,
    ))
    .await;

    let response = app.get_as("/api/v1/admin/audit", &cookie).await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert!(body["total"].as_i64().expect("total") >= 3);
    let actions: Vec<String> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["action"].as_str().expect("action").to_string())
        .collect();
    for expected in ["auth.register", "url.created"] {
        assert!(
            actions.contains(&expected.to_string()),
            "missing {expected} in {actions:?}"
        );
    }

    let filtered = app
        .get_as("/api/v1/admin/audit?action=url.created", &cookie)
        .await
        .json();
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["items"][0]["target_type"], "url");

    // Suspending the member adds an admin entry with metadata.
    app.send(patch_json_as(
        &format!("/api/v1/admin/users/{member_id}"),
        json!({ "status": "suspended" }),
        &cookie,
    ))
    .await;
    let admin_entries = app
        .get_as("/api/v1/admin/audit?action=admin.user_updated", &cookie)
        .await
        .json();
    assert_eq!(admin_entries["total"], 1);
    assert_eq!(admin_entries["items"][0]["metadata"]["status"], "suspended");
}

#[tokio::test]
async fn stats_report_instance_numbers() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let (member_cookie, _) = member(&app, "member@example.com").await;
    app.send(post_json_as(
        "/api/v1/urls",
        json!({ "target_url": "https://example.com", "slug": "counted" }),
        &member_cookie,
    ))
    .await;
    app.get("/counted").await;

    let response = app.get_as("/api/v1/admin/stats", &cookie).await;
    assert_eq!(response.status, StatusCode::OK);
    let stats = response.json();
    assert_eq!(stats["users"], 2);
    assert_eq!(stats["admins"], 1);
    assert_eq!(stats["links"], 1);
    assert_eq!(stats["version"], env!("CARGO_PKG_VERSION"));
    assert!(stats["database_bytes"].as_u64().expect("bytes") > 0);
    assert_eq!(stats["registration_mode"], "open");
}

#[tokio::test]
async fn admin_forms_moderate_users_and_links() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "member@example.com").await;
    let created = app
        .send(post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com", "slug": "moderated" }),
            &member_cookie,
        ))
        .await
        .json();
    let url_id = created["id"].as_i64().expect("id");

    let suspended = app
        .send(post_form_as(
            &format!("/app/admin/users/{member_id}"),
            "role=user&status=suspended",
            &cookie,
        ))
        .await;
    assert_eq!(suspended.status, StatusCode::SEE_OTHER);
    assert_eq!(
        suspended.header("location").as_deref(),
        Some("/app/admin/users?ok=user")
    );

    let disabled = app
        .send(post_form_as(
            &format!("/app/admin/urls/{url_id}"),
            "status=disabled",
            &cookie,
        ))
        .await;
    assert_eq!(
        disabled.header("location").as_deref(),
        Some("/app/admin/urls?ok=updated")
    );
    assert_eq!(app.get("/moderated").await.status, StatusCode::NOT_FOUND);

    let deleted = app
        .send(post_form_as(
            &format!("/app/admin/urls/{url_id}/delete"),
            "",
            &cookie,
        ))
        .await;
    assert_eq!(
        deleted.header("location").as_deref(),
        Some("/app/admin/urls?ok=deleted")
    );

    let reset = app
        .send(post_form_as(
            &format!("/app/admin/users/{member_id}/password"),
            "new_password=resetpassword",
            &cookie,
        ))
        .await;
    assert_eq!(
        reset.header("location").as_deref(),
        Some("/app/admin/users?ok=user")
    );

    // The account is still suspended, so re-enable it before signing in.
    let reactivated = app
        .send(post_form_as(
            &format!("/app/admin/users/{member_id}"),
            "role=user&status=active",
            &cookie,
        ))
        .await;
    assert_eq!(reactivated.status, StatusCode::SEE_OTHER);
    let login = post_json(
        "/api/v1/auth/login",
        json!({ "email": "member@example.com", "password": "resetpassword" }),
    );
    assert_eq!(app.send(login).await.status, StatusCode::OK);
}

#[tokio::test]
async fn maintenance_prunes_stale_rows() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let (member_cookie, _) = member(&app, "member@example.com").await;
    let created = app
        .send(post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com", "slug": "old" }),
            &member_cookie,
        ))
        .await
        .json();
    let url_id = created["id"].as_i64().expect("id");
    app.get("/old").await;
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    // Age the analytics rows and enable retention.
    sqlx::query("UPDATE clicks SET clicked_at = '2000-01-01T00:00:00.000Z'")
        .execute(&app.state.db)
        .await
        .expect("age clicks");
    sqlx::query("UPDATE url_daily_stats SET day = '2000-01-01'")
        .execute(&app.state.db)
        .await
        .expect("age rollups");
    sqlx::query(
        "UPDATE sessions SET expires_at = '2000-01-01T00:00:00.000Z',
                             revoked_at = '2000-01-01T00:00:00.000Z'
         WHERE user_id = (SELECT id FROM users WHERE email_normalized = 'member@example.com')",
    )
    .execute(&app.state.db)
    .await
    .expect("age sessions");
    app.state
        .settings
        .update(&app.state.db, "click_retention_days", "7", None)
        .await
        .expect("retention");

    urlet::services::admin::run_maintenance(&app.state.db, &app.state.settings)
        .await
        .expect("maintenance");

    let clicks: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clicks")
        .fetch_one(&app.state.db)
        .await
        .expect("clicks");
    let rollups: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM url_daily_stats")
        .fetch_one(&app.state.db)
        .await
        .expect("rollups");
    assert_eq!(
        clicks, 0,
        "clicks older than the retention window are pruned"
    );
    assert_eq!(rollups, 0);

    // The link itself is untouched.
    let admin_links = app.get_as("/api/v1/admin/urls", &cookie).await.json();
    assert_eq!(admin_links["total"], 1);
    assert_eq!(admin_links["items"][0]["id"], url_id);
    // The aged session is gone, so the member cookie no longer works.
    assert_eq!(
        app.get_as("/api/v1/auth/me", &member_cookie).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn admin_api_requires_json_errors_not_html() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let response = app
        .send(delete_as("/api/v1/admin/users/9999", &cookie))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.json()["error"]["code"], "not_found");

    let bad_status = app
        .send(patch_json_as(
            "/api/v1/admin/users/1",
            json!({ "status": "nonsense" }),
            &cookie,
        ))
        .await;
    assert_eq!(bad_status.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad_status.json()["error"]["details"]["field"], "status");
}

#[tokio::test]
async fn value_helper_keeps_types_checked() {
    // Guards against accidental removal of the flattened DTO shape.
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;
    let body: Value = app.get_as("/api/v1/admin/users", &cookie).await.json();
    assert!(body["items"].is_array());
    assert!(body["items"][0]["email"].is_string());
    assert!(body["items"][0]["link_count"].is_number());
}
