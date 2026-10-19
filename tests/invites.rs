//! Integration tests for per-group invite issuance quotas.
//!
//! The quota is charged when an invite code is *issued*, lives in the
//! `invite_issuance` ledger (so revoking does not refund a slot) and resets at
//! the next calendar period boundary.

mod common;

use axum::http::StatusCode;
use common::{delete_as, patch_json_as, put_json_as, TestApp, TestResponse};
use serde_json::{json, Value};

async fn admin(app: &TestApp) -> String {
    let (cookie, _) = app.register("admin@example.com", "supersecret").await;
    cookie
}

async fn create_group(app: &TestApp, cookie: &str, body: Value) -> Value {
    let response = app.post_json_as("/api/v1/admin/groups", body, cookie).await;
    assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    response.json()
}

async fn create_invite(app: &TestApp, cookie: &str, group_id: Option<i64>) -> TestResponse {
    let body = match group_id {
        Some(group_id) => json!({ "group_id": group_id, "max_uses": 1 }),
        None => json!({ "max_uses": 1 }),
    };
    app.post_json_as("/api/v1/admin/invites", body, cookie)
        .await
}

async fn group_in_listing(app: &TestApp, cookie: &str, name: &str) -> Value {
    let list = app.get_as("/api/v1/admin/groups", cookie).await.json();
    list["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("group {name} not in {list}"))
        .clone()
}

async fn set_setting(app: &TestApp, cookie: &str, key: &str, value: &str) {
    let response = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({ key: value }),
            cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
}

#[tokio::test]
async fn a_group_cannot_issue_more_codes_than_its_quota() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let group = create_group(
        &app,
        &cookie,
        json!({ "name": "Limited", "max_invites": 2, "invite_period": "month" }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("id");
    assert_eq!(group["max_invites"], 2);
    assert_eq!(group["invite_period"], "month");
    assert_eq!(group["invites_used"], 0);
    assert_eq!(group["invites_remaining"], 2);

    for _ in 0..2 {
        assert_eq!(
            create_invite(&app, &cookie, Some(group_id)).await.status,
            StatusCode::CREATED
        );
    }

    let blocked = create_invite(&app, &cookie, Some(group_id)).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    let body = blocked.json();
    assert_eq!(body["error"]["code"], "forbidden");
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("message")
            .contains("at most 2 invite codes per month"),
        "{body}"
    );

    let listed = group_in_listing(&app, &cookie, "Limited").await;
    assert_eq!(listed["invites_used"], 2);
    assert_eq!(listed["invites_remaining"], 0);
}

#[tokio::test]
async fn groups_and_the_site_pool_are_counted_separately() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let small = create_group(
        &app,
        &cookie,
        json!({ "name": "Small", "max_invites": 1, "invite_period": "month" }),
    )
    .await["id"]
        .as_i64()
        .expect("id");
    let open = create_group(&app, &cookie, json!({ "name": "Open" })).await["id"]
        .as_i64()
        .expect("id");

    assert_eq!(
        create_invite(&app, &cookie, Some(small)).await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        create_invite(&app, &cookie, Some(small)).await.status,
        StatusCode::FORBIDDEN
    );
    // Another group is unaffected.
    assert_eq!(
        create_invite(&app, &cookie, Some(open)).await.status,
        StatusCode::CREATED
    );
    // And so is the site-wide pool used by invites without a group.
    assert_eq!(
        create_invite(&app, &cookie, None).await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        group_in_listing(&app, &cookie, "Open").await["invites_remaining"],
        Value::Null
    );
}

