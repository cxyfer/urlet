//! `/api/v1/domains` — active domains a signed-in user may choose.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::state::AppState;

/// One selectable domain.
#[derive(Debug, Clone, Serialize)]
pub struct DomainChoiceDto {
    pub hostname: String,
    pub is_default: bool,
}

/// `GET /api/v1/domains`
pub async fn list(
    State(state): State<AppState>,
    CurrentUser { .. }: CurrentUser,
) -> Result<Json<Vec<DomainChoiceDto>>, ApiError> {
    let items = state
        .domains
        .active_options()
        .into_iter()
        .map(|domain| DomainChoiceDto {
            hostname: domain.hostname,
            is_default: domain.is_default,
        })
        .collect();
    Ok(Json(items))
}
