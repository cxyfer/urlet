//! HTTP routes.

pub mod assets;
pub mod health;

use crate::error::AppError;

/// Fallback handler: unknown paths are 404, and unknown slugs never reach it.
pub async fn not_found() -> AppError {
    AppError::NotFound
}
