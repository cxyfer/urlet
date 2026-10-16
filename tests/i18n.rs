//! Integration tests for the UI language negotiation, cookie and switcher.

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::{post_json_as, TestApp};
use serde_json::{json, Value};

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

/// GET with an explicit user agent, used to generate click rows.
fn get_as_agent(uri: &str, user_agent: Option<&str>, referrer: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::HOST, "localhost:8080");
    if let Some(value) = user_agent {
        builder = builder.header(header::USER_AGENT, value);
    }
    if let Some(value) = referrer {
        builder = builder.header(header::REFERER, value);
    }
    builder.body(Body::empty()).expect("request")
}

async fn create_url(app: &TestApp, cookie: &str, payload: Value) -> Value {
    let response = app
        .send(post_json_as("/api/v1/urls", payload, cookie))
        .await;
    assert_eq!(
        response.status,
        StatusCode::CREATED,
        "create failed: {}",
        response.body
    );
    response.json()
}

/// Wait until the detached click writer finished committing.
///
/// `urls.click_count` is updated last, so waiting for it means the click row and
/// the daily rollup are already visible to the page.
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
    // The API always answers in English, so the negotiated language is not a
    // property of the response and must not be advertised.
    assert!(
        response.header("content-language").is_none(),
        "the JSON API must not be tagged with the negotiated language"
    );
    assert!(response.header("vary").is_none());
}

#[tokio::test]
async fn only_html_carries_the_language_marker() {
    let app = TestApp::spawn().await;

    // HTML: the negotiated language and the cache key inputs.
    let page = app.send(get_with("/app/login", Some("zh-TW"), None)).await;
    assert_eq!(page.header("content-language").as_deref(), Some("zh-Hant"));
    assert_eq!(
        page.header("vary").as_deref(),
        Some("Accept-Language, Cookie")
    );

    // Static assets are publicly cacheable for an hour: a per-request language
    // marker would let a CDN hand one visitor's language to another.
    for asset in ["/assets/app.css", "/assets/app.js", "/assets/favicon.svg"] {
        let response = app.send(get_with(asset, Some("zh-TW"), None)).await;
        assert_eq!(response.status, StatusCode::OK, "{asset}");
        assert!(
            response.header("content-language").is_none(),
            "{asset} must not carry Content-Language"
        );
        assert!(
            response.header("vary").is_none(),
            "{asset} must keep its cacheable Vary-free response"
        );
    }

    // A redirect is not language dependent either. `/app` without a session is
    // used rather than `/`: the landing-page branch turns the root into a real
    // page (200), and this assertion is about redirects.
    let redirect = app.send(get_with("/app", Some("zh-TW"), None)).await;
    assert_eq!(redirect.status, StatusCode::SEE_OTHER);
    assert!(redirect.header("content-language").is_none());
}

#[tokio::test]
async fn malformed_quality_weights_do_not_change_the_outcome() {
    let app = TestApp::spawn().await;
    for header in [
        "zh;q=NaN,en;q=1",
        "zh;q=inf,en;q=1",
        "zh;q=-inf,en;q=1",
        "zh;q=abc,en;q=1",
        "zh;q=0,en;q=1",
    ] {
        let page = app.send(get_with("/app/login", Some(header), None)).await;
        assert_eq!(
            page.header("content-language").as_deref(),
            Some("en"),
            "`{header}` must not beat a valid q=1"
        );
    }
}

#[tokio::test]
async fn non_ascii_next_targets_are_percent_encoded() {
    let app = TestApp::spawn().await;
    let response = app
        .get("/app/lang/zh-Hant?next=%2F%E6%97%A5%E6%9C%AC")
        .await;
    assert_eq!(response.status, StatusCode::FOUND);
    assert_eq!(
        response.header("location").as_deref(),
        Some("/%E6%97%A5%E6%9C%AC"),
        "Location must stay a valid URI reference"
    );

    // Plain ASCII targets are untouched.
    let response = app.get("/app/lang/zh-Hant?next=%2Fapp%2Faccount").await;
    assert_eq!(response.header("location").as_deref(), Some("/app/account"));
}

#[tokio::test]
async fn admin_pages_label_the_registration_mode() {
    let app = TestApp::spawn().await;
    // The first account bootstraps the instance as administrator.
    let (session, _) = app.register("mode@example.com", "supersecret").await;
    let cookie = format!("{session}; uwurls_lang=zh-Hant");

    let overview = app.send(get_with("/app/admin", None, Some(&cookie))).await;
    assert_eq!(overview.status, StatusCode::OK);
    // The default mode is `open`; the raw token must not leak into the page.
    assert!(overview.body.contains("開放"), "{}", overview.body);
    assert!(
        !overview.body.contains("<dd>open</dd>"),
        "the raw registration token leaked into the overview"
    );

    let invites = app
        .send(get_with("/app/admin/invites", None, Some(&cookie)))
        .await;
    assert_eq!(invites.status, StatusCode::OK);
    assert!(invites.body.contains("開放"), "{}", invites.body);
    assert!(!invites.body.contains("> open<"));
    assert!(!invites.body.contains("<strong>open</strong>"));

    // English keeps the raw token, so the English UI does not shift.
    let english = app.send(get_with("/app/admin", None, Some(&session))).await;
    assert!(english.body.contains("<dd>open</dd>"));

    // The audit log's actor column is a displayed enum as well. The action and
    // the metadata stay raw: the filter above searches by those values.
    let audit = app
        .send(get_with("/app/admin/audit", None, Some(&cookie)))
        .await;
    assert_eq!(audit.status, StatusCode::OK);
    assert!(audit.body.contains("#1 (管理員)"), "{}", audit.body);
    assert!(!audit.body.contains("#1 (admin)"));
    assert!(audit.body.contains("auth.register"));
}

