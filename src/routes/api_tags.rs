//! `/api/v1/tags` — tag listing and cleanup for the authenticated user.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use crate::db::tags;
use crate::error::{ApiError, AppError};
use crate::middleware::auth::CurrentUser;
use crate::routes::dto::TagDto;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct TagList {
    pub items: Vec<TagDto>,
}

/// `GET /api/v1/tags`
pub async fn list(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
) -> Result<Json<TagList>, ApiError> {
    let rows = tags::list_for_user(&state.db, user.id).await?;
    Ok(Json(TagList {
        items: rows.iter().map(TagDto::from).collect(),
    }))
}

/// `DELETE /api/v1/tags/{id}`
pub async fn delete(
    State(state): State<AppState>,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let deleted = tags::delete(&state.db, id, user.id).await?;
    if !deleted {
        return Err(ApiError(AppError::not_found()));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
