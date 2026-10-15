//! HTTP routes.

pub mod api;
pub mod api_auth;
pub mod assets;
pub mod dto;
pub mod health;

use crate::error::{ApiError, AppError};

/// Fallback handler for unknown page paths.
pub async fn not_found() -> AppError {
    AppError::NotFound
}

/// Fallback handler for unknown `/api/v1` paths: JSON, not HTML.
pub async fn api_not_found() -> ApiError {
    ApiError(AppError::not_found())
}