#[tokio::test]
async fn a_null_group_quota_inherits_the_site_setting() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    set_setting(&app, &cookie, "max_invites_per_period", "1").await;
    set_setting(&app, &cookie, "invite_period", "week").await;

    // NULL inherits the site-wide quota and period.
    let inherited = create_group(
        &app,
        &cookie,
        json!({ "name": "Inherited", "invite_period": "week" }),
    )
    .await;
    assert_eq!(inherited["max_invites"], Value::Null);
    let inherited_id = inherited["id"].as_i64().expect("id");
    assert_eq!(
        create_invite(&app, &cookie, Some(inherited_id))
            .await
            .status,
        StatusCode::CREATED
    );
    let blocked = create_invite(&app, &cookie, Some(inherited_id)).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert!(blocked.json()["error"]["message"]
        .as_str()
        .expect("message")
        .contains("at most 1 invite code per week"));

    // 0 is unlimited, not "inherit".
    let unlimited = create_group(
        &app,
        &cookie,
        json!({ "name": "Unlimited", "max_invites": 0 }),
    )
    .await["id"]
        .as_i64()
        .expect("id");
    for _ in 0..3 {
        assert_eq!(
            create_invite(&app, &cookie, Some(unlimited)).await.status,
            StatusCode::CREATED
        );
    }

    // Invites without a group consume the site-wide quota too.
    assert_eq!(
        create_invite(&app, &cookie, None).await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        create_invite(&app, &cookie, None).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_new_period_frees_the_quota() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let group = create_group(
        &app,
        &cookie,
        json!({ "name": "Daily", "max_invites": 1, "invite_period": "day" }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("id");
    assert_eq!(
        create_invite(&app, &cookie, Some(group_id)).await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        create_invite(&app, &cookie, Some(group_id)).await.status,
        StatusCode::FORBIDDEN
    );

    // Move the charged slot into the past: yesterday's issuance must not count.
    sqlx::query("UPDATE invite_issuance SET created_at = ? WHERE group_id = ?")
        .bind("2020-01-01T00:00:00.000Z")
        .bind(group_id)
        .execute(&app.state.db)
        .await
        .expect("age the issuance row");

    assert_eq!(
        create_invite(&app, &cookie, Some(group_id)).await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        group_in_listing(&app, &cookie, "Daily").await["invites_used"],
        1,
        "only the fresh issuance counts in this period"
    );
}

#[tokio::test]
async fn revoking_a_code_does_not_refund_the_slot() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let group = create_group(&app, &cookie, json!({ "name": "One", "max_invites": 1 })).await;
    let group_id = group["id"].as_i64().expect("id");

    let created = create_invite(&app, &cookie, Some(group_id)).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let invite_id = created.json()["invite"]["id"].as_i64().expect("invite id");

    let revoked = app
        .send(delete_as(
            &format!("/api/v1/admin/invites/{invite_id}"),
            &cookie,
        ))
        .await;
    assert_eq!(revoked.status, StatusCode::NO_CONTENT, "{}", revoked.body);

    // The slot was consumed when the code was issued, so revocation cannot be
    // used to issue unlimited codes.
    assert_eq!(
        create_invite(&app, &cookie, Some(group_id)).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn concurrent_issuance_cannot_exceed_the_quota() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let group = create_group(&app, &cookie, json!({ "name": "Race", "max_invites": 1 })).await;
    let group_id = group["id"].as_i64().expect("id");

    let (left, right) = tokio::join!(
        create_invite(&app, &cookie, Some(group_id)),
        create_invite(&app, &cookie, Some(group_id))
    );
    let statuses = [left.status, right.status];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "{statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::FORBIDDEN)
            .count(),
        1,
        "{statuses:?}"
    );
    assert_eq!(
        group_in_listing(&app, &cookie, "Race").await["invites_used"],
        1
    );
}

#[tokio::test]
async fn the_admin_ui_shows_usage_and_rejects_an_exhausted_group() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let group = create_group(
        &app,
        &cookie,
        json!({ "name": "Campaign", "max_invites": 1, "invite_period": "month" }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("id");

    let page = app.get_as("/app/admin/groups", &cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("0 of 1 per month"), "{}", page.body);

    let first = create_invite(&app, &cookie, Some(group_id)).await;
    assert_eq!(first.status, StatusCode::CREATED);
    let page = app.get_as("/app/admin/groups", &cookie).await;
    assert!(page.body.contains("1 of 1 per month"), "{}", page.body);
    assert!(page.body.contains("used up"), "{}", page.body);

    let invites_page = app.get_as("/app/admin/invites", &cookie).await;
    assert_eq!(invites_page.status, StatusCode::OK);
    assert!(
        invites_page.body.contains("1 of 1 per month — used up"),
        "{}",
        invites_page.body
    );

    // The HTML form surfaces the quota error instead of silently doing nothing.
    let rejected = app
        .send(common::post_form_as(
            "/app/admin/invites",
            &format!("note=&max_uses=1&group_id={group_id}"),
            &cookie,
        ))
        .await;
    assert_eq!(rejected.status, StatusCode::SEE_OTHER);
    let location = rejected.header("location").unwrap_or_default();
    assert!(location.contains("err=invite_quota"), "{location}");
}

#[tokio::test]
async fn group_updates_validate_the_period() {
    let app = TestApp::spawn().await;
    let cookie = admin(&app).await;
    let group = create_group(&app, &cookie, json!({ "name": "Periods" })).await;
    let group_id = group["id"].as_i64().expect("id");
    assert_eq!(group["invite_period"], "month", "the default period");

    let updated = app
        .send(put_json_as(
            &format!("/api/v1/admin/groups/{group_id}"),
            json!({ "name": "Periods", "max_invites": 3, "invite_period": "week" }),
            &cookie,
        ))
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    assert_eq!(updated.json()["max_invites"], 3);
    assert_eq!(updated.json()["invite_period"], "week");

    let invalid = app
        .send(put_json_as(
            &format!("/api/v1/admin/groups/{group_id}"),
            json!({ "name": "Periods", "invite_period": "year" }),
            &cookie,
        ))
        .await;
    assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(invalid.json()["error"]["code"], "validation_error");

    // A partial PUT keeps the stored values.
    let unchanged = app
        .send(put_json_as(
            &format!("/api/v1/admin/groups/{group_id}"),
            json!({ "name": "Periods" }),
            &cookie,
        ))
        .await;
    assert_eq!(unchanged.status, StatusCode::OK, "{}", unchanged.body);
    assert_eq!(unchanged.json()["max_invites"], 3);
    assert_eq!(unchanged.json()["invite_period"], "week");
}

// ---------------------------------------------------------------------------
// Member self-service
// ---------------------------------------------------------------------------

/// Register a member, assign them to `group_id`, and return `(cookie, id)`.
async fn member_in_group(
    app: &TestApp,
    admin_cookie: &str,
    email: &str,
    group_id: i64,
) -> (String, i64) {
    let (cookie, body) = app.register(email, "supersecret").await;
    let id = body["user"]["id"].as_i64().expect("member id");
    let assigned = app
        .send(patch_json_as(
            &format!("/api/v1/admin/users/{id}"),
            json!({ "group_id": group_id }),
            admin_cookie,
        ))
        .await;
    assert_eq!(assigned.status, StatusCode::OK, "{}", assigned.body);
    (cookie, id)
}

async fn member_create(app: &TestApp, cookie: &str, max_uses: Option<i64>) -> TestResponse {
    let body = match max_uses {
        Some(uses) => json!({ "note": "team", "max_uses": uses }),
        None => json!({ "note": "team" }),
    };
    app.post_json_as("/api/v1/invites", body, cookie).await
}

#[tokio::test]
async fn members_can_issue_codes_when_the_group_allows_it() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Inviting",
            "allow_invites": true,
            "max_invites": 2,
            "invite_period": "month"
        }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    assert_eq!(group["allow_invites"], true);

    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "alice@example.com", group_id).await;

    let listing = app.get_as("/api/v1/invites", &member_cookie).await;
    assert_eq!(listing.status, StatusCode::OK, "{}", listing.body);
    let listing = listing.json();
    assert_eq!(listing["quota"]["allowed"], true);
    assert_eq!(listing["quota"]["limit"], 2);
    assert_eq!(listing["quota"]["used"], 0);
    assert_eq!(listing["quota"]["remaining"], 2);
    assert_eq!(listing["items"], json!([]));

    let created = member_create(&app, &member_cookie, Some(2)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    let created = created.json();
    let code = created["code"].as_str().expect("code").to_string();
    assert_eq!(created["invite"]["max_uses"], 2);
    assert_eq!(created["invite"]["group_id"], group_id);
    assert_eq!(created["quota"]["used"], 1);
    assert_eq!(created["quota"]["remaining"], 1);

    // Redeeming the member-issued code joins the issuer's group.
    let registered = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "redeemed@example.com", "password": "supersecret", "invite_code": code }),
        )
        .await;
    assert_eq!(
        registered.status,
        StatusCode::CREATED,
        "{}",
        registered.body
    );
    let redeemed_group: Option<i64> =
        sqlx::query_scalar("SELECT group_id FROM users WHERE email = 'redeemed@example.com'")
            .fetch_one(&app.state.db)
            .await
            .expect("redeemed group");
    assert_eq!(redeemed_group, Some(group_id));

    let listing = app.get_as("/api/v1/invites", &member_cookie).await.json();
    assert_eq!(listing["items"][0]["used_count"], 1);
    assert_eq!(listing["items"][0]["remaining"], 1);

    assert_eq!(
        member_create(&app, &member_cookie, Some(1)).await.status,
        StatusCode::CREATED
    );
    let blocked = member_create(&app, &member_cookie, Some(1)).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert!(blocked.json()["error"]["message"]
        .as_str()
        .expect("message")
        .contains("at most 2 invite codes per month"));
}

