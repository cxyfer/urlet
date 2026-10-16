//! Integration tests for the UI language negotiation, cookie and switcher.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::TestApp;

/// GET with an explicit `Accept-Language` header and optional cookies.
fn get_with(uri: &str, accept_language: Option<&str>, cookie: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::HOST, "localhost:8080");
    if let Some(value) = accept_language {
        builder = builder.header(header::ACCEPT_LANGUAGE, value);
    }
    if let Some(value) = cookie {
        builder = builder.header(header::COOKIE, value);
    }
    builder.body(Body::empty()).expect("request")
}

#[tokio::test]
async fn defaults_to_english_without_any_hint() {
    let app = TestApp::spawn().await;
    let page = app.get("/app/login").await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.header("content-language").as_deref(), Some("en"));
    assert!(page.body.contains("<html lang=\"en\">"));
    assert!(page.body.contains("Sign in to manage your short links."));
}

#[tokio::test]
async fn accept_language_selects_traditional_chinese() {
    let app = TestApp::spawn().await;
    let page = app
        .send(get_with(
            "/app/login",
            Some("zh-TW,zh;q=0.9,en;q=0.8"),
            None,
        ))
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.header("content-language").as_deref(), Some("zh-Hant"));
    assert!(page.body.contains("<html lang=\"zh-Hant\">"));
    assert!(page.body.contains("登入以管理你的短網址。"));
    assert!(page.body.contains("電子郵件"));
    // The switcher is visible and marks the active language.
    assert!(page.body.contains("lang-switcher"));
    assert!(page.body.contains(">繁體中文</a>"));
}

#[tokio::test]
async fn accept_language_selects_simplified_chinese() {
    let app = TestApp::spawn().await;
    let page = app
        .send(get_with("/app/login", Some("zh-CN,zh;q=0.9"), None))
        .await;
    assert_eq!(page.header("content-language").as_deref(), Some("zh-Hans"));
    assert!(page.body.contains("登录以管理你的短链接。"));
}

#[tokio::test]
async fn unknown_accept_language_falls_back_to_english() {
    let app = TestApp::spawn().await;
    let page = app
        .send(get_with("/app/login", Some("de-DE,fr;q=0.7"), None))
        .await;
    assert_eq!(page.header("content-language").as_deref(), Some("en"));
    assert!(page.body.contains("Sign in to manage your short links."));
}

#[tokio::test]
async fn cookie_wins_over_accept_language() {
    let app = TestApp::spawn().await;
    let page = app
        .send(get_with(
            "/app/login",
            Some("en-US,en;q=0.9"),
            Some("uwurls_lang=zh-Hant"),
        ))
        .await;
    assert_eq!(page.header("content-language").as_deref(), Some("zh-Hant"));
    assert!(page.body.contains("登入以管理你的短網址。"));

    // The reverse combination is honoured too.
    let page = app
        .send(get_with(
            "/app/login",
            Some("zh-TW"),
            Some("uwurls_lang=en"),
        ))
        .await;
    assert_eq!(page.header("content-language").as_deref(), Some("en"));
    assert!(page.body.contains("Sign in to manage your short links."));
}

#[tokio::test]
async fn language_route_sets_a_cookie_and_redirects() {
    let app = TestApp::spawn().await;
    let response = app.get("/app/lang/zh-Hans").await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location").as_deref(), Some("/app"));

    let cookie = response
        .cookie_named("uwurls_lang")
        .expect("language cookie");
    assert_eq!(cookie, "uwurls_lang=zh-Hans");
    let raw = response
        .headers
        .get_all(header::SET_COOKIE)
        .into_iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("uwurls_lang="))
        .expect("set-cookie header");
    assert!(raw.contains("Path=/"));
    assert!(raw.contains("Max-Age=31536000"));
    assert!(raw.contains("SameSite=Lax"));
    assert!(!raw.contains("HttpOnly"));

    // The cookie is honoured on the next request.
    let page = app.get_as("/app/login", &cookie).await;
    assert_eq!(page.header("content-language").as_deref(), Some("zh-Hans"));
    assert!(page.body.contains("登录以管理你的短链接。"));
}

#[tokio::test]
async fn language_cookie_is_secure_when_configured() {
    let app = TestApp::spawn_with(|config| config.secure_cookies = true).await;
    let response = app.get("/app/lang/en").await;
    let raw = response
        .headers
        .get_all(header::SET_COOKIE)
        .into_iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("uwurls_lang="))
        .expect("set-cookie header");
    assert!(raw.contains("; Secure"));
}

