//! Integration tests for bot protection (Cloudflare Turnstile / reCAPTCHA).
//!
//! Every test points `captcha_verify_url` at a local mock server: the real
//! Cloudflare and Google endpoints are never contacted.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, State as AxumState};
use axum::http::StatusCode as AxumStatus;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use common::{
    post_form, post_form_as, post_json, post_json_as, put_json_as, TestApp, TestResponse,
};
use serde_json::{json, Value};
use uwurls::error::AppError;

/// Which flows the fake instance protects.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Flows {
    register: bool,
    login: bool,
    create: bool,
}

impl Flows {
    const ALL: Self = Self {
        register: true,
        login: true,
        create: true,
    };
    const LOGIN_ONLY: Self = Self {
        register: false,
        login: true,
        create: false,
    };
}

/// What the fake provider answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reply {
    Success,
    Rejected,
    Status(u16),
    Garbage,
    Slow,
}

#[derive(Clone)]
struct MockState {
    reply: Reply,
    requests: Arc<Mutex<Vec<String>>>,
}

/// A local stand-in for the provider `siteverify` endpoint.
struct MockSiteverify {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl MockSiteverify {
    async fn start(reply: Reply) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let state = MockState {
            reply,
            requests: requests.clone(),
        };
        let app = Router::new()
            .route("/siteverify", post(siteverify))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock");
        let addr = listener.local_addr().expect("mock addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            url: format!("http://{addr}/siteverify"),
            requests,
        }
    }

    /// Form bodies the provider received, in order.
    fn bodies(&self) -> Vec<String> {
        self.requests.lock().expect("mock lock").clone()
    }

    fn hits(&self) -> usize {
        self.bodies().len()
    }
}

