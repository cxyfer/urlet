//! A link an administrator disabled stays disabled until an administrator
//! enables it again: its owner cannot lift the lock through the API or the
//! web forms.

mod common;

use axum::http::StatusCode;
use common::{delete_as, patch_json_as, post_form_as, post_json_as, TestApp};
use serde_json::{json, Value};

/// Register the first account (administrator) and return its cookie.
async fn admin(app: &TestApp) -> String {
    app.register("admin@example.com", "supersecret").await.0
}

/// Register a normal account and return its cookie.
async fn member(app: &TestApp, email: &str) -> String {
    app.register(email, "supersecret").await.0
}

async fn create_link(app: &TestApp, cookie: &str, slug: &str) -> i64 {
    let created = app
        .send(post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com", "slug": slug }),
            cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    created.json()["id"].as_i64().expect("id")
}

async fn admin_set_status(app: &TestApp, cookie: &str, id: i64, status: &str) -> Value {
    let response = app
        .send(patch_json_as(
            &format!("/api/v1/admin/urls/{id}"),
            json!({ "status": status }),
            cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    response.json()
}

async fn owner_view(app: &TestApp, cookie: &str, id: i64) -> Value {
    app.get_as(&format!("/api/v1/urls/{id}"), cookie)
        .await
        .json()
}

#[tokio::test]
async fn owner_cannot_reenable_a_link_an_administrator_disabled() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let owner = member(&app, "owner@example.com").await;
    let id = create_link(&app, &owner, "held").await;
    assert_eq!(app.get("/held").await.status, StatusCode::FOUND);

    let disabled = admin_set_status(&app, &admin_cookie, id, "disabled").await;
    assert_eq!(disabled["admin_locked"], true);
    assert_eq!(disabled["state"], "disabled");
    assert_eq!(app.get("/held").await.status, StatusCode::NOT_FOUND);

    // JSON API
    let api = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({ "status": "active" }),
            &owner,
        ))
        .await;
    assert_eq!(api.status, StatusCode::FORBIDDEN, "{}", api.body);
    // 同時送其他欄位時整筆更新都不能生效。
    let mixed = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({ "status": "active", "title": "sneaky" }),
            &owner,
        ))
        .await;
    assert_eq!(mixed.status, StatusCode::FORBIDDEN, "{}", mixed.body);
    assert_eq!(app.get("/held").await.status, StatusCode::NOT_FOUND);

    // Web form: the editor comes back with a readable message.
    let form = app
        .send(post_form_as(
            &format!("/app/urls/{id}"),
            "status=active",
            &owner,
        ))
        .await;
    assert_eq!(
        form.status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        form.body
    );
    assert!(
        form.body.contains("An administrator disabled this link"),
        "{}",
        form.body
    );
    assert_eq!(app.get("/held").await.status, StatusCode::NOT_FOUND);

    let current = owner_view(&app, &owner, id).await;
    assert_eq!(current["status"], "disabled");
    assert_eq!(current["admin_locked"], true);
    assert_ne!(current["title"], "sneaky");
}

