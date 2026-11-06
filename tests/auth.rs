//! Integration tests for authentication, sessions, CSRF and rate limiting.

mod common;

use axum::http::StatusCode;
use common::{
    delete_as, get, patch_json_as, post_form, post_json, post_json_as, post_json_origin,
    with_bearer, TestApp,
};

use serde_json::json;
use urlet::services::settings::RegistrationMode;

#[tokio::test]
async fn first_account_becomes_administrator() {
    let app = TestApp::spawn().await;
    let (_, user) = app.register("owner@example.com", "supersecret").await;
    assert_eq!(user["user"]["role"], "admin");
    assert_eq!(user["user"]["email"], "owner@example.com");
    assert!(
        user["user"]["password_hash"].is_null(),
        "hash must never be serialised"
    );
}

#[tokio::test]
async fn second_account_is_a_regular_user() {
    let app = TestApp::spawn().await;
    app.register("owner@example.com", "supersecret").await;
    let (_, user) = app.register("member@example.com", "supersecret").await;
    assert_eq!(user["user"]["role"], "user");
}

#[tokio::test]
async fn register_normalises_email_and_returns_a_session() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("  Mixed@Case.COM ", "supersecret").await;
    assert!(cookie.starts_with("urlet_session="));

    let me = app.get_as("/api/v1/auth/me", &cookie).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.json()["user"]["email"], "Mixed@Case.COM");
}

#[tokio::test]
async fn duplicate_email_is_rejected_case_insensitively() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;
    let response = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "USER@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(response.json()["error"]["code"], "conflict");
}

#[tokio::test]
async fn invalid_registrations_are_rejected() {
    let app = TestApp::spawn().await;

    let bad_email = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "nope", "password": "supersecret" }),
        )
        .await;
    assert_eq!(bad_email.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(bad_email.json()["error"]["details"]["field"], "email");

    let weak = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "a@example.com", "password": "short" }),
        )
        .await;
    assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(weak.json()["error"]["details"]["field"], "password");
}

#[tokio::test]
async fn login_flow_and_wrong_password() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;

    let wrong = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "user@example.com", "password": "nope" }),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        wrong.json()["error"]["message"],
        "invalid email or password"
    );

    let unknown = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "ghost@example.com", "password": "nope" }),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        unknown.json()["error"]["message"],
        "invalid email or password"
    );

    let cookie = app.login("user@example.com", "supersecret").await;
    let me = app.get_as("/api/v1/auth/me", &cookie).await;
    assert_eq!(me.status, StatusCode::OK);
}

