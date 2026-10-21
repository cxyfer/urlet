//! Embedded static assets.

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::assets;
use crate::db::custom_themes;
use crate::services::custom_theme;
use crate::state::AppState;

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

/// One administrator-uploaded theme stylesheet.
///
/// The database row is the authority for which key/version pairs exist; the
/// request path never becomes a file path directly. A missing file answers 404
/// (for example after a volume was detached), not a 500 with a path in it.
pub async fn custom_theme_css(State(state): State<AppState>, Path(key): Path<String>) -> Response {
    if !custom_theme::validate_key(&key) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let row = match custom_themes::find_by_key(&state.db, &key).await {
        Ok(Some(row)) => row,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!(error = %err, "cannot read custom theme metadata");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let custom = match custom_theme::read_css(&state.config.themes_dir, &row).await {
        Ok(body) => body,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(key = %key, "custom theme stylesheet is missing from the volume");
            return StatusCode::NOT_FOUND.into_response();
        }
        Err(err) => {
            tracing::error!(error = %err, key = %key, "cannot read custom theme stylesheet");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // Uploaded themes use the Verdant templates, so the base stylesheet is the
    // sensible starting point. Prepending it keeps a small colour override
    // usable and lets the upload win the cascade without forcing every author
    // to copy the whole design.
    let mut body = Vec::with_capacity(assets::APP_CSS.len() + custom.len() + 1);
    body.extend_from_slice(assets::APP_CSS.as_bytes());
    body.push(b'\n');
    body.extend_from_slice(&custom);
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/css; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static(CACHE)),
        ],
        body,
    )
        .into_response()
}

pub async fn favicon() -> Response {
    asset("image/svg+xml", assets::FAVICON)
}

/// Browsers still probe `/favicon.ico`; answer with the SVG so no 404 is logged.
pub async fn favicon_ico() -> Response {
    favicon().await
}
