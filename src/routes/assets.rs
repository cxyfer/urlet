//! Embedded static assets.

use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};

use crate::assets;

const CACHE: &str = "public, max-age=3600";

fn asset(content_type: &'static str, body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static(CACHE)),
        ],
        body,
    )
        .into_response()
}

pub async fn css() -> Response {
    asset("text/css; charset=utf-8", assets::APP_CSS)
}

pub async fn js() -> Response {
    asset("application/javascript; charset=utf-8", assets::APP_JS)
}

/// Tiny blocking script used in `<head>` to apply a saved theme before paint.
pub async fn theme_js() -> Response {
    asset("application/javascript; charset=utf-8", assets::THEME_JS)
}

pub async fn favicon() -> Response {
    asset("image/svg+xml", assets::FAVICON)
}

/// Browsers still probe `/favicon.ico`; answer with the SVG so no 404 is logged.
pub async fn favicon_ico() -> Response {
    favicon().await
}