async fn siteverify(AxumState(state): AxumState<MockState>, body: Bytes) -> Response {
    state
        .requests
        .lock()
        .expect("mock lock")
        .push(String::from_utf8_lossy(&body).to_string());
    match state.reply {
        Reply::Success => (AxumStatus::OK, r#"{"success":true}"#).into_response(),
        Reply::Rejected => (
            AxumStatus::OK,
            r#"{"success":false,"error-codes":["invalid-input-response"]}"#,
        )
            .into_response(),
        Reply::Garbage => (AxumStatus::OK, "<html>not json</html>").into_response(),
        Reply::Status(code) => (
            AxumStatus::from_u16(code).expect("status"),
            r#"{"success":true}"#,
        )
            .into_response(),
        Reply::Slow => {
            tokio::time::sleep(Duration::from_secs(8)).await;
            (AxumStatus::OK, r#"{"success":true}"#).into_response()
        }
    }
}

/// Instance with bot protection configured through the environment overrides.
///
/// `url` defaults to a dead local port so a test can never reach the real
/// Cloudflare or Google endpoints by accident.
async fn spawn_app(
    provider: &str,
    url: Option<String>,
    flows: Flows,
    trust_proxy: bool,
) -> TestApp {
    let url = url.unwrap_or_else(|| "http://127.0.0.1:1/siteverify".to_string());
    TestApp::spawn_with(move |config| {
        config.captcha_provider_override = Some(provider.to_string());
        config.captcha_site_key_override = Some("site-key".to_string());
        config.captcha_secret_key_override = Some("secret-key".to_string());
        config.captcha_on_register_override = Some(flows.register);
        config.captcha_on_login_override = Some(flows.login);
        config.captcha_on_create_override = Some(flows.create);
        config.captcha_verify_url = Some(url);
        config.trust_proxy = trust_proxy;
    })
    .await
}

/// Instance whose provider answers with `reply`.
async fn app_with(reply: Reply, flows: Flows) -> (TestApp, MockSiteverify) {
    let mock = MockSiteverify::start(reply).await;
    let app = spawn_app("turnstile", Some(mock.url.clone()), flows, false).await;
    (app, mock)
}

/// The settings form as the browser posts it, plus `extra` fields.
fn settings_form(extra: &str) -> String {
    format!(
        "site_name=uwurls&registration_mode=open&default_slug_length=7&max_urls_per_user=1000\
         &default_redirect_type=302&click_retention_days=0&{extra}"
    )
}

fn api_register_body(email: &str, token: Option<&str>) -> Value {
    let mut body = json!({ "email": email, "password": "supersecret" });
    if let Some(token) = token {
        body["captcha_token"] = json!(token);
    }
    body
}

fn api_login_body(email: &str, token: Option<&str>) -> Value {
    let mut body = json!({ "email": email, "password": "supersecret" });
    if let Some(token) = token {
        body["captcha_token"] = json!(token);
    }
    body
}

/// Register over the API and return the session cookie.
async fn api_register(app: &TestApp, email: &str, token: Option<&str>) -> TestResponse {
    app.post_json("/api/v1/auth/register", api_register_body(email, token))
        .await
}

fn assert_captcha_error(response: &TestResponse) {
    assert_eq!(
        response.status,
        AxumStatus::BAD_REQUEST,
        "body: {}",
        response.body
    );
    assert_eq!(response.json()["error"]["code"], "captcha_failed");
    assert_eq!(
        response.json()["error"]["message"],
        uwurls::services::captcha::FAILURE_MESSAGE
    );
}

fn assert_redirect(response: &TestResponse, target: &str) {
    assert_eq!(
        response.status,
        AxumStatus::SEE_OTHER,
        "body: {}",
        response.body
    );
    assert_eq!(response.header("location").as_deref(), Some(target));
}

// ---------------------------------------------------------------- disabled

#[tokio::test]
async fn disabled_by_default_changes_nothing() {
    let app = TestApp::spawn().await;

    // No provider is configured, so no token is expected anywhere.
    let registered = api_register(&app, "default@example.com", None).await;
    assert_eq!(
        registered.status,
        AxumStatus::CREATED,
        "{}",
        registered.body
    );
    let cookie = registered.session_cookie().expect("session cookie");

    let logged_in = app
        .post_json(
            "/api/v1/auth/login",
            api_login_body("default@example.com", None),
        )
        .await;
    assert_eq!(logged_in.status, AxumStatus::OK, "{}", logged_in.body);

    let created = app
        .send(post_json_as(
            "/api/v1/urls",
            json!({ "target_url": "https://example.com/default" }),
            &cookie,
        ))
        .await;
    assert_eq!(created.status, AxumStatus::CREATED, "{}", created.body);

    // The browser forms work without a token too.
    let html_register = app
        .send(post_form(
            "/app/register",
            "email=html-default%40example.com&password=supersecret",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_redirect(&html_register, "/app");

    let html_login = app
        .send(post_form(
            "/app/login",
            "email=default%40example.com&password=supersecret",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_redirect(&html_login, "/app");

    let html_create = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Fform",
            &cookie,
        ))
        .await;
    assert_eq!(html_create.status, AxumStatus::SEE_OTHER);
    assert!(html_create
        .header("location")
        .expect("location")
        .starts_with("/app/urls/"));

    // Nothing renders the provider script or widget while it is off.
    for body in [
        app.get("/app/login").await.body,
        app.get("/app/register").await.body,
        app.get_as("/app", &cookie).await.body,
    ] {
        assert!(!body.contains("cf-turnstile"), "{body}");
        assert!(!body.contains("g-recaptcha"), "{body}");
        assert!(!body.contains("challenges.cloudflare.com"), "{body}");
        assert!(!body.contains("google.com/recaptcha"), "{body}");
    }
}

// ------------------------------------------------------- missing / invalid

#[tokio::test]
async fn missing_token_is_rejected_on_every_guarded_flow() {
    let (app, mock) = app_with(Reply::Success, Flows::ALL).await;

    // A token-less registration never reaches the provider.
    assert_captcha_error(&api_register(&app, "missing@example.com", None).await);
    assert_redirect(
        &app.send(post_form(
            "/app/register",
            "email=missing-html%40example.com&password=supersecret",
            None,
            Some("http://localhost:8080"),
        ))
        .await,
        "/app/register?err=captcha",
    );
    assert_captcha_error(
        &app.post_json(
            "/api/v1/auth/login",
            api_login_body("missing@example.com", None),
        )
        .await,
    );
    assert_redirect(
        &app.send(post_form(
            "/app/login",
            "email=missing%40example.com&password=supersecret",
            None,
            Some("http://localhost:8080"),
        ))
        .await,
        "/app/login?err=captcha",
    );

    // An empty field counts as missing.
    assert_captcha_error(&api_register(&app, "blank@example.com", Some("   ")).await);

    // The redirect target explains the failure in the visitor's language.
    for page in [
        app.get("/app/register?err=captcha").await,
        app.get("/app/login?err=captcha").await,
    ] {
        assert_eq!(page.status, AxumStatus::OK);
        assert!(
            page.body.contains("The bot protection check failed"),
            "{}",
            page.body
        );
    }
    let localised = app
        .send(
            axum::http::Request::builder()
                .method("GET")
                .uri("/app/login?err=captcha")
                .header(axum::http::header::HOST, "localhost:8080")
                .header(axum::http::header::ACCEPT_LANGUAGE, "zh-Hant")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await;
    assert!(
        localised.body.contains("機器人驗證失敗"),
        "{}",
        localised.body
    );

    assert_eq!(
        mock.hits(),
        0,
        "a missing token must not be sent to the provider"
    );
}

#[tokio::test]
async fn invalid_token_is_rejected_on_every_guarded_flow() {
    let (app, mock) = app_with(Reply::Rejected, Flows::ALL).await;

    assert_captcha_error(&api_register(&app, "rejected@example.com", Some("bad")).await);
    assert_redirect(
        &app.send(post_form(
            "/app/register",
            "email=rejected-html%40example.com&password=supersecret\
             &cf-turnstile-response=bad",
            None,
            Some("http://localhost:8080"),
        ))
        .await,
        "/app/register?err=captcha",
    );

    // A valid session is not enough for the creation form either. Creation is
    // the only guarded flow here, so the account can be created without a token.
    let (app_create, mock_create) = app_with(
        Reply::Rejected,
        Flows {
            register: false,
            login: false,
            create: true,
        },
    )
    .await;
    let registered = api_register(&app_create, "creator@example.com", None).await;
    assert_eq!(
        registered.status,
        AxumStatus::CREATED,
        "{}",
        registered.body
    );
    let cookie = registered.session_cookie().expect("session cookie");
    assert_redirect(
        &app_create
            .send(post_form_as(
                "/app/urls",
                "target_url=https%3A%2F%2Fexample.com%2Fbad&cf-turnstile-response=bad",
                &cookie,
            ))
            .await,
        "/app?err=captcha",
    );
    assert_eq!(mock_create.hits(), 1);
    assert!(
        mock_create.bodies()[0].contains("response=bad"),
        "{}",
        mock_create.bodies()[0]
    );
    // The dashboard banner explains the failure.
    let dashboard = app_create.get_as("/app?err=captcha", &cookie).await;
    assert!(
        dashboard.body.contains("The bot protection check failed"),
        "{}",
        dashboard.body
    );

    // The token reaches the provider together with the secret; neither is logged
    // or echoed back to the caller.
    let bodies = mock.bodies();
    assert_eq!(bodies.len(), 2, "{bodies:?}");
    assert!(bodies[0].contains("response=bad"), "{}", bodies[0]);
    assert!(bodies[0].contains("secret=secret-key"), "{}", bodies[0]);
    assert!(
        !bodies[0].contains("cf-turnstile-response"),
        "only the token value is forwarded: {}",
        bodies[0]
    );
}

// ------------------------------------------------------------------ success

#[tokio::test]
async fn valid_token_passes_every_guarded_flow() {
    let (app, mock) = app_with(Reply::Success, Flows::ALL).await;

    // Browser registration.
    let html_register = app
        .send(post_form(
            "/app/register",
            "email=html%40example.com&password=supersecret&cf-turnstile-response=token-1",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_redirect(&html_register, "/app");

    // API registration and sign in.
    let registered = api_register(&app, "api@example.com", Some("token-2")).await;
    assert_eq!(
        registered.status,
        AxumStatus::CREATED,
        "{}",
        registered.body
    );
    let cookie = registered.session_cookie().expect("session cookie");
    let logged_in = app
        .post_json(
            "/api/v1/auth/login",
            api_login_body("api@example.com", Some("token-3")),
        )
        .await;
    assert_eq!(logged_in.status, AxumStatus::OK, "{}", logged_in.body);

    // Browser sign in.
    let html_login = app
        .send(post_form(
            "/app/login",
            "email=api%40example.com&password=supersecret&cf-turnstile-response=token-4",
            None,
            Some("http://localhost:8080"),
        ))
        .await;
    assert_redirect(&html_login, "/app");

    // Browser link creation.
    let html_create = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Fok&cf-turnstile-response=token-5",
            &cookie,
        ))
        .await;
    assert_eq!(
        html_create.status,
        AxumStatus::SEE_OTHER,
        "{}",
        html_create.body
    );
    assert!(html_create
        .header("location")
        .expect("location")
        .starts_with("/app/urls/"));

    assert_eq!(mock.hits(), 5, "{:?}", mock.bodies());
}

#[tokio::test]
async fn recaptcha_is_verified_with_its_own_endpoint() {
    let mock = MockSiteverify::start(Reply::Success).await;
    let app = spawn_app("recaptcha", Some(mock.url.clone()), Flows::ALL, false).await;

    let registered = api_register(&app, "recaptcha@example.com", Some("rc-token")).await;
    assert_eq!(
        registered.status,
        AxumStatus::CREATED,
        "{}",
        registered.body
    );

    let login_page = app.get("/app/login").await;
    assert!(login_page.body.contains("g-recaptcha"));
    assert!(login_page.body.contains("google.com/recaptcha/api.js"));
    assert!(!login_page.body.contains("cf-turnstile"));

    let bodies = mock.bodies();
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].contains("response=rc-token"), "{}", bodies[0]);
}

#[tokio::test]
async fn the_client_address_is_forwarded_when_it_is_known() {
    // Without a trusted proxy only the socket peer is used.
    let mock = MockSiteverify::start(Reply::Success).await;
    let app = spawn_app("turnstile", Some(mock.url.clone()), Flows::ALL, false).await;
    let mut request = post_json(
        "/api/v1/auth/register",
        api_register_body("peer@example.com", Some("peer-token")),
    );
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([203, 0, 113, 7], 4433))));
    let response = app.send(request).await;
    assert_eq!(response.status, AxumStatus::CREATED, "{}", response.body);
    assert!(
        mock.bodies()[0].contains("remoteip=203.0.113.7"),
        "{}",
        mock.bodies()[0]
    );

    // A forged header is ignored while `trust_proxy` is off.
    let mock = MockSiteverify::start(Reply::Success).await;
    let app = spawn_app("turnstile", Some(mock.url.clone()), Flows::ALL, false).await;
    let mut request = post_json(
        "/api/v1/auth/register",
        api_register_body("spoofed@example.com", Some("spoofed-token")),
    );
    request.headers_mut().insert(
        "x-real-ip",
        axum::http::HeaderValue::from_static("198.51.100.9"),
    );
    let response = app.send(request).await;
    assert_eq!(response.status, AxumStatus::CREATED, "{}", response.body);
    assert!(
        !mock.bodies()[0].contains("remoteip"),
        "a spoofed header must not be forwarded: {}",
        mock.bodies()[0]
    );

    // With a trusted proxy it matches what the rate limiter uses.
    let mock = MockSiteverify::start(Reply::Success).await;
    let app = spawn_app("turnstile", Some(mock.url.clone()), Flows::ALL, true).await;
    let mut request = post_json(
        "/api/v1/auth/register",
        api_register_body("proxied@example.com", Some("proxied-token")),
    );
    request.headers_mut().insert(
        "x-real-ip",
        axum::http::HeaderValue::from_static("198.51.100.9"),
    );
    let response = app.send(request).await;
    assert_eq!(response.status, AxumStatus::CREATED, "{}", response.body);
    assert!(
        mock.bodies()[0].contains("remoteip=198.51.100.9"),
        "{}",
        mock.bodies()[0]
    );
}

