//! `/api/v1/auth/**` — registration, login, sessions, profile.

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::db::{sessions, users};
use crate::error::{ApiError, AppError};
use crate::middleware::auth::{ClientInfo, CurrentUser, MaybeUser};
use crate::routes::dto::{SessionDto, UserDto};
use crate::services::audit::{action, target, Audit};
use crate::services::auth;
use crate::services::auth::{AuthContext, SessionToken};
use crate::services::cookies;
use crate::services::ratelimit::limits;
use crate::services::time;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterPayload {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub invite_code: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProfilePayload {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordPayload {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct UserEnvelope {
    pub user: UserDto,
}

/// Envelope for the session list, matching the other list endpoints.
#[derive(Debug, Serialize)]
pub struct SessionList {
    pub items: Vec<SessionDto>,
}

/// `POST /api/v1/auth/register`
pub async fn register(
    State(state): State<AppState>,
    client: ClientInfo,
    Json(payload): Json<RegisterPayload>,
) -> Result<Response, ApiError> {
    enforce(
        &state,
        &format!("register:{}", client.limiter_key()),
        limits::REGISTER_CAPACITY,
        limits::REGISTER_REFILL_PER_SEC,
    )?;

    let context = context(&state, &client);
    let user = context
        .register(
            &payload.email,
            &payload.password,
            payload.invite_code.as_deref(),
            payload.display_name.as_deref(),
        )
        .await?;

    // Registering signs you in: one less round trip for the web UI.
    let token = context.open_session(user.id).await?;
    let mut response = (
        StatusCode::CREATED,
        Json(UserEnvelope {
            user: UserDto::from(&user),
        }),
    )
        .into_response();
    attach_session_cookie(&mut response, &token, client.secure);
    Ok(response)
}

/// `POST /api/v1/auth/login`
pub async fn login(
    State(state): State<AppState>,
    client: ClientInfo,
    Json(payload): Json<LoginPayload>,
) -> Result<Response, ApiError> {
    // An IP-wide bucket first: it caps how much Argon2 work one host can ask
    // for, even when it rotates the email address on every attempt.
    enforce(
        &state,
        &format!("login-ip:{}", client.limiter_key()),
        limits::LOGIN_IP_CAPACITY,
        limits::LOGIN_IP_REFILL_PER_SEC,
    )?;

    let email_normalized = auth::normalize_email(&payload.email);
    enforce(
        &state,
        &format!("login:{}:{}", client.limiter_key(), email_normalized),
        limits::LOGIN_CAPACITY,
        limits::LOGIN_REFILL_PER_SEC,
    )?;

    let context = context(&state, &client);
    let (user, token) = context.login(&payload.email, &payload.password).await?;

    let mut response = (
        StatusCode::OK,
        Json(UserEnvelope {
            user: UserDto::from(&user),
        }),
    )
        .into_response();
    attach_session_cookie(&mut response, &token, client.secure);
    Ok(response)
}

/// `POST /api/v1/auth/logout`
pub async fn logout(
    State(state): State<AppState>,
    client: ClientInfo,
    MaybeUser(current): MaybeUser,
) -> Result<Response, ApiError> {
    if let Some(current) = current {
        if let Some(session_id) = current.session_id {
            context(&state, &client)
                .logout(session_id, &current.user)
                .await?;
        }
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    clear_session_cookie(&mut response, client.secure);
    Ok(response)
}

/// `GET /api/v1/auth/me`
pub async fn me(CurrentUser { user, .. }: CurrentUser) -> Json<UserEnvelope> {
    Json(UserEnvelope {
        user: UserDto::from(&user),
    })
}

/// `PATCH /api/v1/auth/me`
pub async fn update_me(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Json(payload): Json<UpdateProfilePayload>,
) -> Result<Json<UserEnvelope>, ApiError> {
    if let Some(name) = payload.display_name.as_deref() {
        auth::validate_display_name(name)?;
    }
    let display_name = payload
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());

    users::set_display_name(&state.db, user.id, display_name).await?;
    Audit::new(action::PROFILE_UPDATED)
        .actor(user.id, &user.role)
        .target(target::USER, user.id)
        .ip_hash(client.ip_hash)
        .record(&state.db)
        .await;

    let updated = users::find_by_id(&state.db, user.id)
        .await?
        .ok_or_else(|| ApiError(AppError::not_found()))?;
    Ok(Json(UserEnvelope {
        user: UserDto::from(&updated),
    }))
}

/// `POST /api/v1/auth/password`
pub async fn change_password(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser {
        user, session_id, ..
    }: CurrentUser,
    Json(payload): Json<ChangePasswordPayload>,
) -> Result<Response, ApiError> {
    enforce(
        &state,
        &format!("password:{}", user.id),
        limits::PASSWORD_CAPACITY,
        limits::PASSWORD_REFILL_PER_SEC,
    )?;
    context(&state, &client)
        .change_password(
            &user,
            &payload.current_password,
            &payload.new_password,
            session_id,
        )
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /api/v1/auth/sessions`
pub async fn list_sessions(
    State(state): State<AppState>,
    CurrentUser {
        user, session_id, ..
    }: CurrentUser,
) -> Result<Json<SessionList>, ApiError> {
    let rows = sessions::list_for_user(&state.db, user.id).await?;
    Ok(Json(SessionList {
        items: rows
            .iter()
            .map(|row| SessionDto::new(row, session_id))
            .collect(),
    }))
}

/// `DELETE /api/v1/auth/sessions/{id}`
pub async fn revoke_session(
    State(state): State<AppState>,
    client: ClientInfo,
    CurrentUser { user, .. }: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let revoked = sessions::revoke_owned(&state.db, id, user.id, &time::now_iso()).await?;
    if !revoked {
        return Err(ApiError(AppError::not_found()));
    }
    Audit::new(action::AUTH_SESSION_REVOKED)
        .actor(user.id, &user.role)
        .target(target::USER, user.id)
        .metadata(serde_json::json!({ "session_id": id }))
        .ip_hash(client.ip_hash)
        .record(&state.db)
        .await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn context<'a>(state: &'a AppState, client: &'a ClientInfo) -> AuthContext<'a> {
    AuthContext {
        pool: &state.db,
        config: &state.config,
        settings: &state.settings,
        user_agent: client.user_agent.as_deref(),
        ip_hash: client.ip_hash.as_deref(),
    }
}

fn enforce(state: &AppState, key: &str, capacity: f64, refill: f64) -> Result<(), ApiError> {
    state
        .limiter
        .check(key, capacity, refill)
        .map_err(AppError::rate_limited)
        .map_err(ApiError)
}

/// Append the session cookie to a response.
pub fn attach_session_cookie(response: &mut Response, token: &SessionToken, secure: bool) {
    let value = cookies::build(
        cookies::SESSION_COOKIE,
        &token.token,
        token.max_age_secs,
        secure,
    );
    if let Ok(header_value) = HeaderValue::from_str(&value) {
        response
            .headers_mut()
            .append(header::SET_COOKIE, header_value);
    }
}

/// Append a cookie that clears the session.
pub fn clear_session_cookie(response: &mut Response, secure: bool) {
    let value = cookies::clear(cookies::SESSION_COOKIE, secure);
    if let Ok(header_value) = HeaderValue::from_str(&value) {
        response
            .headers_mut()
            .append(header::SET_COOKIE, header_value);
    }
}

/// Build the raw `Set-Cookie` value for a session (used by HTML form logins).
pub fn session_cookie_value(token: &SessionToken, secure: bool) -> String {
    cookies::build(
        cookies::SESSION_COOKIE,
        &token.token,
        token.max_age_secs,
        secure,
    )
}
