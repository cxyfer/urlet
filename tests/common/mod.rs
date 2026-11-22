//! Shared helpers for the integration tests.
//!
//! Each test gets its own temporary database and a router built exactly like
//! the production one, so the tests exercise the real middleware stack. The
//! database is SQLite unless `URLET_TEST_DATABASE_URL` names a PostgreSQL
//! server, in which case every app gets a fresh PostgreSQL database there
//! (SPEC §10).

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
    /// The PostgreSQL database created for this app, dropped with it.
    postgres_database: Option<String>,
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
        let postgres_database = fresh_postgres_database().await;
        if let Some(url) = &postgres_database {
            config.database_url = url.clone();
        }
        tweak(&mut config);

        let pool = urlet::db::init(&config).await.expect("database init");
        let settings = Settings::load(&pool, &config).await.expect("settings");
        let state = AppState::new(config, pool, settings);
        urlet::server::bootstrap(&state).await.expect("bootstrap");
        let router = urlet::server::build_router(state.clone());
        Self {
            router,
            state,
            dir,
            postgres_database,
        }
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

    pub async fn get_with_host_as(&self, uri: &str, host: &str, cookie: &str) -> TestResponse {
        self.send(get_with_host_as(uri, host, cookie)).await
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
             VALUES ($1, 0, $2, NULL, $3, $4, 'ui', $5)
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
            sqlx::query_scalar("SELECT id FROM domains WHERE hostname = $1 COLLATE NOCASE")
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
        sqlx::query("UPDATE domains SET is_default = 1 WHERE id = $1")
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

/// The PostgreSQL server the suite runs against, if any.
pub fn postgres_url() -> Option<String> {
    std::env::var("URLET_TEST_DATABASE_URL")
        .ok()
        .filter(|url| url.starts_with("postgres://") || url.starts_with("postgresql://"))
}

/// Whether this run uses PostgreSQL instead of SQLite.
pub fn on_postgres() -> bool {
    postgres_url().is_some()
}

/// Create an empty database on the PostgreSQL server named by
/// `URLET_TEST_DATABASE_URL` and return its URL, or `None` on SQLite.
pub async fn fresh_postgres_database() -> Option<String> {
    use sqlx::Connection;

    let admin_url = postgres_url()?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let name = format!(
        "urlet_t_{}_{}_{nanos}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let mut admin = sqlx::postgres::PgConnection::connect(&admin_url)
        .await
        .expect("connect to URLET_TEST_DATABASE_URL");
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&mut admin)
        .await
        .expect("create test database");
    let _ = admin.close().await;
    let mut url = url::Url::parse(&admin_url).expect("valid URLET_TEST_DATABASE_URL");
    url.set_path(&format!("/{name}"));
    Some(url.to_string())
}

/// Run one `UPDATE` that a `CHECK` constraint would refuse, to model a row
/// written by an older or newer build. SQLite skips the checks on one
/// connection; PostgreSQL drops `constraint` from this app's own database.
pub async fn update_bypassing_checks(app: &TestApp, sql: &str, id: i64, constraint: &str) {
    let mut connection = app.state.db.acquire().await.expect("connection");
    if on_postgres() {
        sqlx::query(&format!("ALTER TABLE urls DROP CONSTRAINT {constraint}"))
            .execute(&mut *connection)
            .await
            .expect("drop check");
        sqlx::query(sql)
            .bind(id)
            .execute(&mut *connection)
            .await
            .expect("unchecked update");
        return;
    }
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .expect("relax checks");
    sqlx::query(sql)
        .bind(id)
        .execute(&mut *connection)
        .await
        .expect("unchecked update");
    sqlx::query("PRAGMA ignore_check_constraints = OFF")
        .execute(&mut *connection)
        .await
        .expect("restore checks");
}

/// Make every `UPDATE` of `table` fail, to model a write the database refuses.
pub async fn reject_updates_on(app: &TestApp, table: &str) {
    if on_postgres() {
        sqlx::raw_sql(&format!(
            "CREATE FUNCTION reject_update() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN RAISE EXCEPTION '{table} is read-only'; END; $$;
             CREATE TRIGGER reject_{table}_update BEFORE UPDATE ON {table}
             FOR EACH ROW EXECUTE FUNCTION reject_update();"
        ))
        .execute(&app.state.db)
        .await
        .expect("create trigger");
        return;
    }
    sqlx::query(&format!(
        "CREATE TRIGGER reject_{table}_update BEFORE UPDATE ON {table}
         BEGIN SELECT RAISE(ABORT, '{table} is read-only'); END"
    ))
    .execute(&app.state.db)
    .await
    .expect("create trigger");
}

/// Send one request through an arbitrary router built by a test.
pub async fn send_router(router: Router, request: Request<Body>) -> TestResponse {
    let response = router.oneshot(request).await.expect("router call");
    TestResponse::from_response(response).await
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        if let (Some(url), Some(admin_url)) = (self.postgres_database.take(), postgres_url()) {
            // Drop runs outside any async context, so a short-lived runtime on
            // its own thread removes the database; `FORCE` closes the pool's
            // connections first.
            let _ = std::thread::spawn(move || {
                use sqlx::Connection;
                let Some(name) = url.rsplit('/').next().map(str::to_string) else {
                    return;
                };
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                runtime.block_on(async move {
                    if let Ok(mut admin) = sqlx::postgres::PgConnection::connect(&admin_url).await {
                        let _ =
                            sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
                                .execute(&mut admin)
                                .await;
                        let _ = admin.close().await;
                    }
                });
            })
            .join();
        }
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

pub fn post_json_with_host_as(uri: &str, body: Value, cookie: &str, host: &str) -> Request<Body> {
    base_request_with_host("POST", uri, host)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, format!("http://{host}"))
        .body(Body::from(body.to_string()))
        .expect("request")
}

pub fn patch_json_with_host_as(uri: &str, body: Value, cookie: &str, host: &str) -> Request<Body> {
    base_request_with_host("PATCH", uri, host)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header(header::ORIGIN, format!("http://{host}"))
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
