//! Integration tests for user groups: CRUD, quotas, feature toggles, invites,
//! reassignment and the default marker.

mod common;

use axum::http::StatusCode;
use common::{delete_as, patch_json_as, post_form_as, put_json_as, TestApp, TestResponse};
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

async fn groups(app: &TestApp, cookie: &str) -> Value {
    app.get_as("/api/v1/admin/groups", cookie).await.json()
}

/// Locate one group in the admin listing by name.
fn group_named(list: &Value, name: &str) -> Value {
    list["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("group {name} not found in {list}"))
        .clone()
}

async fn default_group(app: &TestApp, cookie: &str) -> Value {
    groups(app, cookie)
        .await
        .get("items")
        .and_then(|items| items.as_array())
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["is_default"] == true)
                .cloned()
        })
        .expect("a default group")
}

async fn create_group(app: &TestApp, cookie: &str, body: Value) -> TestResponse {
    app.post_json_as("/api/v1/admin/groups", body, cookie).await
}

async fn update_group(app: &TestApp, cookie: &str, id: i64, body: Value) -> TestResponse {
    app.send(put_json_as(
        &format!("/api/v1/admin/groups/{id}"),
        body,
        cookie,
    ))
    .await
}

async fn assign(app: &TestApp, admin_cookie: &str, user_id: i64, group_id: i64) -> TestResponse {
    app.send(patch_json_as(
        &format!("/api/v1/admin/users/{user_id}"),
        json!({ "group_id": group_id }),
        admin_cookie,
    ))
    .await
}

async fn create_link(app: &TestApp, cookie: &str, body: Value) -> TestResponse {
    app.post_json_as("/api/v1/urls", body, cookie).await
}

async fn set_site_quota(app: &TestApp, admin_cookie: &str, key: &str, value: &str) {
    let response = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({ key: value }),
            admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
}

#[tokio::test]
async fn groups_require_an_administrator() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let (member_cookie, _) = member(&app, "member@example.com").await;
    let group_id = default_group(&app, &admin_cookie).await["id"]
        .as_i64()
        .expect("id");

    assert_eq!(
        app.get_as("/api/v1/admin/groups", &member_cookie)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        create_group(&app, &member_cookie, json!({ "name": "Nope" }))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.get("/api/v1/admin/groups").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.get_as("/app/admin/groups", &member_cookie).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.get_as("/app/admin/groups/new", &member_cookie)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.get_as(
            &format!("/app/admin/groups/{group_id}/edit"),
            &member_cookie
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn groups_can_be_created_listed_and_updated() {
    let app = TestApp::spawn().await;
    let (cookie, _) = admin(&app).await;

    let list = groups(&app, &cookie).await;
    assert_eq!(list["items"].as_array().expect("items").len(), 1);
    assert_eq!(list["items"][0]["name"], "Standard");
    assert_eq!(list["items"][0]["is_default"], true);
    assert_eq!(list["items"][0]["members"], 1);

    let created = create_group(
        &app,
        &cookie,
        json!({
            "name": "  Pro  ",
            "description": " paid tier ",
            "max_urls": 5,
            "max_api_tokens": 2,
            "allow_custom_slug": false,
            "allow_tags": false
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let group = created.json();
    assert_eq!(group["name"], "Pro");
    assert_eq!(group["description"], "paid tier");
    assert_eq!(group["max_urls"], 5);
    assert_eq!(group["max_api_tokens"], 2);
    assert_eq!(group["allow_custom_slug"], false);
    assert_eq!(group["allow_tags"], false);
    // Omitted toggles default to allowed.
    assert_eq!(group["allow_password"], true);
    assert_eq!(group["allow_expiry"], true);
    assert_eq!(group["allow_domain_selection"], true);
    assert_eq!(group["is_default"], false);
    assert_eq!(group["members"], 0);
    let id = group["id"].as_i64().expect("id");

    // A name collides case-insensitively.
    let duplicate = create_group(&app, &cookie, json!({ "name": "standard" })).await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);

    // The API list is paginated even though the HTML page needs every group.
    let page = app
        .get_as("/api/v1/admin/groups?page=1&per_page=1", &cookie)
        .await
        .json();
    assert_eq!(page["total"], 2);
    assert_eq!(page["total_pages"], 2);
    assert_eq!(page["items"].as_array().expect("items").len(), 1);

    let updated = update_group(
        &app,
        &cookie,
        id,
        json!({
            "name": "Pro Plus",
            "max_urls": null,
            "allow_custom_slug": true
        }),
    )
    .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    let updated = updated.json();
    assert_eq!(updated["name"], "Pro Plus");
    assert_eq!(updated["max_urls"], Value::Null);
    assert_eq!(updated["allow_custom_slug"], true);

    // The marker moves in one transaction; the previous default is cleared.
    let moved = update_group(
        &app,
        &cookie,
        id,
        json!({ "name": "Pro Plus", "is_default": true }),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    assert_eq!(moved.json()["is_default"], true);
    let list = groups(&app, &cookie).await;
    let standard = group_named(&list, "Standard");
    assert_eq!(standard["is_default"], false);
    // The list puts the default first.
    assert_eq!(list["items"][0]["name"], "Pro Plus");

    // Validation and missing ids.
    let empty = create_group(&app, &cookie, json!({ "name": "   " })).await;
    assert_eq!(empty.status, StatusCode::UNPROCESSABLE_ENTITY);
    let missing = update_group(&app, &cookie, 999_999, json!({ "name": "Ghost" })).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn link_quotas_inherit_and_enforce() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Small", "max_urls": 1 }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "quota@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    assert_eq!(
        create_link(
            &app,
            &member_cookie,
            json!({ "target_url": "https://example.com/one" })
        )
        .await
        .status,
        StatusCode::CREATED
    );
    let blocked = create_link(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/two" }),
    )
    .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert_eq!(blocked.json()["error"]["code"], "forbidden");
    assert!(blocked.json()["error"]["message"]
        .as_str()
        .expect("message")
        .contains("limit of 1 links"));

    // NULL inherits the site-wide value, which is now 2.
    let inherited = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Small", "max_urls": null }),
    )
    .await;
    assert_eq!(inherited.status, StatusCode::OK, "{}", inherited.body);
    set_site_quota(&app, &admin_cookie, "max_urls_per_user", "2").await;

    assert_eq!(
        create_link(
            &app,
            &member_cookie,
            json!({ "target_url": "https://example.com/two" })
        )
        .await
        .status,
        StatusCode::CREATED
    );
    assert_eq!(
        create_link(
            &app,
            &member_cookie,
            json!({ "target_url": "https://example.com/three" })
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );

    // 0 means unlimited, and it is not treated as "inherit".
    let unlimited = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Small", "max_urls": 0 }),
    )
    .await;
    assert_eq!(unlimited.status, StatusCode::OK, "{}", unlimited.body);
    for path in ["three", "four", "five"] {
        assert_eq!(
            create_link(
                &app,
                &member_cookie,
                json!({ "target_url": format!("https://example.com/{path}") })
            )
            .await
            .status,
            StatusCode::CREATED
        );
    }
}

#[tokio::test]
async fn feature_toggles_gate_creation_and_updates() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Locked",
            "allow_custom_slug": false,
            "allow_password": false,
            "allow_expiry": false,
            "allow_max_clicks": false,
            "allow_tags": false,
            "allow_api_tokens": false
        }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "locked@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    for body in [
        json!({ "target_url": "https://example.com/a", "slug": "custom" }),
        json!({ "target_url": "https://example.com/a", "password": "secret1" }),
        json!({ "target_url": "https://example.com/a", "expires_at": "2030-01-01T00:00:00Z" }),
        json!({ "target_url": "https://example.com/a", "max_clicks": 5 }),
        json!({ "target_url": "https://example.com/a", "tags": ["blocked"] }),
    ] {
        let response = create_link(&app, &member_cookie, body.clone()).await;
        assert_eq!(
            response.status,
            StatusCode::FORBIDDEN,
            "must be blocked: {body}"
        );
    }

    let plain = create_link(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/plain", "tags": [] }),
    )
    .await;
    assert_eq!(plain.status, StatusCode::CREATED, "{}", plain.body);
    let url_id = plain.json()["id"].as_i64().expect("id");

    for body in [
        json!({ "slug": "another" }),
        json!({ "password": "secret1" }),
        json!({ "expires_at": "2030-01-01T00:00:00Z" }),
        json!({ "max_clicks": 5 }),
        json!({ "tags": ["blocked"] }),
    ] {
        let response = app
            .send(patch_json_as(
                &format!("/api/v1/urls/{url_id}"),
                body.clone(),
                &member_cookie,
            ))
            .await;
        assert_eq!(
            response.status,
            StatusCode::FORBIDDEN,
            "must be blocked: {body}"
        );
    }

    // Clearing a value and changing an unrelated field stay allowed.
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({ "password": null }),
            &member_cookie,
        ))
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({ "status": "disabled" }),
            &member_cookie,
        ))
        .await
        .status,
        StatusCode::OK
    );

    // Token creation is a toggle too.
    assert_eq!(
        app.post_json_as("/api/v1/tokens", json!({ "name": "nope" }), &member_cookie)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn domain_selection_is_a_group_permission() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    app.add_domain("go.example.com", "active").await;

    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Default domain only", "allow_domain_selection": false }),
    )
    .await
    .json();
    assert_eq!(group["allow_domain_selection"], false);
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "domain-locked@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["limits"]["allow_domain_selection"], false);

    // Selecting the default domain is always allowed.
    let default_host = app.state.domains.canonical_host().to_string();
    let default_link = create_link(
        &app,
        &member_cookie,
        json!({
            "target_url": "https://example.com/default",
            "domain": default_host
        }),
    )
    .await;
    assert_eq!(
        default_link.status,
        StatusCode::CREATED,
        "{}",
        default_link.body
    );
    let url_id = default_link.json()["id"].as_i64().expect("id");

    // A non-default domain is refused on create and on move.
    let blocked = create_link(
        &app,
        &member_cookie,
        json!({
            "target_url": "https://example.com/extra",
            "domain": "go.example.com"
        }),
    )
    .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    let blocked = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({ "domain": "go.example.com" }),
            &member_cookie,
        ))
        .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);

    // Both forms hide the selector while the group forbids it.
    let dashboard = app.get_as("/app", &member_cookie).await;
    assert!(!dashboard.body.contains("name=\"domain\""));
    let detail = app
        .get_as(&format!("/app/urls/{url_id}"), &member_cookie)
        .await;
    assert!(!detail.body.contains("name=\"domain\""));

    // Enabling the toggle allows the move.
    let unlocked = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Default domain only", "allow_domain_selection": true }),
    )
    .await;
    assert_eq!(unlocked.status, StatusCode::OK, "{}", unlocked.body);
    let moved = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({ "domain": "go.example.com" }),
            &member_cookie,
        ))
        .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
}

