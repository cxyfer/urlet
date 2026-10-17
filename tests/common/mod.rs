//! Shared helpers for the integration tests.
//!
//! Each test gets its own temporary SQLite database and a router built exactly
//! like the production one, so the tests exercise the real middleware stack.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;
use urlet::config::Config;
use urlet::services::settings::Settings;
use urlet::state::AppState;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A running application with an isolated database.
pub struct TestApp {
    pub router: Router,
    pub state: AppState,
    pub dir: PathBuf,
}

impl TestApp {
    /// Spawn with the default test configuration.
    pub async fn spawn() -> Self {
        Self::spawn_with(|_| {}).await
    }

    /// Spawn with a chance to tweak the configuration first.
    pub async fn spawn_with(tweak: impl FnOnce(&mut Config)) -> Self {
        let dir = unique_dir();
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let mut config = Config::for_tests(&dir);
        tweak(&mut config);

        let pool = urlet::db::init(&config).await.expect("database init");
        let settings = Settings::load(&pool, &config).await.expect("settings");
        let state = AppState::new(config, pool, settings);
        urlet::server::bootstrap(&state).await.expect("bootstrap");
        let router = urlet::server::build_router(state.clone());
        Self { router, state, dir }
    }

    /// Send a request through the real router.
    pub async fn send(&self, request: Request<Body>) -> TestResponse {
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("router call");
        TestResponse::from_response(response).await
    }

    pub async fn get(&self, uri: &str) -> TestResponse {
        self.send(get(uri)).await
    }

    pub async fn get_as(&self, uri: &str, cookie: &str) -> TestResponse {
        self.send(get_as(uri, cookie)).await
    }

    pub async fn post_json(&self, uri: &str, body: Value) -> TestResponse {
        self.send(post_json(uri, body)).await
    }

    pub async fn post_json_as(&self, uri: &str, body: Value, cookie: &str) -> TestResponse {
        self.send(post_json_as(uri, body, cookie)).await
    }

    pub async fn get_with_host(&self, uri: &str, host: &str) -> TestResponse {
        self.send(get_with_host(uri, host)).await
    }

    pub async fn post_json_with_host(&self, uri: &str, body: Value, host: &str) -> TestResponse {
        self.send(post_json_with_host(uri, body, host)).await
    }

    /// Insert a domain directly and reload the registry.
    ///
    /// Tests that only exercise routing use this; the admin API has its own
    /// lifecycle tests.
    pub async fn add_domain(&self, hostname: &str, status: &str) -> i64 {
        let now = urlet::services::time::now_iso();
        let verified = if status == "active" { 1 } else { 0 };
        sqlx::query(
            "INSERT INTO domains
                 (hostname, is_default, is_verified, created_by, created_at, status, source, updated_at)
             VALUES (?, 0, ?, NULL, ?, ?, 'ui', ?)
             ON CONFLICT(hostname) DO NOTHING",
        )
        .bind(hostname)
        .bind(verified)
        .bind(&now)
        .bind(status)
        .bind(&now)
        .execute(&self.state.db)
        .await
        .expect("insert domain");
        let id: i64 =
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = ? COLLATE NOCASE")
                .bind(hostname)
                .fetch_one(&self.state.db)
                .await
                .expect("domain id");
        self.reload_domains().await;
        id
    }