#[tokio::test]
async fn language_route_honours_a_safe_next_target() {
    let app = TestApp::spawn().await;
    let response = app.get("/app/lang/zh-Hant?next=%2Fapp%2Faccount").await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(response.header("location").as_deref(), Some("/app/account"));

    // Off-site targets are ignored rather than followed.
    let response = app
        .get("/app/lang/zh-Hant?next=https%3A%2F%2Fevil.example")
        .await;
    assert_eq!(response.header("location").as_deref(), Some("/app"));

    // `//host` is protocol-relative, and so is `/\host` once a browser applies
    // the WHATWG backslash normalisation. Control characters must not reach the
    // `Location` header either.
    for target in [
        "%2F%2Fevil.example",
        "%2F%5Cevil.example",
        "%5C%2Fevil.example",
        "%2Fapp%0D%0ASet-Cookie%3A+x%3D1",
    ] {
        let response = app.get(&format!("/app/lang/zh-Hant?next={target}")).await;
        assert_eq!(response.status, StatusCode::FOUND, "{target}");
        assert_eq!(
            response.header("location").as_deref(),
            Some("/app"),
            "off-site target {target} must be ignored"
        );
    }
}

#[tokio::test]
async fn unknown_language_codes_are_treated_as_english() {
    let app = TestApp::spawn().await;
    let response = app.get("/app/lang/klingon").await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(
        response.cookie_named("uwurls_lang").as_deref(),
        Some("uwurls_lang=en")
    );
}

#[tokio::test]
async fn html_error_pages_are_translated() {
    let app = TestApp::spawn().await;
    let response = app
        .send(get_with("/no/such/page", Some("zh-TW"), None))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert!(response.body.contains("找不到"));
    assert!(!response.body.contains("Not found"));

    let response = app.get("/no/such/page").await;
    assert!(response.body.contains("Not found"));

    // The public link page is translated as well.
    let response = app
        .send(get_with("/missing-slug", Some("zh-TW"), None))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert!(response.body.contains("找不到連結"));
}

#[tokio::test]
async fn signed_in_pages_are_translated() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("zh@example.com", "supersecret").await;
    let page = app
        .send(get_with(
            "/app",
            None,
            Some(&format!("{cookie}; uwurls_lang=zh-Hant")),
        ))
        .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("你的連結"));
    assert!(page.body.contains("新增短網址"));
    assert!(page.body.contains("登出"));
    assert!(!page.body.contains("Your links"));
    // Signed-in pages show the switcher in the top bar, exactly once.
    assert_eq!(page.body.matches("data-lang-switcher").count(), 1);
}

#[tokio::test]
async fn account_admin_and_detail_pages_are_translated() {
    let app = TestApp::spawn().await;
    // The first account bootstraps the instance as administrator.
    let (session, _) = app.register("owner@example.com", "supersecret").await;
    let cookie = format!("{session}; uwurls_lang=zh-Hans");

    let account = app
        .send(get_with("/app/account", None, Some(&cookie)))
        .await;
    assert_eq!(account.status, StatusCode::OK);
    assert!(account.body.contains("个人资料"));
    assert!(account.body.contains("API Token"));
    assert!(!account.body.contains("Change password"));

    let admin = app.send(get_with("/app/admin", None, Some(&cookie))).await;
    assert_eq!(admin.status, StatusCode::OK);
    assert!(admin.body.contains("系统概览"));
    assert!(admin.body.contains("数据库大小"));
    assert!(!admin.body.contains("System overview"));

    // A link detail page exercises the chart axis labels and the click log.
    let created = app
        .send(common::post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Fzh&slug=zhlink",
            &session,
        ))
        .await;
    assert_eq!(created.status, StatusCode::SEE_OTHER, "{}", created.body);
    let detail = app.send(get_with("/app/urls/1", None, Some(&cookie))).await;
    assert_eq!(detail.status, StatusCode::OK);
    assert!(detail.body.contains("每日点击数"));
    assert!(detail.body.contains("最近的点击"));
    assert!(!detail.body.contains("Recent clicks"));
}

#[tokio::test]
async fn api_responses_stay_english() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("api@example.com", "supersecret").await;
    let response = app
        .send(get_with(
            "/api/v1/urls/999999",
            Some("zh-TW"),
            Some(&cookie),
        ))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let body = response.json();
    assert_eq!(
        body["error"]["message"],
        "The requested resource does not exist."
    );
}