#[tokio::test]
async fn group_default_domain_and_allowlist_are_enforced() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let a = app.add_domain("a.example.com", "active").await;
    let b = app.add_domain("b.example.com", "active").await;
    app.add_domain("c.example.com", "active").await;

    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "A and B",
            "default_domain_id": a,
            "restrict_domains": true,
            "domain_ids": [a, b]
        }),
    )
    .await
    .json();
    assert_eq!(group["default_domain_id"], a);
    assert_eq!(group["restrict_domains"], true);
    let mut allowed: Vec<i64> = group["allowed_domain_ids"]
        .as_array()
        .expect("allowed ids")
        .iter()
        .map(|value| value.as_i64().expect("id"))
        .collect();
    allowed.sort_unstable();
    assert_eq!(allowed, vec![a, b]);
    let group_id = group["id"].as_i64().expect("id");

    let (member_cookie, member_id) = member(&app, "group-domains@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    // Omitting `domain` uses the group default, not the site default.
    let default_link = create_link(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/group-default" }),
    )
    .await;
    assert_eq!(
        default_link.status,
        StatusCode::CREATED,
        "{}",
        default_link.body
    );
    assert_eq!(default_link.json()["domain"], "a.example.com");

    for (domain, expected) in [
        ("b.example.com", StatusCode::CREATED),
        ("c.example.com", StatusCode::FORBIDDEN),
        (app.state.domains.canonical_host(), StatusCode::FORBIDDEN),
    ] {
        let response = create_link(
            &app,
            &member_cookie,
            json!({ "target_url": "https://example.com/choice", "domain": domain }),
        )
        .await;
        assert_eq!(
            response.status, expected,
            "domain {domain}: {}",
            response.body
        );
    }

    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["limits"]["default_domain"], "a.example.com");
    let mut allowed_hosts: Vec<&str> = me["limits"]["allowed_domains"]
        .as_array()
        .expect("allowed domains")
        .iter()
        .map(|value| value.as_str().expect("host"))
        .collect();
    allowed_hosts.sort_unstable();
    assert_eq!(allowed_hosts, vec!["a.example.com", "b.example.com"]);

    let options = app.get_as("/api/v1/domains", &member_cookie).await.json();
    let mut hosts: Vec<&str> = options
        .as_array()
        .expect("domain list")
        .iter()
        .map(|value| value["hostname"].as_str().expect("host"))
        .collect();
    hosts.sort_unstable();
    assert_eq!(hosts, vec!["a.example.com", "b.example.com"]);
    let a_option = options
        .as_array()
        .expect("domain list")
        .iter()
        .find(|value| value["hostname"] == "a.example.com")
        .expect("a option");
    assert_eq!(a_option["is_default"], true);
    assert_eq!(a_option["id"], a);

    let dashboard = app.get_as("/app", &member_cookie).await;
    assert!(dashboard.body.contains("name=\"domain\""));
    assert!(dashboard.body.contains("a.example.com"));
    assert!(dashboard.body.contains("b.example.com"));
    assert!(!dashboard.body.contains("c.example.com"));

    // Turning restriction off re-opens every active domain.
    let opened = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "A and B", "restrict_domains": false }),
    )
    .await;
    assert_eq!(opened.status, StatusCode::OK, "{}", opened.body);
    assert_eq!(
        create_link(
            &app,
            &member_cookie,
            json!({ "target_url": "https://example.com/c", "domain": "c.example.com" }),
        )
        .await
        .status,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn an_empty_restricted_allowlist_allows_only_the_default() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let a = app.add_domain("a.example.com", "active").await;
    app.add_domain("b.example.com", "active").await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Only default",
            "default_domain_id": a,
            "restrict_domains": true
        }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    assert_eq!(group["allowed_domain_ids"], json!([]));
    let (member_cookie, member_id) = member(&app, "empty-allowlist@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    let default_link = create_link(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/default-only" }),
    )
    .await;
    assert_eq!(
        default_link.status,
        StatusCode::CREATED,
        "{}",
        default_link.body
    );
    assert_eq!(default_link.json()["domain"], "a.example.com");
    assert_eq!(
        create_link(
            &app,
            &member_cookie,
            json!({ "target_url": "https://example.com/b", "domain": "b.example.com" }),
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        create_link(
            &app,
            &member_cookie,
            json!({
                "target_url": "https://example.com/site",
                "domain": app.state.domains.canonical_host()
            }),
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_disabled_group_default_falls_back_to_the_site_default() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let b = app.add_domain("b.example.com", "active").await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Fallback", "default_domain_id": b }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "domain-fallback@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    sqlx::query("UPDATE domains SET status = 'disabled' WHERE id = ?")
        .bind(b)
        .execute(&app.state.db)
        .await
        .expect("disable domain");
    app.reload_domains().await;

    let page = app
        .get_as(&format!("/app/admin/groups/{group_id}/edit"), &admin_cookie)
        .await;
    assert!(page.body.contains(&format!("value=\"{b}\" selected")));

    // A name-only PUT must keep the inactive stored default rather than 422.
    let name_only =
        update_group(&app, &admin_cookie, group_id, json!({ "name": "Fallback" })).await;
    assert_eq!(name_only.status, StatusCode::OK, "{}", name_only.body);
    assert_eq!(name_only.json()["default_domain_id"], b);

    // Explicitly sending the unchanged inactive value is also allowed.
    let unchanged = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Fallback", "default_domain_id": b }),
    )
    .await;
    assert_eq!(unchanged.status, StatusCode::OK, "{}", unchanged.body);
    assert_eq!(unchanged.json()["default_domain_id"], b);

    let canonical = app.state.domains.canonical_host().to_string();
    let options = app.get_as("/api/v1/domains", &member_cookie).await.json();
    let entries = options.as_array().expect("domain list");
    assert!(entries
        .iter()
        .any(|entry| { entry["hostname"] == canonical && entry["is_default"] == true }));
    assert!(!entries
        .iter()
        .any(|entry| entry["hostname"] == "b.example.com"));

    // Saving the form must accept the unchanged disabled default.
    let body = format!(
        "name=Fallback&max_urls=&max_api_tokens=&allow_custom_slug=on&allow_password=on\
         &allow_expiry=on&allow_max_clicks=on&allow_tags=on&allow_api_tokens=on\
         &allow_domain_selection=on&default_domain_id={b}"
    );
    let response = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            &body,
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let list = groups(&app, &admin_cookie).await;
    assert_eq!(group_named(&list, "Fallback")["default_domain_id"], b);

    let created = create_link(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/fallback" }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.json()["domain"], canonical);

    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["limits"]["default_domain"], canonical);

    sqlx::query("DELETE FROM domains WHERE id = ?")
        .bind(b)
        .execute(&app.state.db)
        .await
        .expect("delete group default");
    let list = groups(&app, &admin_cookie).await;
    assert_eq!(
        group_named(&list, "Fallback")["default_domain_id"],
        Value::Null
    );
}

#[tokio::test]
async fn selection_off_keeps_the_current_domain_but_blocks_moves() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let a = app.add_domain("a.example.com", "active").await;
    let b = app.add_domain("b.example.com", "active").await;
    app.add_domain("c.example.com", "active").await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Locked choice",
            "default_domain_id": a,
            "restrict_domains": true,
            "domain_ids": [a, b]
        }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "domain-locked-choice@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    // Put the link on the non-default allowed domain before locking selection.
    let on_b = create_link(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/on-b", "domain": "b.example.com" }),
    )
    .await;
    assert_eq!(on_b.status, StatusCode::CREATED, "{}", on_b.body);
    let url_id = on_b.json()["id"].as_i64().expect("id");

    let locked = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Locked choice", "allow_domain_selection": false }),
    )
    .await;
    assert_eq!(locked.status, StatusCode::OK, "{}", locked.body);

    // Keeping the current domain (even without sending it) is allowed.
    let kept = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({ "title": "Kept" }),
            &member_cookie,
        ))
        .await;
    assert_eq!(kept.status, StatusCode::OK, "{}", kept.body);
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({ "domain": "b.example.com", "title": "Kept again" }),
            &member_cookie,
        ))
        .await
        .status,
        StatusCode::OK
    );
    // A non-default move is refused, moving back to the group default is not.
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({ "domain": "c.example.com" }),
            &member_cookie,
        ))
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({ "domain": "a.example.com" }),
            &member_cookie,
        ))
        .await
        .status,
        StatusCode::OK
    );

    let dashboard = app.get_as("/app", &member_cookie).await;
    assert!(!dashboard.body.contains("name=\"domain\""));
    let detail = app
        .get_as(&format!("/app/urls/{url_id}"), &member_cookie)
        .await;
    assert!(!detail.body.contains("name=\"domain\""));

    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["limits"]["default_domain"], "a.example.com");
    assert_eq!(me["limits"]["allowed_domains"], json!(["a.example.com"]));
    let options = app.get_as("/api/v1/domains", &member_cookie).await.json();
    assert_eq!(
        options,
        json!([{"id": a, "hostname": "a.example.com", "is_default": true}])
    );
}

