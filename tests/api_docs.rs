//! Integration tests for the embedded API reference and its OpenAPI contracts.
//!
//! The reference is served by the binary itself, so these tests drive the real
//! router and never touch the filesystem. They also pin the invariant that makes
//! the bilingual contract trustworthy: the two language documents must describe
//! exactly the same operations.

mod common;

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

/// The operation id of every operation in a contract, keyed by "METHOD path".
fn operations(document: &Value) -> Vec<(String, String)> {
    let paths = document["paths"].as_object().expect("paths object");
    let mut found: Vec<(String, String)> = Vec::new();
    for (path, item) in paths {
        let item = item.as_object().expect("path item");
        for (method, operation) in item {
            // `parameters` and `summary` may live on the path item itself.
            if !matches!(
                method.as_str(),
                "get" | "put" | "post" | "delete" | "patch" | "options" | "head" | "trace"
            ) {
                continue;
            }
            let id = operation["operationId"]
                .as_str()
                .unwrap_or_else(|| panic!("{method} {path} has no operationId"))
                .to_string();
            found.push((format!("{} {path}", method.to_uppercase()), id));
        }
    }
    found.sort();
    found
}

/// The declared tag names and the tags the operations actually reference.
fn tags_of(document: &Value) -> (Vec<String>, Vec<String>) {
    let declared: Vec<String> = document["tags"]
        .as_array()
        .expect("tags array")
        .iter()
        .map(|tag| tag["name"].as_str().expect("tag name").to_string())
        .collect();
    let mut used: Vec<String> = document["paths"]
        .as_object()
        .expect("paths object")
        .values()
        .flat_map(|item| item.as_object().expect("path item").values())
        .filter_map(|operation| operation.get("tags"))
        .flat_map(|tags| tags.as_array().expect("operation tags"))
        .map(|tag| tag.as_str().expect("tag name").to_string())
        .collect();
    used.sort();
    used.dedup();
    (declared, used)
}

#[tokio::test]
async fn docs_page_serves_the_embedded_reference() {
    let app = TestApp::spawn().await;
    let page = app.get("/docs").await;

    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(
        page.header("content-type").as_deref(),
        Some("text/html; charset=utf-8")
    );

    // The bundle is served from this instance, and the page asks for the
    // English contract by default.
    assert!(page.body.contains("/assets/scalar.js"), "{}", page.body);
    assert!(page.body.contains("/docs/openapi.json"), "{}", page.body);

    // Nothing may be fetched from a third party: a self-hosted instance has to
    // work on a network with no outbound access, and the CSP only allows 'self'.
    for remote in [
        "jsdelivr",
        "unpkg",
        "cdn.",
        "fonts.scalar.com",
        "scalar.com",
    ] {
        assert!(
            !page.body.contains(remote),
            "the reference must not reference {remote}: {}",
            page.body
        );
    }
}

#[tokio::test]
async fn docs_page_follows_the_interface_language() {
    let app = TestApp::spawn().await;

    let english = app.send(get_with("/docs", Some("en"), None)).await;
    assert!(english.body.contains("/docs/openapi.json"));

    let traditional = app
        .send(get_with("/docs", Some("zh-TW,zh;q=0.9,en;q=0.8"), None))
        .await;
    assert_eq!(traditional.status, StatusCode::OK);
    assert!(
        traditional.body.contains("/docs/openapi.zh-TW.json"),
        "{}",
        traditional.body
    );
    assert_eq!(
        traditional.header("content-language").as_deref(),
        Some("zh-Hant")
    );
}