#[tokio::test]
async fn anonymous_requests_are_unauthorised() {
    let app = TestApp::spawn().await;
    let response = app.get("/api/v1/auth/me").await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.json()["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn garbage_session_cookie_is_unauthorised() {
    let app = TestApp::spawn().await;
    let response = app
        .get_as("/api/v1/auth/me", "urlet_session=not-a-real-token")
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_revokes_the_session() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let logout = app
        .post_json_as("/api/v1/auth/logout", json!({}), &cookie)
        .await;
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    assert!(
        logout.header("set-cookie").is_some(),
        "session cookie is cleared"
    );

    let me = app.get_as("/api/v1/auth/me", &cookie).await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn changing_the_password_revokes_other_sessions() {
    let app = TestApp::spawn().await;
    let (first, _) = app.register("user@example.com", "supersecret").await;
    let second = app.login("user@example.com", "supersecret").await;

    let response = app
        .post_json_as(
            "/api/v1/auth/password",
            json!({ "current_password": "supersecret", "new_password": "brandnewpass" }),
            &second,
        )
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT, "{}", response.body);

    // The session that changed the password survives, the other one does not.
    assert_eq!(
        app.get_as("/api/v1/auth/me", &second).await.status,
        StatusCode::OK
    );
    assert_eq!(
        app.get_as("/api/v1/auth/me", &first).await.status,
        StatusCode::UNAUTHORIZED
    );

    let fresh = app.login("user@example.com", "brandnewpass").await;
    assert_eq!(
        app.get_as("/api/v1/auth/me", &fresh).await.status,
        StatusCode::OK
    );
    let stale = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "user@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(stale.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn changing_the_password_requires_the_current_one() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let response = app
        .post_json_as(
            "/api/v1/auth/password",
            json!({ "current_password": "wrong", "new_password": "brandnewpass" }),
            &cookie,
        )
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn profile_can_be_updated_and_validated() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let response = app
        .send(patch_json_as(
            "/api/v1/auth/me",
            json!({ "display_name": "Ada" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert_eq!(response.json()["user"]["display_name"], "Ada");

    let too_long = json!({ "display_name": "x".repeat(61) });
    let response = app
        .send(patch_json_as("/api/v1/auth/me", too_long, &cookie))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.json()["error"]["details"]["field"], "display_name");

    // An empty string clears the display name.
    let response = app
        .send(patch_json_as(
            "/api/v1/auth/me",
            json!({ "display_name": "" }),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.json()["user"]["display_name"].is_null());
}

#[tokio::test]
async fn sessions_can_be_listed_and_revoked() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    let other = app.login("user@example.com", "supersecret").await;

    let list = app.get_as("/api/v1/auth/sessions", &cookie).await;
    assert_eq!(list.status, StatusCode::OK);
    // List endpoints share one envelope: {"items": [...]}.
    let sessions = list.json();
    let sessions = sessions["items"].as_array().expect("items");
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions
            .iter()
            .filter(|session| session["current"] == true)
            .count(),
        1
    );

    let other_id = sessions
        .iter()
        .find(|session| session["current"] == false)
        .expect("second session")["id"]
        .as_i64()
        .expect("id");

    let response = app
        .send(delete_as(
            &format!("/api/v1/auth/sessions/{other_id}"),
            &cookie,
        ))
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    assert_eq!(
        app.get_as("/api/v1/auth/me", &other).await.status,
        StatusCode::UNAUTHORIZED
    );

    // Revoking a session that does not belong to the caller is a 404.
    let response = app
        .send(delete_as("/api/v1/auth/sessions/9999", &cookie))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn csrf_blocks_cross_site_cookie_requests() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let blocked = app
        .send(post_json_origin(
            "/api/v1/auth/password",
            json!({ "current_password": "supersecret", "new_password": "brandnewpass" }),
            Some(&cookie),
            Some("https://evil.example"),
        ))
        .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);

    let missing_origin = app
        .send(post_json_origin(
            "/api/v1/auth/password",
            json!({ "current_password": "supersecret", "new_password": "brandnewpass" }),
            Some(&cookie),
            None,
        ))
        .await;
    assert_eq!(missing_origin.status, StatusCode::FORBIDDEN);

    let allowed = app
        .send(post_json_origin(
            "/api/v1/auth/password",
            json!({ "current_password": "supersecret", "new_password": "brandnewpass" }),
            Some(&cookie),
            Some("http://localhost:8080"),
        ))
        .await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT, "{}", allowed.body);
}

#[tokio::test]
async fn csrf_blocks_cross_site_form_posts_without_a_session() {
    let app = TestApp::spawn().await;
    let blocked = app
        .send(post_form(
            "/api/v1/auth/login",
            "email=user@example.com&password=supersecret",
            None,
            Some("https://evil.example"),
        ))
        .await;
    assert_eq!(blocked.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn csrf_lets_bearer_and_plain_json_through() {
    let app = TestApp::spawn().await;
    // No cookie, JSON body, no origin: allowed (cannot be sent cross-site).
    let response = app
        .send(post_json(
            "/api/v1/auth/login",
            json!({ "email": "nobody@example.com", "password": "supersecret" }),
        ))
        .await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_is_rate_limited() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;

    let mut limited = false;
    for _ in 0..15 {
        let response = app
            .post_json(
                "/api/v1/auth/login",
                json!({ "email": "user@example.com", "password": "wrong" }),
            )
            .await;
        if response.status == StatusCode::TOO_MANY_REQUESTS {
            limited = true;
            assert_eq!(response.json()["error"]["code"], "rate_limited");
            assert!(response.header("retry-after").is_some());
            break;
        }
    }
    assert!(
        limited,
        "login should be rate limited after repeated failures"
    );
}

#[tokio::test]
async fn suspended_accounts_cannot_use_existing_sessions() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;
    assert_eq!(
        app.get_as("/api/v1/auth/me", &cookie).await.status,
        StatusCode::OK
    );

    sqlx::query("UPDATE users SET status = 'suspended' WHERE email_normalized = ?")
        .bind("user@example.com")
        .execute(&app.state.db)
        .await
        .expect("suspend");

    assert_eq!(
        app.get_as("/api/v1/auth/me", &cookie).await.status,
        StatusCode::UNAUTHORIZED
    );
    let login = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "user@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(login.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn registration_can_be_closed() {
    let app = TestApp::spawn().await;
    app.state
        .settings
        .update(&app.state.db, "registration_mode", "closed", None)
        .await
        .expect("update setting");

    let response = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "a@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invite_only_registration_requires_a_valid_code() {
    let app = TestApp::spawn().await;
    app.state
        .settings
        .update(&app.state.db, "registration_mode", "invite", None)
        .await
        .expect("update setting");
    assert_eq!(
        app.state.settings.registration_mode(),
        RegistrationMode::Invite
    );

    let without_code = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "a@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(without_code.status, StatusCode::FORBIDDEN);

    let bad_code = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "a@example.com", "password": "supersecret", "invite_code": "NOPE" }),
        )
        .await;
    assert_eq!(bad_code.status, StatusCode::FORBIDDEN);

    // Create a single-use invite directly in the database.
    let code = "ABCD2345";
    let mut conn = app.state.db.acquire().await.expect("connection");
    urlet::db::invites::create(
        &mut conn,
        &urlet::db::invites::NewInvite {
            code_hash: &urlet::services::secret::sha256_hex(code),
            code_prefix: code,
            code_sealed: None,
            note: Some("test"),
            group_id: None,
            created_by: None,
            max_uses: 1,
            expires_at: None,
        },
    )
    .await
    .expect("invite");

    let accepted = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "a@example.com", "password": "supersecret", "invite_code": code }),
        )
        .await;
    assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.body);

    let reused = app
        .post_json(
            "/api/v1/auth/register",
            json!({ "email": "b@example.com", "password": "supersecret", "invite_code": code }),
        )
        .await;
    assert_eq!(
        reused.status,
        StatusCode::FORBIDDEN,
        "a single-use invite cannot be reused"
    );
}