#[tokio::test]
async fn group_domain_policy_works_through_the_html_form() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let a = app.add_domain("a.example.com", "active").await;
    let b = app.add_domain("b.example.com", "active").await;

    let body = format!(
        "name=Form+Domains&max_urls=&max_api_tokens=&allow_custom_slug=on&allow_password=on\
         &allow_expiry=on&allow_max_clicks=on&allow_tags=on&allow_api_tokens=on\
         &allow_domain_selection=on&default_domain_id={a}&restrict_domains=on\
         &domain_{a}=on&domain_{b}=on"
    );
    let response = app
        .send(post_form_as("/app/admin/groups", &body, &admin_cookie))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);

    let list = groups(&app, &admin_cookie).await;
    let row = group_named(&list, "Form Domains");
    assert_eq!(row["default_domain_id"], a);
    assert_eq!(row["restrict_domains"], true);
    let mut ids: Vec<i64> = row["allowed_domain_ids"]
        .as_array()
        .expect("ids")
        .iter()
        .map(|value| value.as_i64().expect("id"))
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![a, b]);
    let group_id = row["id"].as_i64().expect("id");

    let page = app
        .get_as(&format!("/app/admin/groups/{group_id}/edit"), &admin_cookie)
        .await;
    assert!(page.body.contains(&format!("value=\"{a}\" selected")));
    assert!(page
        .body
        .contains(&format!("name=\"domain_{a}\" value=\"on\" checked")));
    assert!(page
        .body
        .contains(&format!("name=\"domain_{b}\" value=\"on\" checked")));

    // Changing the active default through the form persists it.
    let body = format!(
        "name=Form+Domains&max_urls=&max_api_tokens=&allow_custom_slug=on&allow_password=on\
         &allow_expiry=on&allow_max_clicks=on&allow_tags=on&allow_api_tokens=on\
         &allow_domain_selection=on&default_domain_id={b}&restrict_domains=on\
         &domain_{a}=on&domain_{b}=on"
    );
    let response = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            &body,
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let list = groups(&app, &admin_cookie).await;
    assert_eq!(group_named(&list, "Form Domains")["default_domain_id"], b);

    // Clearing the default and restriction through the form removes the rows.
    let body = "name=Form+Domains&max_urls=&max_api_tokens=&allow_custom_slug=on\
                &allow_password=on&allow_expiry=on&allow_max_clicks=on&allow_tags=on\
                &allow_api_tokens=on&allow_domain_selection=on&default_domain_id=";
    let response = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            body,
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);

    let list = groups(&app, &admin_cookie).await;
    let row = group_named(&list, "Form Domains");
    assert_eq!(row["default_domain_id"], Value::Null);
    assert_eq!(row["restrict_domains"], false);
    assert_eq!(row["allowed_domain_ids"], json!([]));
}

