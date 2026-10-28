//! Request body extractors.
//!
//! [`Flexible`] accepts both JSON and form encoding, because the password unlock
//! endpoint is used by the browser form and by API clients for the same payload.
//!
//! [`Json`] is the extractor the JSON API uses everywhere. `axum::Json` rejects
//! an unparseable body itself, before the handler runs, and answers with a
//! plain-text message rather than this crate's error envelope — so the promise
//! the API documents, that every failure is `{"error": {...}}`, held for
//! handler-level errors only. Wrapping the extractor makes it hold for the
//! extractor's rejections too.

use axum::extract::{FromRequest, Request};
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{ApiError, AppError};

/// Body decoded as JSON or as a URL encoded form.
#[derive(Debug, Clone)]
pub struct Flexible<T>(pub T);

/// JSON body, with this crate's error envelope on rejection.
///
/// It serializes as well as extracts, so a handler can name one type for its
/// body and its response.
#[derive(Debug, Clone)]
pub struct Json<T>(pub T);

/// True when the request body is JSON.
pub fn is_json(request: &Request) -> bool {
    request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("json"))
        .unwrap_or(false)
}

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, _state: &S) -> Result<Self, Self::Rejection> {
        axum::Json::<T>::from_request(request, _state)
            .await
            .map(|axum::Json(value)| Json(value))
            .map_err(|rejection| ApiError(AppError::bad_request(rejection.body_text())))
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
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
