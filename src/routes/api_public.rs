//! Public, unauthenticated metadata endpoint.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::domains;
use crate::db::urls::{self, UrlState};
use crate::error::{ApiError, AppError};
use crate::services::domains as domain_service;
use crate::state::AppState;

/// Optional `?domain=<hostname>` selector.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PublicQuery {
    #[serde(default)]
    pub domain: Option<String>,
}

/// Metadata about a short link that is safe to expose without authentication.
///
/// The destination is deliberately absent: it is only revealed by following the
/// link (and by unlocking it when a password is set).
#[derive(Debug, Clone, Serialize)]
pub struct PublicUrlDto {
    pub domain: String,
    pub slug: String,
    pub title: Option<String>,
    pub has_password: bool,
    pub is_expired: bool,
    pub state: String,
}

/// `GET /api/v1/public/urls/{slug}?domain=<hostname>`
///
/// Without `domain` the default domain is used, which keeps callers that
/// predate multi-domain working for every link created on the default domain.
pub async fn show(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<PublicQuery>,
) -> Result<Json<PublicUrlDto>, ApiError> {
    let domain = match query
        .domain
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(raw) => {
            let hostname = domain_service::validate_domain_hostname(raw)
                .map_err(|err| ApiError(AppError::validation("domain", err.message())))?;
            domains::find_by_hostname(&state.db, &hostname)
                .await?
                .filter(|domain| domain.is_active())
                .ok_or_else(|| ApiError(AppError::not_found()))?
        }
        None => domains::active_default(&state.db)
            .await?
            .ok_or_else(|| ApiError(AppError::not_found()))?,
    };

    let Some(url) = urls::find_by_domain_slug(&state.db, domain.id, &slug).await? else {
        return Err(ApiError(AppError::not_found()));
    };

    let url_state = url.state();
    if matches!(url_state, UrlState::Deleted | UrlState::Disabled) {
        return Err(ApiError(AppError::not_found()));
    }

    let has_password = url.has_password();
    Ok(Json(PublicUrlDto {
        domain: domain.hostname,
        slug: url.slug,
        title: url.title,
        has_password,
        is_expired: matches!(url_state, UrlState::Expired | UrlState::LimitReached),
        state: url_state.as_str().to_string(),
    }))
}