#[tokio::test]
async fn group_domain_policy_radio_drives_the_form() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let a = app.add_domain("a.example.com", "active").await;
    let b = app.add_domain("b.example.com", "active").await;

    // "default": members cannot choose, so no allowlist is needed.
    let body = format!(
        "name=Radio+Scope&max_urls=&max_api_tokens=&domain_policy=default\
         &default_domain_id={a}"
    );
    let response = app
        .send(post_form_as("/app/admin/groups", &body, &admin_cookie))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let list = groups(&app, &admin_cookie).await;
    let row = group_named(&list, "Radio Scope");
    assert_eq!(row["allow_domain_selection"], false);
    assert_eq!(row["restrict_domains"], false);
    assert_eq!(row["allowed_domain_ids"], json!([]));
    let group_id = row["id"].as_i64().expect("id");

    // The edit page preselects the radio matching the stored booleans. The new
    // group form never checks "default", so this can only come from that group.
    let page = app
        .get_as(&format!("/app/admin/groups/{group_id}/edit"), &admin_cookie)
        .await;
    assert!(page
        .body
        .contains("name=\"domain_policy\" value=\"default\" checked"));

    // "list": selection on, narrowed to the checked domains.
    let body = format!(
        "name=Radio+Scope&max_urls=&max_api_tokens=&domain_policy=list\
         &default_domain_id={a}&domain_{a}=on&domain_{b}=on"
    );
    let response = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            &body,
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let list = groups(&app, &admin_cookie).await;
    let row = group_named(&list, "Radio Scope");
    assert_eq!(row["allow_domain_selection"], true);
    assert_eq!(row["restrict_domains"], true);
    let mut ids: Vec<i64> = row["allowed_domain_ids"]
        .as_array()
        .expect("ids")
        .iter()
        .map(|value| value.as_i64().expect("id"))
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![a, b]);

    // "all": the restriction is off again, but the submitted allowlist stays
    // stored so switching back to "list" does not lose the domains.
    let body = format!(
        "name=Radio+Scope&max_urls=&max_api_tokens=&domain_policy=all\
         &default_domain_id={a}&domain_{a}=on&domain_{b}=on"
    );
    let response = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            &body,
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let list = groups(&app, &admin_cookie).await;
    let row = group_named(&list, "Radio Scope");
    assert_eq!(row["allow_domain_selection"], true);
    assert_eq!(row["restrict_domains"], false);
    assert_eq!(row["allowed_domain_ids"].as_array().expect("ids").len(), 2);

    // "default" turns selection off, but it must not rewrite the stored
    // restriction: the allowlist is inert while selection is off and the API
    // keeps reporting the flag, so a page save cannot quietly drop it.
    let body = format!(
        "name=Radio+Scope&max_urls=&max_api_tokens=&domain_policy=list         &default_domain_id={a}&domain_{a}=on&domain_{b}=on"
    );
    let response = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            &body,
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let body = format!(
        "name=Radio+Scope&max_urls=&max_api_tokens=&domain_policy=default         &default_domain_id={a}&domain_{a}=on&domain_{b}=on"
    );
    let response = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            &body,
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let list = groups(&app, &admin_cookie).await;
    let row = group_named(&list, "Radio Scope");
    assert_eq!(row["allow_domain_selection"], false);
    assert_eq!(row["restrict_domains"], true);
    assert_eq!(row["allowed_domain_ids"].as_array().expect("ids").len(), 2);

    // An unknown value is refused instead of silently picking a scope.
    let response = app
        .send(post_form_as(
            "/app/admin/groups",
            "name=Bad+Scope&domain_policy=everything",
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let list = groups(&app, &admin_cookie).await;
    assert!(list["items"]
        .as_array()
        .expect("items")
        .iter()
        .all(|item| item["name"] != "Bad Scope"));
}

#[tokio::test]
async fn invalid_or_inactive_group_domains_are_rejected() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let b = app.add_domain("b.example.com", "active").await;
    sqlx::query("UPDATE domains SET status = 'disabled' WHERE id = ?")
        .bind(b)
        .execute(&app.state.db)
        .await
        .expect("disable domain");
    app.reload_domains().await;

    let bad_default = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Bad default", "default_domain_id": b }),
    )
    .await;
    assert_eq!(bad_default.status, StatusCode::UNPROCESSABLE_ENTITY);

    let unknown = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Unknown domain", "domain_ids": [999_999] }),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);

    // An existing but disabled domain may stay in the allowlist.
    let allowed = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Disabled allow",
            "restrict_domains": true,
            "domain_ids": [b]
        }),
    )
    .await;
    assert_eq!(allowed.status, StatusCode::CREATED, "{}", allowed.body);
    let group_id = allowed.json()["id"].as_i64().expect("id");

    let changed_default = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Disabled allow", "default_domain_id": b }),
    )
    .await;
    assert_eq!(changed_default.status, StatusCode::UNPROCESSABLE_ENTITY);

    let changed_allowlist = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Disabled allow", "domain_ids": [999_999] }),
    )
    .await;
    assert_eq!(changed_allowlist.status, StatusCode::UNPROCESSABLE_ENTITY);

    // A missing group is a 404, not a 422 from domain validation.
    let missing = update_group(
        &app,
        &admin_cookie,
        999_999,
        json!({ "name": "Missing", "default_domain_id": 999_999 }),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn editing_a_group_keeps_a_disabled_allowlist_domain() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let a = app.add_domain("a.example.com", "active").await;
    let b = app.add_domain("b.example.com", "active").await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Keep Disabled",
            "default_domain_id": a,
            "restrict_domains": true,
            "domain_ids": [a, b]
        }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    sqlx::query("UPDATE domains SET status = 'disabled' WHERE id = ?")
        .bind(b)
        .execute(&app.state.db)
        .await
        .expect("disable domain");
    app.reload_domains().await;

    // The disabled stored domain is not selectable, but the edit form keeps it
    // in a hidden field so saving another change does not drop it.
    let page = app
        .get_as(&format!("/app/admin/groups/{group_id}/edit"), &admin_cookie)
        .await;
    assert!(page.body.contains(&format!("name=\"domain_{b}\"")));

    let body = format!(
        "name=Keep+Disabled&max_urls=&max_api_tokens=&allow_custom_slug=on&allow_password=on\
         &allow_expiry=on&allow_max_clicks=on&allow_tags=on&allow_api_tokens=on\
         &allow_domain_selection=on&default_domain_id={a}&restrict_domains=on\
         &domain_{a}=on&domain_{b}=on"
    );
    let response = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            &body,
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);

    let list = groups(&app, &admin_cookie).await;
    let row = group_named(&list, "Keep Disabled");
    let mut ids: Vec<i64> = row["allowed_domain_ids"]
        .as_array()
        .expect("ids")
        .iter()
        .map(|value| value.as_i64().expect("id"))
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![a, b]);
}

#[tokio::test]
async fn group_domain_policy_partial_updates_keep_omitted_fields() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let a = app.add_domain("a.example.com", "active").await;
    let b = app.add_domain("b.example.com", "active").await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Partial domains",
            "default_domain_id": a,
            "restrict_domains": true,
            "domain_ids": [a, b]
        }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");

    // A name-only PUT keeps the domain policy.
    let renamed = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Partial domains renamed" }),
    )
    .await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    let row = renamed.json();
    assert_eq!(row["default_domain_id"], a);
    assert_eq!(row["restrict_domains"], true);
    assert_eq!(row["allowed_domain_ids"], json!([a, b]));

    // `domain_ids: null` clears the allowlist just like `[]`.
    let cleared_null = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Partial domains renamed", "domain_ids": null }),
    )
    .await;
    assert_eq!(cleared_null.status, StatusCode::OK, "{}", cleared_null.body);
    assert_eq!(cleared_null.json()["allowed_domain_ids"], json!([]));

    let restored = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Partial domains renamed", "domain_ids": [a, b] }),
    )
    .await;
    assert_eq!(restored.status, StatusCode::OK, "{}", restored.body);
    assert_eq!(restored.json()["allowed_domain_ids"], json!([a, b]));

    // Explicit null clears the default; [] removes the allowlist rows.
    let cleared = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Partial domains renamed", "default_domain_id": null, "domain_ids": [] }),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.body);
    let row = cleared.json();
    assert_eq!(row["default_domain_id"], Value::Null);
    assert_eq!(row["allowed_domain_ids"], json!([]));
    assert_eq!(row["restrict_domains"], true);
}

#[tokio::test]
async fn token_quota_counts_only_usable_tokens() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "One Token", "max_api_tokens": 1 }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "tokens@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    let first = app
        .post_json_as("/api/v1/tokens", json!({ "name": "one" }), &member_cookie)
        .await;
    assert_eq!(first.status, StatusCode::CREATED, "{}", first.body);
    let token_id = first.json()["api_token"]["id"].as_i64().expect("token id");

    let blocked = app
        .post_json_as("/api/v1/tokens", json!({ "name": "two" }), &member_cookie)
        .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert!(blocked.json()["error"]["message"]
        .as_str()
        .expect("message")
        .contains("limit of 1 API tokens"));

    // Revoking frees the slot immediately: only usable tokens are counted.
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/tokens/{token_id}"),
            &member_cookie
        ))
        .await
        .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.post_json_as("/api/v1/tokens", json!({ "name": "two" }), &member_cookie)
            .await
            .status,
        StatusCode::CREATED
    );

    // The toggle closes the door even below the quota.
    let off = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "One Token", "allow_api_tokens": false }),
    )
    .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.body);
    assert_eq!(
        app.post_json_as("/api/v1/tokens", json!({ "name": "three" }), &member_cookie)
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["group"]["id"], group_id);
    assert_eq!(me["group"]["name"], "One Token");
    assert_eq!(me["user"]["group_id"], group_id);
    assert_eq!(me["limits"]["allow_api_tokens"], false);
    assert_eq!(me["limits"]["max_urls"], 1000);
}

#[tokio::test]
async fn invites_can_carry_a_group_and_fall_back_when_it_is_deleted() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let standard = default_group(&app, &admin_cookie).await;
    let standard_id = standard["id"].as_i64().expect("id");
    let group = create_group(&app, &admin_cookie, json!({ "name": "Invited" }))
        .await
        .json();
    let group_id = group["id"].as_i64().expect("id");

    let invite = app
        .post_json_as(
            "/api/v1/admin/invites",
            json!({ "note": "team", "max_uses": 2, "group_id": group_id }),
            &admin_cookie,
        )
        .await;
    assert_eq!(invite.status, StatusCode::CREATED, "{}", invite.body);
    let invite = invite.json();
    assert_eq!(invite["invite"]["group_id"], group_id);
    assert_eq!(invite["invite"]["group_name"], "Invited");
    let code = invite["code"].as_str().expect("code").to_string();
    let invite_id = invite["invite"]["id"].as_i64().expect("invite id");

    let registered = app
        .post_json(
            "/api/v1/auth/register",
            json!({
                "email": "invited@example.com",
                "password": "supersecret",
                "invite_code": code
            }),
        )
        .await;
    assert_eq!(
        registered.status,
        StatusCode::CREATED,
        "{}",
        registered.body
    );
    assert_eq!(registered.json()["user"]["group_id"], group_id);
    let invited_cookie = registered.session_cookie().expect("cookie");
    let me = app.get_as("/api/v1/auth/me", &invited_cookie).await.json();
    assert_eq!(me["group"]["name"], "Invited");

    // Deleting the group nulls the invite's group through the foreign key.
    let deleted = app
        .send(delete_as(
            &format!("/api/v1/admin/groups/{group_id}?reassign_to={standard_id}"),
            &admin_cookie,
        ))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    let invites = app
        .get_as("/api/v1/admin/invites", &admin_cookie)
        .await
        .json();
    let row = invites["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["id"] == invite_id)
        .expect("invite");
    assert_eq!(row["group_id"], Value::Null);

    // A later redemption lands in the default group.
    let second = app
        .post_json(
            "/api/v1/auth/register",
            json!({
                "email": "invited2@example.com",
                "password": "supersecret",
                "invite_code": code
            }),
        )
        .await;
    assert_eq!(second.status, StatusCode::CREATED, "{}", second.body);
    assert_eq!(second.json()["user"]["group_id"], standard_id);
}

