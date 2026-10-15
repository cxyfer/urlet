//! Unified error type.
//!
//! `AppError` renders an HTML error page (used by the browser facing routes),
//! while `ApiError` wraps the same value and renders the JSON error envelope
//! used by `/api/v1/**`.

use askama::Template;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use serde::Serialize;

pub type Result<T> = std::result::Result<T, AppError>;

/// Errors that can be returned from any handler.
#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Validation { field: &'static str, message: String },
    Unauthorized(String),
    Forbidden(String),
    NotFound,
    Conflict(String),
    Gone(String),
    RateLimited { retry_after_secs: u64 },
    Internal(String),
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn validation(field: &'static str, message: impl Into<String>) -> Self {
        Self::Validation { field, message: message.into() }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::Forbidden(message.into())
    }

    pub fn not_found() -> Self {
        Self::NotFound
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn gone(message: impl Into<String>) -> Self {
        Self::Gone(message.into())
    }

    pub fn rate_limited(retry_after_secs: u64) -> Self {
        Self::RateLimited { retry_after_secs }
    }

    /// Wrap an internal failure. The detail is logged, never returned to clients.
    pub fn internal(error: impl std::fmt::Display) -> Self {
        let detail = error.to_string();
        tracing::error!(error = %detail, "internal error");
        Self::Internal(detail)
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Validation { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Gone(_) => StatusCode::GONE,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Validation { .. } => "validation_error",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Gone(_) => "gone",
            Self::RateLimited { .. } => "rate_limited",
            Self::Internal(_) => "internal_error",
        }
    }

    /// Message that is safe to show to the caller.
    pub fn message(&self) -> String {
        match self {
            Self::BadRequest(m) | Self::Unauthorized(m) | Self::Forbidden(m) => m.clone(),
            Self::Conflict(m) | Self::Gone(m) => m.clone(),
            Self::Validation { message, .. } => message.clone(),
            Self::NotFound => "The requested resource does not exist.".to_string(),
            Self::RateLimited { retry_after_secs } => {
                format!("Too many requests. Try again in {retry_after_secs} seconds.")
            }
            Self::Internal(_) => "Something went wrong on our side.".to_string(),
        }
    }

    fn title(&self) -> &'static str {
        match self.status() {
            StatusCode::BAD_REQUEST => "Bad request",
            StatusCode::UNPROCESSABLE_ENTITY => "Invalid input",
            StatusCode::UNAUTHORIZED => "Sign in required",
            StatusCode::FORBIDDEN => "Not allowed",
            StatusCode::NOT_FOUND => "Not found",
            StatusCode::CONFLICT => "Conflict",
            StatusCode::GONE => "Link expired",
            StatusCode::TOO_MANY_REQUESTS => "Slow down",
            _ => "Server error",
        }
    }

    fn details(&self) -> Option<serde_json::Value> {
        match self {
            Self::Validation { field, .. } => Some(serde_json::json!({ "field": field })),
            _ => None,
        }
    }
}

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
    pub status_code: u16,
    pub title: String,
    pub message: String,
    pub site_name: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let mut response = if wants_html() {
            let template = ErrorTemplate {
                status_code: status.as_u16(),
                title: self.title().to_string(),
                message: self.message(),
                site_name: "uwurls".to_string(),
            };
            match template.render() {
                Ok(html) => (status, Html(html)).into_response(),
                Err(err) => {
                    tracing::error!(error = %err, "failed to render error page");
                    (status, self.message()).into_response()
                }
            }
        } else {
            (status, self.message()).into_response()
        };
        if let AppError::RateLimited { retry_after_secs } = &self {
            if let Ok(value) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }
        response
    }
}

/// Error pages are rendered as HTML for browser requests. `ApiError` wraps the
/// same value for JSON callers, so this always renders the HTML variant.
fn wants_html() -> bool {
    true
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Internal(detail) => write!(f, "internal error: {detail}"),
            other => write!(f, "{}: {}", other.code(), other.message()),
        }
    }
}

impl std::error::Error for AppError {}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        Self::internal(format!("database error: {error}"))
    }
}

impl From<askama::Error> for AppError {
    fn from(error: askama::Error) -> Self {
        Self::internal(format!("template error: {error}"))
    }
}

impl From<axum::extract::rejection::FormRejection> for AppError {
    fn from(rejection: axum::extract::rejection::FormRejection) -> Self {
        Self::bad_request(format!("invalid form submission: {rejection}"))
    }
}

impl From<axum::extract::rejection::JsonRejection> for AppError {
    fn from(rejection: axum::extract::rejection::JsonRejection) -> Self {
        Self::bad_request(format!("invalid JSON body: {rejection}"))
    }
}

/// JSON error envelope: `{"error": {"code": ..., "message": ..., "details": ...}}`.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Wrapper that renders an [`AppError`] as a JSON response.
#[derive(Debug)]
pub struct ApiError(pub AppError);

impl ApiError {
    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self(AppError::internal(error))
    }
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        Self(error)
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self(AppError::from(error))
    }
}

impl From<askama::Error> for ApiError {
    fn from(error: askama::Error) -> Self {
        Self(AppError::from(error))
    }
}

impl From<axum::extract::rejection::FormRejection> for ApiError {
    fn from(rejection: axum::extract::rejection::FormRejection) -> Self {
        Self(AppError::from(rejection))
    }
}

impl From<axum::extract::rejection::JsonRejection> for ApiError {
    fn from(rejection: axum::extract::rejection::JsonRejection) -> Self {
        Self(AppError::from(rejection))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.0.status();
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.0.code(),
                message: self.0.message(),
                details: self.0.details(),
            },
        };
        let mut response = (status, axum::Json(body)).into_response();
        if let AppError::RateLimited { retry_after_secs } = &self.0 {
            if let Ok(value) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_variants_to_status_codes() {
        assert_eq!(AppError::not_found().status(), StatusCode::NOT_FOUND);
        assert_eq!(AppError::gone("expired").status(), StatusCode::GONE);
        assert_eq!(AppError::rate_limited(5).status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(AppError::validation("slug", "bad").status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn internal_errors_do_not_leak_details() {
        let err = AppError::internal("connection refused to /var/run/db");
        assert_eq!(err.message(), "Something went wrong on our side.");
        assert_eq!(err.code(), "internal_error");
    }

    #[test]
    fn validation_errors_expose_field() {
        let err = AppError::validation("target_url", "must be absolute");
        let details = err.details().expect("details present");
        assert_eq!(details["field"], "target_url");
    }
}