/// Literals that come from the SQL rollup or the user agent parser and must not
/// survive on a translated page.
const RAW_ANALYTICS_LABELS: &[&str] = &[
    "direct", "Desktop", "Mobile", "Tablet", "Bot", "Unknown", "Other",
];

#[tokio::test]
async fn analytics_labels_are_translated_on_pages_but_raw_in_the_api() {
    let app = TestApp::spawn().await;
    let (session, _) = app.register("stats@example.com", "supersecret").await;
    let url = create_url(
        &app,
        &session,
        json!({ "target_url": "https://example.com/stats", "slug": "langstats" }),
    )
    .await;
    let id = url["id"].as_i64().expect("id");

    // One click per interesting label, including the two fallbacks: a browser
    // with no referrer (`direct`) and a request without a user agent
    // (`Unknown` device, `Other` browser, `Unknown` OS).
    let agents: [Option<&str>; 5] = [
        Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36"),
        None,
        Some("curl/8.4.0"),
        Some("Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) Safari/604.1"),
        Some("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile/15E148 Safari/604.1"),
    ];
    for agent in agents {
        let response = app.send(get_as_agent("/langstats", agent, None)).await;
        assert_eq!(response.status, StatusCode::FOUND, "{agent:?}");
    }
    wait_for_clicks(&app, id, 5).await;

    let cookie = format!("{session}; uwurls_lang=zh-Hant");
    let page = app
        .send(get_with(&format!("/app/urls/{id}"), None, Some(&cookie)))
        .await;
    assert_eq!(page.status, StatusCode::OK);
    for literal in RAW_ANALYTICS_LABELS {
        assert!(
            !page.body.contains(&format!(">{literal}<")),
            "the raw label `{literal}` is still rendered on the translated page"
        );
    }
    // The translated labels are actually there.
    for translated in ["直接", "電腦", "未知", "其他", "機器人", "平板", "行動裝置"]
    {
        assert!(
            page.body.contains(translated),
            "expected the translated label {translated} on the page"
        );
    }
    // Chart range buttons and the dashboard placeholder go through the catalogue.
    assert!(page.body.contains(">30 天</a>"));

    // `/api/v1` keeps the raw labels: the payload is machine readable and stable.
    let stats = app
        .get_as(&format!("/api/v1/urls/{id}/stats?days=7"), &session)
        .await
        .json();
    let labels: Vec<String> = stats["devices"]
        .as_array()
        .expect("devices")
        .iter()
        .map(|entry| entry["label"].as_str().expect("label").to_string())
        .collect();
    assert!(labels.contains(&"Desktop".to_string()), "{labels:?}");
    assert!(labels.contains(&"Unknown".to_string()), "{labels:?}");
    assert!(labels.contains(&"Bot".to_string()), "{labels:?}");
    let referrers: Vec<String> = stats["referrers"]
        .as_array()
        .expect("referrers")
        .iter()
        .map(|entry| entry["label"].as_str().expect("label").to_string())
        .collect();
    assert!(referrers.contains(&"direct".to_string()), "{referrers:?}");
    let browsers: Vec<String> = stats["browsers"]
        .as_array()
        .expect("browsers")
        .iter()
        .map(|entry| entry["label"].as_str().expect("label").to_string())
        .collect();
    assert!(browsers.contains(&"Other".to_string()), "{browsers:?}");

    let clicks = app
        .get_as(&format!("/api/v1/urls/{id}/clicks"), &session)
        .await
        .json();
    let kinds: Vec<String> = clicks["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|entry| {
            entry["device_type"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert!(kinds.contains(&"Desktop".to_string()), "{kinds:?}");

    // The English page renders the same literals as before the change.
    let english = app
        .send(get_with(&format!("/app/urls/{id}"), None, Some(&session)))
        .await;
    assert!(english.body.contains(">direct</span>"), "{}", english.body);
    assert!(english.body.contains(">Desktop</span>"), "{}", english.body);
    assert!(english.body.contains(">Other</span>"), "{}", english.body);
}

#[tokio::test]
async fn dashboard_placeholder_is_translated() {
    let app = TestApp::spawn().await;
    let (session, _) = app.register("ph@example.com", "supersecret").await;
    let english = app.send(get_with("/app", None, Some(&session))).await;
    assert!(english
        .body
        .contains("placeholder=\"https://example.com/very/long/link\""));

    let cookie = format!("{session}; uwurls_lang=zh-Hans");
    let chinese = app.send(get_with("/app", None, Some(&cookie))).await;
    // The example URL is intentionally identical in every language, but it has
    // to come from the catalogue: a mistyped key would print the key itself.
    assert!(chinese
        .body
        .contains("placeholder=\"https://example.com/very/long/link\""));
    assert!(!chinese
        .body
        .contains("dashboard.new.destination_placeholder"));
    assert!(!chinese.body.contains("lang.t("));
}