#[tokio::test]
async fn members_are_blocked_when_the_group_disallows_invites() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(&app, &admin_cookie, json!({ "name": "No invites" })).await;
    let group_id = group["id"].as_i64().expect("group id");
    assert_eq!(group["allow_invites"], false);

    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "quiet@example.com", group_id).await;
    let listing = app.get_as("/api/v1/invites", &member_cookie).await.json();
    assert_eq!(listing["quota"]["allowed"], false);

    let blocked = member_create(&app, &member_cookie, Some(1)).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert_eq!(
        blocked.json()["error"]["message"],
        "your group does not allow invite codes"
    );
}

#[tokio::test]
async fn admins_and_members_share_the_group_quota() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({
            "name": "Shared budget",
            "allow_invites": true,
            "max_invites": 2,
            "invite_period": "month"
        }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "shared@example.com", group_id).await;

    assert_eq!(
        member_create(&app, &member_cookie, Some(1)).await.status,
        StatusCode::CREATED
    );
    assert_eq!(
        create_invite(&app, &admin_cookie, Some(group_id))
            .await
            .status,
        StatusCode::CREATED
    );
    // The administrator's code consumed the last slot of the same budget.
    let blocked = member_create(&app, &member_cookie, Some(1)).await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
    assert_eq!(
        group_in_listing(&app, &admin_cookie, "Shared budget").await["invites_used"],
        2
    );
}