#[tokio::test]
async fn the_reference_moves_to_the_canonical_host_from_an_extra_domain() {
    let app = TestApp::spawn().await;
    app.add_domain("go.example.com", "active").await;

    // The footer links the reference from every design, including the
    // extra-domain short landing, but the reference itself belongs on the
    // canonical host: its Test Request calls the canonical base URL, the CSP
    // allows `connect-src 'self'` only, and there is no CORS layer, so an
    // interactive reference served from an extra domain would have its requests
    // blocked by the browser. A redirect sends the visitor somewhere it works.
    for path in ["/docs", "/docs/openapi.json", "/docs/openapi.zh-TW.json"] {
        let response = app.get_with_host(path, "go.example.com").await;
        assert_eq!(
            response.status,
            StatusCode::PERMANENT_REDIRECT,
            "{path} on an extra domain: {}",
            response.body
        );
        assert!(
            response
                .header("location")
                .is_some_and(|value| value.starts_with(&app.state.config.base_url)),
            "{path} should redirect to the canonical host"
        );
    }

    // The JSON API itself stays canonical-only.
    let api = app.get_with_host("/api/v1/domains", "go.example.com").await;
    assert_eq!(api.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_contract_names_the_canonical_server() {
    let app = TestApp::spawn().await;

    // The API only exists on the canonical host while the reference is served
    // from every host its footer appears on, so the contract has to name the
    // canonical base URL: otherwise the reference's Test Request would target a
    // host that answers 404 for every `/api/v1` path.
    for path in ["/docs/openapi.json", "/docs/openapi.zh-TW.json"] {
        let response = app.get(path).await;
        let document = response.json();
        assert_eq!(
            document["servers"][0]["url"].as_str(),
            Some(app.state.config.base_url.as_str()),
            "{path} should name the configured base URL"
        );
        assert!(
            !response.body.contains("__URLET_BASE_URL__"),
            "{path} still contains the unresolved placeholder"
        );
    }
}

#[tokio::test]
async fn the_english_contract_is_served_as_json() {
    let app = TestApp::spawn().await;
    let response = app.get("/docs/openapi.json").await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.header("content-type").as_deref(),
        Some("application/json; charset=utf-8")
    );

    let document = response.json();
    assert!(document["openapi"].as_str().unwrap_or("").starts_with("3."));
    assert!(document["info"]["title"].is_string());
    assert!(
        document["paths"].as_object().expect("paths").len() > 20,
        "the contract should document the whole API"
    );
    for section in ["schemas", "responses", "securitySchemes"] {
        assert!(
            document["components"][section].is_object(),
            "components.{section} is missing"
        );
    }
    // The two ways of authenticating are part of the contract.
    assert!(document["components"]["securitySchemes"]["sessionCookie"].is_object());
    assert!(document["components"]["securitySchemes"]["bearerToken"].is_object());
}

#[tokio::test]
async fn the_traditional_chinese_contract_is_served_as_json() {
    let app = TestApp::spawn().await;
    let response = app.get("/docs/openapi.zh-TW.json").await;

    assert_eq!(response.status, StatusCode::OK);
    let document = response.json();
    assert!(document["paths"].as_object().expect("paths").len() > 20);

    // The prose is translated; the machine-readable surface is not.
    let english = app.get("/docs/openapi.json").await.json();
    assert_ne!(
        document["info"]["title"], english["info"]["title"],
        "the two contracts should carry different titles"
    );
}

#[tokio::test]
async fn both_contracts_document_the_same_operations() {
    let app = TestApp::spawn().await;

    let english = app.get("/docs/openapi.json").await.json();
    let traditional = app.get("/docs/openapi.zh-TW.json").await.json();

    // A translation that drops or renames an operation would leave the two
    // documents disagreeing about the API, which is worse than no translation.
    assert_eq!(
        operations(&english),
        operations(&traditional),
        "the two contracts must document the same operations"
    );

    // The shared schemas are language neutral, so they must be identical.
    assert_eq!(
        english["components"]["schemas"], traditional["components"]["schemas"],
        "the shared schemas must not differ between languages"
    );
    assert_eq!(
        english["components"]["responses"], traditional["components"]["responses"],
        "the shared responses must not differ between languages"
    );

    // The reference sidebar is grouped by tag, and the group names are
    // translated with the rest of the prose. A rename that missed an operation
    // would leave it ungrouped, so both documents must declare the same number
    // of groups and every operation must reference one of them.
    for (name, document) in [("en", &english), ("zh-TW", &traditional)] {
        let (declared, used) = tags_of(document);
        assert_eq!(
            declared.len(),
            used.len(),
            "{name}: declared and used tag sets should match"
        );
        for tag in &used {
            assert!(
                declared.contains(tag),
                "{name}: operation references undeclared tag {tag:?}"
            );
        }
    }
    let (english_tags, _) = tags_of(&english);
    let (traditional_tags, _) = tags_of(&traditional);
    assert_eq!(
        english_tags.len(),
        traditional_tags.len(),
        "the two contracts must group the API the same way"
    );
    assert_ne!(
        english_tags, traditional_tags,
        "the Chinese contract should carry translated group names"
    );
}

#[tokio::test]
async fn the_footer_links_the_reference_in_the_interface_language() {
    let app = TestApp::spawn().await;

    // The entry point travels with every design's footer, so the label has to
    // be translated rather than hard-coded.
    let english = app.get("/").await;
    assert!(
        english.body.contains("href=\"/docs\""),
        "the footer should link the reference"
    );
    assert!(english
        .body
        .contains("aria-label=\"Read the API reference\""));

    let traditional = app.send(get_with("/", Some("zh-TW,zh;q=0.9"), None)).await;
    assert!(
        traditional
            .body
            .contains("aria-label=\"閱讀 API 參考文件\""),
        "{}",
        traditional.body
    );
}

#[tokio::test]
async fn docs_is_a_reserved_slug() {
    let app = TestApp::spawn().await;
    let (cookie, _) = app.register("user@example.com", "supersecret").await;

    // The reference page owns /docs, so no short link may shadow it.
    let created = post_json_as(
        "/api/v1/urls",
        json!({ "target_url": "https://example.com", "slug": "docs" }),
        &cookie,
    );
    let response = app.send(created).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.json()["error"]["details"]["field"], "slug");

    // Case does not matter: the path is matched case-sensitively but a slug
    // that only differs in case would still be confusing to share.
    let upper = post_json_as(
        "/api/v1/urls",
        json!({ "target_url": "https://example.com", "slug": "DOCS" }),
        &cookie,
    );
    assert_eq!(
        app.send(upper).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn a_live_link_holding_the_docs_slug_is_counted() {
    let app = TestApp::spawn().await;
    let (_, _user) = app.register("user@example.com", "supersecret").await;

    // The slug can no longer be created through the API, so insert the row the
    // way an instance upgraded from an older release would already have it.
    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users LIMIT 1")
        .fetch_one(&app.state.db)
        .await
        .expect("user id");
    let domain_id: i64 = sqlx::query_scalar("SELECT id FROM domains WHERE is_default = 1")
        .fetch_one(&app.state.db)
        .await
        .expect("default domain");
    for (slug, deleted) in [
        ("docs", None::<&str>),
        ("gone", Some("2026-01-01T00:00:00.000Z")),
    ] {
        sqlx::query(
            "INSERT INTO urls (user_id, domain_id, slug, target_url, redirect_type, status,
                               is_custom_slug, created_at, updated_at, deleted_at)
             VALUES (?, ?, ?, 'https://example.com', 302, 'active', 1, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(domain_id)
        .bind(slug)
        .bind("2026-01-01T00:00:00.000Z")
        .bind("2026-01-01T00:00:00.000Z")
        .bind(deleted)
        .execute(&app.state.db)
        .await
        .expect("insert url");
    }

    // Only the live link is at risk; a soft-deleted row is already unreachable.
    let shadowed = urlet::db::urls::count_live_with_slug(&app.state.db, "docs")
        .await
        .expect("count");
    assert_eq!(shadowed, 1);

    let absent = urlet::db::urls::count_live_with_slug(&app.state.db, "promo")
        .await
        .expect("count");
    assert_eq!(absent, 0);
}

#[tokio::test]
async fn a_body_the_extractor_cannot_parse_uses_the_error_envelope() {
    let app = TestApp::spawn().await;

    // The contract documents `BadRequest` as an `Error` envelope and the SPEC
    // states the format is uniform. Axum's own `Json` rejection answers with a
    // plain-text message before the handler runs, so the API routes its body
    // extraction through `routes::extract::Json`; this pins that.
    for body in ["{\"email\": broken", "{}", ""] {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/auth/login")
            .header(header::HOST, "localhost:8080")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://localhost:8080")
            .body(Body::from(body))
            .expect("request");

        let response = app.send(request).await;
        assert_eq!(response.status, StatusCode::BAD_REQUEST, "body {body:?}");
        assert!(
            response
                .header("content-type")
                .is_some_and(|value| value.starts_with("application/json")),
            "body {body:?} was not the documented envelope: {}",
            response.body
        );
        assert_eq!(
            response.json()["error"]["code"],
            "bad_request",
            "body {body:?}"
        );
    }
}
