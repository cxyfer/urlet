//! Security response headers.

use axum::extract::{Request, State};
use axum::http::{header, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

use crate::state::AppState;

/// Content Security Policy. Everything is served from this origin; inline
/// styles are allowed because templates use a few `style` attributes, but
/// scripts are restricted to our own bundle.
const CSP: &str = "default-src 'self'; \
                   img-src 'self' data:; \
                   style-src 'self' 'unsafe-inline'; \
                   script-src 'self'; \
                   connect-src 'self'; \
                   font-src 'self'; \
                   base-uri 'none'; \
                   form-action 'self'; \
                   frame-ancestors 'none'";

const PERMISSIONS_POLICY: &str = "geolocation=(), microphone=(), camera=(), payment=()";

pub async fn security_headers(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );

    if state.config.is_https() {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }

    response
}