#[tokio::test]
async fn bootstrap_administrator_from_configuration() {
    let app = TestApp::spawn_with(|config| {
        config.admin_email = Some("root@example.com".to_string());
        config.admin_password = Some("rootpassword".to_string());
    })
    .await;

    let cookie = app.login("root@example.com", "rootpassword").await;
    let me = app.get_as("/api/v1/auth/me", &cookie).await;
    assert_eq!(me.json()["user"]["role"], "admin");

    // Bootstrapping again must not create a duplicate.
    urlet::server::bootstrap(&app.state)
        .await
        .expect("bootstrap is idempotent");
    let users = urlet::db::users::count(&app.state.db).await.expect("count");
    assert_eq!(users, 1);
}

#[tokio::test]
async fn repeated_failures_lock_the_account_even_across_ips() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;

    // The in-memory limiter is reset between "hosts" to simulate a distributed
    // attempt; the database backed lockout still has to stop it.
    let mut locked = false;
    for _ in 0..40 {
        app.state.limiter.reset();
        let response = app
            .post_json(
                "/api/v1/auth/login",
                json!({ "email": "user@example.com", "password": "wrong" }),
            )
            .await;
        if response.status == StatusCode::TOO_MANY_REQUESTS {
            locked = true;
            break;
        }
    }
    assert!(locked, "the account must lock after repeated failures");

    // Even the correct password is refused while the lock is active.
    app.state.limiter.reset();
    let correct = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "user@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(correct.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(correct.header("retry-after").is_some());

    // Other accounts are unaffected.
    app.register("other@example.com", "supersecret").await;
    app.state.limiter.reset();
    let other = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "other@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(other.status, StatusCode::OK);
}