#[tokio::test]
async fn members_cannot_see_or_revoke_each_others_codes() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Two members", "allow_invites": true, "max_invites": 5 }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let (alice, _) = member_in_group(&app, &admin_cookie, "alice2@example.com", group_id).await;
    let (bob, _) = member_in_group(&app, &admin_cookie, "bob2@example.com", group_id).await;

    let created = member_create(&app, &alice, Some(1)).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let invite_id = created.json()["invite"]["id"].as_i64().expect("invite id");

    let bob_listing = app.get_as("/api/v1/invites", &bob).await.json();
    assert_eq!(bob_listing["items"], json!([]));
    // The shared quota still counts Alice's code for Bob's view.
    assert_eq!(bob_listing["quota"]["used"], 1);

    let stolen = app
        .send(delete_as(&format!("/api/v1/invites/{invite_id}"), &bob))
        .await;
    assert_eq!(stolen.status, StatusCode::NOT_FOUND);
    // The code is still active and revocable by its owner.
    let own = app
        .send(delete_as(&format!("/api/v1/invites/{invite_id}"), &alice))
        .await;
    assert_eq!(own.status, StatusCode::NO_CONTENT, "{}", own.body);
}

#[tokio::test]
async fn member_codes_cap_max_uses() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Capped", "allow_invites": true, "max_invites": 5 }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "capped@example.com", group_id).await;

    let rejected = member_create(&app, &member_cookie, Some(11)).await;
    assert_eq!(rejected.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(rejected.json()["error"]["message"]
        .as_str()
        .expect("message")
        .contains("between 1 and 10"));
    // The rejected request must not consume a slot.
    let listing = app.get_as("/api/v1/invites", &member_cookie).await.json();
    assert_eq!(listing["quota"]["used"], 0);
}

