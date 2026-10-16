//! Request body extractor that accepts both JSON and form encoding.
//!
//! The password unlock endpoint is used by the browser form and by API
//! clients, so it must accept `application/json` and
//! `application/x-www-form-urlencoded` for the same payload type.

use axum::extract::{FromRequest, Request};
use axum::http::header::CONTENT_TYPE;
use serde::de::DeserializeOwned;

use crate::error::{ApiError, AppError};

/// Body decoded as JSON or as a URL encoded form.
#[derive(Debug, Clone)]
pub struct Flexible<T>(pub T);

/// True when the request body is JSON.
pub fn is_json(request: &Request) -> bool {
    request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("json"))
        .unwrap_or(false)
}

impl<T, S> FromRequest<S> for Flexible<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        let json = is_json(&request);
        let bytes = axum::body::to_bytes(request.into_body(), 64 * 1024)
            .await
            .map_err(|err| ApiError(AppError::bad_request(format!("cannot read body: {err}"))))?;

        let value = if json {
            serde_json::from_slice(&bytes).map_err(|err| {
                ApiError(AppError::bad_request(format!("invalid JSON body: {err}")))
            })?
        } else {
            serde_urlencoded::from_bytes(&bytes).map_err(|err| {
                ApiError(AppError::bad_request(format!("invalid form body: {err}")))
            })?
        };
        Ok(Flexible(value))
    }
}