#[tokio::test]
async fn deleting_a_group_reassigns_its_members() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let standard = default_group(&app, &admin_cookie).await;
    let standard_id = standard["id"].as_i64().expect("id");
    let alpha = create_group(&app, &admin_cookie, json!({ "name": "Alpha" }))
        .await
        .json();
    let alpha_id = alpha["id"].as_i64().expect("id");
    let beta = create_group(&app, &admin_cookie, json!({ "name": "Beta" }))
        .await
        .json();
    let beta_id = beta["id"].as_i64().expect("id");

    let (member_cookie, member_id) = member(&app, "moved@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, alpha_id)
            .await
            .status,
        StatusCode::OK
    );

    // The default group is protected, and every other delete needs a target.
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/groups/{standard_id}?reassign_to={alpha_id}"),
            &admin_cookie
        ))
        .await
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/groups/{alpha_id}"),
            &admin_cookie
        ))
        .await
        .status,
        StatusCode::CONFLICT
    );
    // An empty select value means "no target" and is a conflict, not a 400.
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/groups/{alpha_id}?reassign_to="),
            &admin_cookie
        ))
        .await
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/groups/{alpha_id}?reassign_to={alpha_id}"),
            &admin_cookie
        ))
        .await
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        app.send(delete_as(
            &format!("/api/v1/admin/groups/{alpha_id}?reassign_to=999999"),
            &admin_cookie
        ))
        .await
        .status,
        StatusCode::CONFLICT
    );

    let deleted = app
        .send(delete_as(
            &format!("/api/v1/admin/groups/{alpha_id}?reassign_to={beta_id}"),
            &admin_cookie,
        ))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["user"]["group_id"], beta_id);
    assert_eq!(me["group"]["name"], "Beta");
    assert_eq!(
        app.get_as(
            &format!("/api/v1/admin/users?group_id={beta_id}"),
            &admin_cookie
        )
        .await
        .json()["total"],
        1
    );
}

#[tokio::test]
async fn the_default_marker_can_move_and_only_then_can_the_old_one_go() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let standard = default_group(&app, &admin_cookie).await;
    let standard_id = standard["id"].as_i64().expect("id");
    let group = create_group(&app, &admin_cookie, json!({ "name": "Pro" }))
        .await
        .json();
    let group_id = group["id"].as_i64().expect("id");

    let moved = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "Pro", "is_default": true }),
    )
    .await;
    assert_eq!(moved.status, StatusCode::OK, "{}", moved.body);
    assert_eq!(moved.json()["is_default"], true);

    // A second default cannot be created directly.
    assert_eq!(
        create_group(
            &app,
            &admin_cookie,
            json!({ "name": "Third", "is_default": true })
        )
        .await
        .status,
        StatusCode::CONFLICT
    );

    // The old default is now deletable once its members are reassigned.
    let deleted = app
        .send(delete_as(
            &format!("/api/v1/admin/groups/{standard_id}?reassign_to={group_id}"),
            &admin_cookie,
        ))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    let list = groups(&app, &admin_cookie).await;
    assert_eq!(list["items"].as_array().expect("items").len(), 1);
    assert_eq!(list["items"][0]["name"], "Pro");
    assert_eq!(list["items"][0]["is_default"], true);
    assert_eq!(list["items"][0]["members"], 1);
}

#[tokio::test]
async fn group_create_and_edit_share_one_form() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(&app, &admin_cookie, json!({ "name": "Shared Form" }))
        .await
        .json();
    let group_id = group["id"].as_i64().expect("id");

    let new_page = app.get_as("/app/admin/groups/new", &admin_cookie).await;
    let edit_page = app
        .get_as(&format!("/app/admin/groups/{group_id}/edit"), &admin_cookie)
        .await;
    assert_eq!(new_page.status, StatusCode::OK, "{}", new_page.body);
    assert_eq!(edit_page.status, StatusCode::OK, "{}", edit_page.body);

    // Both routes render the same controls, not two drifting forms.
    for needle in [
        "id=\"max_api_tokens\"",
        "name=\"invite_period\"",
        "name=\"allow_custom_slug\"",
        "name=\"domain_policy\"",
        "name=\"redirect_mode_policy\"",
        "name=\"is_default\"",
    ] {
        assert!(new_page.body.contains(needle), "new form lacks {needle}");
        assert!(edit_page.body.contains(needle), "edit form lacks {needle}");
    }

    // The list page is a list: it links to the form instead of embedding it.
    let list_page = app.get_as("/app/admin/groups", &admin_cookie).await;
    assert_eq!(list_page.status, StatusCode::OK, "{}", list_page.body);
    assert!(list_page.body.contains("Shared Form"));
    assert!(!list_page.body.contains("id=\"max_api_tokens\""));
    assert!(list_page
        .body
        .contains(&format!("/app/admin/groups/{group_id}/edit")));

    let missing = app
        .get_as("/app/admin/groups/999999/edit", &admin_cookie)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_group_pages_and_restricted_user_views_render() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;

    let page = app.get_as("/app/admin/groups", &admin_cookie).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("User groups"));
    assert!(page.body.contains("Standard"));
    assert!(page.body.contains("href=\"/app/admin/groups/"));
    assert!(page.body.contains("/edit\""));

    let form_page = app.get_as("/app/admin/groups/new", &admin_cookie).await;
    assert_eq!(form_page.status, StatusCode::OK, "{}", form_page.body);
    assert!(form_page.body.contains("id=\"max_api_tokens\""));
    assert!(form_page.body.contains("name=\"default_domain_id\""));
    assert!(form_page.body.contains("name=\"domain_policy\""));

    // Create a group through the browser form: custom slugs and tags are off.
    let created = app
        .send(post_form_as(
            "/app/admin/groups",
            "name=Restricted&max_urls=2&allow_password=on&allow_expiry=on&allow_max_clicks=on&allow_api_tokens=on",
            &admin_cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::SEE_OTHER, "{}", created.body);
    assert_eq!(
        created.header("location").as_deref(),
        Some("/app/admin/groups?ok=group")
    );

    let list = groups(&app, &admin_cookie).await;
    let restricted = group_named(&list, "Restricted");
    let group_id = restricted["id"].as_i64().expect("id");
    assert_eq!(restricted["max_urls"], 2);
    assert_eq!(restricted["allow_custom_slug"], false);
    assert_eq!(restricted["allow_tags"], false);

    let (member_cookie, member_id) = member(&app, "restricted@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    // Dashboard: the group name, the effective quota and a restriction note.
    let dashboard = app.get_as("/app", &member_cookie).await;
    assert_eq!(dashboard.status, StatusCode::OK, "{}", dashboard.body);
    assert!(dashboard.body.contains("Restricted"));
    assert!(dashboard.body.contains("0 of 2 links used"));
    assert!(dashboard.body.contains("unavailable for your group"));
    assert!(!dashboard.body.contains("id=\"slug\""));
    assert!(!dashboard.body.contains("id=\"tags\""));

    // Account: group name and token usage; the form is still allowed.
    let account = app.get_as("/app/account", &member_cookie).await;
    assert_eq!(account.status, StatusCode::OK, "{}", account.body);
    assert!(account.body.contains("Restricted"));
    assert!(account.body.contains("0 of 20 tokens used"));
    assert!(account.body.contains("action=\"/app/account/tokens\""));

    // Settings: the site-wide token quota is editable.
    let settings = app.get_as("/app/admin/settings", &admin_cookie).await;
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.body);
    assert!(settings.body.contains("id=\"max_api_tokens_per_user\""));

    // Detail page: the slug and tag editors are hidden for this group.
    let link = create_link(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/hidden" }),
    )
    .await;
    assert_eq!(link.status, StatusCode::CREATED, "{}", link.body);
    let url_id = link.json()["id"].as_i64().expect("id");
    let detail = app
        .get_as(&format!("/app/urls/{url_id}"), &member_cookie)
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.body);
    assert!(!detail.body.contains("id=\"slug\""));
    assert!(!detail.body.contains("id=\"tags\""));
    assert!(detail.body.contains("unavailable for your group"));

    // Turning token creation off hides the account form as well.
    let updated = app
        .send(post_form_as(
            &format!("/app/admin/groups/{group_id}"),
            "name=Restricted&max_urls=2&allow_expiry=on&allow_max_clicks=on",
            &admin_cookie,
        ))
        .await;
    assert_eq!(updated.status, StatusCode::SEE_OTHER, "{}", updated.body);
    let account = app.get_as("/app/account", &member_cookie).await;
    assert!(!account.body.contains("action=\"/app/account/tokens\""));
    assert!(account.body.contains("does not allow API tokens"));
}

