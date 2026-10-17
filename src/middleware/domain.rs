//! Host policy and request domain context.
//!
//! The canonical host serves the full application. Extra domains are
//! redirect-only: `/{slug}` and `/{slug}/unlock` work, `/` shows a small
//! landing page, `/app/*` GET/HEAD is sent back to the canonical host, and
//! `/api/v1/*` is rejected. Unknown, pending and disabled hosts are invisible.

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::db::domains::Domain;
use crate::error::{ApiError, AppError};
use crate::services::domains::normalize_host;
use crate::state::AppState;

/// Domain information for the route handlers that need it.
#[derive(Debug, Clone)]
pub struct HostContext {
    /// True for the configured canonical host (also when `Host` is absent).
    pub canonical: bool,
    /// The resolved domain row, if the host is known and active.
    pub domain: Option<Domain>,
}

impl HostContext {
    pub fn canonical(state: &AppState) -> Self {
        let domain = state
            .domains
            .lookup(state.domains.canonical_host())
            .filter(Domain::is_active);
        Self {
            canonical: true,
            domain,
        }
    }

    pub fn domain_id(&self) -> Option<i64> {
        self.domain.as_ref().map(|domain| domain.id)
    }

    pub fn hostname(&self) -> Option<&str> {
        self.domain.as_ref().map(|domain| domain.hostname.as_str())
    }
}

/// Middleware entry point. Runs after locale negotiation and before CSRF.
pub async fn domain_gate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    let method = request.method().clone();

    // Health checks and static assets must work on every Host, including an
    // unknown one, so error pages and container probes never break.
    if is_host_agnostic(&path) {
        return next.run(request).await;
    }

    let host_header = request.headers().get(header::HOST);
    let raw_host = host_header.and_then(|value| value.to_str().ok());
    let host = raw_host.and_then(normalize_host);

    // HTTP/1.0 requests and the test harness may omit Host; treat them as the
    // canonical host. A *present but invalid* Host (including one that is not
    // visible ASCII) is never canonical: it is unknown and gets a 404. The
    // canonical check uses the raw value because the configured canonical host
    // may be an IPv6 literal or an underscore service name.
    let canonical = match host_header {
        None => true,
        Some(_) => raw_host
            .map(|raw| state.domains.is_canonical(raw))
            .unwrap_or(false),
    };

    if canonical {
        request
            .extensions_mut()
            .insert(HostContext::canonical(&state));
        return next.run(request).await;
    }

    let domain = match host.as_deref() {
        Some(host) => state.domains.lookup(host),
        None => raw_host.and_then(|raw| state.domains.lookup_raw(raw)),
    };
    let Some(domain) = domain else {
        return unknown_host_response(&path);
    };

    if !domain.is_active() {
        return unknown_host_response(&path);
    }

    // Extra domain: only the redirect surface is served.
    if path == "/" {
        request.extensions_mut().insert(HostContext {
            canonical: false,
            domain: Some(domain),
        });
        return next.run(request).await;
    }

    if path == "/app" || path.starts_with("/app/") {
        if matches!(method, Method::GET | Method::HEAD) {
            return redirect_to_canonical(&state, &request);
        }
        return AppError::NotFound.into_response();
    }

    if path == "/api" || path.starts_with("/api/") {
        return ApiError(AppError::not_found()).into_response();
    }

    if is_redirect_path(&path) {
        request.extensions_mut().insert(HostContext {
            canonical: false,
            domain: Some(domain),
        });
        return next.run(request).await;
    }

    AppError::NotFound.into_response()
}

/// Paths that are independent of the domain.
fn is_host_agnostic(path: &str) -> bool {
    path == "/health"
        || path == "/healthz"
        || path == "/favicon.ico"
        || path == "/favicon.svg"
        || path == "/assets"
        || path.starts_with("/assets/")
}

/// `/{slug}` or `/{slug}/unlock`.
fn is_redirect_path(path: &str) -> bool {
    let segments: Vec<&str> = path
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    matches!(segments.as_slice(), [_slug] | [_slug, "unlock"])
}

fn redirect_to_canonical(state: &AppState, request: &Request) -> Response {
    let path_and_query = request
        .uri()
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let location = format!("{}{}", state.config.base_url, path_and_query);
    match HeaderValue::from_str(&location) {
        Ok(value) => {
            let mut response = StatusCode::PERMANENT_REDIRECT.into_response();
            response.headers_mut().insert(header::LOCATION, value);
            // A permanent redirect is correct, but do not let browsers pin a
            // canonical target that may change with URLET_BASE_URL.
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(err) => {
            tracing::error!(error = %err, "cannot build canonical redirect");
            AppError::internal("invalid canonical redirect").into_response()
        }
    }
}

fn unknown_host_response(path: &str) -> Response {
    if path == "/api" || path.starts_with("/api/") {
        ApiError(AppError::not_found()).into_response()
    } else {
        AppError::NotFound.into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_redirect_paths() {
        assert!(is_redirect_path("/promo"));
        assert!(is_redirect_path("/Promo/"));
        assert!(is_redirect_path("/promo/unlock"));
        assert!(!is_redirect_path("/"));
        // `/app` is one segment; the gate handles it before this helper.
        assert!(is_redirect_path("/app"));
        assert!(!is_redirect_path("/api/v1/urls"));
        assert!(!is_redirect_path("/a/b/c"));
    }

    #[test]
    fn host_agnostic_paths_bypass_the_gate() {
        for path in ["/health", "/healthz", "/assets/app.css", "/favicon.ico"] {
            assert!(is_host_agnostic(path), "{path}");
        }
        assert!(!is_host_agnostic("/promo"));
        assert!(!is_host_agnostic("/app"));
    }
}