    pub async fn set_default_domain(&self, id: i64) {
        sqlx::query("UPDATE domains SET is_default = 0 WHERE is_default = 1")
            .execute(&self.state.db)
            .await
            .expect("clear default");
        sqlx::query("UPDATE domains SET is_default = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.state.db)
            .await
            .expect("set default");
        self.reload_domains().await;
    }

    pub async fn reload_domains(&self) {
        self.state
            .domains
            .reload(&self.state.db)
            .await
            .expect("reload domains");
    }

    /// Register a user and return `(cookie, user_json)`.
    pub async fn register(&self, email: &str, password: &str) -> (String, Value) {
        let response = self
            .post_json(
                "/api/v1/auth/register",
                serde_json::json!({ "email": email, "password": password }),
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::CREATED,
            "register failed: {}",
            response.body
        );
        let cookie = response.session_cookie().expect("session cookie");
        (cookie, response.json())
    }

    /// Log in and return the session cookie.
    pub async fn login(&self, email: &str, password: &str) -> String {
        let response = self
            .post_json(
                "/api/v1/auth/login",
                serde_json::json!({ "email": email, "password": password }),
            )
            .await;
        assert_eq!(
            response.status,
            StatusCode::OK,
            "login failed: {}",
            response.body
        );
        response.session_cookie().expect("session cookie")
    }
}

/// Send one request through an arbitrary router built by a test.
pub async fn send_router(router: Router, request: Request<Body>) -> TestResponse {
    let response = router.oneshot(request).await.expect("router call");
    TestResponse::from_response(response).await
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Parsed response.
pub struct TestResponse {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: String,
}

impl TestResponse {
    async fn from_response(response: axum::response::Response) -> Self {
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .expect("read body");
        Self {
            status,
            headers,
            body: String::from_utf8_lossy(&bytes).to_string(),
        }
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|err| panic!("body is not JSON ({err}): {}", self.body))
    }

    /// `Cookie:` header value for the session created by this response.
    pub fn session_cookie(&self) -> Option<String> {
        let raw = self.headers.get(header::SET_COOKIE)?.to_str().ok()?;
        let pair = raw.split(';').next()?;
        if pair.starts_with("urlet_session=") {
            Some(pair.to_string())
        } else {
            None
        }
    }

    /// Value of an arbitrary `Set-Cookie` header, name included.
    pub fn cookie_named(&self, name: &str) -> Option<String> {
        for value in self.headers.get_all(header::SET_COOKIE) {
            let raw = value.to_str().ok()?;
            let pair = raw.split(';').next().unwrap_or_default();
            if pair.starts_with(&format!("{name}=")) {
                return Some(pair.to_string());
            }
        }
        None
    }

    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name)?.to_str().ok().map(str::to_string)
    }
}

pub fn unique_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!("urlet-test-{nanos}-{counter}"))
}

fn base_request(method: &str, uri: &str) -> axum::http::request::Builder {
    base_request_with_host(method, uri, "localhost:8080")
}

fn base_request_with_host(method: &str, uri: &str, host: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, host)
}

pub fn get(uri: &str) -> Request<Body> {
    base_request("GET", uri)
        .body(Body::empty())
        .expect("request")
}

pub fn get_with_host(uri: &str, host: &str) -> Request<Body> {
    base_request_with_host("GET", uri, host)
        .body(Body::empty())
        .expect("request")
}

pub fn get_with_host_as(uri: &str, host: &str, cookie: &str) -> Request<Body> {
    base_request_with_host("GET", uri, host)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("request")
}

pub fn get_as(uri: &str, cookie: &str) -> Request<Body> {
    base_request("GET", uri)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("request")
}

pub fn post_json(uri: &str, body: Value) -> Request<Body> {
    base_request("POST", uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

pub fn post_json_with_host(uri: &str, body: Value, host: &str) -> Request<Body> {
    base_request_with_host("POST", uri, host)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

pub fn post_form_with_host(
    uri: &str,
    body: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    host: &str,
) -> Request<Body> {
    let mut builder = base_request_with_host("POST", uri, host)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    builder.body(Body::from(body.to_string())).expect("request")
}

pub fn post_json_as(uri: &str, body: Value, cookie: &str) -> Request<Body> {
    base_request("POST", uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, "http://localhost:8080")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// JSON POST with an explicit origin (or none), used by the CSRF tests.
pub fn post_json_origin(
    uri: &str,
    body: Value,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Request<Body> {
    let mut builder = base_request("POST", uri).header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    builder.body(Body::from(body.to_string())).expect("request")
}

/// Form POST (browser style) with an explicit origin.
pub fn post_form(
    uri: &str,
    body: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> Request<Body> {
    let mut builder =
        base_request("POST", uri).header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    builder.body(Body::from(body.to_string())).expect("request")
}

/// Bearer authenticated request helper.
pub fn with_bearer(mut request: Request<Body>, token: &str) -> Request<Body> {
    request.headers_mut().insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().expect("header"),
    );
    request
}

/// `PATCH` with a JSON body, session cookie and same-origin header.
pub fn patch_json_as(uri: &str, body: Value, cookie: &str) -> Request<Body> {
    base_request("PATCH", uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, "http://localhost:8080")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// `PUT` with a JSON body, session cookie and same-origin header.
pub fn put_json_as(uri: &str, body: Value, cookie: &str) -> Request<Body> {
    base_request("PUT", uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, "http://localhost:8080")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// `DELETE` with a session cookie and same-origin header.
pub fn delete_as(uri: &str, cookie: &str) -> Request<Body> {
    base_request("DELETE", uri)
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, "http://localhost:8080")
        .body(Body::empty())
        .expect("request")
}

/// Form POST with a session cookie and same-origin header.
pub fn post_form_as(uri: &str, body: &str, cookie: &str) -> Request<Body> {
    post_form(uri, body, Some(cookie), Some("http://localhost:8080"))
}