#[tokio::test]
async fn the_member_invite_page_renders_and_revokes() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Invite page", "allow_invites": true, "max_invites": 3, "invite_period": "week" }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "page@example.com", group_id).await;

    let page = app.get_as("/app/invites", &member_cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("0 of 3 per week"), "{}", page.body);
    assert!(
        page.body.contains("action=\"/app/invites\""),
        "{}",
        page.body
    );
    assert!(page.body.contains("name=\"max_uses\""), "{}", page.body);
    assert!(page.body.contains("href=\"/app/invites\""), "{}", page.body);
    assert!(
        !page.body.contains("does not allow members"),
        "{}",
        page.body
    );

    let created = app
        .send(common::post_form_as(
            "/app/invites",
            "note=team&max_uses=2",
            &member_cookie,
        ))
        .await;
    assert_eq!(created.status, StatusCode::OK, "{}", created.body);
    assert!(
        created.body.contains("id=\"new-invite\""),
        "{}",
        created.body
    );
    assert!(created.body.contains("1 of 3 per week"), "{}", created.body);

    let invite_id: i64 = sqlx::query_scalar("SELECT id FROM invite_codes WHERE note = 'team'")
        .fetch_one(&app.state.db)
        .await
        .expect("invite id");
    let revoked = app
        .send(common::post_form_as(
            &format!("/app/invites/{invite_id}/delete"),
            "",
            &member_cookie,
        ))
        .await;
    assert_eq!(revoked.status, StatusCode::SEE_OTHER);
    assert!(revoked
        .header("location")
        .unwrap_or_default()
        .contains("ok=invite_revoked"));
    let status: String = sqlx::query_scalar("SELECT status FROM invite_codes WHERE id = ?")
        .bind(invite_id)
        .fetch_one(&app.state.db)
        .await
        .expect("status");
    assert_eq!(status, "revoked");
}

#[tokio::test]
async fn the_member_page_explains_a_disabled_group() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(&app, &admin_cookie, json!({ "name": "Closed invites" })).await;
    let group_id = group["id"].as_i64().expect("group id");
    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "closed@example.com", group_id).await;

    let page = app.get_as("/app/invites", &member_cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains("Invite codes are not enabled"),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains("action=\"/app/invites\""),
        "{}",
        page.body
    );

    let blocked = app
        .send(common::post_form_as(
            "/app/invites",
            "note=x&max_uses=1",
            &member_cookie,
        ))
        .await;
    assert_eq!(blocked.status, StatusCode::SEE_OTHER);
    assert!(blocked
        .header("location")
        .unwrap_or_default()
        .contains("err=invites_disabled"));
}

