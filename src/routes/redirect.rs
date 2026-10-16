//! Public redirect routes: `GET /{slug}` and `POST /{slug}/unlock`.

use askama::Template;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::db::urls::{self, Url, UrlState};
use crate::error::{AppError, Result};
use crate::middleware::auth::ClientInfo;
use crate::routes::extract::Flexible;
use crate::services::click::{self, ClickInput};
use crate::services::cookies;
use crate::services::ratelimit::limits;
use crate::services::{unlock, password};
use crate::state::AppState;

/// Password prompt for a protected link.
#[derive(Debug, Template)]
#[template(path = "link_password.html")]
pub struct PasswordPage {
    pub site_name: String,
    pub slug: String,
    pub error: Option<String>,
}

/// 404/410 page for a link that cannot be followed.
#[derive(Debug, Template)]
#[template(path = "link_error.html")]
pub struct LinkErrorPage {
    pub site_name: String,
    pub status_code: u16,
    pub title: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct UnlockPayload {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct UnlockResponse {
    pub ok: bool,
    pub redirect: String,
}

/// `GET /{slug}`
pub async fn redirect(
    State(state): State<AppState>,
    client: ClientInfo,
    Path(slug): Path<String>,
) -> Result<Response> {
    let Some(url) = urls::find_by_slug(&state.db, &slug).await? else {
        return Ok(not_found_page(&state));
    };

    match url.state() {
        UrlState::Deleted | UrlState::Disabled => Ok(not_found_page(&state)),
        UrlState::Expired => Ok(gone_page(
            &state,
            "Link expired",
            "This short link reached its expiry date and no longer redirects.",
        )),
        UrlState::LimitReached => Ok(gone_page(
            &state,
            "Link expired",
            "This short link reached its maximum number of clicks and no longer redirects.",
        )),
        UrlState::Active => {
            if url.has_password() && !is_unlocked(&state, &client, &url.slug) {
                return Ok(password_page(&state, &url.slug, None, StatusCode::OK));
            }
            spawn_click(&state, &client, &url);
            Ok(redirect_response(&url))
        }
    }
}

/// `POST /{slug}/unlock`
pub async fn unlock_link(
    State(state): State<AppState>,
    client: ClientInfo,
    Path(slug): Path<String>,
    Flexible(payload): Flexible<UnlockPayload>,
) -> Result<Response> {
    let wants_json = client.wants_json();

    state
        .limiter
        .check(
            &format!("unlock:{}:{}", slug.to_ascii_lowercase(), client.limiter_key()),
            limits::UNLOCK_CAPACITY,
            limits::UNLOCK_REFILL_PER_SEC,
        )
        .map_err(AppError::rate_limited)?;

    let Some(url) = urls::find_by_slug(&state.db, &slug).await? else {
        return Ok(not_found_page(&state));
    };
    match url.state() {
        UrlState::Deleted | UrlState::Disabled => return Ok(not_found_page(&state)),
        UrlState::Expired => {
            return Ok(gone_page(
                &state,
                "Link expired",
                "This short link reached its expiry date and no longer redirects.",
            ))
        }
        UrlState::LimitReached => {
            return Ok(gone_page(
                &state,
                "Link expired",
                "This short link reached its maximum number of clicks and no longer redirects.",
            ))
        }
        UrlState::Active => {}
    }

    // A link without a password simply redirects.
    let Some(password_hash) = url.password_hash.as_deref().filter(|hash| !hash.is_empty()) else {
        return Ok(unlocked_response(&state, &url, wants_json));
    };

    if !password::verify(&payload.password, password_hash) {
        tracing::info!(slug = %url.slug, "failed unlock attempt");
        return if wants_json {
            Ok(crate::error::ApiError(AppError::unauthorized("incorrect password")).into_response())
        } else {
            Ok(password_page(
                &state,
                &url.slug,
                Some("Incorrect password. Try again."),
                StatusCode::UNAUTHORIZED,
            ))
        };
    }

    let token = unlock::issue(&state.config.secret, &url.slug, state.config.unlock_ttl_minutes);
    let cookie = cookies::build(
        &unlock::cookie_name(&url.slug),
        &token,
        state.config.unlock_ttl_minutes * 60,
        client.secure,
    );

    let mut response = if wants_json {
        (
            StatusCode::OK,
            axum::Json(UnlockResponse { ok: true, redirect: format!("/{}", url.slug) }),
        )
            .into_response()
    } else {
        let location = format!("/{}", url.slug);
        let mut response = StatusCode::SEE_OTHER.into_response();
        if let Ok(value) = HeaderValue::from_str(&location) {
            response.headers_mut().insert(header::LOCATION, value);
        }
        response
    };

    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    Ok(response)
}

fn unlocked_response(state: &AppState, url: &Url, wants_json: bool) -> Response {
    if wants_json {
        (
            StatusCode::OK,
            axum::Json(UnlockResponse { ok: true, redirect: format!("/{}", url.slug) }),
        )
            .into_response()
    } else {
        let mut response = StatusCode::SEE_OTHER.into_response();
        if let Ok(value) = HeaderValue::from_str(&format!("/{}", url.slug)) {
            response.headers_mut().insert(header::LOCATION, value);
        }
        let _ = state;
        response
    }
}

/// Does the request carry a valid unlock cookie for this slug?
fn is_unlocked(state: &AppState, client: &ClientInfo, slug: &str) -> bool {
    let name = unlock::cookie_name(slug);
    match client.cookie(&name) {
        Some(token) => unlock::verify(&state.config.secret, slug, token),
        None => false,
    }
}

/// Fire and forget click recording.
fn spawn_click(state: &AppState, client: &ClientInfo, url: &Url) {
    if !state.settings.analytics_enabled() {
        return;
    }
    let pool = state.db.clone();
    let config = state.config.clone();
    let input = ClickInput {
        url_id: url.id,
        ip_hash: client.ip_hash.clone(),
        user_agent: client.user_agent.clone(),
        referrer: client.referrer.clone(),
        password_protected: url.has_password(),
    };
    tokio::spawn(async move {
        click::record(&pool, &config, true, input).await;
    });
}

fn redirect_response(url: &Url) -> Response {
    let status = StatusCode::from_u16(url.redirect_type as u16).unwrap_or(StatusCode::FOUND);
    let mut response = status.into_response();
    if let Ok(value) = HeaderValue::from_str(&url.target_url) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    // Permanent redirects get a short private cache; temporary ones never cache
    // so click counts stay accurate.
    let cache = if matches!(url.redirect_type, 301 | 308) {
        "private, max-age=300"
    } else {
        "private, no-store"
    };
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn password_page(
    state: &AppState,
    slug: &str,
    error: Option<&str>,
    status: StatusCode,
) -> Response {
    let page = PasswordPage {
        site_name: state.settings.site_name(),
        slug: slug.to_string(),
        error: error.map(str::to_string),
    };
    render(page, status)
}

fn not_found_page(state: &AppState) -> Response {
    let page = LinkErrorPage {
        site_name: state.settings.site_name(),
        status_code: 404,
        title: "Link not found".to_string(),
        message: "This short link does not exist, was removed, or is disabled.".to_string(),
    };
    render(page, StatusCode::NOT_FOUND)
}

fn gone_page(state: &AppState, title: &str, message: &str) -> Response {
    let page = LinkErrorPage {
        site_name: state.settings.site_name(),
        status_code: 410,
        title: title.to_string(),
        message: message.to_string(),
    };
    render(page, StatusCode::GONE)
}

fn render<T: Template>(template: T, status: StatusCode) -> Response {
    match template.render() {
        Ok(html) => {
            let mut response = (status, Html(html)).into_response();
            response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(err) => AppError::internal(format!("template error: {err}")).into_response(),
    }
}