/// 以不同 `X-Real-IP`（`10.{net}.x.y`）同時送出 `attempts` 個錯誤密碼的登入請求，
/// email 依序輪替 `emails`；回傳 (401 數, 429 數)。
async fn concurrent_wrong_logins(
    app: &TestApp,
    emails: &[&str],
    attempts: usize,
    net: u8,
) -> (usize, usize) {
    let mut set = tokio::task::JoinSet::new();
    for index in 0..attempts {
        let router = app.router.clone();
        let mut request = post_json(
            "/api/v1/auth/login",
            json!({ "email": emails[index % emails.len()], "password": "wrong-password" }),
        );
        request
            .headers_mut()
            .insert("x-real-ip", test_ip(net, index).parse().expect("header"));
        // 每個請求各自一個 task，在多執行緒 runtime 上真正並行。
        set.spawn(async move { common::send_router(router, request).await.status });
    }
    let (mut unauthorized, mut limited) = (0, 0);
    while let Some(status) = set.join_next().await {
        match status.expect("login task") {
            StatusCode::UNAUTHORIZED => unauthorized += 1,
            StatusCode::TOO_MANY_REQUESTS => limited += 1,
            other => panic!("unexpected status {other}"),
        }
    }
    (unauthorized, limited)
}

fn test_ip(net: u8, index: usize) -> String {
    format!("10.{net}.{}.{}", index / 250, index % 250 + 1)
}

