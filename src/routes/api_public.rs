//! Public, unauthenticated metadata endpoint.

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;

use crate::db::urls::{self, UrlState};
use crate::error::{ApiError, AppError};
use crate::state::AppState;

/// Metadata about a short link that is safe to expose without authentication.
///
/// The destination is deliberately absent: it is only revealed by following the
/// link (and by unlocking it when a password is set).
#[derive(Debug, Clone, Serialize)]
pub struct PublicUrlDto {
    pub slug: String,
    pub title: Option<String>,
    pub has_password: bool,
    pub is_expired: bool,
    pub state: String,
}

/// `GET /api/v1/public/urls/{slug}`
pub async fn show(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<PublicUrlDto>, ApiError> {
    let Some(url) = urls::find_by_slug(&state.db, &slug).await? else {
        return Err(ApiError(AppError::not_found()));
    };

    let url_state = url.state();
    if matches!(url_state, UrlState::Deleted | UrlState::Disabled) {
        return Err(ApiError(AppError::not_found()));
    }

    let has_password = url.has_password();
    Ok(Json(PublicUrlDto {
        slug: url.slug,
        title: url.title,
        has_password,
        is_expired: matches!(url_state, UrlState::Expired | UrlState::LimitReached),
        state: url_state.as_str().to_string(),
    }))
}