#[tokio::test]
async fn the_admin_form_can_unassign_a_user() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(&app, &admin_cookie, json!({ "name": "Assigned" }))
        .await
        .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "unassign@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    // The browser form always submits the select, so an empty value clears it.
    let response = app
        .send(post_form_as(
            &format!("/app/admin/users/{member_id}"),
            "role=user&status=active&group_id=",
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER, "{}", response.body);
    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["group"], Value::Null);
    assert_eq!(me["user"]["group_id"], Value::Null);
}

#[tokio::test]
async fn concurrent_token_creations_cannot_exceed_the_quota() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Race", "max_api_tokens": 1 }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "race@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    let (first, second) = tokio::join!(
        app.post_json_as(
            "/api/v1/tokens",
            json!({ "name": "race-a" }),
            &member_cookie
        ),
        app.post_json_as(
            "/api/v1/tokens",
            json!({ "name": "race-b" }),
            &member_cookie
        ),
    );
    let statuses = [first.status, second.status];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "exactly one creation must win: {:?}",
        statuses
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::FORBIDDEN)
            .count(),
        1,
        "the other creation must be refused: {:?}",
        statuses
    );
    assert_eq!(
        app.get_as("/api/v1/tokens", &member_cookie).await.json()["items"]
            .as_array()
            .expect("items")
            .len(),
        1
    );
}

#[tokio::test]
async fn unchanged_blocked_values_are_accepted_on_update() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(&app, &admin_cookie, json!({ "name": "Keep" }))
        .await
        .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "keep@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    let created = create_link(
        &app,
        &member_cookie,
        json!({
            "target_url": "https://example.com/keep",
            "password": "secret12",
            "expires_at": "2030-01-01T00:00:00Z",
            "max_clicks": 50,
            "tags": ["keep"]
        }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let url_id = created.json()["id"].as_i64().expect("id");

    // Disable the features after the link already uses them.
    let locked = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({
            "name": "Keep",
            "allow_password": false,
            "allow_expiry": false,
            "allow_max_clicks": false,
            "allow_tags": false
        }),
    )
    .await;
    assert_eq!(locked.status, StatusCode::OK, "{}", locked.body);

    // Echoing the current values while changing an unrelated field is allowed.
    let unchanged = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({
                "title": "Renamed",
                "expires_at": "2030-01-01T00:00:00Z",
                "max_clicks": 50,
                "tags": ["keep"]
            }),
            &member_cookie,
        ))
        .await;
    assert_eq!(unchanged.status, StatusCode::OK, "{}", unchanged.body);

    for body in [
        json!({ "expires_at": "2031-01-01T00:00:00Z" }),
        json!({ "max_clicks": 51 }),
        json!({ "tags": ["other"] }),
    ] {
        let refused = app
            .send(patch_json_as(
                &format!("/api/v1/urls/{url_id}"),
                body.clone(),
                &member_cookie,
            ))
            .await;
        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "a new value must be refused: {body}"
        );
    }

    // A password cannot be compared with its hash, so resending the same plain
    // text is treated as a new password and refused while passwords are blocked.
    let same_password = app
        .send(patch_json_as(
            &format!("/api/v1/urls/{url_id}"),
            json!({ "password": "secret12" }),
            &member_cookie,
        ))
        .await;
    assert_eq!(same_password.status, StatusCode::FORBIDDEN);

    // Clearing them stays possible.
    for body in [
        json!({ "expires_at": null }),
        json!({ "max_clicks": null }),
        json!({ "tags": [] }),
        json!({ "password": null }),
    ] {
        let cleared = app
            .send(patch_json_as(
                &format!("/api/v1/urls/{url_id}"),
                body.clone(),
                &member_cookie,
            ))
            .await;
        assert_eq!(
            cleared.status,
            StatusCode::OK,
            "clearing must stay allowed: {body} ({})",
            cleared.body
        );
    }
}

#[tokio::test]
async fn concurrent_link_creations_cannot_exceed_the_quota() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Link Race", "max_urls": 1 }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "link-race@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    let (first, second) = tokio::join!(
        create_link(
            &app,
            &member_cookie,
            json!({ "target_url": "https://example.com/race-a" })
        ),
        create_link(
            &app,
            &member_cookie,
            json!({ "target_url": "https://example.com/race-b" })
        ),
    );
    let statuses = [first.status, second.status];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "exactly one creation must win: {:?}",
        statuses
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::FORBIDDEN)
            .count(),
        1,
        "the other creation must be refused: {:?}",
        statuses
    );
    assert_eq!(
        app.get_as("/api/v1/urls", &member_cookie).await.json()["total"],
        1
    );
}

#[tokio::test]
async fn an_invalid_admin_patch_does_not_change_the_group() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(&app, &admin_cookie, json!({ "name": "Atomic" }))
        .await
        .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "atomic@example.com").await;
    let before =
        app.get_as("/api/v1/auth/me", &member_cookie).await.json()["user"]["group_id"].clone();

    let rejected = app
        .send(patch_json_as(
            &format!("/api/v1/admin/users/{member_id}"),
            json!({ "group_id": group_id, "display_name": "x".repeat(100) }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(rejected.status, StatusCode::UNPROCESSABLE_ENTITY);
    let me = app.get_as("/api/v1/auth/me", &member_cookie).await.json();
    assert_eq!(me["user"]["group_id"], before);
    assert_ne!(me["user"]["group_id"], json!(group_id));
}

#[tokio::test]
async fn detail_page_can_clear_tags_when_tags_are_disabled() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(&app, &admin_cookie, json!({ "name": "No Tags" }))
        .await
        .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "no-tags@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    let created = create_link(
        &app,
        &member_cookie,
        json!({ "target_url": "https://example.com/tagged", "tags": ["keep"] }),
    )
    .await
    .json();
    let url_id = created["id"].as_i64().expect("id");

    let locked = update_group(
        &app,
        &admin_cookie,
        group_id,
        json!({ "name": "No Tags", "allow_tags": false }),
    )
    .await;
    assert_eq!(locked.status, StatusCode::OK, "{}", locked.body);

    let detail = app
        .get_as(&format!("/app/urls/{url_id}"), &member_cookie)
        .await;
    assert_eq!(detail.status, StatusCode::OK, "{}", detail.body);
    assert!(detail.body.contains("name=\"clear_tags\""));

    let cleared = app
        .send(post_form_as(
            &format!("/app/urls/{url_id}"),
            "clear_tags=on",
            &member_cookie,
        ))
        .await;
    assert_eq!(cleared.status, StatusCode::SEE_OTHER, "{}", cleared.body);
    let current = app
        .get_as(&format!("/api/v1/urls/{url_id}"), &member_cookie)
        .await
        .json();
    assert_eq!(
        current["tags"].as_array().expect("tags").len(),
        0,
        "tags must be cleared: {current}"
    );
}

#[tokio::test]
async fn account_usage_counts_links_and_all_tokens() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Account Meter", "max_urls": 5 }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "many-tokens@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );
    for path in ["one", "two"] {
        assert_eq!(
            create_link(
                &app,
                &member_cookie,
                json!({ "target_url": format!("https://example.com/{path}") })
            )
            .await
            .status,
            StatusCode::CREATED
        );
    }

    // `db::tokens::list_for_user` caps the table at 100 rows. Insert 101 rows
    // directly so the usage meter has to use the uncapped COUNT.
    let now = urlet::services::time::now_iso();
    for index in 0..101 {
        sqlx::query(
            "INSERT INTO api_tokens (user_id, name, token_prefix, token_hash, scopes, created_at)
             VALUES (?, ?, 'ur_testprefix', ?, '[]', ?)",
        )
        .bind(member_id)
        .bind(format!("token-{index}"))
        .bind(format!("hash-{index}"))
        .bind(&now)
        .execute(&app.state.db)
        .await
        .expect("insert token");
    }

    let account = app.get_as("/app/account", &member_cookie).await;
    assert_eq!(account.status, StatusCode::OK, "{}", account.body);
    assert!(
        account.body.contains("2 of 5 links used"),
        "the link meter must show the effective group quota: {}",
        account.body
    );
    assert!(
        account.body.contains("101 of 20 tokens used"),
        "the token meter must count past the table cap: {}",
        account.body
    );
}