/// 從指定 IP 以 JSON API 登入。
async fn login_from(app: &TestApp, email: &str, password: &str, ip: &str) -> common::TestResponse {
    let mut request = post_json(
        "/api/v1/auth/login",
        json!({ "email": email, "password": password }),
    );
    request
        .headers_mut()
        .insert("x-real-ip", ip.parse().expect("header"));
    app.send(request).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_failures_from_many_ips_still_lock_the_account() {
    let app = TestApp::spawn_with(|config| config.trust_proxy = true).await;
    app.register("user@example.com", "supersecret").await;

    let attempts = 100;
    let (unauthorized, limited) =
        concurrent_wrong_logins(&app, &["user@example.com"], attempts, 1).await;
    eprintln!("concurrent wrong logins: {unauthorized} x 401, {limited} x 429");
    // 恰好 30 次被驗證：多一次是越過門檻，少一次是過早鎖定。
    assert_eq!((unauthorized, limited), (30, 70));

    // 鎖定後即使密碼正確也被拒絕。
    let correct = login_from(&app, "user@example.com", "supersecret", "192.0.2.10").await;
    assert_eq!(correct.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(correct.header("retry-after").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn spelling_variants_of_one_email_share_the_lockout() {
    let app = TestApp::spawn_with(|config| config.trust_proxy = true).await;
    app.register("user@example.com", "supersecret").await;

    let variants = [
        "user@example.com",
        " USER@Example.COM ",
        "User@Example.Com",
        "\tuser@EXAMPLE.com\n",
    ];
    let (unauthorized, limited) = concurrent_wrong_logins(&app, &variants, 100, 1).await;
    assert_eq!(
        (unauthorized, limited),
        (30, 70),
        "spelling variants must share one budget"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn the_lockout_does_not_depend_on_the_in_memory_limiter() {
    let app = TestApp::spawn_with(|config| config.trust_proxy = true).await;
    app.register("user@example.com", "supersecret").await;

    for index in 0..29 {
        let response = login_from(
            &app,
            "user@example.com",
            "wrong-password",
            &test_ip(1, index),
        )
        .await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }

    // 清空記憶體限流器，等同所有 bucket 被逐出或服務重啟；額度仍由資料庫判定。
    app.state.limiter.reset();
    let (unauthorized, limited) =
        concurrent_wrong_logins(&app, &["user@example.com"], 100, 2).await;
    assert_eq!(unauthorized + limited, 100);
    assert_eq!(unauthorized, 1, "the window allows 30 guesses in total");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_form_failures_from_many_ips_still_lock_the_account() {
    let app = TestApp::spawn_with(|config| config.trust_proxy = true).await;
    app.register("user@example.com", "supersecret").await;

    let attempts = 100;
    let mut set = tokio::task::JoinSet::new();
    for index in 0..attempts {
        let router = app.router.clone();
        let mut request = post_form(
            "/app/login",
            "email=user%40example.com&password=wrong-password",
            None,
            Some("http://localhost:8080"),
        );
        request
            .headers_mut()
            .insert("x-real-ip", test_ip(3, index).parse().expect("header"));
        set.spawn(async move { common::send_router(router, request).await });
    }
    // 表單登入失敗會重新渲染頁面；被限流則轉址到 `?err=rate_limited`。
    let (mut evaluated, mut limited) = (0, 0);
    while let Some(response) = set.join_next().await {
        let response = response.expect("login task");
        match response.header("location").as_deref() {
            Some("/app/login?err=rate_limited") => limited += 1,
            None if response.status == StatusCode::OK => evaluated += 1,
            other => panic!("unexpected response {} {other:?}", response.status),
        }
    }
    assert_eq!(evaluated + limited, attempts);
    assert!(
        evaluated <= 30,
        "at most 30 guesses per account may be evaluated, got {evaluated}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn failures_below_the_lockout_leave_the_correct_password_working() {
    let app = TestApp::spawn_with(|config| config.trust_proxy = true).await;
    app.register("user@example.com", "supersecret").await;

    let (unauthorized, limited) = concurrent_wrong_logins(&app, &["user@example.com"], 29, 1).await;
    assert_eq!((unauthorized, limited), (29, 0));

    // 第 30 次嘗試仍在額度內，正確密碼可以登入。
    let response = login_from(&app, "user@example.com", "supersecret", "192.0.2.200").await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);

    // 成功的登入不計入失敗：還剩一次猜測，之後才鎖定。
    let response = login_from(&app, "user@example.com", "wrong-password", "192.0.2.201").await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    let response = login_from(&app, "user@example.com", "supersecret", "192.0.2.202").await;
    assert_eq!(response.status, StatusCode::TOO_MANY_REQUESTS);

    // 其他帳號不受影響。
    app.register("other@example.com", "supersecret").await;
    app.login("other@example.com", "supersecret").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_correct_password_among_concurrent_failures_still_signs_in() {
    // 多輪重複，讓排程順序每輪不同。
    for round in 0..5u8 {
        let app = TestApp::spawn_with(|config| config.trust_proxy = true).await;
        app.register("user@example.com", "supersecret").await;

        let mut set = tokio::task::JoinSet::new();
        for index in 0..30 {
            let router = app.router.clone();
            // 第 15 個請求帶正確密碼，其餘 29 個錯誤。
            let correct = index == 15;
            let password = if correct {
                "supersecret"
            } else {
                "wrong-password"
            };
            let mut request = post_json(
                "/api/v1/auth/login",
                json!({ "email": "user@example.com", "password": password }),
            );
            request.headers_mut().insert(
                "x-real-ip",
                test_ip(10 + round, index).parse().expect("header"),
            );
            set.spawn(async move { (correct, common::send_router(router, request).await) });
        }

        let mut unauthorized = 0;
        while let Some(result) = set.join_next().await {
            let (correct, response) = result.expect("login task");
            if correct {
                assert_eq!(
                    response.status,
                    StatusCode::OK,
                    "round {round}: {}",
                    response.body
                );
                assert!(response.session_cookie().is_some(), "round {round}");
            } else {
                assert_eq!(response.status, StatusCode::UNAUTHORIZED, "round {round}");
                unauthorized += 1;
            }
        }
        assert_eq!(unauthorized, 29, "round {round}");
    }
}

/// 移除 `login_attempts`，讓預留寫入必定失敗。
async fn break_login_attempts(app: &TestApp) {
    sqlx::query("DROP TABLE login_attempts")
        .execute(&app.state.db)
        .await
        .expect("drop table");
}

#[tokio::test]
async fn the_lockout_fails_closed_when_attempts_cannot_be_recorded() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;
    break_login_attempts(&app).await;

    // 無法預留嘗試時不可放行，即使密碼正確。
    let response = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "user@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        response.body
    );
    assert!(response.session_cookie().is_none());
}

#[tokio::test]
async fn a_success_that_cannot_be_recorded_opens_no_session() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;

    // 預留的 INSERT 可以成功，但標記成功的 UPDATE 一律失敗。
    sqlx::query(
        "CREATE TRIGGER reject_login_attempt_update BEFORE UPDATE ON login_attempts
         BEGIN SELECT RAISE(ABORT, 'login_attempts is read-only'); END",
    )
    .execute(&app.state.db)
    .await
    .expect("create trigger");

    let response = app
        .post_json(
            "/api/v1/auth/login",
            json!({ "email": "user@example.com", "password": "supersecret" }),
        )
        .await;
    assert_eq!(
        response.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "{}",
        response.body
    );
    assert!(response.cookie_named("urlet_session").is_none());

    // 該次嘗試仍計為失敗。
    let failures: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM login_attempts WHERE success = 0")
        .fetch_one(&app.state.db)
        .await
        .expect("count failures");
    assert_eq!(failures, 1);
}

#[tokio::test]
async fn the_form_lockout_fails_closed_when_attempts_cannot_be_recorded() {
    let app = TestApp::spawn().await;
    app.register("user@example.com", "supersecret").await;
    break_login_attempts(&app).await;

    let response = app
        .send(post_form(
            "/app/login",
            "email=user%40example.com&password=supersecret",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_ne!(response.header("location").as_deref(), Some("/app"));
    assert!(response.session_cookie().is_none(), "{}", response.status);
}

#[tokio::test]
async fn changing_the_password_also_revokes_api_tokens() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    let created = app
        .send(post_json_as(
            "/api/v1/tokens",
            json!({ "name": "ci" }),
            &cookie,
        ))
        .await
        .json();
    let token = created["token"].as_str().expect("token").to_string();

    let mut request = common::get("/api/v1/urls");
    request = with_bearer(request, &token);
    assert_eq!(app.send(request).await.status, StatusCode::OK);

    let changed = app
        .post_json_as(
            "/api/v1/auth/password",
            json!({ "current_password": "supersecret", "new_password": "brandnewpass" }),
            &cookie,
        )
        .await;
    assert_eq!(changed.status, StatusCode::NO_CONTENT, "{}", changed.body);

    let mut stale = common::get("/api/v1/urls");
    stale = with_bearer(stale, &token);
    assert_eq!(app.send(stale).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sessions_slide_while_in_use() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    // Give the session a nearly expired deadline and an old last_seen_at, then
    // verify that using it pushes the expiry out instead of leaving it fixed.
    let nearly_expired = urlet::services::time::iso_in_hours(1);
    sqlx::query("UPDATE sessions SET last_seen_at = ?, expires_at = ?")
        .bind("2020-01-01T00:00:00.000Z")
        .bind(&nearly_expired)
        .execute(&app.state.db)
        .await
        .expect("age the session");

    assert_eq!(
        app.get_as("/api/v1/auth/me", &cookie).await.status,
        StatusCode::OK
    );

    let expires_at: String = sqlx::query_scalar("SELECT expires_at FROM sessions LIMIT 1")
        .fetch_one(&app.state.db)
        .await
        .expect("expiry");
    assert_eq!(expires_at.len(), 24, "canonical storage format");
    assert!(
        expires_at.as_str() > nearly_expired.as_str(),
        "expiry must slide forward: {nearly_expired} -> {expires_at}"
    );
}

#[tokio::test]
async fn health_endpoint_reports_ok() {
    let app = TestApp::spawn().await;
    let response = get("/health");
    let response = app.send(response).await;
    assert_eq!(response.status, StatusCode::OK);
    let body = response.json();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["database"], "ok");
}

#[tokio::test]
async fn security_headers_are_present() {
    let app = TestApp::spawn().await;
    let response = app.get("/health").await;
    assert!(response.header("content-security-policy").is_some());
    assert_eq!(
        response.header("x-content-type-options").as_deref(),
        Some("nosniff")
    );
    assert_eq!(response.header("x-frame-options").as_deref(), Some("DENY"));
    assert!(response.header("x-request-id").is_some());
}
