//! `/api/v1/domains` — active domains a signed-in user may choose.

use crate::routes::extract::Json;
use axum::extract::State;
use serde::Serialize;

use crate::error::ApiError;
use crate::middleware::auth::CurrentUser;
use crate::state::AppState;

/// One selectable domain.
#[derive(Debug, Clone, Serialize)]
pub struct DomainChoiceDto {
    pub id: i64,
    pub hostname: String,
    pub is_default: bool,
}

/// `GET /api/v1/domains`
pub async fn list(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
) -> Result<Json<Vec<DomainChoiceDto>>, ApiError> {
    let policy = crate::services::domain_policy::effective(&state.db, &user).await?;
    let items = policy
        .options(&state.domains)
        .into_iter()
        .map(|domain| DomainChoiceDto {
            id: domain.id,
            hostname: domain.hostname,
            is_default: domain.is_default,
        })
        .collect();
    Ok(Json(items))
}