// -------------------------------------------------------------- fail closed

#[tokio::test]
async fn provider_errors_fail_closed() {
    for reply in [Reply::Status(500), Reply::Status(429), Reply::Garbage] {
        let (app, _mock) = app_with(reply, Flows::ALL).await;
        assert_captcha_error(&api_register(&app, "failing@example.com", Some("token")).await);
    }

    // A provider that cannot be reached at all is a rejection too.
    let app = spawn_app(
        "turnstile",
        Some("http://127.0.0.1:1/siteverify".to_string()),
        Flows::ALL,
        false,
    )
    .await;
    assert_captcha_error(&api_register(&app, "offline@example.com", Some("token")).await);
    assert_redirect(
        &app.send(post_form(
            "/app/register",
            "email=offline-html%40example.com&password=supersecret\
             &cf-turnstile-response=token",
            None,
            Some("http://localhost:8080"),
        ))
        .await,
        "/app/register?err=captcha",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_provider_timeout_fails_closed() {
    let (app, _mock) = app_with(Reply::Slow, Flows::ALL).await;
    let started = std::time::Instant::now();
    assert_captcha_error(&api_register(&app, "slow@example.com", Some("token")).await);
    assert!(
        started.elapsed() >= Duration::from_secs(4),
        "the 5s provider timeout must be honoured, took {:?}",
        started.elapsed()
    );
}

// -------------------------------------------------------- per-flow toggles

#[tokio::test]
async fn toggles_are_independent() {
    let (app, mock) = app_with(Reply::Success, Flows::LOGIN_ONLY).await;

    // Registration and creation are untouched while only sign in is guarded.
    let registered = api_register(&app, "independent@example.com", None).await;
    assert_eq!(
        registered.status,
        AxumStatus::CREATED,
        "{}",
        registered.body
    );
    let cookie = registered.session_cookie().expect("session cookie");

    let created = app
        .send(post_form_as(
            "/app/urls",
            "target_url=https%3A%2F%2Fexample.com%2Findependent",
            &cookie,
        ))
        .await;
    assert_eq!(created.status, AxumStatus::SEE_OTHER, "{}", created.body);
    assert!(created
        .header("location")
        .expect("location")
        .starts_with("/app/urls/"));

    // Sign in is guarded.
    assert_captcha_error(
        &app.post_json(
            "/api/v1/auth/login",
            api_login_body("independent@example.com", None),
        )
        .await,
    );
    assert_redirect(
        &app.send(post_form(
            "/app/login",
            "email=independent%40example.com&password=supersecret",
            None,
            Some("http://localhost:8080"),
        ))
        .await,
        "/app/login?err=captcha",
    );

    // Only the guarded form renders the widget.
    let login_page = app.get("/app/login").await;
    assert!(login_page.body.contains("cf-turnstile"));
    let register_page = app.get("/app/register").await;
    assert!(!register_page.body.contains("cf-turnstile"));
    let dashboard = app.get_as("/app", &cookie).await;
    assert!(!dashboard.body.contains("cf-turnstile"));

    assert_eq!(
        mock.hits(),
        0,
        "the unguarded flows never call the provider"
    );
}

#[tokio::test]
async fn a_provider_without_keys_is_inert() {
    // Enabled toggles but empty keys: nothing is verified and nothing renders.
    let app = TestApp::spawn_with(|config| {
        config.captcha_provider_override = Some("turnstile".to_string());
        config.captcha_site_key_override = None;
        config.captcha_secret_key_override = None;
        config.captcha_on_register_override = Some(true);
        config.captcha_on_login_override = Some(true);
        config.captcha_on_create_override = Some(true);
        config.captcha_verify_url = Some("http://127.0.0.1:1/siteverify".to_string());
    })
    .await;

    let registered = api_register(&app, "unkeyed@example.com", None).await;
    assert_eq!(
        registered.status,
        AxumStatus::CREATED,
        "{}",
        registered.body
    );
    assert!(!app.get("/app/login").await.body.contains("cf-turnstile"));

    // `provider = none` alone is inert as well.
    let app = spawn_app("none", None, Flows::ALL, false).await;
    let registered = api_register(&app, "noop@example.com", None).await;
    assert_eq!(
        registered.status,
        AxumStatus::CREATED,
        "{}",
        registered.body
    );
}

// ------------------------------------------------------------------ widgets

#[tokio::test]
async fn the_widget_is_rendered_and_localised() {
    let (app, _mock) = app_with(Reply::Success, Flows::ALL).await;

    let login = app
        .send(
            axum::http::Request::builder()
                .method("GET")
                .uri("/app/login")
                .header(axum::http::header::HOST, "localhost:8080")
                .header(axum::http::header::ACCEPT_LANGUAGE, "zh-TW")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await;
    assert!(login.body.contains("<html lang=\"zh-Hant\">"));
    assert!(login.body.contains("cf-turnstile"));
    assert!(login.body.contains("data-sitekey=\"site-key\""));
    assert!(
        login
            .body
            .contains("https://challenges.cloudflare.com/turnstile/v0/api.js?hl=zh-TW"),
        "{}",
        login.body
    );
    // The label comes from the catalogue, not from the provider script.
    assert!(login.body.contains("驗證"));

    let register = app
        .send(
            axum::http::Request::builder()
                .method("GET")
                .uri("/app/register")
                .header(axum::http::header::HOST, "localhost:8080")
                .header(axum::http::header::ACCEPT_LANGUAGE, "zh-CN")
                .body(axum::body::Body::empty())
                .expect("request"),
        )
        .await;
    assert!(
        register.body.contains("api.js?hl=zh-CN"),
        "{}",
        register.body
    );
}

#[tokio::test]
async fn the_error_page_explains_a_captcha_failure() {
    let response =
        AppError::captcha_failed(uwurls::services::captcha::FAILURE_MESSAGE).into_response();
    assert_eq!(response.status(), AxumStatus::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Verification required"), "{html}");
    assert!(
        html.contains("The bot protection check failed"),
        "the HTML page carries a readable message: {html}"
    );
}

// ------------------------------------------------------ admin configuration

#[tokio::test]
async fn an_admin_can_configure_bot_protection() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("admin@example.com", "supersecret").await;

    let page = app.get_as("/app/admin/settings", &cookie).await;
    assert_eq!(page.status, AxumStatus::OK);
    assert!(page.body.contains("Bot protection"));
    assert!(page.body.contains("No secret key stored"));

    let saved = app
        .send(post_form_as(
            "/app/admin/settings",
            &settings_form(
                "captcha_provider=recaptcha&captcha_site_key=public-site-key\
                 &captcha_secret_key=top-secret-value&captcha_on_login=on",
            ),
            &cookie,
        ))
        .await;
    assert_redirect(&saved, "/app/admin/settings?ok=saved");

    let settings = &app.state.settings;
    assert_eq!(settings.captcha_provider().as_str(), "recaptcha");
    assert_eq!(settings.captcha_site_key(), "public-site-key");
    assert_eq!(settings.captcha_secret_key(), "top-secret-value");
    assert!(settings.captcha_secret_configured());
    assert!(settings.captcha_on_login());
    assert!(!settings.captcha_on_register());
    assert!(!settings.captcha_on_create());

    // The secret never reaches a browser or the admin API.
    let page = app.get_as("/app/admin/settings", &cookie).await;
    assert!(!page.body.contains("top-secret-value"), "{}", page.body);
    assert!(page.body.contains("A secret key is stored"));
    assert!(page.body.contains("public-site-key"));

    let snapshot = app.get_as("/api/v1/admin/settings", &cookie).await;
    assert!(
        !snapshot.body.contains("top-secret-value"),
        "{}",
        snapshot.body
    );
    assert!(snapshot.json().get("captcha_secret_key").is_none());
    assert_eq!(snapshot.json()["captcha_provider"], "recaptcha");
    assert_eq!(snapshot.json()["captcha_site_key"], "public-site-key");
    assert_eq!(snapshot.json()["captcha_on_login"], "true");

    // Leaving the secret field blank keeps the stored value.
    let blank = app
        .send(post_form_as(
            "/app/admin/settings",
            &settings_form(
                "captcha_provider=recaptcha&captcha_site_key=public-site-key\
                 &captcha_secret_key=&captcha_on_login=on&captcha_on_create=on",
            ),
            &cookie,
        ))
        .await;
    assert_redirect(&blank, "/app/admin/settings?ok=saved");
    assert_eq!(app.state.settings.captcha_secret_key(), "top-secret-value");
    assert!(app.state.settings.captcha_on_create());

    // The public site key, on the other hand, can be cleared.
    let cleared = app
        .send(post_form_as(
            "/app/admin/settings",
            &settings_form("captcha_provider=recaptcha&captcha_site_key="),
            &cookie,
        ))
        .await;
    assert_redirect(&cleared, "/app/admin/settings?ok=saved");
    assert_eq!(app.state.settings.captcha_site_key(), "");
    assert_eq!(app.state.settings.captcha_secret_key(), "top-secret-value");

    // An unknown provider is refused and changes nothing.
    let refused = app
        .send(post_form_as(
            "/app/admin/settings",
            &settings_form("captcha_provider=hcaptcha&captcha_site_key="),
            &cookie,
        ))
        .await;
    assert_redirect(&refused, "/app/admin/settings?err=invalid");
    assert_eq!(app.state.settings.captcha_provider().as_str(), "recaptcha");
}

#[tokio::test]
async fn the_admin_api_accepts_the_captcha_keys_without_echoing_the_secret() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("api-admin@example.com", "supersecret").await;

    let updated = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({
                "captcha_provider": "turnstile",
                "captcha_site_key": "api-site-key",
                "captcha_secret_key": "api-secret-value",
                "captcha_on_register": "true",
            }),
            &cookie,
        ))
        .await;
    assert_eq!(updated.status, AxumStatus::OK, "{}", updated.body);
    assert!(
        !updated.body.contains("api-secret-value"),
        "{}",
        updated.body
    );
    assert_eq!(
        app.state.settings.captcha_secret_key(),
        "api-secret-value",
        "the secret is stored even though it is never returned"
    );

    let current = app.get_as("/api/v1/admin/settings", &cookie).await;
    assert!(
        !current.body.contains("api-secret-value"),
        "{}",
        current.body
    );
    assert!(current.json().get("captcha_secret_key").is_none());
    assert_eq!(current.json()["captcha_on_register"], "true");

    // The toggles stored through the API guard the flow immediately.
    let mock = MockSiteverify::start(Reply::Rejected).await;
    let app = TestApp::spawn_with({
        let url = mock.url.clone();
        move |config| config.captcha_verify_url = Some(url)
    })
    .await;
    let (cookie, _) = app.register("api-toggle@example.com", "supersecret").await;
    let updated = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({
                "captcha_provider": "turnstile",
                "captcha_site_key": "site",
                "captcha_secret_key": "secret",
                "captcha_on_login": "true",
            }),
            &cookie,
        ))
        .await;
    assert_eq!(updated.status, AxumStatus::OK, "{}", updated.body);

    let rejected = app
        .post_json(
            "/api/v1/auth/login",
            api_login_body("api-toggle@example.com", None),
        )
        .await;
    assert_captcha_error(&rejected);
    assert_eq!(mock.hits(), 0, "no token, so the provider is not called");

    // Invalid values are refused.
    let invalid = app
        .send(put_json_as(
            "/api/v1/admin/settings",
            json!({ "captcha_provider": "hcaptcha" }),
            &cookie,
        ))
        .await;
    assert_eq!(invalid.status, AxumStatus::UNPROCESSABLE_ENTITY);
}
