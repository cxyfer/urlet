//! HTTP routes.

pub mod api;
pub mod api_admin;
pub mod api_auth;
pub mod api_domains;
pub mod api_invites;
pub mod api_public;
pub mod api_stats;
pub mod api_tags;
pub mod api_tokens;
pub mod api_urls;
pub mod assets;
pub mod dto;
pub mod extract;
pub mod health;
pub mod pages;
pub mod redirect;

use crate::error::{ApiError, AppError};
use crate::middleware::auth::ClientInfo;
use crate::services::captcha::{CaptchaFlow, CaptchaGuard};
use crate::state::AppState;

/// Fallback handler for unknown page paths.
pub async fn not_found() -> AppError {
    AppError::NotFound
}

/// Fallback handler for unknown `/api/v1` paths: JSON, not HTML.
pub async fn api_not_found() -> ApiError {
    ApiError(AppError::not_found())
}

/// Reject a JSON API request when `flow` is guarded and the token does not
/// verify. An unguarded flow never contacts the provider, so the API contract is
/// unchanged on instances that do not use bot protection.
pub async fn verify_captcha(
    state: &AppState,
    client: &ClientInfo,
    flow: CaptchaFlow,
    token: Option<&str>,
) -> Result<(), ApiError> {
    let guard = CaptchaGuard::from_settings(
        &state.settings,
        flow,
        state.config.captcha_verify_url.as_deref(),
        &state.config.base_host,
    );
    if !guard.enabled() {
        return Ok(());
    }
    match guard.verify(token, client.ip).await {
        Ok(()) => Ok(()),
        Err(err) => {
            // The token itself is never logged.
            tracing::debug!(reason = err.code(), "api captcha check rejected a request");
            Err(ApiError(AppError::captcha_failed(
                crate::services::captcha::FAILURE_MESSAGE,
            )))
        }
    }
}