#[tokio::test]
async fn the_database_refuses_to_delete_the_default_group() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let default = default_group(&app, &admin_cookie).await;
    let default_id = default["id"].as_i64().expect("id");
    let other = create_group(&app, &admin_cookie, json!({ "name": "Other" }))
        .await
        .json();
    let other_id = other["id"].as_i64().expect("id");

    // Bypass the service-level check to prove the transaction itself refuses it.
    let deleted = urlet::db::user_groups::delete_with_reassign(&app.state.db, default_id, other_id)
        .await
        .expect("delete call");
    assert!(!deleted, "the default group must not be deleted");
    let list = groups(&app, &admin_cookie).await;
    assert_eq!(group_named(&list, "Standard")["is_default"], true);

    // A non-default group is still deletable through the same function.
    let deleted = urlet::db::user_groups::delete_with_reassign(&app.state.db, other_id, default_id)
        .await
        .expect("delete call");
    assert!(deleted);
}

#[tokio::test]
async fn updating_a_missing_group_does_not_clear_the_default() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let values = urlet::db::user_groups::GroupValues {
        name: "Ghost",
        description: None,
        max_urls: None,
        max_api_tokens: None,
        max_invites: None,
        invite_period: "month",
        allow_invites: false,
        allow_custom_slug: true,
        allow_password: true,
        allow_expiry: true,
        allow_max_clicks: true,
        allow_tags: true,
        allow_api_tokens: true,
        allow_domain_selection: true,
        redirect_mode_policy: "choice",
        default_domain_id: None,
        restrict_domains: false,
    };

    // Moving the marker onto a row that no longer exists must not clear the
    // current default.
    let updated = urlet::db::user_groups::update(&app.state.db, 999_999, &values, true, None)
        .await
        .expect("update call");
    assert!(!updated);
    let list = groups(&app, &admin_cookie).await;
    assert_eq!(group_named(&list, "Standard")["is_default"], true);
}

#[tokio::test]
async fn a_failed_group_assignment_does_not_apply_other_user_changes() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let (_member_cookie, member_id) = member(&app, "atomic-update@example.com").await;
    let before = urlet::db::users::find_by_id(&app.state.db, member_id)
        .await
        .expect("find")
        .expect("member");

    let result = urlet::db::users::apply_admin_update(
        &app.state.db,
        member_id,
        &urlet::db::users::AdminUserUpdate {
            role: Some("admin"),
            status: Some("active"),
            group_id: Some(Some(999_999)),
            display_name: Some(Some("Renamed")),
        },
    )
    .await;
    assert!(result.is_err(), "the foreign key must reject the group");

    let after = urlet::db::users::find_by_id(&app.state.db, member_id)
        .await
        .expect("find")
        .expect("member");
    assert_eq!(after.role, before.role);
    assert_eq!(after.display_name, before.display_name);
    assert_eq!(after.group_id, before.group_id);
    assert_eq!(
        app.get_as("/api/v1/auth/me", &_member_cookie).await.json()["user"]["role"],
        "user"
    );
    let _ = admin_cookie;
}

#[tokio::test]
async fn concurrent_duplicate_group_names_return_a_conflict() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;

    let (first, second) = tokio::join!(
        create_group(&app, &admin_cookie, json!({ "name": "Racer" })),
        create_group(&app, &admin_cookie, json!({ "name": "Racer" })),
    );
    let statuses = [first.status, second.status];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "exactly one creation must win: {:?}",
        statuses
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CONFLICT)
            .count(),
        1,
        "the duplicate must be a conflict, not a 500: {:?}",
        statuses
    );
}

#[tokio::test]
async fn creating_a_user_with_a_missing_group_falls_back_to_the_default() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let default = default_group(&app, &admin_cookie).await;
    let default_id = default["id"].as_i64().expect("id");

    let user_id = urlet::db::users::create(
        &app.state.db,
        &urlet::db::users::NewUser {
            email: "fallback@example.com".into(),
            email_normalized: "fallback@example.com".into(),
            password_hash: "not-a-real-hash".into(),
            display_name: None,
            role: "user".into(),
            group_id: Some(999_999),
        },
    )
    .await
    .expect("create");
    let user = urlet::db::users::find_by_id(&app.state.db, user_id)
        .await
        .expect("find")
        .expect("user");
    assert_eq!(
        user.group_id,
        Some(default_id),
        "a vanished group must fall back to the default"
    );
}

#[tokio::test]
async fn a_group_only_admin_update_preserves_role_and_status() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(&app, &admin_cookie, json!({ "name": "Partial" }))
        .await
        .json();
    let group_id = group["id"].as_i64().expect("id");
    let (_member_cookie, member_id) = member(&app, "partial@example.com").await;

    // Simulate another administrator suspending the account after this request
    // read it; the group-only patch must not restore "active".
    urlet::db::users::set_role_status(&app.state.db, member_id, "user", "suspended")
        .await
        .expect("suspend");
    urlet::db::users::apply_admin_update(
        &app.state.db,
        member_id,
        &urlet::db::users::AdminUserUpdate {
            role: None,
            status: None,
            group_id: Some(Some(group_id)),
            display_name: None,
        },
    )
    .await
    .expect("update");

    let user = urlet::db::users::find_by_id(&app.state.db, member_id)
        .await
        .expect("find")
        .expect("user");
    assert_eq!(user.role, "user");
    assert_eq!(user.status, "suspended");
    assert_eq!(user.group_id, Some(group_id));
}

#[tokio::test]
async fn admin_patch_normalises_role_and_status() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let (_, member_id) = member(&app, "normalise@example.com").await;

    let response = app
        .send(patch_json_as(
            &format!("/api/v1/admin/users/{member_id}"),
            json!({ "role": " ADMIN ", "status": " SUSPENDED " }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json()["role"], "admin");
    assert_eq!(response.json()["status"], "suspended");

    let user = urlet::db::users::find_by_id(&app.state.db, member_id)
        .await
        .expect("find")
        .expect("user");
    assert_eq!(user.role, "admin");
    assert_eq!(user.status, "suspended");
}

#[tokio::test]
async fn group_updates_keep_omitted_fields() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Partial PUT",
            "description": "keep me",
            "max_urls": 5,
            "allow_tags": false
        }),
    )
    .await
    .json();
    let id = group["id"].as_i64().expect("id");

    // A PUT that only renames must not relax the disabled toggle or drop quotas.
    let renamed = update_group(&app, &admin_cookie, id, json!({ "name": "Partial PUT" })).await;
    assert_eq!(renamed.status, StatusCode::OK, "{}", renamed.body);
    let renamed = renamed.json();
    assert_eq!(renamed["description"], "keep me");
    assert_eq!(renamed["max_urls"], 5);
    assert_eq!(renamed["allow_tags"], false);

    // Explicit null inherits the quota; explicit false/true still work.
    let changed = update_group(
        &app,
        &admin_cookie,
        id,
        json!({ "name": "Partial PUT", "max_urls": null, "allow_tags": true }),
    )
    .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.body);
    let changed = changed.json();
    assert_eq!(changed["max_urls"], Value::Null);
    assert_eq!(changed["description"], "keep me");
    assert_eq!(changed["allow_tags"], true);

    // An explicit null description clears it.
    let cleared = update_group(
        &app,
        &admin_cookie,
        id,
        json!({ "name": "Partial PUT", "description": null }),
    )
    .await;
    assert_eq!(cleared.status, StatusCode::OK, "{}", cleared.body);
    assert_eq!(cleared.json()["description"], Value::Null);
}

#[tokio::test]
async fn the_last_active_administrator_cannot_be_demoted_or_deleted() {
    let app = TestApp::spawn().await;
    let (_cookie, admin_id) = admin(&app).await;

    // Demotion is refused inside the UPDATE, not only by the service pre-check.
    let demoted = urlet::db::users::apply_admin_update(
        &app.state.db,
        admin_id,
        &urlet::db::users::AdminUserUpdate {
            role: Some("user"),
            status: None,
            group_id: None,
            display_name: None,
        },
    )
    .await
    .expect("update");
    assert!(!demoted, "the last active administrator must stay admin");
    let user = urlet::db::users::find_by_id(&app.state.db, admin_id)
        .await
        .expect("find")
        .expect("admin");
    assert_eq!(user.role, "admin");

    // Deletion is refused by the same conditional update.
    let deleted = urlet::db::users::soft_delete(&app.state.db, admin_id)
        .await
        .expect("delete");
    assert!(
        !deleted,
        "the last active administrator must not be deleted"
    );
    let user = urlet::db::users::find_by_id(&app.state.db, admin_id)
        .await
        .expect("find")
        .expect("admin");
    assert!(user.deleted_at.is_none());

    // With a second active administrator the demotion is allowed.
    let (_second_cookie, second_id) = member(&app, "second-admin@example.com").await;
    urlet::db::users::set_role_status(&app.state.db, second_id, "admin", "active")
        .await
        .expect("promote");
    let demoted = urlet::db::users::apply_admin_update(
        &app.state.db,
        admin_id,
        &urlet::db::users::AdminUserUpdate {
            role: Some("user"),
            status: None,
            group_id: None,
            display_name: None,
        },
    )
    .await
    .expect("update");
    assert!(demoted, "another active administrator exists");
    let user = urlet::db::users::find_by_id(&app.state.db, admin_id)
        .await
        .expect("find")
        .expect("admin");
    assert_eq!(user.role, "user");
}

