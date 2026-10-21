//! Embedded static assets.

use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
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

/// Shared chrome for the theme picker, loaded after the selected theme.
pub async fn theme_ui_css() -> Response {
    asset("text/css; charset=utf-8", assets::THEME_UI_CSS)
}

/// One compiled-in theme stylesheet.
///
/// The key is matched against the fixed [`crate::services::theme::ThemeKey`]
/// list, never used as a filesystem path, so an arbitrary path cannot escape the
/// embedded asset set.
pub async fn theme_css(Path(key): Path<String>) -> Response {
    let body = match key.as_str() {
        // Verdant keeps the historical `/assets/app.css` URL in themes, but the
        // catalogue route stays complete for API consumers and preview links.
        "verdant" => assets::APP_CSS,
        "aurora" => assets::AURORA_CSS,
        "paper" => assets::PAPER_CSS,
        "signal" => assets::SIGNAL_CSS,
        "gilded" => assets::GILDED_CSS,
        "swiss" => assets::SWISS_CSS,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    asset("text/css; charset=utf-8", body)
}

pub async fn favicon() -> Response {
    asset("image/svg+xml", assets::FAVICON)
}

/// Browsers still probe `/favicon.ico`; answer with the SVG so no 404 is logged.
pub async fn favicon_ico() -> Response {
    favicon().await
}
