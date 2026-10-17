//! CSRF protection for cookie authenticated, state changing requests.
//!
//! Rule set (see `docs/SPEC.md` §5.1 NFR-SEC-3):
//!   1. safe methods pass;
//!   2. `Authorization: Bearer` requests pass — they are not cookie driven;
//!   3. requests carrying a session cookie must prove same origin;
//!   4. requests with a browser form content type must prove same origin
//!      (JSON bodies cannot be sent cross-site without a CORS preflight);
//!   5. everything else (JSON APIs without cookies) passes.

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::Config;
use crate::error::AppError;
use crate::services::cookies;
use crate::services::domains::{normalize_host, raw_host_key};
use crate::state::AppState;

/// Middleware entry point.
pub async fn csrf_protect(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    if is_safe_method(request.method()) {
        return Ok(next.run(request).await);
    }
    if request.headers().contains_key(header::AUTHORIZATION) {
        return Ok(next.run(request).await);
    }

    let cookie_authenticated = cookies::get(request.headers(), cookies::SESSION_COOKIE).is_some();
    let form_content_type = has_form_content_type(request.headers());

    if !cookie_authenticated && !form_content_type {
        return Ok(next.run(request).await);
    }

    if same_origin(request.headers(), &state.config) {
        return Ok(next.run(request).await);
    }

    tracing::warn!(
        path = %request.uri().path(),
        origin = ?request.headers().get(header::ORIGIN),
        "rejected cross-site state changing request"
    );
    Err(AppError::forbidden(
        "request rejected: cross-site submission",
    ))
}

/// `true` for methods that must not change state.
pub fn is_safe_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

fn has_form_content_type(headers: &HeaderMap) -> bool {
    let Some(value) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let value = value.to_ascii_lowercase();
    value.starts_with("application/x-www-form-urlencoded")
        || value.starts_with("multipart/form-data")
        || value.starts_with("text/plain")
}

/// Compare the `Origin`/`Referer` header against the request host.
pub fn same_origin(headers: &HeaderMap, config: &Config) -> bool {
    let raw_host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let request_key = host_key(&raw_host);

    for name in [header::ORIGIN, header::REFERER] {
        let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) else {
            continue;
        };
        let Some(origin_host) = host_of(value) else {
            continue;
        };
        let origin_key = host_key(&origin_host);
        if (!raw_host.is_empty() && origin_key == request_key)
            || origin_key == host_key(&config.base_host)
        {
            return true;
        }
    }
    false
}

/// Normalise both sides of the comparison with the same rules as the host gate:
/// case, trailing dot and `:80`/`:443` are irrelevant, and the canonical host
/// may be an IPv6 literal or an underscore service name.
fn host_key(value: &str) -> String {
    normalize_host(value).unwrap_or_else(|| raw_host_key(value))
}

/// Extract the authority component of an absolute URL.
fn host_of(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case("null") {
        return None;
    }
    let rest = match value.split_once("://") {
        Some((_, rest)) => rest,
        None => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    // Drop any userinfo component.
    let authority = authority.rsplit('@').next()?;
    if authority.is_empty() {
        return None;
    }
    Some(authority.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn config() -> Config {
        Config::for_tests(std::path::Path::new("."))
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes()).expect("name"),
                HeaderValue::from_str(value).expect("value"),
            );
        }
        headers
    }

    #[test]
    fn detects_safe_methods() {
        assert!(is_safe_method(&Method::GET));
        assert!(is_safe_method(&Method::HEAD));
        assert!(!is_safe_method(&Method::POST));
        assert!(!is_safe_method(&Method::DELETE));
    }

    #[test]
    fn accepts_matching_origin() {
        let same = headers(&[
            ("host", "localhost:8080"),
            ("origin", "http://localhost:8080"),
        ]);
        assert!(same_origin(&same, &config()));
    }

    #[test]
    fn accepts_matching_referer() {
        let referer = headers(&[
            ("host", "s.example.com"),
            ("referer", "https://s.example.com/app/urls"),
        ]);
        assert!(same_origin(&referer, &config()));
    }

    #[test]
    fn normalises_default_ports_and_operator_hosts() {
        let same = headers(&[
            ("host", "go.example.com:443"),
            ("origin", "https://go.example.com"),
        ]);
        assert!(same_origin(&same, &config()));

        let custom = headers(&[
            ("host", "go.example.com:8080"),
            ("origin", "http://go.example.com:8080"),
        ]);
        assert!(same_origin(&custom, &config()));

        let mismatch = headers(&[
            ("host", "go.example.com:8080"),
            ("origin", "http://go.example.com:9090"),
        ]);
        assert!(!same_origin(&mismatch, &config()));

        let underscore = headers(&[("host", "urlet_app:443"), ("origin", "https://urlet_app")]);
        assert!(same_origin(&underscore, &config()));
    }

    #[test]
    fn rejects_foreign_origin() {
        let foreign = headers(&[
            ("host", "localhost:8080"),
            ("origin", "https://evil.example"),
        ]);
        assert!(!same_origin(&foreign, &config()));
    }

    #[test]
    fn rejects_opaque_and_missing_origins() {
        let opaque = headers(&[("host", "localhost:8080"), ("origin", "null")]);
        assert!(!same_origin(&opaque, &config()));
        let missing = headers(&[("host", "localhost:8080")]);
        assert!(!same_origin(&missing, &config()));
    }

    #[test]
    fn rejects_lookalike_origin() {
        let lookalike = headers(&[
            ("host", "localhost:8080"),
            ("origin", "http://localhost:8080.evil.example"),
        ]);
        assert!(!same_origin(&lookalike, &config()));
    }

    #[test]
    fn ignores_userinfo_in_origin() {
        assert_eq!(
            host_of("http://user@localhost:8080").as_deref(),
            Some("localhost:8080")
        );
        assert_eq!(
            host_of("https://s.example.com/a/b").as_deref(),
            Some("s.example.com")
        );
        assert_eq!(host_of("not a url"), None);
    }
}