#[tokio::test]
async fn group_names_are_escaped_in_admin_pages() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let name = "<img src=x onerror=alert(1)>";
    let created = create_group(&app, &admin_cookie, json!({ "name": name })).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);

    for uri in [
        "/app/admin/groups",
        "/app/admin/users",
        "/app/admin/invites",
    ] {
        let page = app.get_as(uri, &admin_cookie).await;
        assert_eq!(page.status, StatusCode::OK, "{}", page.body);
        assert!(
            page.body.contains("&lt;img src=x onerror=alert(1)&gt;"),
            "{uri} must escape the group name"
        );
        assert!(
            !page.body.contains(name),
            "{uri} must not render the raw group name"
        );
    }
}

#[tokio::test]
async fn the_open_registration_form_accepts_an_optional_invite_code() {
    let app = TestApp::spawn().await;
    let page = app.get("/app/register").await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains("name=\"invite_code\""),
        "open registration must offer the optional invite field"
    );
    assert!(page
        .body
        .contains("Optional: if you were given a group invite code"));
}

#[tokio::test]
async fn administrators_follow_group_limits_too() {
    let app = TestApp::spawn().await;
    let (admin_cookie, admin_id) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Admin Limits",
            "max_urls": 1,
            "allow_custom_slug": false,
            "allow_api_tokens": false
        }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    assert_eq!(
        assign(&app, &admin_cookie, admin_id, group_id).await.status,
        StatusCode::OK
    );

    // A toggle blocks the admin exactly like a normal user.
    assert_eq!(
        create_link(
            &app,
            &admin_cookie,
            json!({ "target_url": "https://example.com/admin-slug", "slug": "admin-slug" })
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        app.post_json_as(
            "/api/v1/tokens",
            json!({ "name": "admin-token" }),
            &admin_cookie
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );

    // The quota applies to the administrator as well.
    assert_eq!(
        create_link(
            &app,
            &admin_cookie,
            json!({ "target_url": "https://example.com/admin-one" })
        )
        .await
        .status,
        StatusCode::CREATED
    );
    assert_eq!(
        create_link(
            &app,
            &admin_cookie,
            json!({ "target_url": "https://example.com/admin-two" })
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn admin_forms_move_the_default_and_create_a_group_invite() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Form Group", "max_urls": 5 }),
    )
    .await
    .json();
    let id = group["id"].as_i64().expect("id");

    let moved = app
        .send(post_form_as(
            &format!("/app/admin/groups/{id}"),
            "name=Form+Group&max_urls=5&allow_custom_slug=on&allow_password=on&allow_expiry=on\
             &allow_max_clicks=on&allow_tags=on&allow_api_tokens=on&is_default=on",
            &admin_cookie,
        ))
        .await;
    assert_eq!(moved.status, StatusCode::SEE_OTHER, "{}", moved.body);
    let list = groups(&app, &admin_cookie).await;
    assert_eq!(list["items"][0]["name"], "Form Group");
    assert_eq!(list["items"][0]["is_default"], true);

    let invited = app
        .send(post_form_as(
            "/app/admin/invites",
            &format!("note=form&max_uses=1&group_id={id}"),
            &admin_cookie,
        ))
        .await;
    assert_eq!(invited.status, StatusCode::OK, "{}", invited.body);
    let invites = app
        .get_as("/api/v1/admin/invites", &admin_cookie)
        .await
        .json();
    let row = invites["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["group_id"] == id)
        .expect("invite with a group");
    assert_eq!(row["group_name"], "Form Group");

    let invalid = app
        .send(post_form_as(
            "/app/admin/invites",
            "group_id=abc",
            &admin_cookie,
        ))
        .await;
    assert_eq!(invalid.status, StatusCode::SEE_OTHER);
    assert_eq!(
        invalid.header("location").as_deref(),
        Some("/app/admin/invites?err=invalid")
    );
}

#[tokio::test]
async fn quota_flashes_distinguish_tokens_from_links() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Flash", "max_api_tokens": 1, "allow_max_clicks": false }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (member_cookie, member_id) = member(&app, "flash@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    assert_eq!(
        app.post_json_as("/api/v1/tokens", json!({ "name": "one" }), &member_cookie)
            .await
            .status,
        StatusCode::CREATED
    );
    let token = app
        .send(post_form_as(
            "/app/account/tokens",
            "name=second",
            &member_cookie,
        ))
        .await;
    assert_eq!(token.status, StatusCode::SEE_OTHER);
    assert_eq!(
        token.header("location").as_deref(),
        Some("/app/account?err=token_quota")
    );

    // A blocked click cap is a denial, not the link-quota flash.
    let link = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Fx&max_clicks=5",
            &member_cookie,
        ))
        .await;
    assert_eq!(link.status, StatusCode::SEE_OTHER);
    assert_eq!(link.header("location").as_deref(), Some("/app?err=denied"));
}

#[tokio::test]
async fn web_edits_keep_notes_and_an_expired_expiry() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let (member_cookie, member_id) = member(&app, "edit-kept@example.com").await;
    let group = create_group(&app, &admin_cookie, json!({ "name": "Edit Kept" }))
        .await
        .json();
    let group_id = group["id"].as_i64().expect("id");
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    let created = create_link(
        &app,
        &member_cookie,
        json!({
            "target_url": "https://example.com/kept",
            "notes": "keep me",
            "expires_at": "2030-01-01T00:00:00Z"
        }),
    )
    .await
    .json();
    let url_id = created["id"].as_i64().expect("id");

    // Make the link expired, then save it from its own page with the expired
    // instant echoed back. Both the past expiry and the notes must survive.
    sqlx::query("UPDATE urls SET expires_at = '2000-01-01T00:00:00.000Z' WHERE id = ?")
        .bind(url_id)
        .execute(&app.state.db)
        .await
        .expect("expire link");
    let saved = app
        .send(post_form_as(
            &format!("/app/urls/{url_id}"),
            "target_url=https%3A%2F%2Fexample.com%2Fkept&title=Kept\
             &expires_at=2000-01-01T00:00:00.000Z",
            &member_cookie,
        ))
        .await;
    assert_eq!(saved.status, StatusCode::SEE_OTHER, "{}", saved.body);
    assert!(saved
        .header("location")
        .as_deref()
        .unwrap_or_default()
        .contains("ok=updated"));

    let current = app
        .get_as(&format!("/api/v1/urls/{url_id}"), &member_cookie)
        .await
        .json();
    assert_eq!(current["title"], "Kept");
    assert_eq!(current["notes"], "keep me");
    assert_eq!(current["expires_at"], "2000-01-01T00:00:00.000Z");
}

#[tokio::test]
async fn admin_filter_queries_accept_empty_numeric_values() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;

    // The filter forms always submit their selects, including the empty "any"
    // option; an empty numeric query value must mean "not provided".
    for uri in [
        "/app/admin/users?q=&role=&status=&group_id=",
        "/app/admin/users?page=",
        "/app/admin/urls?q=&status=&user_id=",
        "/api/v1/admin/users?group_id=",
        "/api/v1/admin/urls?user_id=",
        "/api/v1/admin/audit?actor=",
    ] {
        let response = app.get_as(uri, &admin_cookie).await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "{uri} failed: {}",
            response.body
        );
    }
}

#[tokio::test]
async fn admin_user_listing_exposes_and_filters_the_group() {
    let app = TestApp::spawn().await;
    let (admin_cookie, _) = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Filtered", "max_urls": 7 }),
    )
    .await
    .json();
    let group_id = group["id"].as_i64().expect("id");
    let (_, member_id) = member(&app, "filtered@example.com").await;
    assert_eq!(
        assign(&app, &admin_cookie, member_id, group_id)
            .await
            .status,
        StatusCode::OK
    );

    let page = app
        .get_as(
            &format!("/api/v1/admin/users?group_id={group_id}"),
            &admin_cookie,
        )
        .await
        .json();
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["group_id"], group_id);
    assert_eq!(page["items"][0]["group_name"], "Filtered");

    // `group_id: null` on PATCH unassigns the user, falling back to site limits.
    let unassigned = app
        .send(patch_json_as(
            &format!("/api/v1/admin/users/{member_id}"),
            json!({ "group_id": null }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(unassigned.status, StatusCode::OK, "{}", unassigned.body);
    assert_eq!(unassigned.json()["group_id"], Value::Null);
}
