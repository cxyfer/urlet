//! `/api/v1/tokens` — API token management.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::tokens::ApiToken;
use crate::error::ApiError;
use crate::middleware::auth::{ClientInfo, CurrentUser};
use crate::services::{time, tokens};
use crate::state::AppState;

/// Public representation of a token (never the secret).
#[derive(Debug, Clone, Serialize)]
pub struct ApiTokenDto {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub scopes: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: Option<String>,
    pub revoked: bool,
    pub active: bool,
}

impl ApiTokenDto {
    fn new(token: &ApiToken, now: &str) -> Self {
        Self {
            id: token.id,
            name: token.name.clone(),
            prefix: token.token_prefix.clone(),
            scopes: token.scopes.clone(),
            created_at: token.created_at.clone(),
            last_used_at: token.last_used_at.clone(),
            expires_at: token.expires_at.clone(),
            revoked: token.revoked_at.is_some(),
            active: token.is_valid_at(now),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TokenList {
    pub items: Vec<ApiTokenDto>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTokenPayload {
    pub name: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreatedTokenResponse {
    /// Plain token value — returned exactly once.
    pub token: String,
    pub api_token: ApiTokenDto,
}

/// `GET /api/v1/tokens`
pub async fn list(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
) -> Result<Json<TokenList>, ApiError> {
    let now = time::now_iso();
    let rows = tokens::list(&state.db, user.id).await?;
    Ok(Json(TokenList {
        items: rows.iter().map(|row| ApiTokenDto::new(row, &now)).collect(),
    }))
}

/// `POST /api/v1/tokens`
pub async fn create(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Json(payload): Json<CreateTokenPayload>,
) -> Result<Response, ApiError> {
    let created = tokens::create(
        &state.db,
        &user,
        &payload.name,
        payload.expires_at.as_deref(),
        client.ip_hash.as_deref(),
    )
    .await?;

    let body = CreatedTokenResponse {
        token: created.plain,
        api_token: ApiTokenDto::new(&created.record, &time::now_iso()),
    };
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

/// `DELETE /api/v1/tokens/{id}`
pub async fn revoke(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    tokens::revoke(&state.db, &user, id, client.ip_hash.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Used by the admin API to render someone else's token list.
pub fn dto_for(token: &ApiToken, now: &str) -> ApiTokenDto {
    ApiTokenDto::new(token, now)
}