#[tokio::test]
async fn locked_link_shows_the_lock_and_keeps_other_edits_working() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let owner = member(&app, "owner@example.com").await;
    let id = create_link(&app, &owner, "held").await;
    admin_set_status(&app, &admin_cookie, id, "disabled").await;

    let edit = app.get_as(&format!("/app/urls/{id}/edit"), &owner).await;
    assert_eq!(edit.status, StatusCode::OK, "{}", edit.body);
    assert!(edit
        .body
        .contains("Only an administrator can enable it again"));
    assert!(edit
        .body
        .contains("id=\"edit_status\" name=\"status\" disabled"));
    let detail = app.get_as(&format!("/app/urls/{id}"), &owner).await;
    assert!(detail.body.contains("Disabled by administrator"));
    let list = app.get_as("/app", &owner).await;
    assert!(list.body.contains("Disabled by administrator"));
    let admin_list = app.get_as("/app/admin/urls", &admin_cookie).await;
    assert!(admin_list.body.contains("Disabled by administrator"));

    // 其他欄位仍可修改（表單的狀態選單被停用，不會送出 status）。
    let title = app
        .send(post_form_as(
            &format!("/app/urls/{id}"),
            "title=Renamed&target_url=https%3A%2F%2Fexample.org",
            &owner,
        ))
        .await;
    assert_eq!(title.status, StatusCode::SEE_OTHER, "{}", title.body);
    let api = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({ "notes": "kept", "expires_at": null, "status": "disabled" }),
            &owner,
        ))
        .await;
    assert_eq!(api.status, StatusCode::OK, "{}", api.body);
    let current = owner_view(&app, &owner, id).await;
    assert_eq!(current["title"], "Renamed");
    assert_eq!(current["target_url"], "https://example.org");
    assert_eq!(current["admin_locked"], true);
    assert_eq!(app.get("/held").await.status, StatusCode::NOT_FOUND);

    // Extending the expiry does not bring the link back either.
    let extended = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({ "expires_at": "2099-01-01T00:00:00Z" }),
            &owner,
        ))
        .await;
    assert_eq!(extended.status, StatusCode::OK, "{}", extended.body);
    assert_eq!(app.get("/held").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn administrator_enabling_releases_the_lock() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let owner = member(&app, "owner@example.com").await;
    let id = create_link(&app, &owner, "held").await;
    admin_set_status(&app, &admin_cookie, id, "disabled").await;

    // The admin form path enables as well and clears the lock.
    let enabled = app
        .send(post_form_as(
            &format!("/app/admin/urls/{id}"),
            "status=active",
            &admin_cookie,
        ))
        .await;
    assert_eq!(enabled.status, StatusCode::SEE_OTHER, "{}", enabled.body);
    let current = owner_view(&app, &owner, id).await;
    assert_eq!(current["admin_locked"], false);
    assert_eq!(current["status"], "active");
    assert_eq!(app.get("/held").await.status, StatusCode::FOUND);

    // The owner is in control again.
    for (status, expected) in [
        ("disabled", StatusCode::NOT_FOUND),
        ("active", StatusCode::FOUND),
    ] {
        let changed = app
            .send(patch_json_as(
                &format!("/api/v1/urls/{id}"),
                json!({ "status": status }),
                &owner,
            ))
            .await;
        assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
        assert_eq!(app.get("/held").await.status, expected);
    }
    let form = app
        .send(post_form_as(
            &format!("/app/urls/{id}"),
            "status=disabled",
            &owner,
        ))
        .await;
    assert_eq!(form.status, StatusCode::SEE_OTHER, "{}", form.body);
    let form = app
        .send(post_form_as(
            &format!("/app/urls/{id}"),
            "status=active",
            &owner,
        ))
        .await;
    assert_eq!(form.status, StatusCode::SEE_OTHER, "{}", form.body);
    assert_eq!(app.get("/held").await.status, StatusCode::FOUND);
}

#[tokio::test]
async fn owner_disabled_links_are_not_locked() {
    let app = TestApp::spawn().await;
    admin(&app).await;
    let owner = member(&app, "owner@example.com").await;
    let id = create_link(&app, &owner, "mine").await;

    let off = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({ "status": "disabled" }),
            &owner,
        ))
        .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.body);
    assert_eq!(off.json()["admin_locked"], false);
    assert_eq!(app.get("/mine").await.status, StatusCode::NOT_FOUND);

    let edit = app.get_as(&format!("/app/urls/{id}/edit"), &owner).await;
    assert!(!edit
        .body
        .contains("Only an administrator can enable it again"));
    let on = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({ "status": "active" }),
            &owner,
        ))
        .await;
    assert_eq!(on.status, StatusCode::OK, "{}", on.body);
    assert_eq!(app.get("/mine").await.status, StatusCode::FOUND);
}

