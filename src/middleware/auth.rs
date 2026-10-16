//! Authentication extractors.
//!
//! Two credential types are supported: the session cookie (browser) and an
//! `Authorization: Bearer uw_...` API token (scripts, integrations).

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;

use crate::db::users::User;
use crate::error::{ApiError, AppError};
use crate::services::auth as auth_service;
use crate::services::cookies;
use crate::state::AppState;

/// An authenticated caller.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub user: User,
    pub session_id: Option<i64>,
    pub token_id: Option<i64>,
}

impl CurrentUser {
    /// True when the request was authenticated with an API token.
    pub fn via_token(&self) -> bool {
        self.token_id.is_some()
    }

    pub fn is_admin(&self) -> bool {
        self.user.is_admin()
    }

    /// Reject the request unless the caller is an administrator.
    pub fn ensure_admin(&self) -> Result<(), AppError> {
        if self.is_admin() {
            Ok(())
        } else {
            Err(AppError::forbidden("administrator privileges are required"))
        }
    }
}

/// Optional authentication: `None` for anonymous requests.
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<CurrentUser>);

/// Client metadata useful for rate limiting, audit trails and analytics.
#[derive(Debug, Clone, Default)]
pub struct ClientInfo {
    /// Best effort client address, resolved exactly like the rate limiter key.
    /// Forwarded to the captcha provider as `remoteip` when present.
    pub ip: Option<std::net::IpAddr>,
    /// `Host` header of the request, checked against the `hostname` a captcha
    /// provider reports for the token it just validated.
    pub host: Option<String>,
    pub ip_hash: Option<String>,
    pub user_agent: Option<String>,
    pub referrer: Option<String>,
    pub cookies: Vec<(String, String)>,
    pub secure: bool,
    /// `true` when the request body is JSON (drives JSON vs HTML responses).
    pub json_body: bool,
    /// `true` when the client explicitly prefers JSON.
    pub accepts_json: bool,
}

impl ClientInfo {
    /// Key fragment identifying the caller for rate limiting.
    pub fn limiter_key(&self) -> String {
        self.ip_hash
            .clone()
            .unwrap_or_else(|| "unknown".to_string())
    }

    /// Value of a request cookie.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Should the response be JSON rather than HTML?
    pub fn wants_json(&self) -> bool {
        self.json_body || self.accepts_json
    }
}

/// Resolve credentials without failing: returns `None` for anonymous callers.
pub async fn authenticate(parts: &Parts, state: &AppState) -> Option<CurrentUser> {
    if let Some(token) = bearer_token(parts) {
        match auth_service::resolve_api_token(&state.db, &token).await {
            Ok(Some((record, user))) => {
                return Some(CurrentUser {
                    user,
                    session_id: None,
                    token_id: Some(record.id),
                })
            }
            Ok(None) => return None,
            Err(err) => {
                tracing::error!(error = %err, "api token lookup failed");
                return None;
            }
        }
    }

    let token = cookies::get(&parts.headers, cookies::SESSION_COOKIE)?;
    match auth_service::resolve_session(&state.db, &token, state.config.session_ttl_hours).await {
        Ok(Some((session, user))) => Some(CurrentUser {
            user,
            session_id: Some(session.id),
            token_id: None,
        }),
        Ok(None) => None,
        Err(err) => {
            tracing::error!(error = %err, "session lookup failed");
            None
        }
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    /// JSON shaped rejection: this extractor is used by the API. The HTML pages
    /// use [`MaybeUser`] and redirect to the login screen instead.
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        authenticate(parts, state).await.ok_or_else(|| {
            let error = if bearer_token(parts).is_some() {
                AppError::unauthorized("invalid or expired API token")
            } else {
                AppError::unauthorized("sign in to continue")
            };
            ApiError(error)
        })
    }
}

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(MaybeUser(authenticate(parts, state).await))
    }
}

impl FromRequestParts<AppState> for ClientInfo {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header_value = |name: axum::http::HeaderName| {
            parts
                .headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        };

        Ok(ClientInfo {
            ip: crate::middleware::client_ip_parts(
                &parts.headers,
                &parts.extensions,
                &state.config,
            ),
            host: header_value(axum::http::header::HOST),
            ip_hash: crate::middleware::hashed_client_ip_parts(
                &parts.headers,
                &parts.extensions,
                &state.config,
            ),
            user_agent: header_value(axum::http::header::USER_AGENT)
                .map(|value| value.chars().take(512).collect()),
            referrer: header_value(axum::http::header::REFERER)
                .map(|value| value.chars().take(512).collect()),
            cookies: cookies::get_all(&parts.headers),
            secure: crate::middleware::is_secure_request(&parts.headers, &state.config),
            json_body: parts
                .headers
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_ascii_lowercase().contains("json"))
                .unwrap_or(false),
            accepts_json: parts
                .headers
                .get(axum::http::header::ACCEPT)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.contains("application/json"))
                .unwrap_or(false),
        })
    }
}