#[tokio::test]
async fn member_invites_require_a_session() {
    let app = TestApp::spawn().await;
    assert_eq!(
        app.get("/api/v1/invites").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.post_json("/api/v1/invites", json!({})).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn existing_codes_stay_visible_when_invites_are_disabled() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Revocable", "allow_invites": true, "max_invites": 5 }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "kept@example.com", group_id).await;

    let created = member_create(&app, &member_cookie, Some(1)).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let invite_id = created.json()["invite"]["id"].as_i64().expect("invite id");

    // An administrator turns the toggle off again.
    let disabled = app
        .send(put_json_as(
            &format!("/api/v1/admin/groups/{group_id}"),
            json!({ "name": "Revocable", "allow_invites": false }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(disabled.status, StatusCode::OK, "{}", disabled.body);

    // The creation form disappears, but the existing code stays visible and
    // revocable from the page.
    let page = app.get_as("/app/invites", &member_cookie).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.body.contains("Invite codes are not enabled"),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains("action=\"/app/invites\""),
        "{}",
        page.body
    );
    assert!(page.body.contains("team"), "{}", page.body);
    assert!(
        page.body
            .contains(&format!("/app/invites/{invite_id}/delete")),
        "{}",
        page.body
    );

    let revoked = app
        .send(common::post_form_as(
            &format!("/app/invites/{invite_id}/delete"),
            "",
            &member_cookie,
        ))
        .await;
    assert_eq!(revoked.status, StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn member_codes_are_paginated() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Paged", "allow_invites": true, "max_invites": 0 }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let (member_cookie, member_id) =
        member_in_group(&app, &admin_cookie, "paged@example.com", group_id).await;

    // Seed one older code directly so the page needs a second page without
    // tripping the 20-per-hour burst limit.
    let mut conn = app.state.db.acquire().await.expect("connection");
    urlet::db::invites::create(
        &mut conn,
        &urlet::db::invites::NewInvite {
            code_hash: "seeded-oldest-code",
            code_prefix: "OLD0",
            note: Some("code 0"),
            group_id: Some(group_id),
            created_by: Some(member_id),
            max_uses: 1,
            expires_at: None,
        },
    )
    .await
    .expect("seed oldest code");
    drop(conn);

    // The burst limit is 20 codes, which is exactly what this test needs.
    for index in 1..=20 {
        let response = app
            .post_json_as(
                "/api/v1/invites",
                json!({ "note": format!("code {index}"), "max_uses": 1 }),
                &member_cookie,
            )
            .await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.body);
    }

    let second = app
        .get_as("/api/v1/invites?page=2&per_page=10", &member_cookie)
        .await;
    assert_eq!(second.status, StatusCode::OK, "{}", second.body);
    let second = second.json();
    assert_eq!(second["page"], 2);
    assert_eq!(second["per_page"], 10);
    assert_eq!(second["total"], 21);
    assert_eq!(second["items"].as_array().expect("items").len(), 10);
    assert_eq!(second["items"][0]["note"], "code 10");

    let first_page = app.get_as("/app/invites", &member_cookie).await;
    assert_eq!(first_page.status, StatusCode::OK);
    assert!(
        first_page.body.contains("Page 1 of 2"),
        "{}",
        first_page.body
    );
    assert!(
        first_page.body.contains("/app/invites?page=2"),
        "{}",
        first_page.body
    );
    assert!(!first_page.body.contains("code 0"), "{}", first_page.body);

    let second_page = app.get_as("/app/invites?page=2", &member_cookie).await;
    assert_eq!(second_page.status, StatusCode::OK);
    assert!(second_page.body.contains("code 0"), "{}", second_page.body);
    assert!(
        second_page.body.contains("Page 2 of 2"),
        "{}",
        second_page.body
    );
}

#[tokio::test]
async fn a_rolled_back_issuance_does_not_consume_quota() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Rollback", "allow_invites": true, "max_invites": 1 }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let stored = urlet::db::user_groups::find_by_id(&app.state.db, group_id)
        .await
        .expect("group lookup")
        .expect("group");
    let quota =
        urlet::services::invite_quota::InviteQuota::resolve(&app.state.settings, Some(&stored));

    // Reserving then dropping the transaction (cancellation/crash) must not
    // leave an orphan slot behind.
    let mut tx = app.state.db.begin().await.expect("transaction");
    let reserved = quota
        .reserve(&mut tx, Some(group_id), None, urlet::services::time::now())
        .await
        .expect("reserve");
    assert!(reserved.is_some());
    drop(tx);

    let used = quota
        .used(&app.state.db, Some(group_id), urlet::services::time::now())
        .await
        .expect("usage");
    assert_eq!(used, 0);

    // The slot is still available through the normal path.
    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "rollback@example.com", group_id).await;
    assert_eq!(
        member_create(&app, &member_cookie, Some(1)).await.status,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn member_creation_is_rate_limited() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Burst", "allow_invites": true, "max_invites": 0 }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "burst@example.com", group_id).await;

    for index in 0..20 {
        let response = member_create(&app, &member_cookie, Some(1)).await;
        assert_eq!(
            response.status,
            StatusCode::CREATED,
            "code {index}: {}",
            response.body
        );
    }
    let blocked = member_create(&app, &member_cookie, Some(1)).await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a_partial_group_update_keeps_allow_invites() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Keep toggle", "allow_invites": true }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");

    let updated = app
        .send(put_json_as(
            &format!("/api/v1/admin/groups/{group_id}"),
            json!({ "name": "Still inviting" }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(updated.status, StatusCode::OK, "{}", updated.body);
    assert_eq!(updated.json()["allow_invites"], true);
}

#[tokio::test]
async fn deleting_a_group_removes_its_issuance_rows() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Doomed", "allow_invites": true, "max_invites": 5 }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let (_, member_id) = member_in_group(&app, &admin_cookie, "doomed@example.com", group_id).await;
    let default_id = group_in_listing(&app, &admin_cookie, "Standard").await["id"]
        .as_i64()
        .expect("default id");

    // Issue through the administrator path so the member's group change does
    // not matter for this cascade check.
    assert_eq!(
        create_invite(&app, &admin_cookie, Some(group_id))
            .await
            .status,
        StatusCode::CREATED
    );
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invite_issuance WHERE group_id = ?")
        .bind(group_id)
        .fetch_one(&app.state.db)
        .await
        .expect("issuance count");
    assert_eq!(before, 1);

    let deleted = app
        .send(delete_as(
            &format!("/api/v1/admin/groups/{group_id}?reassign_to={default_id}"),
            &admin_cookie,
        ))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invite_issuance WHERE group_id = ?")
        .bind(group_id)
        .fetch_one(&app.state.db)
        .await
        .expect("issuance count");
    assert_eq!(after, 0);
    let _ = member_id;
}

#[tokio::test]
async fn an_ungrouped_account_uses_the_default_group() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let standard = group_in_listing(&app, &admin_cookie, "Standard").await;
    let default_id = standard["id"].as_i64().expect("default id");
    let enabled = app
        .send(put_json_as(
            &format!("/api/v1/admin/groups/{default_id}"),
            json!({ "name": "Standard", "allow_invites": true, "max_invites": 2 }),
            &admin_cookie,
        ))
        .await;
    assert_eq!(enabled.status, StatusCode::OK, "{}", enabled.body);

    let (member_cookie, member_id) =
        member_in_group(&app, &admin_cookie, "orphan@example.com", default_id).await;
    // Simulate an account whose group link was lost outside the normal paths.
    sqlx::query("UPDATE users SET group_id = NULL WHERE id = ?")
        .bind(member_id)
        .execute(&app.state.db)
        .await
        .expect("detach group");

    let listing = app.get_as("/api/v1/invites", &member_cookie).await.json();
    assert_eq!(listing["quota"]["allowed"], true);
    assert_eq!(listing["quota"]["group_id"], default_id);
    assert_eq!(listing["quota"]["group_name"], "Standard");

    let created = member_create(&app, &member_cookie, Some(1)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
    assert_eq!(created.json()["invite"]["group_id"], default_id);
}

#[tokio::test]
async fn concurrent_member_issuance_cannot_exceed_the_quota() {
    let app = TestApp::spawn().await;
    let admin_cookie = admin(&app).await;
    let group = create_group(
        &app,
        &admin_cookie,
        json!({ "name": "Member race", "allow_invites": true, "max_invites": 1 }),
    )
    .await;
    let group_id = group["id"].as_i64().expect("group id");
    let (member_cookie, _) =
        member_in_group(&app, &admin_cookie, "member-race@example.com", group_id).await;

    let (left, right) = tokio::join!(
        member_create(&app, &member_cookie, Some(1)),
        member_create(&app, &member_cookie, Some(1))
    );
    let statuses = [left.status, right.status];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1,
        "{statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::FORBIDDEN)
            .count(),
        1,
        "{statuses:?}"
    );
}