#[tokio::test]
async fn owner_can_still_delete_a_locked_link() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let owner = member(&app, "owner@example.com").await;
    let id = create_link(&app, &owner, "held").await;
    admin_set_status(&app, &admin_cookie, id, "disabled").await;

    let deleted = app
        .send(delete_as(&format!("/api/v1/urls/{id}"), &owner))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    assert_eq!(app.get("/held").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn redisabling_by_an_administrator_keeps_the_lock() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let owner = member(&app, "owner@example.com").await;
    let id = create_link(&app, &owner, "held").await;
    admin_set_status(&app, &admin_cookie, id, "disabled").await;
    let again = admin_set_status(&app, &admin_cookie, id, "disabled").await;
    assert_eq!(again["admin_locked"], true);

    // 只改轉址狀態碼不會動到鎖。
    let redirect = app
        .send(patch_json_as(
            &format!("/api/v1/admin/urls/{id}"),
            json!({ "redirect_type": 301 }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(redirect.status, StatusCode::OK, "{}", redirect.body);
    assert_eq!(redirect.json()["admin_locked"], true);
    let blocked = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{id}"),
            json!({ "status": "active" }),
            &owner,
        ))
        .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
}

/// 擁有者讀到連結之後、寫入之前管理員恰好停用：UPDATE 自己的條件必須擋下。
#[tokio::test]
async fn the_update_statement_itself_refuses_a_lock_taken_after_the_read() {
    use urlet::db::urls::{self, UrlUpdate};

    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let owner = member(&app, "owner@example.com").await;
    let id = create_link(&app, &owner, "held").await;
    let stale = urls::find_by_id(&app.state.db, id)
        .await
        .expect("query")
        .expect("link");
    assert!(!stale.is_admin_locked());
    admin_set_status(&app, &admin_cookie, id, "disabled").await;

    let activate = UrlUpdate {
        status: Some("active".into()),
        ..UrlUpdate::default()
    };
    let changed = urls::update(&app.state.db, id, Some(stale.user_id), &activate)
        .await
        .expect("update");
    assert_eq!(changed, 0, "the owner path must not match a locked link");
    let current = urls::find_by_id(&app.state.db, id)
        .await
        .expect("query")
        .expect("link");
    assert_eq!(current.status, "disabled");
    assert!(current.is_admin_locked());

    // 沒有 status 的擁有者更新、以及管理員路徑（沒有 user_id）不受條件影響。
    let rename = UrlUpdate {
        title: Some(Some("Renamed".into())),
        ..UrlUpdate::default()
    };
    assert_eq!(
        urls::update(&app.state.db, id, Some(stale.user_id), &rename)
            .await
            .expect("update"),
        1
    );
    assert_eq!(
        urls::update(&app.state.db, id, None, &activate)
            .await
            .expect("update"),
        1
    );
}

/// 同一個競態走 service：預檢看到的是過期的連結，必須由 UPDATE 落空後回 403。
#[tokio::test]
async fn the_service_reports_a_lock_taken_after_the_read_as_forbidden() {
    use urlet::db::urls;
    use urlet::db::users;
    use urlet::error::AppError;
    use urlet::services::urls::{UpdateInput, UrlContext, ADMIN_LOCKED};

    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let owner_cookie = member(&app, "owner@example.com").await;
    let id = create_link(&app, &owner_cookie, "held").await;
    let stale = urls::find_by_id(&app.state.db, id)
        .await
        .expect("query")
        .expect("link");
    admin_set_status(&app, &admin_cookie, id, "disabled").await;

    let actor = users::find_by_id(&app.state.db, stale.user_id)
        .await
        .expect("query")
        .expect("owner");
    let context = UrlContext {
        pool: &app.state.db,
        settings: &app.state.settings,
        actor: &actor,
        ip_hash: None,
    };
    let result = context
        .update(
            &stale,
            UpdateInput {
                status: Some("active".into()),
                ..UpdateInput::default()
            },
        )
        .await;
    assert!(
        matches!(&result, Err(AppError::Forbidden(message)) if message == ADMIN_LOCKED),
        "{result:?}"
    );
    assert_eq!(app.get("/held").await.status, StatusCode::NOT_FOUND);
}
